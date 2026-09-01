import sqlite3
import time

import pytest

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
        self._last_sql = ""

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql, params=None):
        self._last_sql = sql
        self._conn.executed.append((sql.strip().split()[0].upper(), params))

    def fetchone(self):
        # 컬럼 존재 확인(information_schema)은 마이그레이션 전용 질의라 응답 모양이 다르다.
        if "information_schema.columns" in self._last_sql:
            return {"cnt": self._conn.existing_column_count}
        return self._conn.select_row

    def fetchall(self):
        # 세션 집계(list_sessions)는 여러 row 를 읽는다. select_row 가 없으면 빈 결과.
        if self._conn.select_row is None:
            return []
        return [self._conn.select_row]


class _FakeConn:
    """rotate 상태머신 검증용 pymysql 연결 대역."""

    def __init__(self, select_row=None, existing_column_count=1):
        self.select_row = select_row
        # 1 이면 client_ip/user_agent 가 이미 있는 테이블(= ALTER 생략), 0 이면 마이그레이션 대상.
        self.existing_column_count = existing_column_count
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


class _BoomCursor(_FakeCursor):
    """execute 시 예외를 던지는 커서 대역 (rollback/raise 경로 검증용)."""

    def execute(self, sql, params=None):
        raise RuntimeError("db down")


class _BoomConn(_FakeConn):
    def cursor(self):
        return _BoomCursor(self)


def _install_init_then_boom(monkeypatch):
    """_init_db 는 정상 연결로 통과시키고, 이후 연산은 예외를 던지는 연결을 반환한다."""
    init_conn = _FakeConn(None)
    monkeypatch.setattr("pymysql.connect", lambda **kw: init_conn)
    from backend.refresh_token_store import MySQLRefreshTokenStore

    store = MySQLRefreshTokenStore()  # _init_db 는 정상 연결 사용
    boom = _BoomConn(None)
    monkeypatch.setattr("pymysql.connect", lambda **kw: boom)
    return store, boom


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


def test_rotation_grace_seconds_invalid_env_falls_back(monkeypatch):
    """TM_REFRESH_ROTATION_GRACE_SECONDS 가 정수가 아니면 기본값으로 폴백한다 (line 20-21)."""
    from backend.refresh_token_store import DEFAULT_ROTATION_GRACE_SECONDS, _rotation_grace_seconds

    monkeypatch.setenv("TM_REFRESH_ROTATION_GRACE_SECONDS", "not-an-int")
    assert _rotation_grace_seconds() == DEFAULT_ROTATION_GRACE_SECONDS


def test_mysql_store_issued_inserts_and_commits(monkeypatch):
    """MySQLRefreshTokenStore.store_issued 는 INSERT 후 commit 한다 (line 211-225)."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, None)
    store = MySQLRefreshTokenStore()

    store.store_issued(token_id="t", family_id="F", email="e", issued_at=1, expires_at=2)

    issued = conns[-1]  # store_issued 가 연 연결
    assert "INSERT" in _verbs(issued)
    assert issued.committed


def test_mysql_rotate_mismatch_rolls_back(monkeypatch):
    """family/email 불일치 시 rollback 하고 mismatch 를 반환한다 (line 240-241)."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, {"family_id": "OTHER", "email": "e", "expires_at": int(time.time()) + 1000, "replaced_by": None, "revoked_at": None})
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]
    assert result == "mismatch"
    assert rot.rolled_back and not rot.committed
    assert "INSERT" not in _verbs(rot)


def test_mysql_rotate_expired_rolls_back(monkeypatch):
    """만료된 토큰은 rollback 하고 expired 를 반환한다 (line 243-244)."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, {"family_id": "F", "email": "e", "expires_at": int(time.time()) - 1, "replaced_by": None, "revoked_at": None})
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]
    assert result == "expired"
    assert rot.rolled_back and not rot.committed
    assert "INSERT" not in _verbs(rot)


def test_mysql_rotate_revoked_token_revokes_family(monkeypatch):
    """이미 폐기된(revoked_at 설정, replaced_by 없음) 토큰 재제출 시 family 를 폐기하고 revoked 를 반환한다 (line 262-264)."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, {"family_id": "F", "email": "e", "expires_at": int(time.time()) + 1000, "replaced_by": None, "revoked_at": int(time.time()) - 10, "revoke_reason": "manual"})
    store = MySQLRefreshTokenStore()

    result = store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    rot = conns[-1]
    assert result == "revoked"
    assert "INSERT" not in _verbs(rot)
    assert "UPDATE" in _verbs(rot)
    assert rot.committed


