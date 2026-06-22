import sqlite3
import time

from backend.refresh_token_store import RefreshTokenStore


def test_rotate_success(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)

    result = store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now + 1, expires_at=now + 1001)

    assert result == "ok"


def test_concurrent_reuse_within_grace_reissues_without_revoking_family(tmp_path):
    """방금 회전된 토큰이 grace window 내에 다시 제출되면(멀티탭 동시 refresh) 재사용
    공격이 아닌 정상 동시 요청으로 보고, 패밀리를 폐기하지 않고 후속 토큰을 재발급한다."""
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)
    assert store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now + 1, expires_at=now + 1001) == "ok"

    # token-1 을 grace window 내에 한 번 더 제출 -> 폐기 대신 재발급("ok")
    assert store.rotate(current_token_id="token-1", new_token_id="token-3", family_id="family-1", email="admin@example.com", issued_at=now + 2, expires_at=now + 1002) == "ok"

    # 패밀리가 살아 있으므로 두 후속 토큰 모두 계속 사용 가능
    assert store.rotate(current_token_id="token-2", new_token_id="token-4", family_id="family-1", email="admin@example.com", issued_at=now + 3, expires_at=now + 1003) == "ok"
    assert store.rotate(current_token_id="token-3", new_token_id="token-5", family_id="family-1", email="admin@example.com", issued_at=now + 4, expires_at=now + 1004) == "ok"


def test_reuse_detection_revokes_family_after_grace(tmp_path):
    """grace window 를 벗어난 뒤 회전 완료된 토큰이 재제출되면 실제 재사용 공격으로
    간주하고 패밀리 전체를 폐기한다."""
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now - 1000, expires_at=now + 1000)
    # 회전이 grace window 보다 한참 전에 발생(revoked_at = now-500)
    assert store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now - 500, expires_at=now + 1001) == "ok"

    # 회전된 지 오래된 token-1 을 재제출 -> 진짜 재사용 -> 패밀리 폐기
    assert store.rotate(current_token_id="token-1", new_token_id="token-3", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1002) == "reused"
    # 패밀리가 폐기되어 정상 후속 토큰 token-2 도 차단됨
    assert store.rotate(current_token_id="token-2", new_token_id="token-4", family_id="family-1", email="admin@example.com", issued_at=now + 1, expires_at=now + 1003) == "revoked"


def test_revoke_family_blocks_rotation(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)

    store.revoke_family("family-1", reason="logout")

    result = store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now + 1, expires_at=now + 1001)

    assert result == "revoked"


def test_rotate_missing_token(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    result = store.rotate(current_token_id="missing-token", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)
    assert result == "missing"


def test_rotate_rejects_family_or_email_mismatch(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)

    family_result = store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="other-family", email="admin@example.com", issued_at=now + 1, expires_at=now + 1001)
    email_result = store.rotate(current_token_id="token-1", new_token_id="token-3", family_id="family-1", email="other@example.com", issued_at=now + 1, expires_at=now + 1001)

    assert family_result == "mismatch"
    assert email_result == "mismatch"


def test_rotate_rejects_expired_token_without_creating_replacement(tmp_path):
    db_path = tmp_path / "refresh_tokens.sqlite3"
    store = RefreshTokenStore(str(db_path))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now - 100, expires_at=now - 1)

    result = store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)

    assert result == "expired"
    with sqlite3.connect(db_path) as conn:
        row = conn.execute("SELECT COUNT(*) FROM refresh_tokens WHERE jti = ?", ("token-2",)).fetchone()
    assert row[0] == 0


# --- N3: MySQL 공유 백엔드 (replica 간 일관성) ---


class _FakeCursor:
    def __init__(self, conn):
        self._conn = conn

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql, params=None):
        self._conn.executed.append((sql.strip().split()[0].upper(), params))

    def fetchone(self):
        return self._conn.select_row


class _FakeConn:
    """rotate 상태머신 검증용 pymysql 연결 대역."""

    def __init__(self, select_row=None):
        self.select_row = select_row
        self.executed = []
        self.committed = False
        self.rolled_back = False

    def cursor(self):
        return _FakeCursor(self)

    def begin(self):
        pass

    def commit(self):
        self.committed = True

    def rollback(self):
        self.rolled_back = True

    def close(self):
        pass


def _verbs(conn):
    return [v for v, _ in conn.executed]


def test_factory_defaults_to_sqlite(monkeypatch, tmp_path):
    from backend.refresh_token_store import RefreshTokenStore, create_refresh_token_store

    monkeypatch.delenv("TM_REFRESH_TOKEN_BACKEND", raising=False)
    monkeypatch.setenv("TM_REFRESH_TOKEN_DB", str(tmp_path / "x.sqlite3"))
    assert isinstance(create_refresh_token_store(), RefreshTokenStore)


def test_factory_selects_mysql_when_configured(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore, create_refresh_token_store

    monkeypatch.setenv("TM_REFRESH_TOKEN_BACKEND", "mysql")
    monkeypatch.setattr("pymysql.connect", lambda **kw: _FakeConn())
    assert isinstance(create_refresh_token_store(), MySQLRefreshTokenStore)


def _install_fresh_conns(monkeypatch, select_row):
    """pymysql.connect 가 호출마다 새 _FakeConn 을 반환하도록 한다(_init_db 와 rotate 분리)."""
    conns: list[_FakeConn] = []

    def _connect(**kw):
        conn = _FakeConn(select_row)
        conns.append(conn)
        return conn

    monkeypatch.setattr("pymysql.connect", _connect)
    return conns


def test_mysql_rotate_ok_inserts_new_and_revokes_old(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, {"family_id": "F", "email": "e", "expires_at": int(time.time()) + 1000, "replaced_by": None, "revoked_at": None})
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]  # rotate 가 연 연결
    assert result == "ok"
    assert rot.committed and not rot.rolled_back
    assert "INSERT" in _verbs(rot) and "UPDATE" in _verbs(rot)


def test_mysql_rotate_detects_reuse_and_revokes_family(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    # grace window 밖에서 회전된 토큰(revoked_at 이 오래 전, reason='rotated') 재사용
    conns = _install_fresh_conns(monkeypatch, {"family_id": "F", "email": "e", "expires_at": int(time.time()) + 1000, "replaced_by": "already-used", "revoked_at": int(time.time()) - 1000, "revoke_reason": "rotated"})
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]
    assert result == "reused"
    # 재사용 탐지 시 신규 토큰을 만들지 않고 family revoke(UPDATE)만 수행
    assert "INSERT" not in _verbs(rot)
    assert "UPDATE" in _verbs(rot)
    assert rot.committed


def test_mysql_rotate_within_grace_reissues_without_revoking_family(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    # 방금(grace window 내) 회전된 토큰 재제출 -> 폐기 없이 후속 토큰 재발급
    conns = _install_fresh_conns(monkeypatch, {"family_id": "F", "email": "e", "expires_at": int(time.time()) + 1000, "replaced_by": "successor", "revoked_at": int(time.time()), "revoke_reason": "rotated"})
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]
    assert result == "ok"
    # 신규 토큰 INSERT 만 수행하고 family revoke(UPDATE) 는 하지 않음
    assert "INSERT" in _verbs(rot)
    assert "UPDATE" not in _verbs(rot)
    assert rot.committed and not rot.rolled_back


def test_mysql_rotate_missing_rolls_back(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, None)
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]
    assert result == "missing"
    assert rot.rolled_back and not rot.committed
