import os
import time

import pytest
import jwt


@pytest.fixture(autouse=True)
def setup_env(monkeypatch):
    monkeypatch.setenv("TM_JWT_SECRET", "testsecret123")
    monkeypatch.setenv("TM_ADMIN_EMAIL", "admin@example.com")
    monkeypatch.setenv("TM_ALLOWED_EMAILS", "viewer1@example.com,viewer2@example.com")
    # 모듈 재로드하여 환경변수 반영
    import importlib
    import backend.auth as auth_mod

    importlib.reload(auth_mod)
    yield auth_mod


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


class TestCreateJwtToken:
    def test_token_contains_claims(self, setup_env):
        token = setup_env.create_jwt_token(
            "admin@example.com", "admin", "Admin", "pic.jpg"
        )
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
        token = setup_env.create_refresh_token("admin@example.com", "admin")
        payload = jwt.decode(token, "testsecret123", algorithms=["HS256"])
        assert payload["email"] == "admin@example.com"
        assert payload["role"] == "admin"
        assert payload["type"] == "refresh"
        assert "name" not in payload
        assert "picture" not in payload

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

        expired_payload = {
            "type": "refresh",
            "email": "admin@example.com",
            "role": "admin",
            "exp": int(time.time()) - 100,
            "iat": int(time.time()) - 200,
        }
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


class TestExtractPayload:
    def _make_request(self, token=None):
        class FakeRequest:
            def __init__(self, token):
                self.headers = {}
                if token:
                    self.headers["Authorization"] = f"Bearer {token}"

        return FakeRequest(token)

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

    def test_expired_token(self, setup_env):
        from fastapi import HTTPException

        expired_payload = {
            "type": "access",
            "email": "admin@example.com",
            "role": "admin",
            "exp": int(time.time()) - 100,
            "iat": int(time.time()) - 200,
        }
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


class TestRequireAuth:
    def _make_request(self, token):
        class FakeRequest:
            def __init__(self, token):
                self.headers = {"Authorization": f"Bearer {token}"} if token else {}

        return FakeRequest(token)

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
    async def test_invalid_role_fails(self, setup_env):
        from fastapi import HTTPException

        token = setup_env.create_jwt_token("nobody@example.com", "unknown")
        req = self._make_request(token)
        with pytest.raises(HTTPException) as exc_info:
            await setup_env.require_auth(req)
        assert exc_info.value.status_code == 403


class TestRequireAdmin:
    def _make_request(self, token):
        class FakeRequest:
            def __init__(self, token):
                self.headers = {"Authorization": f"Bearer {token}"} if token else {}

        return FakeRequest(token)

    @pytest.mark.asyncio
    async def test_admin_passes(self, setup_env):
        token = setup_env.create_jwt_token("admin@example.com", "admin")
        req = self._make_request(token)
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
