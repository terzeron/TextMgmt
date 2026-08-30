"""Route-level mock tests for backend/main.py — no ES/MySQL required."""

import importlib
import time
import pytest
from unittest.mock import AsyncMock, MagicMock, patch
from fastapi.responses import Response
from fastapi.testclient import TestClient

import backend.main as main_module
from backend.auth import create_refresh_token

ADMIN_PAYLOAD = {"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": int(time.time()) + 3600}
VIEWER_PAYLOAD = {"email": "viewer@test.com", "role": "viewer", "name": "Viewer", "picture": "", "exp": int(time.time()) + 3600}

BOOK_DICT = {"book_id": 1, "category": "_epub", "title": "Test Book", "author": "Test Author", "file_path": "_epub/test.epub", "file_type": "epub", "file_size": 1024, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0}


def _make_book(overrides=None):
    b = MagicMock()
    d = {**BOOK_DICT, **(overrides or {})}
    b.dict.return_value = d
    b.book_id = d["book_id"]
    b.file_type = d["file_type"]
    return b


def _get_main_module():
    return importlib.import_module("backend.main")


@pytest.fixture(autouse=True)
def override_auth():
    global main_module
    main_module = _get_main_module()
    main_module.app.dependency_overrides[main_module.require_auth] = lambda: ADMIN_PAYLOAD
    main_module.app.dependency_overrides[main_module.require_admin] = lambda: ADMIN_PAYLOAD
    yield
    main_module.app.dependency_overrides.clear()


@pytest.fixture
def client():
    return TestClient(main_module.app)


@pytest.fixture
def mock_bm():
    """Inject an AsyncMock into the book_manager _LazyProxy without triggering BookManager()."""
    m = AsyncMock()
    m.es_manager = MagicMock()
    prev = main_module.book_manager._instance
    object.__setattr__(main_module.book_manager, "_instance", m)
    yield m
    object.__setattr__(main_module.book_manager, "_instance", prev)


@pytest.fixture
def mock_cat():
    """Inject a MagicMock into the category_mapping _LazyProxy (all methods are sync via to_thread)."""
    m = MagicMock()
    prev = main_module.category_mapping._instance
    object.__setattr__(main_module.category_mapping, "_instance", m)
    yield m
    object.__setattr__(main_module.category_mapping, "_instance", prev)


# ── /wake ────────────────────────────────────────────────────────────────────


class TestWake:
    def test_success(self, client, monkeypatch):
        monkeypatch.setenv("TM_FRONTEND_URL", "http://testserver")
        with patch("os.listdir", return_value=["a", "b", "c"]):
            r = client.get("/wake")
        assert r.status_code == 200
        assert r.json() == {"status": "success"}

    def test_failure(self, client, monkeypatch):
        monkeypatch.setenv("TM_FRONTEND_URL", "http://testserver")
        with patch("os.listdir", side_effect=OSError("not mounted")):
            r = client.get("/wake")
        assert r.status_code == 503
        data = r.json()
        assert data == {"status": "failure"}

    def test_non_frontend_host_is_hidden(self, client, monkeypatch):
        monkeypatch.setenv("TM_FRONTEND_URL", "https://tm.terzeron.com")
        r = client.get("/wake")
        assert r.status_code == 404


# ── /validate/{book_id} ──────────────────────────────────────────────────────


class TestValidateBook:
    def test_epub_success(self, client, mock_bm):
        mock_bm.get_book.return_value = (_make_book({"file_type": "epub"}), None)
        mock_bm.validate_epub.return_value = ({"valid": True}, None)

        r = client.get("/validate/1")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": {"valid": True}}

    def test_pdf_success(self, client, mock_bm):
        mock_bm.get_book.return_value = (_make_book({"file_type": "pdf"}), None)
        mock_bm.validate_pdf.return_value = ({"pages": 5}, None)

        r = client.get("/validate/1")
        assert r.status_code == 200
        assert r.json()["status"] == "success"
        assert r.json()["result"] == {"pages": 5}

    def test_unsupported_type(self, client, mock_bm):
        mock_bm.get_book.return_value = (_make_book({"file_type": "txt"}), None)

        r = client.get("/validate/1")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "failure"
        assert "txt" in data["error"]

    def test_book_not_found(self, client, mock_bm):
        mock_bm.get_book.return_value = (None, "Book not found: 99")

        r = client.get("/validate/99")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "failure"
        assert "99" in data["error"]

    def test_epub_validation_error(self, client, mock_bm):
        mock_bm.get_book.return_value = (_make_book({"file_type": "epub"}), None)
        mock_bm.validate_epub.return_value = (None, "invalid epub structure")

        r = client.get("/validate/1")
        assert r.status_code == 200
        assert r.json()["status"] == "failure"
        assert "invalid epub structure" in r.json()["error"]


