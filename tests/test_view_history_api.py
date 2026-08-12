"""조회 이력 엔드포인트 테스트 — 스토어는 대역으로 두고 라우팅·인증·스냅샷 규칙을 본다.

실제 SQL 동작은 tests/test_view_history_store.py 가 MySQL 컨테이너로 검증한다.
"""

import time
from unittest.mock import MagicMock

import pytest
from fastapi import HTTPException
from fastapi.testclient import TestClient

import backend.main as main_mod

ADMIN_PAYLOAD = {"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": int(time.time()) + 3600}
VIEWER_PAYLOAD = {"email": "viewer@test.com", "role": "viewer", "name": "Viewer", "picture": "", "exp": int(time.time()) + 3600}


class _FakeStore:
    """record_view 호출을 그대로 모아두는 스토어 대역."""

    def __init__(self):
        self.recorded = []
        self.list_result = {"limit": 50, "users": []}
        self.list_calls = []

    def record_view(self, **kwargs):
        self.recorded.append(kwargs)

    def list_recent_views(self, **kwargs):
        self.list_calls.append(kwargs)
        return self.list_result


def _make_book(title="원래 제목", category="소설"):
    book = MagicMock()
    book.title = title
    book.category = category
    return book


@pytest.fixture()
def store(monkeypatch):
    fake = _FakeStore()
    monkeypatch.setattr(main_mod, "view_history_store", fake)
    return fake


@pytest.fixture()
def admin_client():
    main_mod.app.dependency_overrides[main_mod.require_auth] = lambda: ADMIN_PAYLOAD
    main_mod.app.dependency_overrides[main_mod.require_admin] = lambda: ADMIN_PAYLOAD
    with TestClient(main_mod.app) as c:
        yield c
    main_mod.app.dependency_overrides.clear()


@pytest.fixture()
def allow_book(monkeypatch):
    """책 조회와 viewer 접근 검사를 통과시키고 호출 인자를 기록한다."""
    calls = []

    async def _ok(manager, book_id, payload, content_type):
        calls.append({"book_id": book_id, "payload": payload, "content_type": content_type})
        return _make_book(), None

    monkeypatch.setattr(main_mod, "_get_book_and_ensure_viewer_access", _ok)
    return calls


class TestRecordBookView:
    def test_records_book_view_with_server_side_snapshot(self, admin_client, store, allow_book):
        resp = admin_client.post("/books/view-history/42")

        assert resp.status_code == 200
        assert resp.json()["result"]["recorded"] is True
        assert store.recorded == [{"email": "admin@test.com", "content_type": "book", "book_id": 42, "title": "원래 제목", "category": "소설"}]

    def test_comics_route_records_comic_type(self, admin_client, store, allow_book):
        resp = admin_client.post("/comics/view-history/7")

        assert resp.status_code == 200
        assert store.recorded[0]["content_type"] == "comic"
        assert store.recorded[0]["book_id"] == 7

    def test_ignores_client_supplied_title(self, admin_client, store, allow_book):
        """제목은 서버 레코드에서만 온다. 본문으로 위조한 값은 무시된다."""
        resp = admin_client.post("/books/view-history/42", json={"title": "위조된 제목", "category": "위조 카테고리"})

        assert resp.status_code == 200
        assert store.recorded[0]["title"] == "원래 제목"
        assert store.recorded[0]["category"] == "소설"

    def test_enforces_viewer_access_check(self, admin_client, store, allow_book):
        admin_client.post("/books/view-history/42")

        # 기록 전에 접근 검사를 반드시 거친다
        assert allow_book[0]["book_id"] == 42
        assert allow_book[0]["content_type"] == "book"

    def test_viewer_without_access_is_rejected_and_nothing_recorded(self, store, monkeypatch):
        async def _forbidden(manager, book_id, payload, content_type):
            raise HTTPException(status_code=403, detail="Access denied")

        monkeypatch.setattr(main_mod, "_get_book_and_ensure_viewer_access", _forbidden)
        main_mod.app.dependency_overrides[main_mod.require_auth] = lambda: VIEWER_PAYLOAD
        try:
            with TestClient(main_mod.app) as c:
                resp = c.post("/books/view-history/42")
        finally:
            main_mod.app.dependency_overrides.clear()

        assert resp.status_code == 403
        assert store.recorded == []

    def test_unknown_book_returns_404_and_records_nothing(self, admin_client, store, monkeypatch):
        async def _missing(manager, book_id, payload, content_type):
            return None, "Book not found"

        monkeypatch.setattr(main_mod, "_get_book_and_ensure_viewer_access", _missing)

        resp = admin_client.post("/books/view-history/999")

        assert resp.status_code == 404
        assert store.recorded == []

    def test_requires_authentication(self):
        main_mod.app.dependency_overrides.clear()
        with TestClient(main_mod.app) as c:
            assert c.post("/books/view-history/1").status_code == 401
            assert c.post("/comics/view-history/1").status_code == 401

    def test_store_failure_does_not_break_viewing(self, admin_client, store, allow_book, monkeypatch, caplog):
        """이력 기록 실패는 열람을 막지 않지만 조용히 넘기지도 않는다."""

        def _boom(**kwargs):
            raise RuntimeError("db down")

        monkeypatch.setattr(store, "record_view", _boom)

        import logging

        with caplog.at_level(logging.WARNING):
            resp = admin_client.post("/books/view-history/42")

        assert resp.status_code == 200
        assert resp.json()["result"]["recorded"] is False
        assert "조회 이력 기록 실패" in caplog.text

    def test_rejects_non_integer_book_id(self, admin_client, store, allow_book):
        assert admin_client.post("/books/view-history/abc").status_code == 422
        assert store.recorded == []


