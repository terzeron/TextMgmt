import os
from unittest.mock import patch
import logging
import sys
import time
import importlib
import pytest
import jwt
from fastapi.testclient import TestClient
from starlette.requests import Request
from fastapi.exceptions import RequestValidationError

import backend.main as main_mod

os.environ.setdefault("TM_FRONTEND_URL", "http://localhost:3000")
os.environ.setdefault("TM_BOOK_DIR", "/tmp")
os.environ.setdefault("TM_COMICS_DIR", "/tmp")
os.environ.setdefault("TM_ES_URL", "http://localhost:9200")
os.environ.setdefault("TM_ES_BOOK_INDEX", "test_books")
os.environ.setdefault("TM_ES_COMICS_INDEX", "test_comics")
os.environ.setdefault("TM_ES_USER", "test")
os.environ.setdefault("TM_ES_PASSWORD", "test")


@pytest.fixture(autouse=True)
def setup_env(monkeypatch):
    monkeypatch.setenv("TM_JWT_SECRET", "testsecret123")
    monkeypatch.setenv("TM_ADMIN_EMAIL", "admin@example.com")
    monkeypatch.setenv("TM_ALLOWED_EMAILS", "viewer1@example.com,viewer2@example.com")
    # 모듈 재로드하여 환경변수 반영
    import backend.auth as auth_mod

    importlib.reload(auth_mod)
    yield auth_mod
    monkeypatch.undo()
    importlib.reload(auth_mod)


class TestDetermineRole:
    def test_admin_email(self, setup_env):
        assert setup_env.determine_role("admin@example.com") == "admin"

    def test_viewer_email(self, setup_env):
        assert setup_env.determine_role("viewer1@example.com") == "viewer"
        assert setup_env.determine_role("viewer2@example.com") == "viewer"

    def test_unknown_email(self, setup_env):
        assert setup_env.determine_role("unknown@example.com") is None

    def test_empty_email(self, setup_env):
        assert setup_env.determine_role("") is None

    def test_reads_current_env_without_reload(self, setup_env, monkeypatch):
        monkeypatch.setenv("TM_ADMIN_EMAIL", "new-admin@example.com")
        monkeypatch.setenv("TM_ALLOWED_EMAILS", "new-viewer@example.com")

        assert setup_env.determine_role("new-admin@example.com") == "admin"
        assert setup_env.determine_role("new-viewer@example.com") == "viewer"
        assert setup_env.determine_role("admin@example.com") is None


def test_auth_requires_jwt_secret(monkeypatch):
    monkeypatch.delenv("TM_JWT_SECRET", raising=False)
    prev_mod = sys.modules.pop("backend.auth", None)
    try:
        with pytest.raises(SystemExit):
            importlib.import_module("backend.auth")
    finally:
        sys.modules.pop("backend.auth", None)
        if prev_mod is not None:
            sys.modules["backend.auth"] = prev_mod


