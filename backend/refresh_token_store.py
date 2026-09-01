import logging
import os
import re
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
# IPv6 최대 표기 길이(45자). IPv4-mapped 표기까지 포함한다.
MAX_CLIENT_IP_LENGTH = 45
# User-Agent 는 상한이 없으므로 저장 전에 자른다.
MAX_USER_AGENT_LENGTH = 512

# family 단위 집계. `{p}` 는 store 별 placeholder(?, %s)로 치환한다.
# 접속 IP/UA 는 family 안에서 가장 최근에 발급된 토큰(= 마지막 refresh 갱신) 값을 쓴다.
# `{p}` 두 개가 서브쿼리의 {where} 보다 앞서 나오므로 파라미터 순서는 (now, now, ...) 다.
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
           MAX(revoked_at) AS revoked_at,
           MAX(CASE WHEN recency_rank = 1 THEN client_ip END) AS client_ip,
           MAX(CASE WHEN recency_rank = 1 THEN user_agent END) AS user_agent
    FROM (
        SELECT family_id,
               email,
               issued_at,
               expires_at,
               revoked_at,
               revoke_reason,
               client_ip,
               user_agent,
               ROW_NUMBER() OVER (PARTITION BY family_id ORDER BY issued_at DESC, jti DESC) AS recency_rank
        FROM refresh_tokens
        {where}
    ) ranked
    GROUP BY family_id, email
    ORDER BY MAX(issued_at) DESC
    LIMIT {limit}
