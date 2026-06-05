import os
import sqlite3
import tempfile
import time
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterator


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
                SELECT family_id, email, expires_at, replaced_by, revoked_at
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
                    cur.execute("SELECT family_id, email, expires_at, replaced_by, revoked_at FROM refresh_tokens WHERE jti = %s FOR UPDATE", (current_token_id,))
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
