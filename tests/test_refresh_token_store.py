import sqlite3
import time

from backend.refresh_token_store import RefreshTokenStore


def test_rotate_success(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(
        token_id="token-1",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now,
        expires_at=now + 1000,
    )

    result = store.rotate(
        current_token_id="token-1",
        new_token_id="token-2",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now + 1,
        expires_at=now + 1001,
    )

    assert result == "ok"


def test_reuse_detection_revokes_family(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(
        token_id="token-1",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now,
        expires_at=now + 1000,
    )
    assert (
        store.rotate(
            current_token_id="token-1",
            new_token_id="token-2",
            family_id="family-1",
            email="admin@example.com",
            issued_at=now + 1,
            expires_at=now + 1001,
        )
        == "ok"
    )

    assert (
        store.rotate(
            current_token_id="token-1",
            new_token_id="token-3",
            family_id="family-1",
            email="admin@example.com",
            issued_at=now + 2,
            expires_at=now + 1002,
        )
        == "reused"
    )
    assert (
        store.rotate(
            current_token_id="token-2",
            new_token_id="token-4",
            family_id="family-1",
            email="admin@example.com",
            issued_at=now + 3,
            expires_at=now + 1003,
        )
        == "revoked"
    )


def test_revoke_family_blocks_rotation(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(
        token_id="token-1",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now,
        expires_at=now + 1000,
    )

    store.revoke_family("family-1", reason="logout")

    result = store.rotate(
        current_token_id="token-1",
        new_token_id="token-2",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now + 1,
        expires_at=now + 1001,
    )

    assert result == "revoked"


def test_rotate_missing_token(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    result = store.rotate(
        current_token_id="missing-token",
        new_token_id="token-2",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now,
        expires_at=now + 1000,
    )
    assert result == "missing"


def test_rotate_rejects_family_or_email_mismatch(tmp_path):
    store = RefreshTokenStore(str(tmp_path / "refresh_tokens.sqlite3"))
    now = int(time.time())
    store.store_issued(
        token_id="token-1",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now,
        expires_at=now + 1000,
    )

    family_result = store.rotate(
        current_token_id="token-1",
        new_token_id="token-2",
        family_id="other-family",
        email="admin@example.com",
        issued_at=now + 1,
        expires_at=now + 1001,
    )
    email_result = store.rotate(
        current_token_id="token-1",
        new_token_id="token-3",
        family_id="family-1",
        email="other@example.com",
        issued_at=now + 1,
        expires_at=now + 1001,
    )

    assert family_result == "mismatch"
    assert email_result == "mismatch"


def test_rotate_rejects_expired_token_without_creating_replacement(tmp_path):
    db_path = tmp_path / "refresh_tokens.sqlite3"
    store = RefreshTokenStore(str(db_path))
    now = int(time.time())
    store.store_issued(
        token_id="token-1",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now - 100,
        expires_at=now - 1,
    )

    result = store.rotate(
        current_token_id="token-1",
        new_token_id="token-2",
        family_id="family-1",
        email="admin@example.com",
        issued_at=now,
        expires_at=now + 1000,
    )

    assert result == "expired"
    with sqlite3.connect(db_path) as conn:
        row = conn.execute("SELECT COUNT(*) FROM refresh_tokens WHERE jti = ?", ("token-2",)).fetchone()
    assert row[0] == 0