# ── /preview/{book_id} ───────────────────────────────────────────────────────


class TestPreview:
    def test_returns_response(self, client, mock_bm):
        mock_bm.get_book_preview.return_value = Response(content=b"<html/>", media_type="text/html")

        r = client.get("/preview/1")
        assert r.status_code == 200

    def test_default_params(self, client, mock_bm):
        mock_bm.get_book_preview.return_value = Response(content=b"preview")
        client.get("/preview/1")
        mock_bm.get_book_preview.assert_called_once_with(book_id=1, pages=5, chapters=10, resource_base_url="/html-resource/1")

    def test_custom_params(self, client, mock_bm):
        mock_bm.get_book_preview.return_value = Response(content=b"preview")
        client.get("/preview/1?pages=10&chapters=5")
        mock_bm.get_book_preview.assert_called_once_with(book_id=1, pages=10, chapters=5, resource_base_url="/html-resource/1")

    def test_html_resource_proxy(self, client, mock_bm):
        mock_bm.get_html_resource.return_value = Response(content=b"body{}", media_type="text/css")
        r = client.get("/html-resource/1?path=style.css")
        assert r.status_code == 200
        mock_bm.get_html_resource.assert_called_once_with(book_id=1, resource_path="style.css")


class TestViewerHiddenAccess:
    def test_viewer_hidden_book_download_forbidden(self, client, mock_bm, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_hidden_categories.return_value = ["secret"]
        mock_bm.get_book.return_value = (_make_book({"category": "secret"}), None)

        r = client.get("/download/1")

        assert r.status_code == 403
        assert r.json()["detail"] == "접근 권한이 없는 카테고리입니다."
        mock_bm.get_book_content.assert_not_called()

    def test_viewer_hidden_category_listing_forbidden(self, client, mock_bm, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_hidden_categories.return_value = ["secret"]

        r = client.get("/categories/secret/sub")

        assert r.status_code == 403
        assert r.json()["detail"] == "접근 권한이 없는 카테고리입니다."
        mock_bm.get_books_in_category.assert_not_called()

    def test_viewer_categories_filter_hidden_entries(self, client, mock_bm, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_hidden_categories.return_value = ["secret"]
        mock_bm.get_categories.return_value = ({"public": 1, "secret": 2, "secret/sub": 3}, None)

        r = client.get("/categories")

        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": {"public": 1}}

    def test_viewer_search_merges_hidden_categories(self, client, mock_bm, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_hidden_categories.return_value = ["secret", "secret/sub"]
        mock_bm.search_by_keyword_paged.return_value = ([], 0, None)

        r = client.get("/search/test?exclude_categories=public,secret")

        assert r.status_code == 200
        mock_bm.search_by_keyword_paged.assert_called_once_with("test", size=10, offset=0, exclude_categories=["public", "secret", "secret/sub"])

    def test_latest_uses_latest_excluded_categories(self, client, mock_bm, mock_cat):
        mock_cat.get_latest_excluded_categories.return_value = ["no_latest"]
        mock_bm.get_latest_books.return_value = ([], 0, None)

        r = client.get("/latest?limit=5")

        assert r.status_code == 200
        mock_bm.get_latest_books.assert_called_once_with(size=5, exclude_categories=["no_latest"])

    def test_viewer_latest_merges_hidden_and_latest_excluded_categories(self, client, mock_bm, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_hidden_categories.return_value = ["secret", "shared"]
        mock_cat.get_latest_excluded_categories.return_value = ["no_latest", "shared"]
        mock_bm.get_latest_books.return_value = ([], 0, None)

        r = client.get("/latest")

        assert r.status_code == 200
        mock_bm.get_latest_books.assert_called_once_with(size=100, exclude_categories=["secret", "shared", "no_latest"])


# ── /pdf-pages/{book_id} ─────────────────────────────────────────────────────


class TestPdfPages:
    def test_returns_response(self, client, mock_bm):
        mock_bm.get_pdf_pages.return_value = Response(content=b"%PDF", media_type="application/pdf")

        r = client.get("/pdf-pages/1?start=2&end=4")
        assert r.status_code == 200

    def test_default_params(self, client, mock_bm):
        mock_bm.get_pdf_pages.return_value = Response(content=b"%PDF")
        client.get("/pdf-pages/1")
        mock_bm.get_pdf_pages.assert_called_once_with(book_id=1, start=1, end=1)


# ── /category-mismatches admin endpoints ─────────────────────────────────────


class TestCategoryMismatchAdmin:
    def test_index_file_success(self, client, mock_bm):
        mock_bm.index_single_file.return_value = (42, None)
        r = client.post("/category-mismatches/index-file", json={"file_path": "_epub/test.epub"})
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": {"book_id": 42}}

    def test_index_file_missing_path(self, client, mock_bm):
        r = client.post("/category-mismatches/index-file", json={})
        assert r.status_code == 400

    def test_index_file_error(self, client, mock_bm):
        mock_bm.index_single_file.return_value = (None, "file not found")
        r = client.post("/category-mismatches/index-file", json={"file_path": "bad.epub"})
        assert r.status_code == 200
        assert r.json()["status"] == "failure"
        assert "file not found" in r.json()["error"]

    def test_delete_file_success(self, client, mock_bm):
        mock_bm.delete_file.return_value = ("Ok", None)
        r = client.post("/category-mismatches/delete-file", json={"file_path": "_epub/old.epub"})
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": "Ok"}

    def test_delete_file_missing_path(self, client, mock_bm):
        r = client.post("/category-mismatches/delete-file", json={})
        assert r.status_code == 400

    def test_delete_file_error(self, client, mock_bm):
        mock_bm.delete_file.return_value = (None, "permission denied")
        r = client.post("/category-mismatches/delete-file", json={"file_path": "bad.epub"})
        assert r.status_code == 200
        assert r.json()["status"] == "failure"

    def test_delete_es_doc_success(self, client, mock_bm):
        mock_bm.es_manager.delete.return_value = True
        r = client.delete("/category-mismatches/es-doc/1")
        assert r.status_code == 200
        assert r.json()["status"] == "success"

    def test_delete_es_doc_failure(self, client, mock_bm):
        mock_bm.es_manager.delete.return_value = False
        r = client.delete("/category-mismatches/es-doc/1")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "failure"
        assert "1" in data["error"]

    def test_reload_success(self, client, mock_bm):
        mock_bm.reload_category.return_value = ({"reloaded": 3}, None)
        r = client.post("/category-mismatches/reload", json={"category": "_epub"})
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": {"reloaded": 3}}

    def test_reload_failure(self, client, mock_bm):
        mock_bm.reload_category.return_value = (None, "reload failed")
        r = client.post("/category-mismatches/reload", json={"category": "_epub"})
        assert r.status_code == 200
        assert r.json()["status"] == "failure"
        assert "reload failed" in r.json()["error"]

    def test_get_details_success(self, client, mock_bm):
        details = {"es": ["a.epub"], "fs": ["b.epub"]}
        # get_category_mismatch_details is called via asyncio.to_thread → must be synchronous
        mock_bm.get_category_mismatch_details = MagicMock(return_value=details)
        r = client.get("/category-mismatches/_epub")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": details}

    def test_get_details_exception(self, client, mock_bm):
        mock_bm.get_category_mismatch_details = MagicMock(side_effect=RuntimeError("ES down"))
        r = client.get("/category-mismatches/_epub")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "failure"
        assert data["error"] == main_module.GENERIC_MISMATCH_ERROR


# ── /search/bookstore/{store_name} ───────────────────────────────────────────


class TestSearchBookstore:
    def test_unknown_store(self, client):
        r = client.get("/search/bookstore/unknown?title=test")
        assert r.status_code == 404

    def test_missing_all_params(self, client):
        r = client.get("/search/bookstore/yes24")
        assert r.status_code == 400

    def test_yes24_with_results(self, client):
        fake_store = MagicMock()
        fake_store.search.return_value = ([("Book A", "Author A", "소설", "http://url", None, "9781234567890")], "Book A", "title")
        fake_store.build_search_url.return_value = "http://search"
        with patch("backend.main.Yes24Bookstore", return_value=fake_store):
            r = client.get("/search/bookstore/yes24?title=Book+A")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "success"
        assert data["result"][0]["title"] == "Book A"
        assert data["result"][0]["isbn"] == "9781234567890"

    def test_yes24_empty_results(self, client):
        fake_store = MagicMock()
        fake_store.search.return_value = ([], "NoTitle", "title")
        fake_store.build_search_url.return_value = "http://search"
        with patch("backend.main.Yes24Bookstore", return_value=fake_store):
            r = client.get("/search/bookstore/yes24?title=NoTitle")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "not_found"
        assert data["result"] == []

    def test_result_without_isbn(self, client):
        fake_store = MagicMock()
        fake_store.search.return_value = ([("Book B", "Author B", "소설", "http://url2", None, "")], "Book B", "title")
        fake_store.build_search_url.return_value = ""
        with patch("backend.main.AladinBookstore", return_value=fake_store):
            r = client.get("/search/bookstore/aladin?title=Book+B")
        assert r.status_code == 200
        assert "isbn" not in r.json()["result"][0]

    @pytest.mark.parametrize("store,cls", [("ridi", "RidibooksBookstore"), ("naver", "NaverShoppingBookstore"), ("naverseries", "NaverSeriesBookstore"), ("munpia", "MunpiaBookstore")])
    def test_other_stores(self, client, store, cls):
        fake_store = MagicMock()
        fake_store.search.return_value = ([], "q", "title")
        fake_store.build_search_url.return_value = ""
        with patch(f"backend.main.{cls}", return_value=fake_store):
            r = client.get(f"/search/bookstore/{store}?title=q")
        assert r.status_code == 200
        assert r.json()["status"] == "not_found"


# ── /auth/logout, /auth/me ───────────────────────────────────────────────────


class TestAuthLogoutAndMe:
    def test_logout(self, client):
        r = client.post("/auth/logout")
        assert r.status_code == 200
        assert r.json() == {"status": "success"}

    def test_auth_me_returns_payload(self, client):
        r = client.get("/auth/me")
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "success"
        assert data["result"]["email"] == ADMIN_PAYLOAD["email"]
        assert data["result"]["role"] == ADMIN_PAYLOAD["role"]
        assert data["result"]["name"] == ADMIN_PAYLOAD["name"]
        assert "expires_in" in data["result"]


# ── /auth/refresh ────────────────────────────────────────────────────────────


class TestAuthRefresh:
    def test_missing_token_returns_400(self, client):
        r = client.post("/auth/refresh")
        assert r.status_code == 400

    def test_invalid_token_returns_401(self, client):
        r = client.post("/auth/refresh", cookies={"tm_refresh_token": "bad.token.here"})
        assert r.status_code == 401

    def test_success(self, client):
        token = create_refresh_token(email="admin@test.com", role="admin", name="Admin")
        with patch("backend.main.determine_role", return_value="admin"), patch("backend.main.refresh_token_store.rotate", return_value="ok"):
            r = client.post("/auth/refresh", cookies={"tm_refresh_token": token})
        assert r.status_code == 200
        assert r.json()["status"] == "success"
        assert "expires_in" in r.json()

    def test_unauthorized_email_returns_403(self, client):
        token = create_refresh_token(email="stranger@other.com", role="viewer", name="X")
        r = client.post("/auth/refresh", cookies={"tm_refresh_token": token})
        assert r.status_code == 403

    def test_rotation_rejection_clears_cookies(self, client):
        token = create_refresh_token(email="admin@test.com", role="admin", name="Admin")
        with patch("backend.main.determine_role", return_value="admin"), patch("backend.main.refresh_token_store.rotate", return_value="reused"):
            r = client.post("/auth/refresh", cookies={"tm_refresh_token": token})
        assert r.status_code == 401
        assert r.json()["detail"] == "Invalid refresh token state"
        cookies = r.headers.get_list("set-cookie")
        assert any("tm_access_token=" in cookie and "Max-Age=0" in cookie for cookie in cookies)
        assert any("tm_refresh_token=" in cookie and "Max-Age=0" in cookie for cookie in cookies)

    def test_db_error_in_rotate_returns_503(self, client):
        """refresh_token_store.rotate()가 DB 예외를 던지면 503을 반환한다."""
        token = create_refresh_token(email="admin@test.com", role="admin", name="Admin")
        with patch("backend.main.determine_role", return_value="admin"), patch("backend.main.refresh_token_store.rotate", side_effect=Exception("DB connection lost")):
            r = client.post("/auth/refresh", cookies={"tm_refresh_token": token})
        assert r.status_code == 503

    def test_legacy_refresh_token_without_claims_returns_401(self, client):
        """fid/jti 없는 구형 refresh token은 401과 함께 재로그인 안내 메시지를 반환한다."""
        import jwt as pyjwt

        legacy_payload = {"type": "refresh", "email": "admin@test.com", "role": "admin", "exp": int(time.time()) + 3600, "iat": int(time.time())}
        secret = __import__("os").environ.get("TM_JWT_SECRET", "test_jwt_secret_for_testing_minimum_32bytes")
        legacy_token = pyjwt.encode(legacy_payload, secret, algorithm="HS256")
        r = client.post("/auth/refresh", cookies={"tm_refresh_token": legacy_token})
        assert r.status_code == 401
        assert "log in again" in r.json().get("detail", "").lower()


# ── /auth/google ─────────────────────────────────────────────────────────────


class TestAuthGoogle:
    def test_missing_credential_returns_400(self, client):
        r = client.post("/auth/google", json={})
        assert r.status_code == 400

    def test_google_api_error_returns_401(self, client):
        with patch("backend.main.google_id_token.verify_oauth2_token", side_effect=ValueError("bad")):
            r = client.post("/auth/google", json={"credential": "bad_token"})
        assert r.status_code == 401

    def test_missing_google_client_id_returns_500(self, client):
        with patch("backend.main.TM_GOOGLE_CLIENT_ID", None):
            r = client.post("/auth/google", json={"credential": "token"})
        assert r.status_code == 500

    def test_issuer_mismatch_returns_401(self, client):
        payload = {"aud": "expected_client_id", "iss": "https://evil.example.com", "email": "admin@test.com", "email_verified": True}
        with patch("backend.main.google_id_token.verify_oauth2_token", return_value=payload), patch("backend.main.TM_GOOGLE_CLIENT_ID", "expected_client_id"):
            r = client.post("/auth/google", json={"credential": "token"})
        assert r.status_code == 401

    def test_unverified_email_returns_401(self, client):
        payload = {"aud": "cid", "iss": "https://accounts.google.com", "email": "admin@test.com", "email_verified": False}
        with patch("backend.main.google_id_token.verify_oauth2_token", return_value=payload), patch("backend.main.TM_GOOGLE_CLIENT_ID", "cid"):
            r = client.post("/auth/google", json={"credential": "token"})
        assert r.status_code == 401

    def test_unauthorized_email_returns_403(self, client):
        payload = {"aud": "cid", "iss": "https://accounts.google.com", "email": "stranger@other.com", "email_verified": True, "name": "X", "picture": ""}
        with patch("backend.main.google_id_token.verify_oauth2_token", return_value=payload), patch("backend.main.TM_GOOGLE_CLIENT_ID", "cid"):
            r = client.post("/auth/google", json={"credential": "token"})
        assert r.status_code == 403

    def test_success(self, client):
        payload = {"aud": "cid", "iss": "https://accounts.google.com", "email": "admin@test.com", "email_verified": True, "name": "Admin", "picture": "http://pic"}
        with patch("backend.main.google_id_token.verify_oauth2_token", return_value=payload), patch("backend.main.TM_GOOGLE_CLIENT_ID", "cid"), patch("backend.main.determine_role", return_value="admin"):
            r = client.post("/auth/google", json={"credential": "token"})
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "success"
        assert data["email"] == "admin@test.com"
        assert data["role"] == "admin"
        assert "expires_in" in data


class TestAuthLogout:
    def test_logout_revokes_refresh_family_when_cookie_is_present(self, client):
        token = create_refresh_token(email="admin@test.com", role="admin", name="Admin")
        with patch("backend.main.refresh_token_store.revoke_family") as revoke_family:
            r = client.post("/auth/logout", cookies={"tm_refresh_token": token})
        assert r.status_code == 200
        revoke_family.assert_called_once()


# ── /category-mappings ───────────────────────────────────────────────────────


class TestCategoryMappings:
    def test_get_all(self, client, mock_cat):
        mock_cat.get_all_mappings.return_value = {"소설": ["fantasy", "romance"]}
        r = client.get("/category-mappings")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": {"소설": ["fantasy", "romance"]}}

    def test_get_keywords(self, client, mock_cat):
        mock_cat.get_keywords.return_value = ["fantasy"]
        r = client.get("/category-mappings/소설")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": ["fantasy"]}

    def test_set_keywords_success(self, client, mock_cat):
        mock_cat.set_keywords.return_value = True
        mock_cat.get_keywords.return_value = ["drama"]
        r = client.put("/category-mappings/소설", json={"keywords": ["drama"]})
        assert r.status_code == 200
        assert r.json()["status"] == "success"
        assert r.json()["result"] == ["drama"]

    def test_set_keywords_failure_returns_500(self, client, mock_cat):
        mock_cat.set_keywords.return_value = False
        r = client.put("/category-mappings/소설", json={"keywords": ["drama"]})
        assert r.status_code == 500

    def test_add_keyword_missing_returns_400(self, client, mock_cat):
        r = client.post("/category-mappings/소설/keywords", json={})
        assert r.status_code == 400

    def test_add_keyword_success(self, client, mock_cat):
        mock_cat.add_keyword.return_value = True
        mock_cat.get_keywords.return_value = ["fantasy", "sci-fi"]
        r = client.post("/category-mappings/소설/keywords", json={"keyword": "sci-fi"})
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "success"
        assert "sci-fi" in data["result"]

    def test_add_keyword_duplicate(self, client, mock_cat):
        mock_cat.add_keyword.return_value = False
        mock_cat.get_keywords.return_value = ["fantasy"]
        r = client.post("/category-mappings/소설/keywords", json={"keyword": "fantasy"})
        assert r.status_code == 200
        assert r.json()["status"] == "duplicate"

    def test_remove_keyword_success(self, client, mock_cat):
        mock_cat.remove_keyword.return_value = True
        mock_cat.get_keywords.return_value = []
        r = client.delete("/category-mappings/소설/keywords/fantasy")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": []}

    def test_remove_keyword_not_found_returns_404(self, client, mock_cat):
        mock_cat.remove_keyword.return_value = False
        r = client.delete("/category-mappings/소설/keywords/nonexistent")
        assert r.status_code == 404

    def test_delete_category_mapping_success(self, client, mock_cat):
        mock_cat.delete_category.return_value = True
        r = client.delete("/category-mappings/소설")
        assert r.status_code == 200
        assert r.json() == {"status": "success"}

    def test_delete_category_mapping_not_found_returns_404(self, client, mock_cat):
        mock_cat.delete_category.return_value = False
        r = client.delete("/category-mappings/없는카테고리")
        assert r.status_code == 404

    def test_update_all_success(self, client, mock_cat):
        mock_cat.update_all_mappings.return_value = True
        mock_cat.get_all_mappings.return_value = {"소설": ["a"]}
        r = client.put("/category-mappings", json={"mappings": {"소설": ["a"]}})
        assert r.status_code == 200
        assert r.json()["status"] == "success"

    def test_update_all_failure_returns_500(self, client, mock_cat):
        mock_cat.update_all_mappings.return_value = False
        r = client.put("/category-mappings", json={"mappings": {"소설": ["a"]}})
        assert r.status_code == 500


# ── /hidden-categories ───────────────────────────────────────────────────────


class TestHiddenCategories:
    def test_get(self, client, mock_cat):
        mock_cat.get_hidden_categories.return_value = ["_draft", "_archive"]
        r = client.get("/hidden-categories")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": ["_draft", "_archive"]}

    def test_get_viewer_hides_hidden_category_names(self, client, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_hidden_categories.return_value = ["_draft", "_archive"]

        r = client.get("/hidden-categories")

        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": []}
        mock_cat.get_hidden_categories.assert_not_called()

    def test_set_hidden_success(self, client, mock_cat):
        mock_cat.set_hidden.return_value = True
        mock_cat.get_hidden_categories.return_value = ["_draft"]
        r = client.post("/hidden-categories/_draft", json={"hidden": True})
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "success"
        assert "_draft" in data["result"]

    def test_set_hidden_failure_returns_500(self, client, mock_cat):
        mock_cat.set_hidden.return_value = False
        r = client.post("/hidden-categories/_draft", json={"hidden": True})
        assert r.status_code == 500


class TestLatestExcludedCategories:
    def test_get(self, client, mock_cat):
        mock_cat.get_latest_excluded_categories.return_value = ["_draft", "_archive"]
        r = client.get("/latest-excluded-categories")
        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": ["_draft", "_archive"]}

    def test_get_viewer_hides_category_names(self, client, mock_cat):
        main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD
        mock_cat.get_latest_excluded_categories.return_value = ["_draft", "_archive"]

        r = client.get("/latest-excluded-categories")

        assert r.status_code == 200
        assert r.json() == {"status": "success", "result": []}
        mock_cat.get_latest_excluded_categories.assert_not_called()

    def test_set_latest_excluded_success(self, client, mock_cat):
        mock_cat.set_latest_excluded.return_value = True
        mock_cat.get_latest_excluded_categories.return_value = ["_draft"]
        r = client.post("/latest-excluded-categories/_draft", json={"excluded": True})
        assert r.status_code == 200
        data = r.json()
        assert data["status"] == "success"
        assert "_draft" in data["result"]

    def test_set_latest_excluded_failure_returns_500(self, client, mock_cat):
        mock_cat.set_latest_excluded.return_value = False
        r = client.post("/latest-excluded-categories/_draft", json={"excluded": True})
        assert r.status_code == 500


# ── utility function unit tests ──────────────────────────────────────────────


class TestSummarizeRequestBody:
    """_summarize_request_body 유틸리티 함수 (main.py:138-143)."""

    def test_list_body(self):
        result = main_module._summarize_request_body([1, 2, 3])
        assert result == {"type": "list", "length": 3}

    def test_none_body(self):
        result = main_module._summarize_request_body(None)
        assert result == {"type": "none"}

    def test_str_body(self):
        result = main_module._summarize_request_body("hello")
        assert result == {"type": "str", "length": 5}


def test_is_local_frontend_origin_remote_returns_false():
    """원격 호스트는 False (main.py:148)."""
    assert main_module._is_local_frontend_origin("https://remote.example.com") is False


def test_is_local_frontend_origin_localhost_returns_true():
    """localhost 계열은 True (main.py:154-156)."""
    assert main_module._is_local_frontend_origin("http://localhost:5173") is True
    assert main_module._is_local_frontend_origin("http://127.0.0.1:8080") is True
    assert main_module._is_local_frontend_origin("http://[::1]:3000") is True


def test_is_local_frontend_origin_empty_returns_false():
    """빈 값/None 은 False (main.py:152-153)."""
    assert main_module._is_local_frontend_origin(None) is False
    assert main_module._is_local_frontend_origin("") is False


def test_is_request_from_frontend_host_no_env_returns_false(monkeypatch):
    """TM_FRONTEND_URL 미설정 시 False (main.py:157)."""
    from starlette.requests import Request

    monkeypatch.delenv("TM_FRONTEND_URL", raising=False)
    scope = {"type": "http", "method": "GET", "path": "/", "query_string": b"", "headers": [], "server": ("testserver", 80)}
    request = Request(scope)
    assert main_module._is_request_from_frontend_host(request) is False


def test_category_matches_hidden_empty_category_returns_false():
    """카테고리가 빈 문자열이면 False (main.py:252)."""
    assert main_module._category_matches_hidden("", ["cat1", "cat2"]) is False


def test_search_similar_books_filters_hidden_categories_for_viewer(client, mock_bm, mock_cat):
    """viewer 권한에서 hidden 카테고리 책이 유사도 결과에서 제외된다 (main.py:479-480)."""
    main_module.app.dependency_overrides[main_module.require_auth] = lambda: VIEWER_PAYLOAD

    source_book = _make_book({"book_id": 1, "category": "visible_cat"})
    source_book.category = "visible_cat"
    hidden_similar = _make_book({"book_id": 2, "category": "hidden_cat"})
    hidden_similar.category = "hidden_cat"
    visible_similar = _make_book({"book_id": 3, "category": "visible_cat"})
    visible_similar.category = "visible_cat"

    mock_bm.get_book.return_value = (source_book, None)
    mock_bm.search_similar_books_paged.return_value = ([hidden_similar, visible_similar], 2, None)
    mock_cat.get_hidden_categories.return_value = ["hidden_cat"]

    r = client.get("/similar/1")
    assert r.status_code == 200
    data = r.json()
    assert data["status"] == "success"
    assert data["total"] == 1  # hidden_similar 제외, visible_similar 만 남음
