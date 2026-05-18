import os
import sqlite3
import tempfile
import time
from pathlib import Path


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