class TestCreateJwtToken:
    def test_token_contains_claims(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin", "Admin", "pic.jpg")
        payload = jwt.decode(token, "testsecret123", algorithms=["HS256"])
        assert payload["email"] == "admin@example.com"
        assert payload["role"] == "admin"
        assert payload["name"] == "Admin"
        assert payload["picture"] == "pic.jpg"
        assert payload["type"] == "access"
        assert "exp" in payload
        assert "iat" in payload

    def test_token_expiration(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        payload = jwt.decode(token, "testsecret123", algorithms=["HS256"])
        assert payload["exp"] > time.time()
        assert payload["exp"] <= time.time() + 2 * 3600 + 5


class TestCreateRefreshToken:
    def test_refresh_token_contains_claims(self, setup_env):
        token = setup_env.create_refresh_token("admin@example.com", "admin", "Admin", "pic.jpg")
        payload = jwt.decode(token, "testsecret123", algorithms=["HS256"])
        assert payload["email"] == "admin@example.com"
        assert payload["role"] == "admin"
        assert payload["type"] == "refresh"
        assert payload["name"] == "Admin"
        assert payload["picture"] == "pic.jpg"
        assert payload["fid"]
        assert payload["jti"]

    def test_refresh_token_expiration(self, setup_env):
        token = setup_env.create_refresh_token("admin@example.com", "admin")
        payload = jwt.decode(token, "testsecret123", algorithms=["HS256"])
        assert payload["exp"] > time.time()
        assert payload["exp"] <= time.time() + 7 * 24 * 3600 + 5


class TestDecodeRefreshToken:
    def test_valid_refresh_token(self, setup_env):
        token = setup_env.create_refresh_token("admin@example.com", "admin")
        payload = setup_env.decode_refresh_token(token)
        assert payload["email"] == "admin@example.com"
        assert payload["type"] == "refresh"

    def test_expired_refresh_token(self, setup_env):
        from fastapi import HTTPException

        expired_payload = {"type": "refresh", "email": "admin@example.com", "role": "admin", "exp": int(time.time()) - 100, "iat": int(time.time()) - 200}
        token = jwt.encode(expired_payload, "testsecret123", algorithm="HS256")
        with pytest.raises(HTTPException) as exc_info:
            setup_env.decode_refresh_token(token)
        assert exc_info.value.status_code == 401
        assert "expired" in exc_info.value.detail.lower()

    def test_access_token_rejected(self, setup_env):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("admin@example.com", "admin")
        with pytest.raises(HTTPException) as exc_info:
            setup_env.decode_refresh_token(token)
        assert exc_info.value.status_code == 401
        assert "type" in exc_info.value.detail.lower()

    def test_invalid_token(self, setup_env):
        from fastapi import HTTPException

        with pytest.raises(HTTPException) as exc_info:
            setup_env.decode_refresh_token("invalid.token.value")
        assert exc_info.value.status_code == 401

    def test_refresh_token_without_rotation_claims_is_rejected(self, setup_env):
        from fastapi import HTTPException

        payload = {"type": "refresh", "email": "admin@example.com", "role": "admin", "exp": int(time.time()) + 100, "iat": int(time.time())}
        token = jwt.encode(payload, "testsecret123", algorithm="HS256")
        with pytest.raises(HTTPException) as exc_info:
            setup_env.decode_refresh_token(token)
        assert exc_info.value.status_code == 401
        assert "claims" in exc_info.value.detail.lower()

    def test_refresh_token_missing_fid_only_is_rejected(self, setup_env):
        """fid만 없는 경우도 claims 에러로 거부된다."""
        from fastapi import HTTPException

        payload = {"type": "refresh", "email": "admin@example.com", "role": "admin", "jti": "some-jti", "exp": int(time.time()) + 100, "iat": int(time.time())}
        token = jwt.encode(payload, "testsecret123", algorithm="HS256")
        with pytest.raises(HTTPException) as exc_info:
            setup_env.decode_refresh_token(token)
        assert exc_info.value.status_code == 401
        assert "claims" in exc_info.value.detail.lower()
        assert "log in again" in exc_info.value.detail.lower()

    def test_refresh_token_missing_jti_only_is_rejected(self, setup_env):
        """jti만 없는 경우도 claims 에러로 거부된다."""
        from fastapi import HTTPException

        payload = {"type": "refresh", "email": "admin@example.com", "role": "admin", "fid": "some-fid", "exp": int(time.time()) + 100, "iat": int(time.time())}
        token = jwt.encode(payload, "testsecret123", algorithm="HS256")
        with pytest.raises(HTTPException) as exc_info:
            setup_env.decode_refresh_token(token)
        assert exc_info.value.status_code == 401
        assert "claims" in exc_info.value.detail.lower()


class TestExtractPayload:
    def _make_request(self, token=None, cookie_token=None):
        class FakeRequest:
            def __init__(self, token, cookie_token):
                self.headers = {}
                self.cookies = {}
                if token:
                    self.headers["Authorization"] = f"Bearer {token}"
                if cookie_token:
                    self.cookies["tm_access_token"] = cookie_token

        return FakeRequest(token, cookie_token)

    def test_missing_header(self, setup_env):
        from fastapi import HTTPException

        req = self._make_request(None)
        with pytest.raises(HTTPException) as exc_info:
            setup_env._extract_payload(req)
        assert exc_info.value.status_code == 401

    def test_valid_token(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(token)
        payload = setup_env._extract_payload(req)
        assert payload["email"] == "admin@example.com"
        assert payload["role"] == "admin"

    def test_valid_cookie_token(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(cookie_token=token)
        payload = setup_env._extract_payload(req)
        assert payload["email"] == "admin@example.com"
        assert payload["role"] == "admin"

    def test_expired_token(self, setup_env):
        from fastapi import HTTPException

        expired_payload = {"type": "access", "email": "admin@example.com", "role": "admin", "exp": int(time.time()) - 100, "iat": int(time.time()) - 200}
        token = jwt.encode(expired_payload, "testsecret123", algorithm="HS256")
        req = self._make_request(token)
        with pytest.raises(HTTPException) as exc_info:
            setup_env._extract_payload(req)
        assert exc_info.value.status_code == 401
        assert "expired" in exc_info.value.detail.lower()

    def test_invalid_token(self, setup_env):
        from fastapi import HTTPException

        req = self._make_request("invalid.token.value")
        with pytest.raises(HTTPException) as exc_info:
            setup_env._extract_payload(req)
        assert exc_info.value.status_code == 401

    def test_refresh_token_rejected(self, setup_env):
        from fastapi import HTTPException

        token = setup_env.create_refresh_token("admin@example.com", "admin")
        req = self._make_request(token)
        with pytest.raises(HTTPException) as exc_info:
            setup_env._extract_payload(req)
        assert exc_info.value.status_code == 401
        assert "type" in exc_info.value.detail.lower()

    def test_bearer_header_takes_priority_over_cookie(self, setup_env):
        header_token = setup_env.create_jwt_token("admin@example.com", "admin")
        cookie_token = setup_env.create_jwt_token("viewer1@example.com", "viewer")
        req = self._make_request(token=header_token, cookie_token=cookie_token)
        payload = setup_env._extract_payload(req)
        assert payload["email"] == "admin@example.com"

    def test_empty_bearer_falls_back_to_cookie(self, setup_env):
        cookie_token = setup_env.create_jwt_token("viewer1@example.com", "viewer")

        class FakeRequest:
            def __init__(self):
                self.headers = {"Authorization": "Bearer "}
                self.cookies = {"tm_access_token": cookie_token}

        payload = setup_env._extract_payload(FakeRequest())
        assert payload["email"] == "viewer1@example.com"


class TestRequireAuth:
    def _make_request(self, token=None, cookie_token=None):
        class FakeRequest:
            def __init__(self, token, cookie_token):
                self.headers = {"Authorization": f"Bearer {token}"} if token else {}
                self.cookies = {}
                if cookie_token:
                    self.cookies["tm_access_token"] = cookie_token

        return FakeRequest(token, cookie_token)

    @pytest.mark.asyncio
    async def test_admin_passes(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(token)
        payload = await setup_env.require_auth(req)
        assert payload["role"] == "admin"

    @pytest.mark.asyncio
    async def test_viewer_passes(self, setup_env):
        token = setup_env.create_jwt_token("viewer1@example.com", "viewer")
        req = self._make_request(token)
        payload = await setup_env.require_auth(req)
        assert payload["role"] == "viewer"

    @pytest.mark.asyncio
    async def test_cookie_token_passes(self, setup_env):
        token = setup_env.create_jwt_token("viewer1@example.com", "viewer")
        req = self._make_request(cookie_token=token)
        payload = await setup_env.require_auth(req)
        assert payload["role"] == "viewer"

    @pytest.mark.asyncio
    async def test_invalid_role_fails(self, setup_env):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("nobody@example.com", "unknown")
        req = self._make_request(token)
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_auth(req)
        assert exc_info.value.status_code == 403

    @pytest.mark.asyncio
    async def test_revoked_viewer_fails(self, setup_env, monkeypatch):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("viewer1@example.com", "viewer")
        req = self._make_request(token)
        monkeypatch.setattr(setup_env, "determine_role", lambda email: None)
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_auth(req)
        assert exc_info.value.status_code == 403
        assert exc_info.value.detail == "Access denied"

    @pytest.mark.asyncio
    async def test_role_mismatch_fails(self, setup_env, monkeypatch):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("viewer1@example.com", "viewer")
        req = self._make_request(token)
        monkeypatch.setattr(setup_env, "determine_role", lambda email: "admin")
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_auth(req)
        assert exc_info.value.status_code == 403
        assert exc_info.value.detail == "Access denied"

    @pytest.mark.asyncio
    async def test_no_token_fails(self, setup_env):
        from fastapi import HTTPException

        req = self._make_request()
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_auth(req)
        assert exc_info.value.status_code == 401


class TestRequireAdmin:
    def _make_request(self, token=None, cookie_token=None):
        class FakeRequest:
            def __init__(self, token, cookie_token):
                self.headers = {"Authorization": f"Bearer {token}"} if token else {}
                self.cookies = {}
                if cookie_token:
                    self.cookies["tm_access_token"] = cookie_token

        return FakeRequest(token, cookie_token)

    @pytest.mark.asyncio
    async def test_admin_passes(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(token)
        payload = await setup_env.require_admin(req)
        assert payload["role"] == "admin"

    @pytest.mark.asyncio
    async def test_admin_cookie_passes(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(cookie_token=token)
        payload = await setup_env.require_admin(req)
        assert payload["role"] == "admin"

    @pytest.mark.asyncio
    async def test_viewer_blocked(self, setup_env):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("viewer1@example.com", "viewer")
        req = self._make_request(token)
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_admin(req)
        assert exc_info.value.status_code == 403
        assert "admin" in exc_info.value.detail.lower()

    @pytest.mark.asyncio
    async def test_admin_revoked_fails(self, setup_env, monkeypatch):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(token)
        monkeypatch.setattr(setup_env, "determine_role", lambda email: None)
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_admin(req)
        assert exc_info.value.status_code == 403
        assert exc_info.value.detail == "Access denied"

    @pytest.mark.asyncio
    async def test_admin_demoted_to_viewer_fails(self, setup_env, monkeypatch):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(token)
        monkeypatch.setattr(setup_env, "determine_role", lambda email: "viewer")
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_admin(req)
        assert exc_info.value.status_code == 403
        assert exc_info.value.detail == "Access denied"

    @pytest.mark.asyncio
    async def test_no_token_fails(self, setup_env):
        from fastapi import HTTPException

        req = self._make_request()
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_admin(req)
        assert exc_info.value.status_code == 401


class TestCreateJwtTokenDefaults:
    def test_default_name_and_picture(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        payload = jwt.decode(token, "testsecret123", algorithms=["HS256"])
        assert payload["name"] == ""
        assert payload["picture"] == ""


class TestGetCookieSettings:
    """_get_cookie_settings의 SameSite/Secure 검증 테스트."""

    @pytest.fixture(autouse=True)
    def mock_managers(self):
        with patch("backend.main.BookManager"), patch("backend.main.ComicsManager"), patch("backend.main.CategoryMapping"), patch("backend.main.Yes24Bookstore"):
            yield

    def test_default_values(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
        monkeypatch.delenv("TM_COOKIE_SECURE", raising=False)
        monkeypatch.delenv("TM_COOKIE_SAMESITE", raising=False)
        secure, samesite = main_mod._get_cookie_settings()
        assert secure is False
        assert samesite == "lax"

    def test_secure_true(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "https://app.example.com")
        monkeypatch.setenv("TM_COOKIE_SECURE", "true")
        monkeypatch.delenv("TM_COOKIE_SAMESITE", raising=False)
        secure, samesite = main_mod._get_cookie_settings()
        assert secure is True
        assert samesite == "lax"

    def test_samesite_strict(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
        monkeypatch.delenv("TM_COOKIE_SECURE", raising=False)
        monkeypatch.setenv("TM_COOKIE_SAMESITE", "strict")
        secure, samesite = main_mod._get_cookie_settings()
        assert samesite == "strict"

    def test_invalid_samesite_falls_back_to_lax(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
        monkeypatch.delenv("TM_COOKIE_SECURE", raising=False)
        monkeypatch.setenv("TM_COOKIE_SAMESITE", "invalid_value")
        secure, samesite = main_mod._get_cookie_settings()
        assert samesite == "lax"

    def test_samesite_none_with_secure_true(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
        monkeypatch.setenv("TM_COOKIE_SECURE", "true")
        monkeypatch.setenv("TM_COOKIE_SAMESITE", "none")
        secure, samesite = main_mod._get_cookie_settings()
        assert secure is True
        assert samesite == "none"

    def test_samesite_none_without_secure_falls_back_to_lax(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
        monkeypatch.setenv("TM_COOKIE_SECURE", "false")
        monkeypatch.setenv("TM_COOKIE_SAMESITE", "none")
        secure, samesite = main_mod._get_cookie_settings()
        assert secure is False
        assert samesite == "lax"

    def test_non_local_origin_forces_secure_even_when_disabled(self, monkeypatch):
        import backend.main as main_mod
        import importlib

        importlib.reload(main_mod)
        monkeypatch.setenv("TM_FRONTEND_URL", "https://app.example.com")
        monkeypatch.setenv("TM_COOKIE_SECURE", "false")
        secure, samesite = main_mod._get_cookie_settings()
        assert secure is True
        assert samesite == "lax"


@pytest.fixture()
def auth_client():
    def _auth_override():
        return {"email": "user@example.com", "role": "user"}

    def _admin_override():
        return {"email": "admin@example.com", "role": "admin"}

    main_mod.app.dependency_overrides[main_mod.require_auth] = _auth_override
    main_mod.app.dependency_overrides[main_mod.require_admin] = _admin_override
    with TestClient(main_mod.app) as c:
        yield c
    main_mod.app.dependency_overrides.clear()


def test_auth_google_success(auth_client, monkeypatch):
    monkeypatch.setattr(main_mod, "TM_GOOGLE_CLIENT_ID", "cid")
    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda credential, request, audience: {"aud": audience, "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": True, "name": "N", "picture": "P"})
    monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
    monkeypatch.setattr(main_mod, "_issue_auth_tokens", lambda **kwargs: ("jwt", "rjwt"))

    resp = auth_client.post("/auth/google", json={"credential": "c"})
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "success"
    # 선제적 갱신을 위한 expires_in 필드 포함 확인
    assert "expires_in" in data
    assert data["expires_in"] == main_mod.ACCESS_TOKEN_EXPIRATION_SECONDS


def test_auth_google_errors(auth_client, monkeypatch):
    resp = auth_client.post("/auth/google", json={})
    assert resp.status_code == 400

    monkeypatch.setattr(main_mod, "TM_GOOGLE_CLIENT_ID", "cid")

    def raise_invalid(*args, **kwargs):
        raise ValueError("nope")

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", raise_invalid)
    resp = auth_client.post("/auth/google", json={"credential": "c"})
    assert resp.status_code == 401

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: {"aud": "cid", "iss": "https://evil.example.com", "email": "e@example.com", "email_verified": True})
    resp = auth_client.post("/auth/google", json={"credential": "c"})
    assert resp.status_code == 401

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: {"aud": "cid", "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": False})
    resp = auth_client.post("/auth/google", json={"credential": "c"})
    assert resp.status_code == 401

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: {"aud": "cid", "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": True})
    monkeypatch.setattr(main_mod, "determine_role", lambda email: None)
    resp = auth_client.post("/auth/google", json={"credential": "c"})
    assert resp.status_code == 403


def test_auth_refresh_and_logout(auth_client, monkeypatch):
    resp = auth_client.post("/auth/refresh")
    assert resp.status_code == 400

    monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": "e@example.com", "role": "user", "name": "n", "picture": "p", "fid": "family1", "jti": "old-token"})
    monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
    monkeypatch.setattr(main_mod, "create_jwt_token", lambda **kwargs: "jwt")
    monkeypatch.setattr(main_mod, "create_refresh_token", lambda **kwargs: "rjwt")
    monkeypatch.setattr(main_mod.refresh_token_store, "rotate", lambda **kwargs: "ok")
    revoke_calls = []
    monkeypatch.setattr(main_mod.refresh_token_store, "revoke_family", lambda family_id, reason="manual": revoke_calls.append((family_id, reason)))
    resp = auth_client.post("/auth/refresh", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})
    assert resp.status_code == 200
    # sliding expiration: refresh token도 새로 발급되어야 함
    assert main_mod.REFRESH_COOKIE_NAME in resp.cookies
    assert resp.cookies[main_mod.REFRESH_COOKIE_NAME] == "rjwt"
    # 선제적 갱신을 위한 expires_in 필드 포함 확인
    refresh_data = resp.json()
    assert "expires_in" in refresh_data
    assert refresh_data["expires_in"] == main_mod.ACCESS_TOKEN_EXPIRATION_SECONDS

    resp = auth_client.get("/auth/me")
    assert resp.status_code == 200
    # /auth/me도 expires_in(잔여 초) 반환
    me_data = resp.json()
    assert "expires_in" in me_data["result"]
    assert isinstance(me_data["result"]["expires_in"], int)

    resp = auth_client.post("/auth/logout")
    assert resp.status_code == 200
    assert revoke_calls == [("family1", "logout")]

    resp = auth_client.post("/auth/logout", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})
    assert resp.status_code == 200
    assert revoke_calls == [("family1", "logout"), ("family1", "logout")]


def test_auth_refresh_rejected_token_state_clears_cookies(auth_client, monkeypatch):
    monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": "e@example.com", "role": "user", "name": "n", "picture": "p", "fid": "family1", "jti": "old-token"})
    monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
    monkeypatch.setattr(main_mod.refresh_token_store, "rotate", lambda **kwargs: "reused")

    resp = auth_client.post("/auth/refresh", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})

    assert resp.status_code == 401
    assert resp.json()["detail"] == "Invalid refresh token state"
    set_cookie = resp.headers.get("set-cookie", "")
    assert f"{main_mod.ACCESS_COOKIE_NAME}=" in set_cookie
    assert f"{main_mod.REFRESH_COOKIE_NAME}=" in set_cookie
    assert "Max-Age=0" in set_cookie


def test_logout_ignores_invalid_refresh_token(auth_client, monkeypatch):
    monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: (_ for _ in ()).throw(main_mod.HTTPException(status_code=401, detail="bad token")))
    revoke_calls = []
    monkeypatch.setattr(main_mod.refresh_token_store, "revoke_family", lambda family_id, reason="manual": revoke_calls.append((family_id, reason)))

    resp = auth_client.post("/auth/logout", cookies={main_mod.REFRESH_COOKIE_NAME: "bad"})

    assert resp.status_code == 200
    assert revoke_calls == []
    set_cookie = resp.headers.get("set-cookie", "")
    assert f"{main_mod.ACCESS_COOKIE_NAME}=" in set_cookie
    assert f"{main_mod.REFRESH_COOKIE_NAME}=" in set_cookie
    assert "Max-Age=0" in set_cookie


def test_auth_me_expires_in_with_real_token(monkeypatch):
    """실제 JWT로 /auth/me 호출 시 expires_in이 남은 초를 정확히 반환하는지 검증."""
    from backend.auth import create_jwt_token, ACCESS_TOKEN_EXPIRATION_SECONDS

    monkeypatch.setattr(main_mod, "determine_role", lambda email: "admin")
    token = create_jwt_token(email="admin@example.com", role="admin", name="A", picture="P")

    # dependency override 없이 실제 토큰으로 호출
    with TestClient(main_mod.app) as client:
        resp = client.get("/auth/me", cookies={main_mod.ACCESS_COOKIE_NAME: token})
        assert resp.status_code == 200
        data = resp.json()
        expires_in = data["result"]["expires_in"]
        # 방금 생성한 토큰이므로 남은 시간은 만료 시간에 근접해야 함 (±5초 오차 허용)
        assert ACCESS_TOKEN_EXPIRATION_SECONDS - 5 <= expires_in <= ACCESS_TOKEN_EXPIRATION_SECONDS


def test_handlers_direct(caplog):
    scope = {"type": "http", "method": "GET", "path": "/x", "headers": []}
    req = Request(scope)

    secret = "super-secret-token"
    exc = RequestValidationError([{"loc": ("body",), "msg": "bad", "type": "value_error"}], body={"credential": secret})
    resp = run_async(main_mod.validation_exception_handler(req, exc))
    assert resp.status_code == 422
    assert secret not in caplog.text

    http_exc = main_mod.HTTPException(status_code=404, detail="no")
    resp = run_async(main_mod.http_exception_handler(req, http_exc))
    assert resp.status_code == 404

    resp = run_async(main_mod.general_exception_handler(req, Exception("boom")))
    assert resp.status_code == 500


def test_cookie_settings_defaults(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
    monkeypatch.delenv("TM_COOKIE_SECURE", raising=False)
    monkeypatch.delenv("TM_COOKIE_SAMESITE", raising=False)
    secure, samesite = main_mod._get_cookie_settings()
    assert secure is False
    assert samesite == "lax"


def test_cookie_settings_none_requires_secure(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
    monkeypatch.setenv("TM_COOKIE_SECURE", "false")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "none")
    secure, samesite = main_mod._get_cookie_settings()
    assert secure is False
    assert samesite == "lax"


def test_set_and_clear_auth_cookies(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("TM_COOKIE_SECURE", "true")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "strict")
    response = main_mod.CustomJSONResponse({"ok": True})
    main_mod._set_auth_cookies(response, "access-token", "refresh-token")
    cookies = response.headers.getlist("set-cookie")
    assert any(main_mod.ACCESS_COOKIE_NAME in cookie for cookie in cookies)
    assert any(main_mod.REFRESH_COOKIE_NAME in cookie for cookie in cookies)

    response2 = main_mod.CustomJSONResponse({"ok": True})
    main_mod._clear_auth_cookies(response2)
    cookies2 = response2.headers.getlist("set-cookie")
    assert any(f"{main_mod.ACCESS_COOKIE_NAME}=" in cookie for cookie in cookies2)
    assert any(f"{main_mod.REFRESH_COOKIE_NAME}=" in cookie for cookie in cookies2)


def test_custom_jsonable_encoder_preserves_strings():
    payload = {"k": "한글", "list": ["테스트", {"x": "값"}]}
    encoded = main_mod.custom_jsonable_encoder(payload)
    assert encoded["k"] == "한글"
    assert encoded["list"][0] == "테스트"


def run_async(coro):
    import asyncio

    return asyncio.run(coro)


# ---- merged from test_auth_env_guard.py ----


def test_auth_requires_jwt_secret_os_environ():
    prev = os.environ.pop("TM_JWT_SECRET", None)
    saved_mod = sys.modules.get("backend.auth")
    try:
        if "backend.auth" in sys.modules:
            del sys.modules["backend.auth"]
        with pytest.raises(SystemExit):
            importlib.import_module("backend.auth")
    finally:
        if prev is not None:
            os.environ["TM_JWT_SECRET"] = prev
        # 모듈 복원 (autouse fixture teardown에서 reload 필요)
        if saved_mod is not None:
            sys.modules["backend.auth"] = saved_mod


@pytest.mark.asyncio
async def test_require_auth_non_standard_role_raises_403(setup_env, monkeypatch):
    """determine_role 이 "admin"/"viewer" 이외 역할을 반환할 때 require_auth 가 403 을 올린다 (line 101)."""
    from fastapi import HTTPException

    auth_mod = setup_env
    # determine_role 이 허용되지 않은 역할("superuser")을 반환하도록 패치
    monkeypatch.setattr(auth_mod, "determine_role", lambda email: "superuser")

    token = auth_mod.create_jwt_token("admin@example.com", "superuser")

    class FakeRequest:
        headers = {"Authorization": f"Bearer {token}"}
        cookies = {}

    with pytest.raises(HTTPException) as exc_info:
        await auth_mod.require_auth(FakeRequest())
    assert exc_info.value.status_code == 403
    assert "permissions" in exc_info.value.detail.lower()


# ========== Phase 0: refresh 회전 실패 관측 로깅 ==========


class TestRefreshRotationObservation:
    """refresh 회전 거부 사건을 마스킹된 필드로만 남기는지 검증한다.

    관측 목적은 `reuse-detected` 가 같은 브라우저의 멀티탭 동시성에서 오는지, 다른
    환경에서 복사된 상태에서 오는지 구분하는 것이다.
    """

    RAW_EMAIL = "observed-user@example.com"
    RAW_UA = "Mozilla/5.0 (X11; Linux x86_64) ObservedBrowser/123.45"
    RAW_IP = "203.0.113.77"

    def _reject_refresh(self, auth_client, monkeypatch, caplog, status="reused"):
        monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": self.RAW_EMAIL, "role": "user", "name": "n", "picture": "p", "fid": "observed-family", "jti": "observed-jti"})
        monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
        monkeypatch.setattr(main_mod.refresh_token_store, "rotate", lambda **kwargs: status)
        monkeypatch.setattr(main_mod.refresh_token_store, "get_token_observation", lambda token_id: {"replaced_by_present": True, "revoked_at": 1, "revoke_reason": "rotated"})
        with caplog.at_level(logging.WARNING):
            resp = auth_client.post("/auth/refresh", cookies={main_mod.REFRESH_COOKIE_NAME: "raw-refresh-cookie-value"}, headers={"User-Agent": self.RAW_UA, "X-Forwarded-For": f"{self.RAW_IP}, 10.0.0.1"})
        return resp, caplog.text

    def test_emits_structured_masked_event(self, auth_client, monkeypatch, caplog):
        resp, text = self._reject_refresh(auth_client, monkeypatch, caplog)

        assert resp.status_code == 401
        assert "event=refresh-rotation-rejected" in text
        assert "status=reused" in text
        assert "replaced_by_present=true" in text
        # 모든 식별자는 HMAC 해시로만 남는다
        for field, raw in (("email_hash", self.RAW_EMAIL), ("family_hash", "observed-family"), ("jti_hash", "observed-jti"), ("request_user_agent_hash", self.RAW_UA), ("request_ip_prefix_hash", "203.0.113")):
            assert f"{field}={main_mod.observation_hash(raw)}" in text

    def test_never_logs_raw_identifiers(self, auth_client, monkeypatch, caplog):
        """원문 토큰·쿠키·전체 이메일·전체 IP·raw user-agent 는 로그에 남지 않는다."""
        _, text = self._reject_refresh(auth_client, monkeypatch, caplog)

        assert self.RAW_EMAIL not in text
        assert self.RAW_UA not in text
        assert self.RAW_IP not in text  # /24 프리픽스도 해시로만 기록
        assert "raw-refresh-cookie-value" not in text
        assert "observed-family" not in text
        assert "observed-jti" not in text

    def test_observation_lookup_failure_does_not_break_response(self, auth_client, monkeypatch, caplog):
        """관측 조회가 실패해도 refresh 응답 경로는 그대로 401 을 반환한다."""

        def _boom(token_id):
            raise RuntimeError("store down")

        monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": self.RAW_EMAIL, "role": "user", "name": "n", "picture": "p", "fid": "F", "jti": "J"})
        monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
        monkeypatch.setattr(main_mod.refresh_token_store, "rotate", lambda **kwargs: "reused")
        monkeypatch.setattr(main_mod.refresh_token_store, "get_token_observation", _boom)

        with caplog.at_level(logging.WARNING):
            resp = auth_client.post("/auth/refresh", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})

        assert resp.status_code == 401
        assert "event=refresh-rotation-rejected" in caplog.text
        assert "replaced_by_present=false" in caplog.text

    def test_successful_refresh_emits_no_observation_event(self, auth_client, monkeypatch, caplog):
        monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": self.RAW_EMAIL, "role": "user", "name": "n", "picture": "p", "fid": "F", "jti": "J"})
        monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
        monkeypatch.setattr(main_mod, "create_jwt_token", lambda **kwargs: "jwt")
        monkeypatch.setattr(main_mod, "create_refresh_token", lambda **kwargs: "rjwt")
        monkeypatch.setattr(main_mod.refresh_token_store, "rotate", lambda **kwargs: "ok")

        with caplog.at_level(logging.WARNING):
            resp = auth_client.post("/auth/refresh", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})

        assert resp.status_code == 200
        assert "event=refresh-rotation-rejected" not in caplog.text


def _ip_request(headers, client_host: str | None = "192.0.2.9"):
    scope_headers = [(k.lower().encode(), v.encode()) for k, v in headers.items()]
    scope = {"type": "http", "headers": scope_headers, "client": (client_host, 1234) if client_host else None}
    return Request(scope)


class TestClientIp:
    """실제 접속 IP 는 경유 프록시(Cloudflare edge, Traefik) IP 가 아니어야 한다."""

    def test_prefers_cf_connecting_ip_over_proxy_hops(self):
        """운영 경로에서 XFF 첫 항목은 Cloudflare edge IP 라 신뢰할 수 없다."""
        req = _ip_request({"CF-Connecting-IP": "203.0.113.77", "X-Forwarded-For": "104.22.17.33"}, client_host="10.1.202.80")
        assert main_mod._client_ip(req) == "203.0.113.77"

    def test_falls_back_to_true_client_ip(self):
        req = _ip_request({"True-Client-IP": "203.0.113.78", "X-Forwarded-For": "104.22.17.33"})
        assert main_mod._client_ip(req) == "203.0.113.78"

    def test_falls_back_to_forwarded_for_without_cloudflare(self):
        """Cloudflare 를 거치지 않는 경로에서는 XFF 첫 항목을 쓴다."""
        req = _ip_request({"X-Forwarded-For": "198.51.100.23, 10.1.2.3"})
        assert main_mod._client_ip(req) == "198.51.100.23"

    def test_falls_back_to_peer_when_no_headers(self):
        assert main_mod._client_ip(_ip_request({})) == "192.0.2.9"

    def test_ignores_blank_headers(self):
        req = _ip_request({"CF-Connecting-IP": "  ", "X-Forwarded-For": " , 10.1.2.3"})
        assert main_mod._client_ip(req) == "192.0.2.9"

    def test_returns_empty_when_no_client_info(self):
        assert main_mod._client_ip(_ip_request({}, client_host=None)) == ""

    def test_keeps_full_ipv6(self):
        req = _ip_request({"CF-Connecting-IP": "2001:db8:abcd:1234::1"})
        assert main_mod._client_ip(req) == "2001:db8:abcd:1234::1"


class TestRequestIpPrefix:
    """관측용 IP 프리픽스는 IPv4 /24, IPv6 /48 까지만 남긴다."""

    def _request(self, headers, client_host: str | None = "192.0.2.9"):
        return _ip_request(headers, client_host)

    def test_masks_cf_connecting_ip(self):
        """프리픽스도 프록시 IP 가 아니라 실제 클라이언트 IP 를 기준으로 만든다."""
        req = self._request({"CF-Connecting-IP": "203.0.113.77", "X-Forwarded-For": "104.22.17.33"})
        assert main_mod._request_ip_prefix(req) == "203.0.113"

    def test_uses_forwarded_for_first_hop_ipv4(self):
        req = self._request({"X-Forwarded-For": "198.51.100.23, 10.1.2.3"})
        assert main_mod._request_ip_prefix(req) == "198.51.100"

    def test_truncates_ipv6_to_48_bits(self):
        req = self._request({"X-Forwarded-For": "2001:db8:abcd:1234::1"})
        assert main_mod._request_ip_prefix(req) == "2001:db8:abcd"

    def test_falls_back_to_client_host(self):
        req = self._request({})
        assert main_mod._request_ip_prefix(req) == "192.0.2"

    def test_returns_empty_when_no_client_info(self):
        req = self._request({}, client_host=None)
        assert main_mod._request_ip_prefix(req) == ""


# ========== Admin 로그인 세션 관리 API ==========

FAMILY_A = "a" * 32
FAMILY_B = "b" * 32


def _session_item(family_id=FAMILY_A, **overrides):
    item = {"session_id": family_id, "session_label": f"{family_id[:8]}...", "email": "admin@example.com", "status": "active", "created_at": 1786500000, "last_seen_at": 1786501200, "expires_at": 1787106000, "revoked_at": None, "revoke_reason": None, "token_count": 6, "valid_token_count": 1, "is_current": False}
    item.update(overrides)
    return item


def _session_page(items):
    return {"items": items, "pagination": {"page": 1, "pageSize": 50, "totalItems": len(items), "totalPages": 1}, "summary": {"active": len(items), "expired": 0, "revoked": 0, "total": len(items)}}


class TestListLoginSessions:
    def test_returns_session_page(self, auth_client, monkeypatch):
        captured = {}

        def _list_sessions(**kwargs):
            captured.update(kwargs)
            return _session_page([_session_item(is_current=True)])

        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", _list_sessions)

        resp = auth_client.get("/auth/sessions")

        assert resp.status_code == 200
        body = resp.json()
        assert body["status"] == "success"
        assert body["result"]["items"][0]["is_current"] is True
        assert body["result"]["summary"]["total"] == 1
        # 기본값: 활성만, 1페이지 50건
        assert captured["status"] == "active"
        assert captured["page"] == 1
        assert captured["page_size"] == 50

    def test_passes_query_params_through(self, auth_client, monkeypatch):
        captured = {}

        def _list_sessions(**kwargs):
            captured.update(kwargs)
            return _session_page([])

        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", _list_sessions)

        resp = auth_client.get("/auth/sessions", params={"page": 2, "pageSize": 10, "status": "all", "email": "viewer@example.com"})

        assert resp.status_code == 200
        assert captured["page"] == 2
        assert captured["page_size"] == 10
        assert captured["status"] == "all"
        assert captured["email"] == "viewer@example.com"

    def test_marks_requester_session_as_current(self, auth_client, monkeypatch):
        captured = {}

        def _list_sessions(**kwargs):
            captured.update(kwargs)
            return _session_page([])

        monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"fid": FAMILY_A, "jti": "j"})
        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", _list_sessions)

        auth_client.get("/auth/sessions", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})

        assert captured["current_family_id"] == FAMILY_A

    def test_invalid_refresh_cookie_is_ignored(self, auth_client, monkeypatch):
        captured = {}

        def _boom(token):
            raise main_mod.HTTPException(status_code=401, detail="Invalid refresh token")

        monkeypatch.setattr(main_mod, "decode_refresh_token", _boom)
        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", lambda **kwargs: captured.update(kwargs) or _session_page([]))

        resp = auth_client.get("/auth/sessions", cookies={main_mod.REFRESH_COOKIE_NAME: "garbage"})

        # 잘못된 refresh 쿠키가 관리자 목록 조회를 막지 않는다
        assert resp.status_code == 200
        assert captured["current_family_id"] is None

    def test_rejects_unknown_status_filter(self, auth_client, monkeypatch):
        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", lambda **kwargs: _session_page([]))

        resp = auth_client.get("/auth/sessions", params={"status": "bogus"})

        assert resp.status_code == 400
        assert "status" in resp.json()["detail"]

    def test_rejects_out_of_range_pagination(self, auth_client, monkeypatch):
        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", lambda **kwargs: _session_page([]))

        assert auth_client.get("/auth/sessions", params={"page": 0}).status_code == 422
        assert auth_client.get("/auth/sessions", params={"pageSize": 0}).status_code == 422
        # pageSize 상한 100
        assert auth_client.get("/auth/sessions", params={"pageSize": 101}).status_code == 422
        assert auth_client.get("/auth/sessions", params={"pageSize": 100}).status_code == 200

    def test_store_failure_returns_503(self, auth_client, monkeypatch):
        def _boom(**kwargs):
            raise RuntimeError("db down")

        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", _boom)

        resp = auth_client.get("/auth/sessions")

        assert resp.status_code == 503

    def test_response_excludes_token_internals(self, auth_client, monkeypatch):
        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", lambda **kwargs: _session_page([_session_item()]))

        body = resp_text = auth_client.get("/auth/sessions").text

        assert "jti" not in body
        assert "replaced_by" not in resp_text
        assert main_mod.REFRESH_COOKIE_NAME not in body
        assert main_mod.ACCESS_COOKIE_NAME not in body


class TestRevokeLoginSession:
    def test_revokes_one_family(self, auth_client, monkeypatch):
        captured = {}

        def _revoke_session(**kwargs):
            captured.update(kwargs)
            return {"found": True, "revoked": True, "session": _session_item(status="revoked", revoke_reason="admin-revoked")}

        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", _revoke_session)

        resp = auth_client.delete(f"/auth/sessions/{FAMILY_A}")

        assert resp.status_code == 200
        result = resp.json()["result"]
        assert captured["family_id"] == FAMILY_A
        assert result["revoked"] is True
        assert result["revoked_current"] is False
        assert result["status"] == "revoked"
        assert result["revoke_reason"] == "admin-revoked"
        # 본인 세션이 아니면 쿠키를 지우지 않는다
        assert "Max-Age=0" not in resp.headers.get("set-cookie", "")

    def test_revoking_current_session_clears_cookies(self, auth_client, monkeypatch):
        monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"fid": FAMILY_A, "jti": "j"})
        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", lambda **kwargs: {"found": True, "revoked": True, "session": _session_item(status="revoked", revoke_reason="admin-revoked")})

        resp = auth_client.delete(f"/auth/sessions/{FAMILY_A}", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"})

        assert resp.status_code == 200
        assert resp.json()["result"]["revoked_current"] is True
        set_cookie = resp.headers.get("set-cookie", "")
        assert f"{main_mod.ACCESS_COOKIE_NAME}=" in set_cookie
        assert f"{main_mod.REFRESH_COOKIE_NAME}=" in set_cookie
        assert "Max-Age=0" in set_cookie

    def test_already_inactive_family_still_succeeds(self, auth_client, monkeypatch):
        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", lambda **kwargs: {"found": True, "revoked": False, "session": _session_item(status="revoked", revoke_reason="logout")})

        resp = auth_client.delete(f"/auth/sessions/{FAMILY_B}")

        assert resp.status_code == 200
        result = resp.json()["result"]
        assert result["revoked"] is False
        assert result["status"] == "revoked"
        assert result["revoke_reason"] == "logout"

    def test_unknown_family_returns_404(self, auth_client, monkeypatch):
        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", lambda **kwargs: {"found": False, "revoked": False, "session": None})

        resp = auth_client.delete(f"/auth/sessions/{FAMILY_B}")

        assert resp.status_code == 404

    def test_malformed_session_id_returns_400(self, auth_client, monkeypatch):
        called = []
        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", lambda **kwargs: called.append(kwargs) or {"found": True, "revoked": True, "session": _session_item()})

        for bad in ("short", "A" * 32, "z" * 32, "a" * 31, "a" * 33):
            resp = auth_client.delete(f"/auth/sessions/{bad}")
            assert resp.status_code == 400, bad
        # 형식 검증 실패 시 store 를 건드리지 않는다
        assert called == []

    def test_store_failure_returns_503(self, auth_client, monkeypatch):
        def _boom(**kwargs):
            raise RuntimeError("db down")

        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", _boom)

        assert auth_client.delete(f"/auth/sessions/{FAMILY_A}").status_code == 503

    def test_audit_log_masks_target_session(self, auth_client, monkeypatch, caplog):
        monkeypatch.setattr(main_mod.refresh_token_store, "revoke_session", lambda **kwargs: {"found": True, "revoked": True, "session": _session_item(status="revoked")})

        with caplog.at_level(logging.WARNING):
            auth_client.delete(f"/auth/sessions/{FAMILY_A}")

        assert f"session_hash={main_mod.observation_hash(FAMILY_A)}" in caplog.text
        # 대상 family_id 원문은 남기지 않는다
        assert FAMILY_A not in caplog.text


class TestLoginSessionAuthorization:
    """세션 관리 엔드포인트는 admin 전용이다."""

    @pytest.fixture()
    def viewer_client(self):
        main_mod.app.dependency_overrides[main_mod.require_auth] = lambda: {"email": "viewer@example.com", "role": "viewer"}
        with TestClient(main_mod.app) as c:
            yield c
        main_mod.app.dependency_overrides.clear()

    def test_viewer_is_forbidden(self, viewer_client):
        # require_admin 은 override 하지 않았으므로 실제 의존성이 동작한다
        assert viewer_client.get("/auth/sessions").status_code in (401, 403)
        assert viewer_client.delete(f"/auth/sessions/{FAMILY_A}").status_code in (401, 403)

    def test_unauthenticated_is_rejected(self):
        main_mod.app.dependency_overrides.clear()
        with TestClient(main_mod.app) as c:
            assert c.get("/auth/sessions").status_code == 401
            assert c.delete(f"/auth/sessions/{FAMILY_A}").status_code == 401

    def test_endpoints_declare_admin_dependency(self):
        """엔드포인트가 require_admin 의존성을 명시적으로 갖는지 확인한다."""
        routes = {(r.path, tuple(sorted(r.methods))): r for r in main_mod.app.routes if getattr(r, "path", "").startswith("/auth/sessions")}
        assert routes, "세션 관리 라우트를 찾지 못했습니다"
        for (path, methods), route in routes.items():
            dependency_calls = [d.call for d in route.dependant.dependencies]
            assert main_mod.require_admin in dependency_calls, f"{methods} {path} 에 require_admin 이 없습니다"


class TestLoginSessionClientInfo:
    """로그인·갱신 시점의 실제 접속 IP 와 User-Agent 가 세션 저장소까지 전달되는지 본다."""

    REAL_IP = "203.0.113.77"
    PROXY_IP = "104.22.17.33"  # Cloudflare edge. XFF 로만 오면 이 값이 잡힌다.
    UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"

    @property
    def _headers(self):
        return {"CF-Connecting-IP": self.REAL_IP, "X-Forwarded-For": self.PROXY_IP, "User-Agent": self.UA}

    def test_google_login_records_real_client_ip(self, auth_client, monkeypatch):
        captured: dict = {}
        monkeypatch.setattr(main_mod, "TM_GOOGLE_CLIENT_ID", "cid")
        monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda credential, request, audience: {"aud": audience, "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": True, "name": "N", "picture": "P"})
        monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
        monkeypatch.setattr(main_mod, "create_jwt_token", lambda **kwargs: "jwt")
        monkeypatch.setattr(main_mod, "create_refresh_token", lambda **kwargs: "rjwt")
        monkeypatch.setattr(main_mod.refresh_token_store, "store_issued", lambda **kwargs: captured.update(kwargs))

        resp = auth_client.post("/auth/google", json={"credential": "c"}, headers=self._headers)

        assert resp.status_code == 200
        assert captured["client_ip"] == self.REAL_IP
        assert captured["client_ip"] != self.PROXY_IP
        assert captured["user_agent"] == self.UA

    def test_refresh_updates_client_info(self, auth_client, monkeypatch):
        """세션의 IP/UA 는 마지막 갱신 값이므로 refresh 마다 다시 전달돼야 한다."""
        captured: dict = {}

        def _rotate(**kwargs):
            captured.update(kwargs)
            return "ok"

        monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": "e@example.com", "role": "user", "name": "n", "picture": "p", "fid": "F", "jti": "J"})
        monkeypatch.setattr(main_mod, "determine_role", lambda email: "user")
        monkeypatch.setattr(main_mod, "create_jwt_token", lambda **kwargs: "jwt")
        monkeypatch.setattr(main_mod, "create_refresh_token", lambda **kwargs: "rjwt")
        monkeypatch.setattr(main_mod.refresh_token_store, "rotate", _rotate)

        resp = auth_client.post("/auth/refresh", cookies={main_mod.REFRESH_COOKIE_NAME: "tok"}, headers=self._headers)

        assert resp.status_code == 200
        assert captured["client_ip"] == self.REAL_IP
        assert captured["user_agent"] == self.UA

    def test_sessions_api_exposes_client_info(self, auth_client, monkeypatch):
        session = {"session_id": "a" * 32, "session_label": "aaaaaaaa...", "email": "admin@example.com", "status": "active", "created_at": 1, "last_seen_at": 2, "expires_at": 3, "revoked_at": None, "revoke_reason": None, "token_count": 1, "valid_token_count": 1, "client_ip": self.REAL_IP, "user_agent": self.UA, "user_agent_summary": "Chrome 131 / macOS 10.15", "is_current": False}
        monkeypatch.setattr(main_mod.refresh_token_store, "list_sessions", lambda **kwargs: {"items": [session], "pagination": {"page": 1, "pageSize": 50, "totalItems": 1, "totalPages": 1}, "summary": {"active": 1, "expired": 0, "revoked": 0, "total": 1}})

        item = auth_client.get("/auth/sessions").json()["result"]["items"][0]

        assert item["client_ip"] == self.REAL_IP
        assert item["user_agent"] == self.UA
        assert item["user_agent_summary"] == "Chrome 131 / macOS 10.15"
