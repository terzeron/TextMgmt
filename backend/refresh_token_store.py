import logging
import os
import sqlite3
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterator

LOGGER = logging.getLogger(__name__)

# 멀티탭/멀티기기 환경에서 여러 요청이 같은 refresh token 으로 거의 동시에 회전을
# 시도하면, 먼저 처리된 요청이 토큰을 회전(replaced_by 설정)시킨 직후 나머지 요청이
# 이미 회전된 토큰을 제출하게 된다. 이를 재사용 공격으로 오판해 패밀리를 폐기하면
# 세션이 통째로 풀린다. 회전 직후 이 grace window 안의 재제출은 정상 동시 요청으로
# 보고 패밀리를 폐기하지 않으며, window 를 벗어난 재제출만 진짜 재사용으로 차단한다.
DEFAULT_ROTATION_GRACE_SECONDS = 30


def _rotation_grace_seconds() -> int:
    try:
        return int(os.getenv("TM_REFRESH_ROTATION_GRACE_SECONDS", str(DEFAULT_ROTATION_GRACE_SECONDS)))
    except ValueError:
        return DEFAULT_ROTATION_GRACE_SECONDS


def _is_within_rotation_grace(revoke_reason: Any, revoked_at: Any, now: int) -> bool:
    """방금 정상 회전(reason='rotated')되었고 grace window 안이면 True."""
    return revoke_reason == "rotated" and revoked_at is not None and (now - int(revoked_at)) <= _rotation_grace_seconds()


ADMIN_REVOKE_REASON = "admin-revoked"
# 관리자 화면에 family_id 전체를 노출하지 않기 위한 표시용 접두사 길이.
SESSION_LABEL_LENGTH = 8
# 세션 목록은 family 단위로 집계되지만 만료 row 정리(purge)가 없으므로 상한을 둔다.
MAX_SESSION_GROUPS = 2000

# family 단위 집계. `{p}` 는 store 별 placeholder(?, %s)로 치환한다.
_SESSION_AGGREGATE_SQL = """
    SELECT family_id,
           email,
           MIN(issued_at) AS created_at,
           MAX(issued_at) AS last_seen_at,
           MAX(expires_at) AS max_expires_at,
           COUNT(*) AS token_count,
           SUM(CASE WHEN revoked_at IS NULL AND expires_at >= {p} THEN 1 ELSE 0 END) AS valid_token_count,
           MAX(CASE WHEN revoked_at IS NULL AND expires_at >= {p} THEN expires_at ELSE 0 END) AS active_expires_at,
           MAX(CASE WHEN revoke_reason IS NOT NULL AND revoke_reason <> 'rotated' THEN revoke_reason END) AS terminal_reason,
           MAX(revoked_at) AS revoked_at
    FROM refresh_tokens
    {where}
    GROUP BY family_id, email
    ORDER BY MAX(issued_at) DESC
    LIMIT {limit}
"""


def _session_aggregate_sql(placeholder: str, *, by_email: bool, by_family: bool) -> str:
    conditions = []
    if by_email:
        conditions.append(f"email = {placeholder}")
    if by_family:
        conditions.append(f"family_id = {placeholder}")
    where = f"WHERE {' AND '.join(conditions)}" if conditions else ""
    return _SESSION_AGGREGATE_SQL.format(p=placeholder, where=where, limit=MAX_SESSION_GROUPS)