"""

# UA 요약용 패턴. 파생 브라우저가 Chrome/Safari 토큰을 함께 달고 다니므로 순서가 중요하다.
_UA_BROWSER_PATTERNS = (
    (re.compile(r"Edg(?:e|A|iOS)?/(\d+)"), "Edge {0}"),
    (re.compile(r"OPR/(\d+)"), "Opera {0}"),
    (re.compile(r"SamsungBrowser/(\d+)"), "Samsung Internet {0}"),
    (re.compile(r"Whale/(\d+)"), "Whale {0}"),
    (re.compile(r"(?:FxiOS|Firefox)/(\d+)"), "Firefox {0}"),
    (re.compile(r"(?:CriOS|Chrome)/(\d+)"), "Chrome {0}"),
    (re.compile(r"Version/(\d+)[\d.]* (?:Mobile/\S+ )?Safari"), "Safari {0}"),
)
_UA_OS_PATTERNS = (
    (re.compile(r"iPhone OS (\d+)"), "iOS {0}"),
    (re.compile(r"CPU OS (\d+)"), "iPadOS {0}"),
    (re.compile(r"Mac OS X (\d+[._]\d+)"), "macOS {0}"),
    (re.compile(r"Android (\d+)"), "Android {0}"),
    (re.compile(r"Windows NT 10\.0"), "Windows 10+"),
    (re.compile(r"Windows NT ([\d.]+)"), "Windows NT {0}"),
    (re.compile(r"CrOS"), "ChromeOS"),
    (re.compile(r"Linux"), "Linux"),
)
# 브라우저를 못 알아본 UA(스크립트·봇)는 앞쪽 제품 토큰만 남긴다.
UA_FALLBACK_LENGTH = 40


def _match_ua_pattern(user_agent: str, patterns: tuple) -> str:
    for pattern, template in patterns:
        match = pattern.search(user_agent)
        if match:
            return template.format(*(g.replace("_", ".") for g in match.groups()))
    return ""


def summarize_user_agent(user_agent: str) -> str:
    """UA 원문을 '브라우저 / OS' 한 줄로 줄인다. 원문은 별도 필드로 함께 노출한다.

    외부 UA 파서를 새로 의존성으로 들이지 않고, 관리 화면에서 기기를 구분할 정도만
    정규식으로 뽑는다. 판별에 실패하면 원문 앞부분을 그대로 보여줘 봇/스크립트도
    식별할 수 있게 한다.
    """
    if not user_agent:
        return ""
    browser = _match_ua_pattern(user_agent, _UA_BROWSER_PATTERNS)
    os_name = _match_ua_pattern(user_agent, _UA_OS_PATTERNS)
    if browser and os_name:
        return f"{browser} / {os_name}"
    if browser or os_name:
        return browser or os_name
    return user_agent[:UA_FALLBACK_LENGTH]


def _parse_browser_name_version(user_agent: str) -> tuple[str, int] | None:
    """브라우저 이름과 메이저 버전을 분리해서 뽑는다.

    summarize_user_agent 와 같은 패턴을 쓰되, 세션 병합에서 버전 인접 여부(±1)를
    비교하려면 정수 버전이 필요해 별도로 둔다.
    """
    if not user_agent:
        return None
    for pattern, template in _UA_BROWSER_PATTERNS:
        match = pattern.search(user_agent)
        if match:
            try:
                version = int(match.group(1))
            except (IndexError, ValueError):
                return None
            return template.split(" {0}")[0], version
    return None


def _truncate(value: str | None, limit: int) -> str | None:
    """빈 값은 NULL 로, 긴 값은 컬럼 폭에 맞게 자른다."""
    if not value:
        return None
    return value[:limit]


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
        # 마지막 refresh 갱신 시점의 접속 정보. 컬럼 추가 이전 세션은 빈 문자열이다.
        "client_ip": row["client_ip"] or "",
        "user_agent": row["user_agent"] or "",
        "user_agent_summary": summarize_user_agent(row["user_agent"] or ""),
        "is_current": False,
        "merged_family_ids": [family_id],
    }


# 브라우저 자동 업데이트로 세션이 갈라져 보이지 않도록 병합을 허용하는 최대 버전 차이.
SESSION_MERGE_VERSION_TOLERANCE = 1


def _combine_sessions(members: list[dict[str, Any]]) -> dict[str, Any]:
    """같은 기기의 브라우저 업데이트로 판단된 세션 묶음을 화면 표시용 한 행으로 합친다.

    family_id 회전·재사용탐지 상태는 절대 건드리지 않는다. revoke 는 여전히
    최신(primary) family 하나만 대상으로 한다 (main.py:revoke_login_session).
    """
    if len(members) == 1:
        return members[0]
    # 표시 필드(client_ip/user_agent/expires_at 등)는 실제로 살아 있는 세션을
    # 우선한다 - 더 최근에 만들어졌지만 이미 만료된 family 를 대표로 삼지 않는다.
    active_members = [m for m in members if m["status"] == "active"]
    primary = max(active_members or members, key=lambda s: s["last_seen_at"])
    combined = dict(primary)
    combined["status"] = "active" if active_members else "expired"
    combined["created_at"] = min(m["created_at"] for m in members)
    combined["last_seen_at"] = max(m["last_seen_at"] for m in members)
    combined["token_count"] = sum(m["token_count"] for m in members)
    combined["valid_token_count"] = sum(m["valid_token_count"] for m in members)
    combined["is_current"] = any(m["is_current"] for m in members)
    combined["merged_family_ids"] = [m["session_id"] for m in members]
    return combined


def _merge_similar_sessions(sessions: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """같은 계정·접속 IP·OS에서 브라우저 메이저 버전만 인접(±1)하게 바뀐 세션들을
    하나의 표시 행으로 묶는다 (브라우저 자동 업데이트로 세션이 여러 개로 보이는 문제).

    - revoked 세션은 관리자의 명시적 폐기 이력을 가리지 않도록 병합하지 않는다.
    - UA 를 브라우저/OS 로 파싱하지 못하거나 client_ip 가 없는 세션도 병합하지 않는다.
    - 병합은 표시 전용이며 family_id/rotate()/재사용탐지에는 영향을 주지 않는다.
    """
    buckets: dict[tuple[str, str, str, str], list[tuple[int, dict[str, Any]]]] = {}
    passthrough: list[dict[str, Any]] = []
    for session in sessions:
        if session["status"] == "revoked":
            passthrough.append(session)
            continue
        browser = _parse_browser_name_version(session["user_agent"])
        os_name = _match_ua_pattern(session["user_agent"], _UA_OS_PATTERNS)
        if not browser or not os_name or not session["client_ip"]:
            passthrough.append(session)
            continue
        key = (session["email"], session["client_ip"], os_name, browser[0])
        buckets.setdefault(key, []).append((browser[1], session))

    merged = passthrough
    for entries in buckets.values():
        entries.sort(key=lambda entry: entry[1]["last_seen_at"])
        chain_version, chain = entries[0][0], [entries[0][1]]
        for version, session in entries[1:]:
            if abs(version - chain_version) <= SESSION_MERGE_VERSION_TOLERANCE:
                chain.append(session)
            else:
                merged.append(_combine_sessions(chain))
                chain = [session]
            chain_version = version
        merged.append(_combine_sessions(chain))
    return merged


def _build_session_page(rows: list[Any], *, status: str, page: int, page_size: int, current_family_id: str | None) -> dict[str, Any]:
    """집계 row 들을 병합·상태 요약·필터·페이징이 적용된 응답으로 만든다."""
    sessions = [_derive_session(row) for row in rows]
    if current_family_id:
        for session in sessions:
            if session["session_id"] == current_family_id:
                session["is_current"] = True
    sessions = _merge_similar_sessions(sessions)

    summary = {"active": 0, "expired": 0, "revoked": 0, "total": len(sessions)}
    for session in sessions:
        summary[session["status"]] += 1

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
                    revoke_reason TEXT,
                    client_ip TEXT,
                    user_agent TEXT
                )
                """
            )
            conn.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_refresh_tokens_family
                ON refresh_tokens (family_id)
                """
            )
            self._migrate_add_client_columns(conn)

    @staticmethod
    def _migrate_add_client_columns(conn: sqlite3.Connection) -> None:
        """컬럼 추가 이전에 만들어진 DB 에 client_ip/user_agent 를 붙인다."""
        existing = {row["name"] for row in conn.execute("PRAGMA table_info(refresh_tokens)")}
        for column in ("client_ip", "user_agent"):
            if column not in existing:
                LOGGER.info("Migrating refresh_tokens: adding %s column", column)
                conn.execute(f"ALTER TABLE refresh_tokens ADD COLUMN {column} TEXT")

    def store_issued(self, *, token_id: str, family_id: str, email: str, issued_at: int, expires_at: int, client_ip: str = "", user_agent: str = "") -> None:
        with self._connect() as conn:
            conn.execute(
                """
                INSERT OR REPLACE INTO refresh_tokens
                    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
                VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL, ?, ?)
                """,
                (token_id, family_id, email, issued_at, expires_at, _truncate(client_ip, MAX_CLIENT_IP_LENGTH), _truncate(user_agent, MAX_USER_AGENT_LENGTH)),
            )

    def rotate(self, *, current_token_id: str, new_token_id: str, family_id: str, email: str, issued_at: int, expires_at: int, client_ip: str = "", user_agent: str = "") -> str:
        now = int(time.time())
        stored_ip = _truncate(client_ip, MAX_CLIENT_IP_LENGTH)
        stored_ua = _truncate(user_agent, MAX_USER_AGENT_LENGTH)
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
                            (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
                        VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL, ?, ?)
                        """,
                        (new_token_id, family_id, email, issued_at, expires_at, stored_ip, stored_ua),
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
                    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
                VALUES (?, ?, ?, ?, ?, NULL, NULL, NULL, ?, ?)
                """,
                (new_token_id, family_id, email, issued_at, expires_at, stored_ip, stored_ua),
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
                        client_ip VARCHAR(45),
                        user_agent VARCHAR(512),
                        INDEX idx_refresh_tokens_family (family_id)
                    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
                    """
                )
                self._migrate_add_client_columns(cur)
            conn.commit()

    def _migrate_add_client_columns(self, cur: Any) -> None:
        """컬럼 추가 이전에 만들어진 테이블에 client_ip/user_agent 를 붙인다."""
        for column, definition in (("client_ip", "VARCHAR(45)"), ("user_agent", "VARCHAR(512)")):
            cur.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = %s AND table_name = 'refresh_tokens' AND column_name = %s", (self.database, column))
            row = cur.fetchone()
            if row and row["cnt"] == 0:
                LOGGER.info("Migrating refresh_tokens: adding %s column", column)
                cur.execute(f"ALTER TABLE refresh_tokens ADD COLUMN {column} {definition}")

    def store_issued(self, *, token_id: str, family_id: str, email: str, issued_at: int, expires_at: int, client_ip: str = "", user_agent: str = "") -> None:
        with self._connect() as conn:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO refresh_tokens
                        (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
                    VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL, %s, %s)
                    ON DUPLICATE KEY UPDATE
                        family_id = VALUES(family_id), email = VALUES(email),
                        issued_at = VALUES(issued_at), expires_at = VALUES(expires_at),
                        replaced_by = NULL, revoked_at = NULL, revoke_reason = NULL,
                        client_ip = VALUES(client_ip), user_agent = VALUES(user_agent)
                    """,
                    (token_id, family_id, email, issued_at, expires_at, _truncate(client_ip, MAX_CLIENT_IP_LENGTH), _truncate(user_agent, MAX_USER_AGENT_LENGTH)),
                )
            conn.commit()

    def rotate(self, *, current_token_id: str, new_token_id: str, family_id: str, email: str, issued_at: int, expires_at: int, client_ip: str = "", user_agent: str = "") -> str:
        now = int(time.time())
        stored_ip = _truncate(client_ip, MAX_CLIENT_IP_LENGTH)
        stored_ua = _truncate(user_agent, MAX_USER_AGENT_LENGTH)
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
                                    (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
                                VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL, %s, %s)
                                """,
                                (new_token_id, family_id, email, issued_at, expires_at, stored_ip, stored_ua),
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
                            (jti, family_id, email, issued_at, expires_at, replaced_by, revoked_at, revoke_reason, client_ip, user_agent)
                        VALUES (%s, %s, %s, %s, %s, NULL, NULL, NULL, %s, %s)
                        """,
                        (new_token_id, family_id, email, issued_at, expires_at, stored_ip, stored_ua),
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