class TestListViewHistory:
    def test_returns_store_result(self, admin_client, store):
        store.list_result = {"limit": 50, "users": [{"email": "u@example.com", "last_viewed_at": 100, "book": [{"book_id": 1, "title": "책", "category": "소설", "viewed_at": 100}], "comic": []}]}

        resp = admin_client.get("/view-history")

        assert resp.status_code == 200
        body = resp.json()
        assert body["status"] == "success"
        assert body["result"]["users"][0]["email"] == "u@example.com"
        assert body["result"]["users"][0]["book"][0]["title"] == "책"

    def test_default_limit_is_max_recent_views(self, admin_client, store):
        admin_client.get("/view-history")

        assert store.list_calls[0]["limit"] == main_mod.MAX_RECENT_VIEWS

    def test_limit_param_is_passed_through(self, admin_client, store):
        admin_client.get("/view-history", params={"limit": 5})

        assert store.list_calls[0]["limit"] == 5

    def test_rejects_out_of_range_limit(self, admin_client, store):
        assert admin_client.get("/view-history", params={"limit": 0}).status_code == 422
        assert admin_client.get("/view-history", params={"limit": main_mod.MAX_RECENT_VIEWS + 1}).status_code == 422
        assert admin_client.get("/view-history", params={"limit": main_mod.MAX_RECENT_VIEWS}).status_code == 200

    def test_store_failure_returns_503(self, admin_client, store, monkeypatch):
        def _boom(**kwargs):
            raise RuntimeError("db down")

        monkeypatch.setattr(store, "list_recent_views", _boom)

        assert admin_client.get("/view-history").status_code == 503

    def test_requires_admin(self, store):
        main_mod.app.dependency_overrides[main_mod.require_auth] = lambda: VIEWER_PAYLOAD
        try:
            with TestClient(main_mod.app) as c:
                # require_admin 은 override 하지 않아 실제 의존성이 동작한다
                assert c.get("/view-history").status_code in (401, 403)
        finally:
            main_mod.app.dependency_overrides.clear()

    def test_unauthenticated_is_rejected(self, store):
        main_mod.app.dependency_overrides.clear()
        with TestClient(main_mod.app) as c:
            assert c.get("/view-history").status_code == 401

    def test_declares_admin_dependency(self):
        route = next(r for r in main_mod.app.routes if getattr(r, "path", "") == "/view-history")
        assert main_mod.require_admin in [d.call for d in route.dependant.dependencies]


class TestEndToEndWithRealStore:
    """엔드포인트가 실제 MySQL 스토어에 쓰고, admin 조회로 되읽히는지 확인한다.

    책 조회만 대역으로 두고 스토어는 진짜를 쓴다(스토어 대역 테스트와 실제 SQL 테스트
    사이의 배선 공백을 닫는다).
    """

    @pytest.fixture()
    def real_store(self, mysql_container, monkeypatch):
        from backend.view_history_store import ViewHistoryStore

        store = ViewHistoryStore()
        with store._get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute("TRUNCATE TABLE view_history")
            conn.commit()
        monkeypatch.setattr(main_mod, "view_history_store", store)
        return store

    def test_recorded_view_is_visible_to_admin(self, admin_client, real_store, monkeypatch):
        async def _book(manager, book_id, payload, content_type):
            title = "만화 제목" if content_type == "comic" else "책 제목"
            return _make_book(title=f"{title} {book_id}", category="분류"), None

        monkeypatch.setattr(main_mod, "_get_book_and_ensure_viewer_access", _book)

        assert admin_client.post("/books/view-history/1").json()["result"]["recorded"] is True
        assert admin_client.post("/comics/view-history/2").json()["result"]["recorded"] is True
        # 같은 책 재조회는 중복 row 를 만들지 않는다
        assert admin_client.post("/books/view-history/1").json()["result"]["recorded"] is True

        result = admin_client.get("/view-history").json()["result"]

        assert result["limit"] == main_mod.MAX_RECENT_VIEWS
        assert len(result["users"]) == 1
        user = result["users"][0]
        assert user["email"] == "admin@test.com"
        assert [item["title"] for item in user["book"]] == ["책 제목 1"]
        assert [item["title"] for item in user["comic"]] == ["만화 제목 2"]

    def test_limit_narrows_returned_items(self, admin_client, real_store, monkeypatch):
        async def _book(manager, book_id, payload, content_type):
            return _make_book(title=f"책 {book_id}", category="분류"), None

        monkeypatch.setattr(main_mod, "_get_book_and_ensure_viewer_access", _book)
        for book_id in range(4):
            admin_client.post(f"/books/view-history/{book_id}")

        result = admin_client.get("/view-history", params={"limit": 2}).json()["result"]

        assert len(result["users"][0]["book"]) == 2