def test_mysql_rotate_exception_rolls_back_and_raises(monkeypatch):
    """rotate 중 예외가 나면 rollback 후 예외를 재전파한다 (line 277-279)."""
    store, boom = _install_init_then_boom(monkeypatch)

    with pytest.raises(RuntimeError, match="db down"):
        store.rotate(current_token_id="old", new_token_id="new", family_id="F", email="e", issued_at=1, expires_at=2)

    assert boom.rolled_back and not boom.committed


def test_mysql_revoke_family_commits(monkeypatch):
    """MySQLRefreshTokenStore.revoke_family 는 UPDATE 후 commit 한다 (line 282-291)."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, None)
    store = MySQLRefreshTokenStore()

    store.revoke_family("F", reason="logout")

    rev = conns[-1]
    assert "UPDATE" in _verbs(rev)
    assert rev.committed and not rev.rolled_back


def test_mysql_revoke_family_exception_rolls_back_and_raises(monkeypatch):
    """revoke_family 중 예외가 나면 rollback 후 예외를 재전파한다 (line 289-291)."""
    store, boom = _install_init_then_boom(monkeypatch)

    with pytest.raises(RuntimeError, match="db down"):
        store.revoke_family("F", reason="logout")

    assert boom.rolled_back and not boom.committed


# ========== Phase 0: 관측 전용 읽기 조회 ==========


def test_get_token_observation_reports_rotation_state(tmp_path):
    """회전 전에는 replaced_by_present=False, 회전 후에는 True 를 보고한다."""
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)

    before = store.get_token_observation("token-1")
    assert before == {"replaced_by_present": False, "revoked_at": None, "revoke_reason": None}

    assert store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now + 1, expires_at=now + 1001) == "ok"

    after = store.get_token_observation("token-1")
    assert after is not None
    assert after["replaced_by_present"] is True
    assert after["revoke_reason"] == "rotated"


def test_get_token_observation_returns_none_for_unknown_token(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    assert store.get_token_observation("no-such-token") is None


def test_get_token_observation_does_not_mutate_state(tmp_path):
    """관측 조회는 읽기 전용이므로 이후 회전 결과에 영향을 주지 않는다."""
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="token-1", family_id="family-1", email="admin@example.com", issued_at=now, expires_at=now + 1000)

    store.get_token_observation("token-1")
    store.get_token_observation("token-1")

    assert store.rotate(current_token_id="token-1", new_token_id="token-2", family_id="family-1", email="admin@example.com", issued_at=now + 1, expires_at=now + 1001) == "ok"


def test_mysql_get_token_observation_reads_row(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, {"replaced_by": "successor", "revoked_at": 123, "revoke_reason": "rotated"})
    store = MySQLRefreshTokenStore()

    result = store.get_token_observation("old")

    assert result == {"replaced_by_present": True, "revoked_at": 123, "revoke_reason": "rotated"}
    # 읽기 전용이므로 커밋/롤백이 없어야 한다
    obs = conns[-1]
    assert _verbs(obs) == ["SELECT"]
    assert not obs.committed and not obs.rolled_back


def test_mysql_get_token_observation_returns_none_for_unknown_token(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    _install_fresh_conns(monkeypatch, None)
    store = MySQLRefreshTokenStore()

    assert store.get_token_observation("missing") is None


# ========== Admin 세션 관리: 집계/폐기 ==========


def _session_fixture(tmp_path):
    """활성/폐기/만료 family 를 각각 하나씩 만든 store 를 돌려준다."""
    store = RefreshTokenStore(str(tmp_path / "sessions.sqlite3"))
    now = int(time.time())
    # 활성 family: 두 번 회전해 토큰 row 3개
    store.store_issued(token_id="a1", family_id="a" * 32, email="admin@example.com", issued_at=now - 100, expires_at=now + 1000)
    store.rotate(current_token_id="a1", new_token_id="a2", family_id="a" * 32, email="admin@example.com", issued_at=now - 50, expires_at=now + 1500)
    store.rotate(current_token_id="a2", new_token_id="a3", family_id="a" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 2000)
    # 로그아웃으로 폐기된 family
    store.store_issued(token_id="b1", family_id="b" * 32, email="viewer@example.com", issued_at=now - 200, expires_at=now + 1000)
    store.revoke_family("b" * 32, reason="logout")
    # 자연 만료된 family
    store.store_issued(token_id="c1", family_id="c" * 32, email="viewer@example.com", issued_at=now - 9999, expires_at=now - 10)
    return store, now


def test_list_sessions_groups_by_family_and_derives_status(tmp_path):
    store, now = _session_fixture(tmp_path)

    page = store.list_sessions(status="all")

    by_email_status = {(i["email"], i["status"]) for i in page["items"]}
    assert by_email_status == {("admin@example.com", "active"), ("viewer@example.com", "revoked"), ("viewer@example.com", "expired")}
    assert page["summary"] == {"active": 1, "expired": 1, "revoked": 1, "total": 3}

    active = next(i for i in page["items"] if i["status"] == "active")
    # 회전 이력이 모두 한 family 로 묶이고, 살아 있는 토큰은 최신 하나뿐이다
    assert active["token_count"] == 3
    assert active["valid_token_count"] == 1
    assert active["created_at"] == now - 100
    assert active["last_seen_at"] == now - 10
    assert active["expires_at"] == now + 2000
    assert active["revoked_at"] is None
    assert active["revoke_reason"] is None


def test_list_sessions_reports_revoke_reason_only_for_revoked(tmp_path):
    store, _ = _session_fixture(tmp_path)

    items = {i["status"]: i for i in store.list_sessions(status="all")["items"]}

    assert items["revoked"]["revoke_reason"] == "logout"
    assert items["revoked"]["revoked_at"] is not None
    # 자연 만료는 폐기 사유가 없다
    assert items["expired"]["revoke_reason"] is None


def test_list_sessions_active_filter_and_default(tmp_path):
    store, _ = _session_fixture(tmp_path)

    default_page = store.list_sessions()
    assert [i["status"] for i in default_page["items"]] == ["active"]
    # 요약은 필터와 무관하게 전체를 센다
    assert default_page["summary"]["total"] == 3
    assert default_page["pagination"]["totalItems"] == 1


def test_list_sessions_does_not_expose_token_internals(tmp_path):
    store, _ = _session_fixture(tmp_path)

    item = store.list_sessions(status="all")["items"][0]

    assert "jti" not in item
    assert "replaced_by" not in item
    assert set(item.keys()) == {"session_id", "session_label", "email", "status", "created_at", "last_seen_at", "expires_at", "revoked_at", "revoke_reason", "token_count", "valid_token_count", "client_ip", "user_agent", "user_agent_summary", "is_current", "merged_family_ids"}
    # 라벨은 family_id 앞부분만 보여준다
    assert item["session_label"].endswith("...")
    assert len(item["session_label"]) == 11


def _chrome_ua(version: str) -> str:
    return f"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version}.0.0.0 Safari/537.36"


def test_list_sessions_merges_adjacent_browser_version_upgrade(tmp_path):
    """같은 계정·IP·OS 에서 브라우저 버전만 1 올라간 두 family 는 한 행으로 합친다."""
    store = RefreshTokenStore(str(tmp_path / "sessions.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="old1", family_id="d" * 32, email="admin@example.com", issued_at=now - 500, expires_at=now + 500, client_ip="1.2.3.4", user_agent=_chrome_ua("130"))
    store.store_issued(token_id="new1", family_id="e" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 1000, client_ip="1.2.3.4", user_agent=_chrome_ua("131"))

    items = store.list_sessions(status="all")["items"]

    assert len(items) == 1
    merged = items[0]
    assert merged["session_id"] == "e" * 32
    assert set(merged["merged_family_ids"]) == {"d" * 32, "e" * 32}
    assert merged["status"] == "active"
    assert merged["created_at"] == now - 500
    assert merged["last_seen_at"] == now - 10
    assert merged["token_count"] == 2
    assert merged["valid_token_count"] == 2
    assert merged["user_agent"] == _chrome_ua("131")


def test_list_sessions_does_not_merge_across_different_ip(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "sessions.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="f1", family_id="f" * 32, email="admin@example.com", issued_at=now - 500, expires_at=now + 500, client_ip="1.2.3.4", user_agent=_chrome_ua("130"))
    store.store_issued(token_id="g1", family_id="0" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 1000, client_ip="9.9.9.9", user_agent=_chrome_ua("131"))

    items = store.list_sessions(status="all")["items"]

    assert len(items) == 2


def test_list_sessions_does_not_merge_when_version_gap_exceeds_tolerance(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "sessions.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="h1", family_id="1" * 32, email="admin@example.com", issued_at=now - 500, expires_at=now + 500, client_ip="1.2.3.4", user_agent=_chrome_ua("125"))
    store.store_issued(token_id="i1", family_id="2" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 1000, client_ip="1.2.3.4", user_agent=_chrome_ua("131"))

    items = store.list_sessions(status="all")["items"]

    assert len(items) == 2


def test_list_sessions_never_merges_revoked_sessions(tmp_path):
    """관리자가 명시적으로 폐기한 세션은 최신 활성 세션과 절대 합치지 않는다."""
    store = RefreshTokenStore(str(tmp_path / "sessions.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="j1", family_id="3" * 32, email="admin@example.com", issued_at=now - 500, expires_at=now + 500, client_ip="1.2.3.4", user_agent=_chrome_ua("130"))
    store.revoke_family("3" * 32, reason="admin-revoked")
    store.store_issued(token_id="k1", family_id="4" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 1000, client_ip="1.2.3.4", user_agent=_chrome_ua("131"))

    items = store.list_sessions(status="all")["items"]

    assert len(items) == 2
    assert {i["status"] for i in items} == {"revoked", "active"}


def test_list_sessions_marks_current_session(tmp_path):
    store, _ = _session_fixture(tmp_path)

    page = store.list_sessions(status="all", current_family_id="b" * 32)

    current = [i for i in page["items"] if i["is_current"]]
    assert len(current) == 1
    assert current[0]["session_id"] == "b" * 32


def test_list_sessions_paginates_newest_activity_first(tmp_path):
    store, _ = _session_fixture(tmp_path)

    first = store.list_sessions(status="all", page=1, page_size=2)
    second = store.list_sessions(status="all", page=2, page_size=2)

    assert first["pagination"] == {"page": 1, "pageSize": 2, "totalItems": 3, "totalPages": 2}
    assert len(first["items"]) == 2
    assert len(second["items"]) == 1
    # last_seen_at 내림차순
    seen = [i["last_seen_at"] for i in first["items"] + second["items"]]
    assert seen == sorted(seen, reverse=True)
    # 페이지 간 중복 없음
    assert not ({i["session_id"] for i in first["items"]} & {i["session_id"] for i in second["items"]})


def test_list_sessions_page_beyond_end_is_empty(tmp_path):
    store, _ = _session_fixture(tmp_path)
    assert store.list_sessions(status="all", page=99, page_size=50)["items"] == []


def test_list_sessions_filters_by_email(tmp_path):
    store, _ = _session_fixture(tmp_path)

    page = store.list_sessions(status="all", email="viewer@example.com")

    assert {i["email"] for i in page["items"]} == {"viewer@example.com"}
    assert page["summary"]["total"] == 2


def test_list_sessions_empty_store(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "empty.sqlite3"))

    page = store.list_sessions(status="all")

    assert page["items"] == []
    assert page["summary"] == {"active": 0, "expired": 0, "revoked": 0, "total": 0}
    assert page["pagination"]["totalPages"] == 1


def test_revoke_session_revokes_whole_family(tmp_path):
    store, _ = _session_fixture(tmp_path)

    result = store.revoke_session(family_id="a" * 32)

    assert result["found"] is True
    assert result["revoked"] is True
    assert result["session"]["status"] == "revoked"
    assert result["session"]["revoke_reason"] == "admin-revoked"
    # 폐기 후에는 회전이 막힌다
    now = int(time.time())
    assert store.rotate(current_token_id="a3", new_token_id="a4", family_id="a" * 32, email="admin@example.com", issued_at=now, expires_at=now + 1000) == "revoked"


def test_revoke_session_is_idempotent_for_inactive_family(tmp_path):
    store, _ = _session_fixture(tmp_path)
    store.revoke_session(family_id="a" * 32)

    again = store.revoke_session(family_id="a" * 32)

    # 이미 활성 토큰이 없으므로 revoked=False 지만 성공이고 현재 상태를 돌려준다
    assert again["found"] is True
    assert again["revoked"] is False
    assert again["session"]["status"] == "revoked"


def test_revoke_session_keeps_existing_terminal_reason(tmp_path):
    store, _ = _session_fixture(tmp_path)

    result = store.revoke_session(family_id="b" * 32)

    assert result["found"] is True
    assert result["revoked"] is False
    # logout 으로 이미 폐기된 family 의 사유를 admin-revoked 로 덮어쓰지 않는다
    assert result["session"]["revoke_reason"] == "logout"


def test_revoke_session_unknown_family(tmp_path):
    store, _ = _session_fixture(tmp_path)

    result = store.revoke_session(family_id="d" * 32)

    assert result == {"found": False, "revoked": False, "session": None}


def test_revoke_session_does_not_touch_other_families(tmp_path):
    store, _ = _session_fixture(tmp_path)

    store.revoke_session(family_id="a" * 32)

    others = {i["session_id"]: i for i in store.list_sessions(status="all")["items"]}
    assert others["b" * 32]["revoke_reason"] == "logout"
    assert others["c" * 32]["status"] == "expired"


def test_mysql_list_sessions_uses_same_derivation(monkeypatch):
    """MySQL store 도 SQLite 와 동일한 집계 결과 형태를 돌려준다."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    now = int(time.time())
    row = {
        "family_id": "a" * 32,
        "email": "admin@example.com",
        "created_at": now - 100,
        "last_seen_at": now - 10,
        "max_expires_at": now + 2000,
        "token_count": 3,
        "valid_token_count": 1,
        "active_expires_at": now + 2000,
        "terminal_reason": None,
        "revoked_at": None,
        "client_ip": "203.0.113.7",
        "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    }
    _install_fresh_conns(monkeypatch, row)
    store = MySQLRefreshTokenStore()

    page = store.list_sessions(status="all", current_family_id="a" * 32)

    assert page["summary"] == {"active": 1, "expired": 0, "revoked": 0, "total": 1}
    item = page["items"][0]
    assert item["status"] == "active"
    assert item["is_current"] is True
    assert item["token_count"] == 3
    assert item["session_label"] == "aaaaaaaa..."


def test_mysql_revoke_session_revokes_and_reports_status(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    now = int(time.time())
    row = {
        "family_id": "a" * 32,
        "email": "admin@example.com",
        "created_at": now - 100,
        "last_seen_at": now - 10,
        "max_expires_at": now + 2000,
        "token_count": 3,
        "valid_token_count": 1,
        "active_expires_at": now + 2000,
        "terminal_reason": None,
        "revoked_at": None,
        "client_ip": "203.0.113.7",
        "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    }
    conns = _install_fresh_conns(monkeypatch, row)
    store = MySQLRefreshTokenStore()

    result = store.revoke_session(family_id="a" * 32)

    assert result["found"] is True
    assert result["revoked"] is True
    # family revoke UPDATE 가 커밋된 연결이 있어야 한다
    assert any("UPDATE" in _verbs(c) and c.committed for c in conns)


def test_mysql_revoke_session_unknown_family(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    _install_fresh_conns(monkeypatch, None)
    store = MySQLRefreshTokenStore()

    assert store.revoke_session(family_id="d" * 32) == {"found": False, "revoked": False, "session": None}


# ========== 접속 IP / User-Agent 수집 ==========


CHROME_MAC_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
SAFARI_IOS_UA = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1"


def test_list_sessions_reports_latest_client_info(tmp_path):
    """세션의 IP/UA 는 family 안에서 가장 최근 발급된 토큰 값을 보여준다."""
    from backend.refresh_token_store import RefreshTokenStore

    store = RefreshTokenStore(str(tmp_path / "client_info.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="t1", family_id="a" * 32, email="admin@example.com", issued_at=now - 100, expires_at=now + 1000, client_ip="203.0.113.7", user_agent=CHROME_MAC_UA)
    # 같은 세션이 다른 회선에서 갱신되면 최신 값으로 덮인다
    store.rotate(current_token_id="t1", new_token_id="t2", family_id="a" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 2000, client_ip="198.51.100.42", user_agent=SAFARI_IOS_UA)

    item = store.list_sessions(status="all")["items"][0]

    assert item["client_ip"] == "198.51.100.42"
    assert item["user_agent"] == SAFARI_IOS_UA
    assert item["user_agent_summary"] == "Safari 17 / iOS 17"


def test_list_sessions_returns_empty_client_info_for_legacy_rows(tmp_path):
    """컬럼 추가 이전에 쌓인 세션은 빈 문자열로 나온다(None 이 화면까지 새지 않는다)."""
    store, _ = _session_fixture(tmp_path)

    for item in store.list_sessions(status="all")["items"]:
        assert item["client_ip"] == ""
        assert item["user_agent"] == ""
        assert item["user_agent_summary"] == ""


def test_store_truncates_oversized_client_info(tmp_path):
    """UA 는 상한이 없으므로 컬럼 폭에 맞게 잘라 저장한다."""
    from backend.refresh_token_store import MAX_USER_AGENT_LENGTH, RefreshTokenStore

    store = RefreshTokenStore(str(tmp_path / "truncate.sqlite3"))
    now = int(time.time())
    store.store_issued(token_id="t1", family_id="a" * 32, email="admin@example.com", issued_at=now, expires_at=now + 1000, client_ip="203.0.113.7", user_agent="U" * (MAX_USER_AGENT_LENGTH * 2))

    item = store.list_sessions(status="all")["items"][0]

    assert len(item["user_agent"]) == MAX_USER_AGENT_LENGTH


def test_sqlite_migration_adds_client_columns(tmp_path):
    """컬럼 추가 이전 스키마의 DB 를 열어도 ALTER 로 보정되고 조회가 동작한다."""
    from backend.refresh_token_store import RefreshTokenStore

    db_path = tmp_path / "legacy.sqlite3"
    legacy = sqlite3.connect(db_path)
    legacy.execute("CREATE TABLE refresh_tokens (jti TEXT PRIMARY KEY, family_id TEXT NOT NULL, email TEXT NOT NULL, issued_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, replaced_by TEXT, revoked_at INTEGER, revoke_reason TEXT)")
    now = int(time.time())
    legacy.execute("INSERT INTO refresh_tokens VALUES ('t1', ?, 'admin@example.com', ?, ?, NULL, NULL, NULL)", ("a" * 32, now, now + 1000))
    legacy.commit()
    legacy.close()

    store = RefreshTokenStore(str(db_path))

    with sqlite3.connect(db_path) as check:
        columns = {row[0] for row in check.execute("SELECT name FROM pragma_table_info('refresh_tokens')")}
    assert {"client_ip", "user_agent"} <= columns
    assert store.list_sessions(status="all")["items"][0]["client_ip"] == ""


def test_mysql_migration_adds_client_columns_when_missing(monkeypatch):
    """MySQL 도 기존 테이블에 컬럼이 없으면 ALTER 를 실행한다."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns: list[_FakeConn] = []

    def _connect(**kw):
        conn = _FakeConn(None, existing_column_count=0)
        conns.append(conn)
        return conn

    monkeypatch.setattr("pymysql.connect", _connect)
    MySQLRefreshTokenStore()

    assert _verbs(conns[0]).count("ALTER") == 2


def test_mysql_migration_skips_alter_when_columns_exist(monkeypatch):
    from backend.refresh_token_store import MySQLRefreshTokenStore

    conns = _install_fresh_conns(monkeypatch, None)
    MySQLRefreshTokenStore()

    assert "ALTER" not in _verbs(conns[0])


@pytest.mark.parametrize(
    "user_agent,expected",
    [
        (CHROME_MAC_UA, "Chrome 131 / macOS 10.15"),
        (SAFARI_IOS_UA, "Safari 17 / iOS 17"),
        ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0", "Edge 131 / Windows 10+"),
        ("Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0", "Firefox 133 / Linux"),
        ("Mozilla/5.0 (Linux; Android 14; SM-S918N) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/23.0 Chrome/115.0.0.0 Mobile Safari/537.36", "Samsung Internet 23 / Android 14"),
        # 파생 브라우저는 Chrome 토큰을 함께 달고 다니므로 파생 쪽이 이겨야 한다
        ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Whale/3.28.266.14 Safari/537.36", "Whale 3 / Windows 10+"),
        # 브라우저를 못 알아보면 원문 앞부분으로 봇/스크립트를 식별한다
        ("curl/8.5.0", "curl/8.5.0"),
        ("", ""),
    ],
)
def test_summarize_user_agent(user_agent, expected):
    from backend.refresh_token_store import summarize_user_agent

    assert summarize_user_agent(user_agent) == expected


# ========== 실제 MySQL 대상 검증 (집계 SQL / 마이그레이션) ==========


@pytest.fixture()
def mysql_store(mysql_container):
    """실제 MySQL 컨테이너에 붙은 스토어. 테스트마다 테이블을 비운다."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    store = MySQLRefreshTokenStore()
    with store._connect() as conn:
        with conn.cursor() as cur:
            cur.execute("TRUNCATE TABLE refresh_tokens")
        conn.commit()
    return store


def test_mysql_aggregate_sql_runs_and_reports_latest_client_info(mysql_store):
    """윈도우 함수 기반 집계가 MySQL 8 에서 실제로 실행되고 최신 IP/UA 를 돌려준다."""
    now = int(time.time())
    mysql_store.store_issued(token_id="t1", family_id="a" * 32, email="admin@example.com", issued_at=now - 100, expires_at=now + 1000, client_ip="203.0.113.7", user_agent=CHROME_MAC_UA)
    assert mysql_store.rotate(current_token_id="t1", new_token_id="t2", family_id="a" * 32, email="admin@example.com", issued_at=now - 10, expires_at=now + 2000, client_ip="198.51.100.42", user_agent=SAFARI_IOS_UA) == "ok"

    page = mysql_store.list_sessions(status="all")

    assert len(page["items"]) == 1
    item = page["items"][0]
    assert item["status"] == "active"
    assert item["token_count"] == 2
    assert item["client_ip"] == "198.51.100.42"
    assert item["user_agent"] == SAFARI_IOS_UA
    assert item["user_agent_summary"] == "Safari 17 / iOS 17"


def test_mysql_aggregate_sql_filters_by_email(mysql_store):
    """{where} 가 서브쿼리로 들어가도 파라미터 순서(now, now, email)가 맞는지 본다."""
    now = int(time.time())
    mysql_store.store_issued(token_id="t1", family_id="a" * 32, email="admin@example.com", issued_at=now, expires_at=now + 1000, client_ip="203.0.113.7", user_agent=CHROME_MAC_UA)
    mysql_store.store_issued(token_id="t2", family_id="b" * 32, email="viewer@example.com", issued_at=now, expires_at=now + 1000, client_ip="198.51.100.42", user_agent=SAFARI_IOS_UA)

    page = mysql_store.list_sessions(status="all", email="viewer@example.com")

    assert [i["email"] for i in page["items"]] == ["viewer@example.com"]
    assert page["items"][0]["client_ip"] == "198.51.100.42"


def test_mysql_migration_adds_columns_to_legacy_table(mysql_container):
    """컬럼 없는 기존 테이블을 열어도 ALTER 로 보정되고 기존 row 는 NULL 로 남는다."""
    from backend.refresh_token_store import MySQLRefreshTokenStore

    bootstrap = MySQLRefreshTokenStore()
    with bootstrap._connect() as conn:
        with conn.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS refresh_tokens")
            cur.execute(
                "CREATE TABLE refresh_tokens (jti VARCHAR(64) PRIMARY KEY, family_id VARCHAR(64) NOT NULL, email VARCHAR(320) NOT NULL, issued_at BIGINT NOT NULL, expires_at BIGINT NOT NULL, replaced_by VARCHAR(64), revoked_at BIGINT, revoke_reason VARCHAR(64), INDEX idx_refresh_tokens_family (family_id)) ENGINE=InnoDB"
            )
            now = int(time.time())
            cur.execute("INSERT INTO refresh_tokens VALUES ('t1', %s, 'admin@example.com', %s, %s, NULL, NULL, NULL)", ("a" * 32, now, now + 1000))
        conn.commit()

    store = MySQLRefreshTokenStore()  # _init_db 가 마이그레이션을 수행한다

    item = store.list_sessions(status="all")["items"][0]
    assert item["client_ip"] == ""
    assert item["user_agent"] == ""