def _derive_session(row: Any) -> dict[str, Any]:
    """family 집계 row 를 관리자 화면용 세션 표현으로 바꾼다.

    상태 판정은 SQLite/MySQL 양쪽이 동일해야 하므로 SQL 이 아니라 여기서 한다.
    - active: 아직 살아 있는 토큰이 하나라도 있다.
    - revoked: 살아 있는 토큰이 없고 종료 사유('rotated' 이외)가 있다.
    - expired: 살아 있는 토큰이 없고 종료 사유도 없다(자연 만료).
    """
    valid_token_count = int(row["valid_token_count"] or 0)
    terminal_reason = row["terminal_reason"]
    if valid_token_count > 0:
        status = "active"
    elif terminal_reason:
        status = "revoked"
    else:
        status = "expired"
    active_expires_at = int(row["active_expires_at"] or 0)
    family_id = row["family_id"]
    return {
        "session_id": family_id,
        "session_label": f"{family_id[:SESSION_LABEL_LENGTH]}...",
        "email": row["email"],
        "status": status,
        "created_at": int(row["created_at"]),
        "last_seen_at": int(row["last_seen_at"]),
        # 활성 세션은 살아 있는 토큰의 만료를, 그 외에는 마지막 토큰의 만료를 보여준다.
        "expires_at": active_expires_at if status == "active" and active_expires_at else int(row["max_expires_at"]),
        "revoked_at": int(row["revoked_at"]) if status != "active" and row["revoked_at"] is not None else None,
        "revoke_reason": terminal_reason if status == "revoked" else None,
        "token_count": int(row["token_count"]),
        "valid_token_count": valid_token_count,
        "is_current": False,
    }


