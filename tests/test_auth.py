import os
from unittest.mock import patch
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


def test_auth_requires_jwt_secret():
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
