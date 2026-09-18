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
def mock_cm():
    """Inject an AsyncMock into the comics_manager _LazyProxy without triggering ComicsManager()."""
    m = AsyncMock()
    m.es_manager = MagicMock()
    prev = main_module.comics_manager._instance
    object.__setattr__(main_module.comics_manager, "_instance", m)
    yield m
    object.__setattr__(main_module.comics_manager, "_instance", prev)


def install_classify_status_store(mock_cat, initial: dict | None = None) -> dict:
    """분류 제안 작업 상태를 메모리로 흉내 내어 mock_cat 에 붙인다.

    이 상태는 corpus 디렉토리의 JSON 파일에서 MySQL 행으로 옮겨갔다. 그래서 테스트가
    상태를 준비하는 방법도 '파일 쓰기'에서 '이 저장소 설정'으로 바뀌었다.

    단순히 return_value 를 박지 않고 전이 규칙을 흉내 내는 이유는, "도는 중에는 새
    제안을 거절한다" 같은 성질이 main.py 와 DB 계층에 나뉘어 있어서다. 여기서는
    main.py 가 규칙을 제대로 태우는지만 본다. 잠금이 실제로 동시 요청을 막는지는
    tests/test_classify_proposal_status_db.py 가 실제 MySQL 로 확인한다.

    항목에 _age_seconds 를 넣으면 '갱신이 그만큼 끊긴 작업'이 된다. 죽은 작업을
    이어받는 경로를 검증할 때 쓴다.
    """
    store: dict[str, dict] = {"book": {"status": "idle", "content_type": "book"}, "comic": {"status": "idle", "content_type": "comic"}}
    if initial is not None:
        store["book"] = {**initial, "content_type": initial.get("content_type", "book")}

    def _effective(content_type: str, stale_seconds: int) -> dict:
        current = dict(store.get(content_type) or {"status": "idle", "content_type": content_type})
        if current.get("status") in ("running", "applying") and current.pop("_age_seconds", 0) > stale_seconds:
            current["status"] = "failed"
            current["error"] = current.get("error") or "응답 없이 중단된 것으로 보입니다."
        current.pop("_age_seconds", None)
        return current

    def _get(content_type: str = "book", stale_seconds: int = 300) -> dict:
        return _effective(content_type, stale_seconds)

    def _set(status: dict, content_type: str = "book") -> None:
        store[content_type] = {**status, "content_type": content_type}

    def _merge(fields: dict, content_type: str = "book") -> None:
        store.setdefault(content_type, {"status": "idle", "content_type": content_type}).update({key: value for key, value in fields.items() if key != "status"})

    def _try_start(status: dict, content_type: str = "book", stale_seconds: int = 300) -> tuple[bool, dict]:
        current = _effective(content_type, stale_seconds)
        if current.get("status") in ("running", "applying"):
            return False, current
        _set(status, content_type)
        return True, _effective(content_type, stale_seconds)

    def _try_begin_apply(apply_token: str, content_type: str = "book", stale_seconds: int = 300) -> tuple[bool, dict]:
        current = _effective(content_type, stale_seconds)
        if current.get("status") not in ("ready", "done", "failed"):
            return False, current
        _set({**current, "status": "applying", "apply_token": apply_token}, content_type)
        return True, _effective(content_type, stale_seconds)

    mock_cat.get_classify_proposal_status.side_effect = _get
    mock_cat.set_classify_proposal_status.side_effect = _set
    mock_cat.merge_classify_proposal_status.side_effect = _merge
    mock_cat.try_start_classify_proposal.side_effect = _try_start
    mock_cat.try_begin_classify_apply.side_effect = _try_begin_apply
    mock_cat.classify_status_store = store
    return store


