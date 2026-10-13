#!/usr/bin/env python
"""main.py 라우터의 실패 응답과 입력 검증 분기를 mock 기반으로 확인한다."""

import asyncio
import importlib
from unittest.mock import AsyncMock, MagicMock

import pytest

from tests.test_main_mock import client, install_classify_status_store, mock_bm, mock_cat, override_auth  # noqa: F401  (fixture 재사용)


def _main():
    # 다른 테스트가 backend.main 을 다시 불러올 수 있어, fixture 와 같은 모듈을 호출 시점에 얻는다.
    return importlib.import_module("backend.main")


def _apply_job(mock_bm, tmp_path):
    main_module = _main()
    mock_bm.path_prefix = tmp_path
    router = main_module.create_item_router(mock_bm, content_type="book")
    endpoint = next(r.endpoint for r in router.routes if getattr(r, "path", None) == "/categories/classify-proposal/apply")
    freevars = dict(zip(endpoint.__code__.co_freevars, [c.cell_contents for c in endpoint.__closure__]))
    return freevars["_run_classify_apply_job"]


def test_apply_job_marks_state_failed_when_book_manager_returns_error(mock_bm, mock_cat, tmp_path):
    install_classify_status_store(mock_cat, {"status": "applying", "source_category": "0_inbox", "apply_token": "tok"})
    mock_bm.apply_category_changes = AsyncMock(return_value=({}, "적용 오류"))

    asyncio.run(_apply_job(mock_bm, tmp_path)([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}], {"0_inbox/a.epub"}, False, "tok"))

    state = mock_cat.classify_status_store["book"]
    assert state["status"] == "failed"
    assert state["error"] == "적용 오류"


@pytest.mark.parametrize("running_status", ["running", "applying"])
def test_delete_classify_proposal_is_refused_while_job_is_running(client, mock_bm, mock_cat, tmp_path, running_status):
    mock_bm.path_prefix = tmp_path
    install_classify_status_store(mock_cat, {"status": running_status})

    response = client.delete("/categories/classify-proposal")

    assert response.json() == {"status": "failure", "error": "작업이 진행 중입니다."}
    mock_cat.clear_classify_proposal_items.assert_not_called()


def test_similar_names_requires_a_name(client, mock_bm, mock_cat):
    response = client.get("/similar-names", params={"name": "   "})
    assert response.json() == {"status": "failure", "error": "name is required"}


def test_similar_names_reports_category_lookup_error(client, mock_bm, mock_cat):
    mock_bm.get_categories = AsyncMock(return_value=({}, "카테고리 조회 실패"))

    response = client.get("/similar-names", params={"name": "소설"})

    assert response.json() == {"status": "failure", "error": "카테고리 조회 실패"}


def test_similar_names_hides_internal_errors(client, mock_bm, mock_cat):
    mock_bm.get_categories = AsyncMock(return_value=({"소설": 1}, None))
    mock_bm.es_manager.search_categories_pdf_stats = MagicMock(side_effect=RuntimeError("ES 내부 오류 상세"))

    response = client.get("/similar-names", params={"name": "소설"})

    assert response.json() == {"status": "failure", "error": "유사 이름 검색에 실패했습니다."}


@pytest.mark.parametrize("category", ["_root", ""])
def test_category_pdf_stats_rejects_root_and_empty_category(client, mock_bm, mock_cat, category):
    response = client.get("/category-pdf-stats", params={"category": category})
    assert response.status_code == 400


def test_category_pdf_stats_returns_aggregated_stats(client, mock_bm, mock_cat):
    stats = {"file_count": 2, "page_count": 10, "total_file_size": 99}
    mock_bm.es_manager.search_category_pdf_stats = MagicMock(return_value=stats)

    response = client.get("/category-pdf-stats", params={"category": "2_science"})

    assert response.json() == {"status": "success", "result": stats}


def test_category_pdf_stats_hides_internal_errors(client, mock_bm, mock_cat):
    mock_bm.es_manager.search_category_pdf_stats = MagicMock(side_effect=RuntimeError("ES 내부 오류 상세"))

    response = client.get("/category-pdf-stats", params={"category": "2_science"})

    assert response.json() == {"status": "failure", "error": "PDF 통계를 불러오지 못했습니다."}


def test_client_error_model_accepts_explicit_null_optional_fields():
    body = _main().ClientErrorLogModel(error_type="CUSTOM_ERROR", message="m", url="u", stack=None, component_stack=None, user_agent=None, timestamp=None)
    assert body.stack is None
    assert body.timestamp is None