def _build_session_page(rows: list[Any], *, status: str, page: int, page_size: int, current_family_id: str | None) -> dict[str, Any]:
    """집계 row 들을 상태 요약·필터·페이징이 적용된 응답으로 만든다."""
    sessions = [_derive_session(row) for row in rows]
    summary = {"active": 0, "expired": 0, "revoked": 0, "total": len(sessions)}
    for session in sessions:
        summary[session["status"]] += 1
        if current_family_id and session["session_id"] == current_family_id:
            session["is_current"] = True

    filtered = sessions if status == "all" else [s for s in sessions if s["status"] == status]
    filtered.sort(key=lambda s: s["last_seen_at"], reverse=True)
    total_items = len(filtered)
    start = (page - 1) * page_size
    return {"items": filtered[start : start + page_size], "pagination": {"page": page, "pageSize": page_size, "totalItems": total_items, "totalPages": max(1, (total_items + page_size - 1) // page_size)}, "summary": summary}


class RefreshTokenStore:
    """Persist refresh token state to support rotation and revocation."""

    def __init__(self, db_path: str | None = None) -> None:
        default_path = Path(tempfile.gettempdir()) / "tm_refresh_tokens.sqlite3"
        self.db_path = Path(db_path or os.getenv("TM_REFRESH_TOKEN_DB", str(default_path)))
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._init_db()

    def _connect(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.db_path, timeout=30, isolation_level=None)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA foreign_keys=ON")
        return conn

    def _init_db(self) -> None:
        with self._connect() as conn:
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS refresh_tokens (
                    jti TEXT PRIMARY KEY,
                    family_id TEXT NOT NULL,
                    email TEXT NOT NULL,
                    issued_at INTEGER NOT NULL,
                    expires_at INTEGER NOT NULL,
                    replaced_by TEXT,
                    revoked_at INTEGER,
                    revoke_reason TEXT
                )
                """
            )
            conn.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_refresh_tokens_family
                ON refresh_tokens (family_id)
                """
            )

    def store_issued(self, *, token_id: str, family_id: str, email: str, issued_at: int, expires_at: int) -> None:
        with self._connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO refresh_tokens
                    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason)
                VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL)
                """,
                (token_id, family_id, email, issued_at, expires_at),
            )

    def rotate(self, *, current_token_id: str, new_token_id: str, family_id: str, email: str, issued_at: int, expires_at: int) -> str:
        now = int(time.time())
        with self._connect() as conn:
            conn.execute("BEGIN IMMEDIATE")
            row = conn.execute(
                """
                SELECT family_id, email, expires_at, replaced_by, revoked_at, revoke_reason
                FROM refresh_tokens
                WHERE jti = ?
                """,
                (current_token_id,),
            ).fetchone()

            if row is None:
                conn.execute("ROLLBACK")
                return "missing"
            if row["family_id"] != family_id or row["email"] != email:
                conn.execute("ROLLBACK")
                return "mismatch"
            if row["expires_at"] < now:
                conn.execute("ROLLBACK")
                return "expired"
            if row["replaced_by"] is not None:
                if _is_within_rotation_grace(row["revoke_reason"], row["revoked_at"], now):
                    # grace window 내 동시 요청: 패밀리를 살린 채 후속 토큰만 재발급
                    conn.execute(
                        """
                        INSERT INTO refresh_tokens
                            (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason)
                        VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL)
                        """,
                        (new_token_id, family_id, email, issued_at, expires_at),
                    )
                    conn.execute("COMMIT")
                    return "ok"
                self._revoke_family(conn, family_id=family_id, reason="reuse-detected", revoked_at=now)
                conn.execute("COMMIT")
                return "reused"
            if row["revoked_at"] is not None:
                self._revoke_family(conn, family_id=family_id, reason="revoked-token-reuse", revoked_at=now)
                conn.execute("COMMIT")
                return "revoked"

            conn.execute(
                """
                INSERT INTO refresh_tokens
                    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason)
                VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL)
                """,
                (new_token_id, family_id, email, issued_at, expires_at),
            )
            conn.execute(
                """
                UPDATE refresh_tokens
                SET replaced_by = ?, revoked_at = ?, revoke_reason = ?
                WHERE jti = ?
                """,
                (new_token_id, issued_at, "rotated", current_token_id),
            )
            conn.execute("COMMIT")
            return "ok"

    def get_token_observation(self, token_id: str) -> dict[str, Any] | None:
        """관측 로깅 전용 읽기 조회. 토큰 상태 판정이나 인가에 사용하지 않는다."""
        with self._connect() as conn:
            row = conn.execute("SELECT replaced_by, revoked_at, revoke_reason FROM refresh_tokens WHERE jti = ?", (token_id,)).fetchone()
        if row is None:
            return None
        return {"replaced_by_present": row["replaced_by"] is not None, "revoked_at": row["revoked_at"], "revoke_reason": row["revoke_reason"]}

    def _session_rows(self, *, email: str | None = None, family_id: str | None = None) -> list[Any]:
        now = int(time.time())
        sql = _session_aggregate_sql("?", by_email=email is not None, by_family=family_id is not None)
        # SQL 안에서 `now` placeholder 는 두 CASE 식에 각각 한 번씩 쓰인다.
        params: list[Any] = [now, now]
        if email is not None:
            params.append(email)
        if family_id is not None:
            params.append(family_id)
        with self._connect() as conn:
            rows = conn.execute(sql, tuple(params)).fetchall()
        if len(rows) >= MAX_SESSION_GROUPS:
            LOGGER.warning("세션 목록이 상한(%d)에 도달해 잘렸습니다. 오래된 refresh token row 정리가 필요합니다.", MAX_SESSION_GROUPS)
        return list(rows)

    def list_sessions(self, *, status: str = "active", page: int = 1, page_size: int = 50, email: str | None = None, current_family_id: str | None = None) -> dict[str, Any]:
        rows = self._session_rows(email=email)
        return _build_session_page(rows, status=status, page=page, page_size=page_size, current_family_id=current_family_id)

    def revoke_session(self, *, family_id: str, reason: str = ADMIN_REVOKE_REASON) -> dict[str, Any]:
        """관리자 요청으로 family 하나를 폐기한다. 존재하지 않으면 found=False."""
        before = self._session_rows(family_id=family_id)
        if not before:
            return {"found": False, "revoked": False, "session": None}
        had_active = int(before[0]["valid_token_count"] or 0) > 0
        self.revoke_family(family_id, reason=reason)
        after = self._session_rows(family_id=family_id)
        return {"found": True, "revoked": had_active, "session": _derive_session(after[0]) if after else None}

    def revoke_family(self, family_id: str, reason: str = "manual") -> None:
        now = int(time.time())
        with self._connect() as conn:
            conn.execute("BEGIN IMMEDIATE")
            self._revoke_family(conn, family_id=family_id, reason=reason, revoked_at=now)
            conn.execute("COMMIT")

    def _revoke_family(self, conn: sqlite3.Connection, *, family_id: str, reason: str, revoked_at: int) -> None:
        conn.execute(
            """
            UPDATE refresh_tokens
            SET revoked_at = COALESCE(revoked_at, ?),
                revoke_reason = CASE
                    WHEN revoke_reason IS NULL OR revoke_reason = 'rotated' THEN ?
                    ELSE revoke_reason
                END
            WHERE family_id = ?
            """,
            (revoked_at, reason, family_id),
        )


class MySQLRefreshTokenStore:
    """MySQL 백엔드 refresh token 저장소.

    여러 replica/worker 가 같은 테이블을 공유하므로 SQLite(/tmp, pod 별) 방식과 달리
    토큰 회전·재사용 탐지가 전 인스턴스에서 일관되게 동작한다. 회전은 InnoDB 트랜잭션 +
    SELECT ... FOR UPDATE 행 잠금으로 원자성을 보장한다.
    """

    def __init__(self) -> None:
        self.host = os.environ.get("TM_MYSQL_HOST", "localhost")
        self.port = int(os.environ.get("TM_MYSQL_PORT", "3306"))
        self.database = os.environ.get("TM_MYSQL_DATABASE", "textmanager")
        self.user = os.environ.get("TM_MYSQL_USER", "textmanager")
        self.password = os.environ.get("TM_MYSQL_PASSWORD", "")
        self._init_db()

    @contextmanager
    def _connect(self) -> "Iterator[Any]":
        import pymysql
        from pymysql.cursors import DictCursor

        conn = pymysql.connect(host=self.host, port=self.port, database=self.database, user=self.user, password=self.password, charset="utf8mb4", cursorclass=DictCursor, autocommit=False)
        try:
            yield conn
        finally:
            conn.close()

    def _init_db(self) -> None:
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    CREATE TABLE IF NOT EXISTS refresh_tokens (
                        jti VARCHAR(64) PRIMARY KEY,
                        family_id VARCHAR(64) NOT NULL,
                        email VARCHAR(320) NOT NULL,
                        issued_at BIGINT NOT NULL,
                        expires_at BIGINT NOT NULL,
                        replaced_by VARCHAR(64),
                        revoked_at BIGINT,
                        revoke_reason VARCHAR(64),
                        INDEX idx_refresh_tokens_family (family_id)
                    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
                    """
                )
            conn.commit()

    def store_issued(self, *, token_id: str, family_id: str, email: str, issued_at: int, expires_at: int) -> None:
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO refresh_tokens
                        (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason)
                    VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL)
                    ON DUPLICATE KEY UPDATE
                        family_id = VALUES(family_id), email = VALUES(email),
                        issued_at = VALUES(issued_at), expires_at = VALUES(expires_at),
                        replaced_by = NULL, revoked_at = NULL, revoke_reason = NULL
                    """,
                    (token_id, family_id, email, issued_at, expires_at),
                )
            conn.commit()

    def rotate(self, *, current_token_id: str, new_token_id: str, family_id: str, email: str, issued_at: int, expires_at: int) -> str:
        now = int(time.time())
        with self._connect() as conn:
            try:
                conn.begin()
                with conn.cursor() as cur:
                    cur.execute("SELECT family_id, email, expires_at, replaced_by, revoked_at, revoke_reason FROM refresh_tokens WHERE jti = %s FOR UPDATE", (current_token_id,))
                    row = cur.fetchone()

                    if row is None:
                        conn.rollback()
                        return "missing"
                    if row["family_id"] != family_id or row["email"] != email:
                        conn.rollback()
                        return "mismatch"
                    if row["expires_at"] < now:
                        conn.rollback()
                        return "expired"
                    if row["replaced_by"] is not None:
                        if _is_within_rotation_grace(row["revoke_reason"], row["revoked_at"], now):
                            # grace window 내 동시 요청: 패밀리를 살린 채 후속 토큰만 재발급
                            cur.execute(
                                """
                                INSERT INTO refresh_tokens
                                    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason)
                                VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL)
                                """,
                                (new_token_id, family_id, email, issued_at, expires_at),
                            )
                            conn.commit()
                            return "ok"
                        self._revoke_family(cur, family_id=family_id, reason="reuse-detected", revoked_at=now)
                        conn.commit()
                        return "reused"
                    if row["revoked_at"] is not None:
                        self._revoke_family(cur, family_id=family_id, reason="revoked-token-reuse", revoked_at=now)
                        conn.commit()
                        return "revoked"

                    cur.execute(
                        """
                        INSERT INTO refresh_tokens
                            (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason)
                        VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL)
                        """,
                        (new_token_id, family_id, email, issued_at, expires_at),
                    )
                    cur.execute("UPDATE refresh_tokens SET replaced_by = %s, revoked_at = %s, revoke_reason = %s WHERE jti = %s", (new_token_id, issued_at, "rotated", current_token_id))
                conn.commit()
                return "ok"
            except Exception:
                conn.rollback()
                raise

    def get_token_observation(self, token_id: str) -> dict[str, Any] | None:
        """관측 로깅 전용 읽기 조회. 토큰 상태 판정이나 인가에 사용하지 않는다."""
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute("SELECT replaced_by, revoked_at, revoke_reason FROM refresh_tokens WHERE jti = %s", (token_id,))
                row = cur.fetchone()
        if row is None:
            return None
        return {"replaced_by_present": row["replaced_by"] is not None, "revoked_at": row["revoked_at"], "revoke_reason": row["revoke_reason"]}

    def _session_rows(self, *, email: str | None = None, family_id: str | None = None) -> list[Any]:
        now = int(time.time())
        sql = _session_aggregate_sql("%s", by_email=email is not None, by_family=family_id is not None)
        # SQL 안에서 `now` placeholder 는 두 CASE 식에 각각 한 번씩 쓰인다.
        params: list[Any] = [now, now]
        if email is not None:
            params.append(email)
        if family_id is not None:
            params.append(family_id)
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(sql, tuple(params))
                rows = cur.fetchall()
        rows = list(rows or [])
        if len(rows) >= MAX_SESSION_GROUPS:
            LOGGER.warning("세션 목록이 상한(%d)에 도달해 잘렸습니다. 오래된 refresh token row 정리가 필요합니다.", MAX_SESSION_GROUPS)
        return rows

    def list_sessions(self, *, status: str = "active", page: int = 1, page_size: int = 50, email: str | None = None, current_family_id: str | None = None) -> dict[str, Any]:
        rows = self._session_rows(email=email)
        return _build_session_page(rows, status=status, page=page, page_size=page_size, current_family_id=current_family_id)

    def revoke_session(self, *, family_id: str, reason: str = ADMIN_REVOKE_REASON) -> dict[str, Any]:
        """관리자 요청으로 family 하나를 폐기한다. 존재하지 않으면 found=False."""
        before = self._session_rows(family_id=family_id)
        if not before:
            return {"found": False, "revoked": False, "session": None}
        had_active = int(before[0]["valid_token_count"] or 0) > 0
        self.revoke_family(family_id, reason=reason)
        after = self._session_rows(family_id=family_id)
        return {"found": True, "revoked": had_active, "session": _derive_session(after[0]) if after else None}

    def revoke_family(self, family_id: str, reason: str = "manual") -> None:
        now = int(time.time())
        with self._connect() as conn:
            try:
                conn.begin()
                with conn.cursor() as cur:
                    self._revoke_family(cur, family_id=family_id, reason=reason, revoked_at=now)
                conn.commit()
            except Exception:
                conn.rollback()
                raise

    @staticmethod
    def _revoke_family(cur: Any, *, family_id: str, reason: str, revoked_at: int) -> None:
        cur.execute(
            """
            UPDATE refresh_tokens
            SET revoked_at = COALESCE(revoked_at, %s),
                revoke_reason = CASE
                    WHEN revoke_reason IS NULL OR revoke_reason = 'rotated' THEN %s
                    ELSE revoke_reason
                END
            WHERE family_id = %s
            """,
            (revoked_at, reason, family_id),
        )


def create_refresh_token_store() -> "RefreshTokenStore | MySQLRefreshTokenStore":
    """TM_REFRESH_TOKEN_BACKEND 에 따라 토큰 저장소를 생성한다.

    - "mysql": replica 간 공유되는 MySQLRefreshTokenStore (운영 권장)
    - 그 외(기본 "sqlite"): 단일 인스턴스/로컬/테스트용 SQLite RefreshTokenStore
    """
    backend = os.getenv("TM_REFRESH_TOKEN_BACKEND", "sqlite").lower()
    if backend == "mysql":
        return MySQLRefreshTokenStore()
    return RefreshTokenStore()