@pytest.fixture
def mock_cat():
    """Inject a MagicMock into the category_mapping _LazyProxy (all methods are sync via to_thread)."""
    m = MagicMock()
    install_classify_status_store(m)
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
    def test_classify_proposal_comics_prefix_uses_comic_mapping(self, client, mock_cm, mock_cat, tmp_path):
        """/comics 접두사로 마운트된 라우터도 같은 제안 엔드포인트를 쓰는지 본다."""
        mappings = {"2_science": ["SF"]}
        result = {"source_category": "0_inbox", "total_count": 1, "processed_count": 1, "items": [], "failures": []}
        mock_cm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = mappings
        mock_cm.propose_category_changes.return_value = (result, None)

        r = client.post("/comics/categories/classify-proposal", json={"category": "0_inbox"})

        assert r.status_code == 200
        assert r.json()["result"]["started"] is True
        mock_cat.get_all_mappings.assert_called_once_with(content_type="comic")
        mock_cm.propose_category_changes.assert_awaited_once()
        _, kwargs = mock_cm.propose_category_changes.await_args
        assert kwargs["content_type"] == "comic"

    def test_classify_proposal_get_reads_shared_status_file(self, client, mock_bm, mock_cat, tmp_path):
        """폴링용 GET이 상태 파일 내용을 그대로 돌려준다."""
        mock_bm.path_prefix = tmp_path
        mock_cat.get_classify_proposal_items.return_value = []
        install_classify_status_store(mock_cat, {"status": "running", "source_category": "shared_file", "total_count": 9, "processed_count": 4})

        status = client.get("/categories/classify-proposal")

        assert status.status_code == 200
        assert status.json()["result"]["status"] == "running"
        assert status.json()["result"]["source_category"] == "shared_file"
        assert status.json()["result"]["processed_count"] == 4

    def test_classify_proposal_passes_delay_option(self, client, mock_bm, mock_cat, tmp_path):
        mock_bm.path_prefix = tmp_path
        # 상태 파일이 없으면 프로세스 내 메모리 폴백을 쓰는데, 이 폴백은 라우터가 앱과 함께
        # 한 번만 만들어져 다른 테스트가 남긴 running 상태를 물려받을 수 있다.
        install_classify_status_store(mock_cat, {"status": "idle"})
        mock_cat.get_all_mappings.return_value = {}
        mock_bm.propose_category_changes.return_value = ({"source_category": "0_inbox", "total_count": 0, "processed_count": 0, "items": [], "failures": []}, None)

        r = client.post("/categories/classify-proposal", json={"category": "0_inbox", "use_bookstore": False, "use_content_meta": True, "delay": 2.0})

        assert r.status_code == 200
        _, kwargs = mock_bm.propose_category_changes.await_args
        assert kwargs["use_bookstore"] is False
        assert kwargs["use_content_meta"] is True
        assert kwargs["delay"] == 2.0

    def test_classify_proposal_success_polling(self, client, mock_bm, mock_cat, tmp_path):
        """제안 시작은 202류 응답 대신 started 플래그를 주고, GET으로 완료 상태를 본다."""
        mappings = {"3_SF": ["과학소설"]}
        result = {"content_type": "book", "source_category": "0_inbox", "total_count": 2, "processed_count": 2, "items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}], "failures": []}
        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = mappings
        # 상태 파일이 없으면 프로세스 내 메모리 폴백을 쓰는데, 이 폴백은 라우터가 앱과 함께
        # 한 번만 만들어져 다른 테스트가 남긴 running 상태를 물려받을 수 있다. 파일을 미리
        # idle로 채워 이 테스트가 그 잔여 상태에 기대지 않게 한다.
        install_classify_status_store(mock_cat, {"status": "idle"})

        async def fake_propose(*args, on_progress=None, **kwargs):
            # 실제 book_manager처럼, on_progress가 코루틴(awaitable)을 돌려주면
            # await한다 — main.py의 _on_progress는 이제 DB flush를 asyncio.to_thread로
            # 넘기는 async 콜백이라 await하지 않으면 아무 일도 안 일어난다.
            if on_progress:
                outcome = on_progress({"total_count": 2, "processed_count": 1})
                if outcome is not None:
                    await outcome
            return result, None

        mock_bm.propose_category_changes.side_effect = fake_propose
        # 실제 book_manager는 on_progress에 new_items로 델타를 보내고, 그 항목이 DB에
        # 쌓인다. 이 테스트는 배관(propose -> ready -> GET)만 보므로, DB 조회 결과를
        # 직접 고정해 GET이 그 값을 그대로 합쳐 돌려주는지 확인한다.
        mock_cat.get_classify_proposal_items.return_value = result["items"]

        r = client.post("/categories/classify-proposal", json={"category": "0_inbox"})

        assert r.status_code == 200
        assert r.json()["result"]["started"] is True
        status = client.get("/categories/classify-proposal")
        assert status.status_code == 200
        assert status.json()["result"]["status"] == "ready"
        assert status.json()["result"]["items"] == result["items"]
        assert mock_cat.classify_status_store["book"]["status"] == "ready"
        mock_bm.propose_category_changes.assert_awaited_once()
        _, kwargs = mock_bm.propose_category_changes.await_args
        assert kwargs["content_type"] == "book"
        assert kwargs["use_bookstore"] is True
        assert kwargs["use_content_meta"] is True
        assert callable(kwargs["on_progress"])

    def test_classify_proposal_progress_tick_exposes_items_so_far(self, client, mock_bm, mock_cat, tmp_path):
        """제안이 도는 도중 GET은 status: running과 함께 지금까지 DB에 쌓인 항목을
        돌려준다.

        항목 목록이 JSON 상태 파일 대신 DB(classify_proposal_items)로 옮겨갔으므로,
        상태 파일 직접 읽기가 아니라 GET 응답이 상태 파일의 status/카운트와 DB의
        항목을 합쳐 돌려주는지로 검증한다.
        """
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "running", "source_category": "0_inbox", "total_count": 2, "processed_count": 1})
        db_items_so_far = [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]
        mock_cat.get_classify_proposal_items.return_value = db_items_so_far

        status = client.get("/categories/classify-proposal")

        assert status.status_code == 200
        assert status.json()["result"]["status"] == "running"
        assert status.json()["result"]["items"] == db_items_so_far
        mock_cat.get_classify_proposal_items.assert_called_once_with(content_type="book")

    def test_classify_proposal_job_flushes_new_items_in_batches_of_10_and_flushes_tail(self, mock_bm, mock_cat, tmp_path):
        """새 항목을 10건 단위로 모아 DB에 넘기고, 10의 배수가 아닌 꼬리도 끝에
        반드시 넘긴다.

        255권을 처리하면 add_classify_proposal_items가 10건씩 25번 + 5건 1번,
        총 26번 호출돼야 한다. 마지막 5건(꼬리)이 누락되면 관리자가 목록 끝의
        책들을 못 보고 승인하게 된다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = {}
        install_classify_status_store(mock_cat, {"status": "running", "total_count": 0, "processed_count": 0})

        all_items = [{"file_path": f"0_inbox/{i}.epub", "target_category": "3_SF"} for i in range(255)]

        async def fake_propose(*args, on_progress=None, **kwargs):
            for i, item in enumerate(all_items, start=1):
                outcome = on_progress({"total_count": 255, "processed_count": i, "new_items": [item]})
                if outcome is not None:
                    await outcome
            return {"content_type": "book", "source_category": "0_inbox", "total_count": 255, "processed_count": 255, "items": all_items, "failures": []}, None

        mock_bm.propose_category_changes.side_effect = fake_propose
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_proposal_job = freevars["_run_classify_proposal_job"]

        asyncio.run(_run_proposal_job("0_inbox", True, True, 1.2))

        mock_cat.clear_classify_proposal_items.assert_called_once_with(content_type="book")
        add_calls = mock_cat.add_classify_proposal_items.call_args_list
        batch_sizes = [len(call.args[0]) for call in add_calls]
        assert batch_sizes == [10] * 25 + [5]
        assert sum(batch_sizes) == 255
        flattened = [item for call in add_calls for item in call.args[0]]
        assert flattened == all_items

    def test_classify_proposal_job_stores_names_first_then_fills_them_in(self, mock_bm, mock_cat, tmp_path):
        """대상 파일의 이름을 먼저 저장하고, 분류가 끝난 항목은 그 행을 채운다.

        이름이 먼저 깔려야 관리자가 작업 도중에 들어와도 무엇이 대상인지 볼 수 있다.
        채우는 쪽이 INSERT면 같은 책이 두 줄로 보이므로 UPDATE여야 한다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = {}
        install_classify_status_store(mock_cat, {"status": "running", "total_count": 0, "processed_count": 0})

        placeholders = [{"file_path": f"0_inbox/{i}.epub", "title": f"{i}", "grade": None, "target_category": None} for i in range(3)]
        classified = [{**item, "grade": "certain", "target_category": "3_SF"} for item in placeholders]

        async def fake_propose(*args, on_progress=None, **kwargs):
            outcome = on_progress({"total_count": 3, "processed_count": 0, "new_items": placeholders})
            if outcome is not None:
                await outcome
            for i, item in enumerate(classified, start=1):
                outcome = on_progress({"total_count": 3, "processed_count": i, "updated_items": [item]})
                if outcome is not None:
                    await outcome
            return {"content_type": "book", "source_category": "0_inbox", "total_count": 3, "processed_count": 3, "items": classified, "failures": []}, None

        mock_bm.propose_category_changes.side_effect = fake_propose
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_proposal_job = freevars["_run_classify_proposal_job"]

        asyncio.run(_run_proposal_job("0_inbox", True, True, 1.2))

        inserted = [item for call in mock_cat.add_classify_proposal_items.call_args_list for item in call.args[0]]
        assert inserted == placeholders, "이름 행이 먼저 들어가야 한다"
        updated = [item for call in mock_cat.update_classify_proposal_item_payloads.call_args_list for item in call.args[0]]
        assert updated == classified, "분류 결과는 기존 행을 채워야 한다"

    def test_classify_proposal_job_flushes_the_update_tail(self, mock_bm, mock_cat, tmp_path):
        """10의 배수가 아닌 갱신 꼬리도 반드시 넘긴다.

        꼬리가 누락되면 목록 끝의 책들이 영원히 '분류 중'으로 남아, 다 끝났는데도
        승인할 수 없는 행이 생긴다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = {}
        install_classify_status_store(mock_cat, {"status": "running", "total_count": 0, "processed_count": 0})
        classified = [{"file_path": f"0_inbox/{i}.epub", "grade": "certain"} for i in range(25)]

        async def fake_propose(*args, on_progress=None, **kwargs):
            for i, item in enumerate(classified, start=1):
                outcome = on_progress({"total_count": 25, "processed_count": i, "updated_items": [item]})
                if outcome is not None:
                    await outcome
            return {"content_type": "book", "source_category": "0_inbox", "total_count": 25, "processed_count": 25, "items": classified, "failures": []}, None

        mock_bm.propose_category_changes.side_effect = fake_propose
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_proposal_job = freevars["_run_classify_proposal_job"]

        asyncio.run(_run_proposal_job("0_inbox", True, True, 1.2))

        batch_sizes = [len(call.args[0]) for call in mock_cat.update_classify_proposal_item_payloads.call_args_list]
        assert batch_sizes == [10, 10, 5]

    def test_classify_proposal_job_batch_size_10_makes_small_categories_visible_before_job_ends(self, mock_bm, mock_cat, tmp_path):
        """배치 크기가 10이어야, 100건 미만인 대부분의 카테고리(이 저장소 카테고리
        중앙값 5권)도 작업이 끝나기 전에 DB에서 이미 진행 상황을 볼 수 있다.

        25건을 처리하면 10+10+5로 나뉘어 flush돼야 하고, 그중 첫 flush는 11번째
        항목을 처리하는 시점(=작업이 끝나기 전)에 이미 일어나 있어야 한다. 배치
        크기가 100이면 25건 전부가 끝난 뒤 한 번에만 flush되므로 이 검증에서
        실패한다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = {}
        install_classify_status_store(mock_cat, {"status": "running", "total_count": 0, "processed_count": 0})

        all_items = [{"file_path": f"0_inbox/{i}.epub", "target_category": "3_SF"} for i in range(25)]
        flush_count_mid_job = {"value": None}

        async def fake_propose(*args, on_progress=None, **kwargs):
            for i, item in enumerate(all_items, start=1):
                outcome = on_progress({"total_count": 25, "processed_count": i, "new_items": [item]})
                if outcome is not None:
                    await outcome
                if i == 11:
                    flush_count_mid_job["value"] = mock_cat.add_classify_proposal_items.call_count
            return {"content_type": "book", "source_category": "0_inbox", "total_count": 25, "processed_count": 25, "items": all_items, "failures": []}, None

        mock_bm.propose_category_changes.side_effect = fake_propose
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_proposal_job = freevars["_run_classify_proposal_job"]

        asyncio.run(_run_proposal_job("0_inbox", True, True, 1.2))

        assert flush_count_mid_job["value"] == 1
        batch_sizes = [len(call.args[0]) for call in mock_cat.add_classify_proposal_items.call_args_list]
        assert batch_sizes == [10, 10, 5]

    def test_classify_proposal_flush_routes_db_write_through_to_thread(self, mock_bm, mock_cat, tmp_path, monkeypatch):
        """I5: 제안 진행 중 DB flush(add_classify_proposal_items)가 asyncio.to_thread를
        거쳐 실행된다.

        pymysql.connect + executemany + commit은 블로킹 호출이다. 이걸 이벤트 루프
        위에서 직접 부르면(과거 버그) 단일 uvicorn 워커가 매 10건마다 통째로 멈춰
        다른 API 요청까지 막힌다. apply 경로는 이미 to_thread로 옮겨졌는데 propose
        경로만 남아 있던 것을 여기서 확인한다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = {}
        install_classify_status_store(mock_cat, {"status": "running", "total_count": 0, "processed_count": 0})

        to_thread_funcs = []
        real_to_thread = asyncio.to_thread

        async def spy_to_thread(func, *args, **kwargs):
            to_thread_funcs.append(func)
            return await real_to_thread(func, *args, **kwargs)

        monkeypatch.setattr(asyncio, "to_thread", spy_to_thread)

        all_items = [{"file_path": f"0_inbox/{i}.epub", "target_category": "3_SF"} for i in range(10)]

        async def fake_propose(*args, on_progress=None, **kwargs):
            for i, item in enumerate(all_items, start=1):
                outcome = on_progress({"total_count": 10, "processed_count": i, "new_items": [item]})
                if outcome is not None:
                    await outcome
            return {"content_type": "book", "source_category": "0_inbox", "total_count": 10, "processed_count": 10, "items": all_items, "failures": []}, None

        mock_bm.propose_category_changes.side_effect = fake_propose
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_proposal_job = freevars["_run_classify_proposal_job"]

        asyncio.run(_run_proposal_job("0_inbox", True, True, 1.2))

        assert mock_cat.add_classify_proposal_items in to_thread_funcs

    def test_classify_proposal_already_running_blocks_restart(self, client, mock_bm, mock_cat, tmp_path):
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "running", "source_category": "0_inbox"})

        r = client.post("/categories/classify-proposal", json={"category": "0_inbox"})

        assert r.status_code == 200
        assert r.json()["result"]["already_running"] is True

    def test_classify_proposal_apply_uses_server_side_allowed_paths(self, client, mock_bm, mock_cat, tmp_path):
        """승인 요청의 목적지는 받되, 허용 경로는 DB에 저장된 제안 항목에서 가져온다.

        클라이언트가 보낸 file_path를 그대로 허용 집합으로 쓰면 apply_category_changes의
        검증이 통째로 무의미해진다. 항목 목록이 JSON 상태 파일에서 DB(classify_proposal_items)로
        옮겨갔으므로, 이 보안 불변식도 DB 기준으로 확인해야 한다.
        """
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "ready", "source_category": "0_inbox"})
        mock_cat.get_classify_proposal_items.return_value = [{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]

        captured = {}

        async def fake_apply(items, allowed_file_paths, **kwargs):
            captured["items"] = items
            captured["allowed"] = allowed_file_paths
            return {"total_count": len(items), "applied_count": len(items), "failed_count": 0, "results": []}, None

        mock_bm.apply_category_changes = fake_apply

        r = client.post("/categories/classify-proposal/apply", json={"items": [{"file_path": "0_inbox/a.epub", "target_category": "5_음악"}, {"file_path": "0_inbox/침입.epub", "target_category": "5_음악"}]})

        assert r.status_code == 200
        # 서버 제안(DB)에 있던 경로만 허용 집합에 들어간다. 클라이언트가 끼워넣은 경로는 빠진다
        assert captured["allowed"] == {"0_inbox/a.epub"}
        # 사용자가 화면에서 고친 목적지는 그대로 전달된다
        assert captured["items"][0]["target_category"] == "5_음악"
        # 허용 집합이 실제로 DB 조회에서 나왔는지 — body에서 만들어졌다면 이 호출이 없다.
        # (백그라운드 잡의 병합 단계도 같은 메서드를 한 번 더 부르므로 any_call로 본다.)
        mock_cat.get_classify_proposal_items.assert_any_call(content_type="book")

    def test_classify_proposal_apply_rejects_when_not_ready(self, client, mock_bm, mock_cat, tmp_path):
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "idle"})

        r = client.post("/categories/classify-proposal/apply", json={"items": []})

        assert r.status_code == 200
        assert r.json()["status"] == "failure"
        assert "제안" in r.json()["error"]

    def test_classify_proposal_delete_clears_state(self, client, mock_bm, mock_cat, tmp_path):
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "ready"})

        r = client.delete("/categories/classify-proposal")

        assert r.status_code == 200
        assert r.json()["result"]["cleared"] is True
        cleared = mock_cat.classify_status_store["book"]
        assert cleared["status"] == "idle"
        # items는 더 이상 상태 파일에 담지 않는다 — DB(classify_proposal_items)를 지운다
        assert "items" not in cleared
        mock_cat.clear_classify_proposal_items.assert_called_once_with(content_type="book")

    def test_classify_proposal_apply_job_updates_item_status_per_item_in_db(self, mock_bm, mock_cat, tmp_path):
        """적용 백그라운드 잡이 book_manager의 on_item_done 콜백을 받아 항목마다 바로
        DB 상태를 갱신한다.

        예전에는 다 끝난 뒤 clear+add로 한꺼번에 병합했다 — 중간에 중단되면 무엇이
        옮겨졌는지 알 수 없었다. 이제는 book_manager가 항목 하나를 처리할 때마다
        on_item_done을 불러 update_classify_proposal_item_status가 그 즉시 실행된다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "ready", "source_category": "0_inbox", "apply_token": "test-token"})

        async def fake_apply(items, allowed_file_paths, on_item_done=None, **kwargs):
            # 실제 book_manager처럼 항목을 처리하는 그 순간 on_item_done을 부른다. 반환값이
            # awaitable일 수 있으므로(asyncio.to_thread로 감싼 async 콜백) book_manager와
            # 똑같이 await한다.
            if on_item_done:
                outcome = on_item_done({"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": None})
                if outcome is not None:
                    await outcome
                outcome = on_item_done({"file_path": "0_inbox/b.epub", "apply_status": "failed", "apply_error": "파일을 찾을 수 없습니다"})
                if outcome is not None:
                    await outcome
            return {"total_count": 2, "applied_count": 1, "failed_count": 1, "results": []}, None

        mock_bm.apply_category_changes = fake_apply
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_apply_job = freevars["_run_classify_apply_job"]

        asyncio.run(_run_apply_job([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/b.epub", "target_category": "3_SF"}], {"0_inbox/a.epub", "0_inbox/b.epub"}, False, "test-token"))

        done = mock_cat.classify_status_store["book"]
        assert done["status"] == "done"
        assert done["applied_count"] == 1
        assert done["failed_count"] == 1
        assert "items" not in done

        # 병합용 clear/add는 더 이상 쓰지 않는다 — 건별 update가 그 일을 대신한다.
        mock_cat.clear_classify_proposal_items.assert_not_called()
        mock_cat.add_classify_proposal_items.assert_not_called()
        update_calls = mock_cat.update_classify_proposal_item_status.call_args_list
        assert update_calls[0].args == ("0_inbox/a.epub", "moved", None)
        assert update_calls[0].kwargs == {"content_type": "book"}
        assert update_calls[1].args == ("0_inbox/b.epub", "failed", "파일을 찾을 수 없습니다")
        assert update_calls[1].kwargs == {"content_type": "book"}

    def test_classify_proposal_job_handles_exceptions(self, mock_bm, mock_cat, tmp_path):
        """제안/적용 잡이 예외를 던지면 상태를 failed로 굳혀 화면이 계속 회전하지 않게 한다."""
        import asyncio

        mock_bm.path_prefix = tmp_path
        mock_cat.get_all_mappings.return_value = {}
        router = main_module.create_item_router(mock_bm, content_type="book")
        propose_endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal")
        freevars = dict(zip(propose_endpoint.__code__.co_freevars, [c.cell_contents for c in propose_endpoint.__closure__]))
        _run_proposal_job = freevars["_run_classify_proposal_job"]

        mock_bm.propose_category_changes.side_effect = RuntimeError("boom")
        asyncio.run(_run_proposal_job("0_inbox", True, True, 1.2))
        assert mock_cat.classify_status_store["book"]["status"] == "failed"

        apply_endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        apply_freevars = dict(zip(apply_endpoint.__code__.co_freevars, [c.cell_contents for c in apply_endpoint.__closure__]))
        _run_apply_job = apply_freevars["_run_classify_apply_job"]
        mock_bm.apply_category_changes = AsyncMock(side_effect=RuntimeError("boom"))
        asyncio.run(_run_apply_job([], set(), False, None))
        assert mock_cat.classify_status_store["book"]["status"] == "failed"

    def test_classify_proposal_applying_second_request_is_refused_while_claimed(self, mock_bm, mock_cat, tmp_path):
        """두 번째 apply 요청이 첫 번째가 실제로 끝나기 전에 상태를 가로채지 못한다.

        Finding 3: 핸들러가 응답을 돌려주기 전에 동기로 applying을 선점하므로,
        백그라운드 작업이 아직 실행되지 않은 시점에도 두 번째 요청은 ready가
        아님을 보고 거부돼야 한다.
        """
        import asyncio
        from fastapi import BackgroundTasks

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "ready"})
        mock_cat.get_classify_proposal_items.return_value = [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]

        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        body = main_module.ClassifyApplyModel(items=[main_module.ClassifyApplyItemModel(file_path="0_inbox/a.epub", target_category="3_SF")])

        # BackgroundTasks 객체는 만들기만 하고 await 하지 않는다 - 실제 백그라운드
        # 작업은 아직 실행되지 않은 채로 첫 요청의 응답만 받는 상태를 재현한다.
        first = asyncio.run(endpoint(body=body, background_tasks=BackgroundTasks()))
        assert first["status"] == "success"
        assert first["result"]["started"] is True
        assert mock_cat.classify_status_store["book"]["status"] == "applying"

        second = asyncio.run(endpoint(body=body, background_tasks=BackgroundTasks()))
        assert second["status"] == "failure"
        assert "제안" in second["error"]

    def test_classify_proposal_apply_job_no_longer_depends_on_results_key(self, mock_bm, mock_cat, tmp_path):
        """건별 콜백으로 바뀐 뒤에는 "results" 키가 없어도 잡이 정상적으로 끝난다.

        이전에는 끝나고 result["results"]를 병합에 썼기 때문에 이 키가 없으면
        KeyError로 실패 처리됐다. on_item_done 콜백 방식은 이 키를 아예 읽지
        않으므로 더는 여기서 실패할 이유가 없다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "applying", "apply_token": "test-token"})

        async def fake_apply_missing_results(items, allowed_file_paths, **kwargs):
            # "results" 키가 없다 — 더 이상 이 키를 읽지 않으므로 문제가 안 된다.
            return {"total_count": 1, "applied_count": 1, "failed_count": 0}, None

        mock_bm.apply_category_changes = fake_apply_missing_results
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_apply_job = freevars["_run_classify_apply_job"]

        asyncio.run(_run_apply_job([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}], {"0_inbox/a.epub"}, False, "test-token"))

        done = mock_cat.classify_status_store["book"]
        assert done["status"] == "done"

    def test_classify_proposal_apply_progress_tick_does_not_revert_status(self, mock_bm, mock_cat, tmp_path):
        """Finding 4: 적용 단계 진행률 콜백이 status를 running으로 되돌리지 않는다."""
        import asyncio

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "applying", "apply_token": "test-token", "items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})

        seen = {}

        async def fake_apply_with_progress(items, allowed_file_paths, on_progress=None, **kwargs):
            if on_progress:
                on_progress({"total_count": 1, "applied_count": 0, "failed_count": 0})
            seen["status_after_tick"] = mock_cat.classify_status_store["book"]["status"]
            return {"total_count": 1, "applied_count": 1, "failed_count": 0, "results": [{"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": None}]}, None

        mock_bm.apply_category_changes = fake_apply_with_progress
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_apply_job = freevars["_run_classify_apply_job"]

        asyncio.run(_run_apply_job([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}], {"0_inbox/a.epub"}, False, "test-token"))

        assert seen["status_after_tick"] == "applying"

    def test_classify_proposal_apply_progress_does_not_clobber_propose_counters(self, mock_bm, mock_cat, tmp_path):
        """I1: 적용 단계 진행률이 제안 단계의 total_count/processed_count를 덮어쓰지 않는다.

        제안 1,200권 중 50건만 승인해 적용하면, 예전에는 apply의 on_progress가 같은
        "total_count" 키로 상태 파일에 병합돼 헤더가 "1200 / 50"처럼 뒤바뀌었다.
        book_manager가 이제 apply 전용 키(apply_total_count)로 보고하므로, 제안
        단계가 남긴 total_count/processed_count는 적용이 끝난 뒤에도 그대로다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "ready", "apply_token": "test-token", "source_category": "0_inbox", "total_count": 1200, "processed_count": 1200})
        items = [{"file_path": f"0_inbox/{i}.epub", "target_category": "3_SF"} for i in range(50)]

        async def fake_apply_with_progress(items, allowed_file_paths, on_progress=None, **kwargs):
            if on_progress:
                # 실제 book_manager 처럼, 진행률 콜백이 코루틴을 돌려주면 await 한다.
                outcome = on_progress({"apply_total_count": len(items), "applied_count": 10, "failed_count": 0})
                if outcome is not None:
                    await outcome
            return {"total_count": len(items), "applied_count": len(items), "failed_count": 0, "results": []}, None

        mock_bm.apply_category_changes = fake_apply_with_progress
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_apply_job = freevars["_run_classify_apply_job"]

        asyncio.run(_run_apply_job(items, {item["file_path"] for item in items}, False, "test-token"))

        done = mock_cat.classify_status_store["book"]
        # 제안 단계 카운터는 그대로 살아있다 — 승인 50건이 1200을 덮어쓰지 않는다.
        assert done["total_count"] == 1200
        assert done["processed_count"] == 1200
        assert done["apply_total_count"] == 50
        assert done["status"] == "done"

    def test_classify_proposal_apply_accepted_when_status_is_failed(self, client, mock_bm, mock_cat, tmp_path):
        """상태가 failed(중단 후 굳음)인 제안에도 승인 요청이 받아들여진다.

        승인 도중 중단되면 상태가 applying -> (하트비트 만료) failed로 굳는다. 이때도
        남은 pending 행을 다시 승인해 이어갈 수 있어야 한다 — 그러지 않으면 관리자는
        파일이 반쯤 옮겨진 채로 아무것도 할 수 없다.
        """
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "failed", "source_category": "0_inbox", "error": "백엔드가 다시 시작되어 자동 분류가 중단되었습니다. 다시 실행해 주세요."})
        mock_cat.get_classify_proposal_items.return_value = [{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]

        r = client.post("/categories/classify-proposal/apply", json={"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})

        assert r.status_code == 200
        assert r.json()["status"] == "success"
        assert r.json()["result"]["started"] is True

    def test_classify_proposal_apply_rejected_while_applying(self, client, mock_bm, mock_cat, tmp_path):
        """작업이 이미 도는(applying) 동안에는 새 승인 요청을 거절한다."""
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "applying"})

        r = client.post("/categories/classify-proposal/apply", json={"items": []})

        assert r.status_code == 200
        assert r.json()["status"] == "failure"

    def test_classify_proposal_apply_allowed_set_excludes_moved_rows(self, client, mock_bm, mock_cat, tmp_path):
        """이미 moved인 행은 허용 경로 집합에 들어가지 않는다.

        moved 행이 allowed에 들어가도 apply_category_changes가 파일 없음으로 걸러
        이중 이동은 안 일어나지만, 불필요하므로 애초에 pending 행만 담는다.
        """
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "ready", "source_category": "0_inbox"})
        mock_cat.get_classify_proposal_items.return_value = [{"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF", "apply_status": "moved"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF", "apply_status": "pending"}]

        captured = {}

        async def fake_apply(items, allowed_file_paths, **kwargs):
            captured["allowed"] = allowed_file_paths
            return {"total_count": len(items), "applied_count": 0, "failed_count": 0, "results": []}, None

        mock_bm.apply_category_changes = fake_apply

        r = client.post("/categories/classify-proposal/apply", json={"items": [{"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF"}]})

        assert r.status_code == 200
        assert captured["allowed"] == {"0_inbox/still_pending.epub"}

    def test_classify_proposal_apply_allowed_set_includes_failed_and_moving_rows(self, client, mock_bm, mock_cat, tmp_path):
        """failed(재시도)와 moving(중단된 이동 재확인) 행 모두 허용 집합에 들어간다.

        C1: failed 행을 재시도하지 못하면 관리자는 "제안 목록에 없는 파일입니다"만
        영원히 다시 보게 된다. I4: moving 행도 마찬가지로 재시도할 방법이 있어야
        한다 — apply_category_changes가 이동 직전 파일 존재를 다시 확인하므로,
        이미 끝난 이동은 이중으로 옮겨지지 않고 "파일을 찾을 수 없습니다"로
        안전하게 failed 처리된다. moved만 재시도 대상에서 빠진다.
        """
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "failed", "source_category": "0_inbox"})
        mock_cat.get_classify_proposal_items.return_value = [
            {"file_path": "0_inbox/interrupted.epub", "target_category": "3_SF", "apply_status": "moving"},
            {"file_path": "0_inbox/rejected.epub", "target_category": "3_SF", "apply_status": "failed"},
            {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF", "apply_status": "pending"},
            {"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF", "apply_status": "moved"},
        ]

        captured = {}

        async def fake_apply(items, allowed_file_paths, **kwargs):
            captured["allowed"] = allowed_file_paths
            return {"total_count": len(items), "applied_count": 0, "failed_count": 0, "results": []}, None

        mock_bm.apply_category_changes = fake_apply

        r = client.post(
            "/categories/classify-proposal/apply",
            json={"items": [{"file_path": "0_inbox/interrupted.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/rejected.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF"}]},
        )

        assert r.status_code == 200
        assert captured["allowed"] == {"0_inbox/interrupted.epub", "0_inbox/rejected.epub", "0_inbox/still_pending.epub"}

    def test_classify_proposal_apply_job_never_overwrites_rows_outside_this_runs_allowed_set(self, mock_bm, mock_cat, tmp_path):
        """이미 moved인 행이 클라이언트가 보낸 목록에 여전히 남아 있어도 그 기록을
        덮어쓰지 않는다.

        재현: 200권 중 80권을 옮긴 뒤 중단. 화면은 GET으로 받은 목록(200권 전부)을
        그대로 들고 있다가 재승인을 누른다. book_manager는 allowed에 없는(이미
        moved인) 1~80번 항목도 "제안 목록에 없는 파일입니다"로 on_item_done을
        부른다 — 이 콜백이 file_path로만 매칭해 덮어쓰면 moved였던 행이 failed로
        둔갑해 다시는 지우거나 재승인할 수 없는 유령 실패가 된다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "failed", "apply_token": "test-token"})

        async def fake_apply(items, allowed_file_paths, on_item_done=None, **kwargs):
            if on_item_done:
                # 이미 moved라 이번 실행의 allowed에는 없는 행 — book_manager는 여전히
                # on_item_done을 부르지만 여기서 걸러져야 한다.
                outcome = on_item_done({"file_path": "0_inbox/already_moved.epub", "apply_status": "failed", "apply_error": "제안 목록에 없는 파일입니다"})
                if outcome is not None:
                    await outcome
                # 이번 실행이 실제로 허용한 행
                outcome = on_item_done({"file_path": "0_inbox/still_pending.epub", "apply_status": "moved", "apply_error": None})
                if outcome is not None:
                    await outcome
            return {"total_count": 2, "applied_count": 1, "failed_count": 1, "results": []}, None

        mock_bm.apply_category_changes = fake_apply
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_apply_job = freevars["_run_classify_apply_job"]

        # 이번 실행의 allowed에는 still_pending만 있다 — already_moved는 이미 moved라 빠졌다
        asyncio.run(_run_apply_job([{"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF"}], {"0_inbox/still_pending.epub"}, False, "test-token"))

        updated_paths = [call.args[0] for call in mock_cat.update_classify_proposal_item_status.call_args_list]
        assert "0_inbox/already_moved.epub" not in updated_paths
        assert "0_inbox/still_pending.epub" in updated_paths

    def test_classify_proposal_apply_job_stops_writing_when_token_no_longer_matches(self, mock_bm, mock_cat, tmp_path):
        """다른 승인 작업이 상태 파일의 apply_token을 바꾸면, 이 작업은 더 이상 처리하지
        않고 물러난다.

        하트비트 만료로 상태가 failed로 굳은 뒤에도 원래 작업이 실제로는 계속 돌고
        있었다면, 두 번째 apply가 새 토큰으로 시작한 뒤 원래 작업이 계속 행을 써서
        새 작업의 결과를 덮어쓸 수 있다. 토큰이 바뀌면 진 쪽은 should_continue에서
        멈추고, 최종 상태(status/token)도 건드리지 않아야 한다.
        """
        import asyncio

        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "applying", "apply_token": "token-A"})

        async def fake_apply(items, allowed_file_paths, on_item_done=None, should_continue=None, **kwargs):
            if on_item_done:
                outcome = on_item_done({"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": None})
                if outcome is not None:
                    await outcome
            # 다른 요청이 이 작업을 대체해 토큰을 바꿨다고 가정한다.
            mock_cat.classify_status_store["book"] = {"status": "applying", "apply_token": "token-B", "content_type": "book"}
            # 상태가 DB로 옮겨가면서 should_continue 는 코루틴을 돌려준다. 실제
            # book_manager 처럼 여기서 await 해야 판단 결과를 볼 수 있다.
            assert should_continue is not None and (await should_continue()) is False
            return {"total_count": 1, "applied_count": 1, "failed_count": 0, "results": []}, None

        mock_bm.apply_category_changes = fake_apply
        router = main_module.create_item_router(mock_bm, content_type="book")
        endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
        freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
        _run_apply_job = freevars["_run_classify_apply_job"]

        asyncio.run(_run_apply_job([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}], {"0_inbox/a.epub"}, False, "token-A"))

        # 대체된 뒤에는 이 작업이 상태를 더 이상 덮어쓰지 않는다 - "applying"/"token-B" 그대로여야 한다
        final_status = mock_cat.classify_status_store["book"]
        assert final_status["status"] == "applying"
        assert final_status["apply_token"] == "token-B"

    def test_delete_applied_removes_moved_only_and_returns_count(self, client, mock_bm, mock_cat, tmp_path):
        """DELETE .../applied가 완료(moved) 행만 지우고 지운 개수를 돌려준다."""
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "done"})
        mock_cat.delete_applied_classify_proposal_items.return_value = 3

        r = client.delete("/categories/classify-proposal/applied")

        assert r.status_code == 200
        assert r.json()["status"] == "success"
        assert r.json()["result"]["deleted_count"] == 3
        mock_cat.delete_applied_classify_proposal_items.assert_called_once_with(content_type="book")

    def test_delete_applied_rejected_while_job_is_running(self, client, mock_bm, mock_cat, tmp_path):
        """작업이 도는 중에는 DELETE .../applied가 거절된다.

        도는 중에 지우면 방금 건별로 기록한 행을 진행 중인 작업 밑에서 지울 수 있다.
        """
        mock_bm.path_prefix = tmp_path
        install_classify_status_store(mock_cat, {"status": "applying"})

        r = client.delete("/categories/classify-proposal/applied")

        assert r.status_code == 200
        assert r.json()["status"] == "failure"
        mock_cat.delete_applied_classify_proposal_items.assert_not_called()

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

    @pytest.mark.parametrize("store,cls", [("ridi", "RidibooksBookstore"), ("naver", "NaverShoppingBookstore"), ("naverseries", "NaverSeriesBookstore"), ("munpia", "MunpiaBookstore"), ("kyobo", "KyoboBookstore"), ("joara", "JoaraBookstore")])
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
        with patch("backend.main.TM_GOOGLE_CLIENT_ID", "mock_client_id"), patch("backend.main.google_id_token.verify_oauth2_token", side_effect=ValueError("bad")):
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

    def test_add_keyword_duplicate_is_not_a_failure(self, client, mock_cat):
        """이미 있는 키워드는 실패가 아니다 — "등록해 달라"는 요청이 이미 충족돼 있다.

        예전에는 status를 "duplicate"로 돌려줬다. 공용 응답 처리기가 success가 아닌 것을
        전부 오류로 보내는데 error 키까지 없어서, 화면에는 이유가 안 적힌 실패 창이
        떴다 — 등록은 되어 있는데도. 실제로 관리자가 그 창을 봤다.
        """
        mock_cat.add_keyword.return_value = False
        mock_cat.get_keywords.return_value = ["fantasy"]
        r = client.post("/category-mappings/소설/keywords", json={"keyword": "fantasy"})
        assert r.status_code == 200
        body = r.json()
        assert body["status"] == "success"
        assert body["result"] == ["fantasy"]
        # 아무 일도 안 일어났다는 사실은 경고로 알린다.
        assert body["warning"]

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


# ── Additional edge case tests to reach 100% coverage on backend/main.py ──







def test_get_latest_books_error(client, mock_bm, mock_cat):
    # 621: get_latest_books returns error
    mock_cat.get_latest_excluded_categories.return_value = []
    mock_bm.get_latest_books.return_value = ([], 0, "failed to get latest")
    r = client.get("/latest")
    assert r.status_code == 200
    assert r.json()["status"] == "failure"
    assert r.json()["error"] == "failed to get latest"


def test_reload_progress_flush_exception(client, mock_bm, mock_cat, monkeypatch):
    # 924-926: exception in _flush_reload_progress_periodically
    monkeypatch.setattr(main_module, "RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS", 0.001)
    mock_cat.acquire_reload_lock.return_value = (True, None, None)
    mock_cat.heartbeat_reload_lock.side_effect = RuntimeError("heartbeat db failed")

    async def slow_reload(*args, **kwargs):
        import asyncio

        await asyncio.sleep(0.01)
        return ({"indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0}, None)

    mock_bm.reload_category_mismatches.side_effect = slow_reload
    r = client.post("/category-mismatches/reload-all")
    assert r.status_code == 200


def test_reload_all_mismatches_job_exception(client, mock_bm, mock_cat):
    # 959-962: exception in _run_reload_all_mismatches_job
    mock_cat.acquire_reload_lock.return_value = (True, None, None)
    mock_bm.reload_category_mismatches.side_effect = RuntimeError("mismatch scan crashed")

    r = client.post("/category-mismatches/reload-all")
    assert r.status_code == 200
    mock_cat.complete_reload_lock.assert_called_with("book", "failed", main_module.GENERIC_MISMATCH_ERROR)


def test_reload_locks_already_running(client, mock_cat):
    # 981-984: reload_category_mismatch_files lock busy
    mock_cat.acquire_reload_lock.return_value = (False, "lock busy", {"status": "running"})
    r1 = client.post("/category-mismatches/reload-mismatches", json={"category": "0_inbox"})
    assert r1.status_code == 200
    assert r1.json()["result"]["already_running"] is True

    # 1002-1005: reload_all_category_mismatches lock busy
    r2 = client.post("/category-mismatches/reload-all")
    assert r2.status_code == 200
    assert r2.json()["result"]["already_running"] is True


def test_classify_proposal_restarts_when_the_previous_run_died(client, mock_bm, mock_cat, tmp_path):
    """재배포로 죽은 작업은 새 실행을 막지 않아야 한다.

    상태 파일이 running 인 채로 굳으면 버튼을 눌러도 already_running 만 돌아와
    화면이 계속 회전했다. 갱신이 끊긴 상태는 죽은 것으로 본다.
    """
    mock_bm.path_prefix = tmp_path
    install_classify_status_store(mock_cat, {"status": "running", "source_category": "0_inbox", "_age_seconds": 3600})

    r = client.post("/categories/classify-proposal", json={"category": "0_inbox"})

    assert r.status_code == 200
    assert r.json()["result"].get("already_running") is None
    assert r.json()["result"]["started"] is True
