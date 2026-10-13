#!/usr/bin/env python
"""ESManager의 빈 입력·페이지 끝·유사 이름 검색 분기를 가짜 ES 클라이언트로 확인한다."""

import os
from unittest.mock import MagicMock

os.environ.setdefault("TM_ES_URL", "http://localhost:9200")
os.environ.setdefault("TM_ES_BOOK_INDEX", "test")
os.environ.setdefault("TM_ES_USER", "test")
os.environ.setdefault("TM_ES_PASSWORD", "test")

from backend.es_manager import ESManager  # noqa: E402


def _manager() -> tuple[ESManager, MagicMock]:
    manager = ESManager.__new__(ESManager)
    manager.es = MagicMock()
    manager.index_name = "idx"
    return manager, manager.es


def test_existing_created_time_metadata_is_empty_for_no_ids():
    manager, es = _manager()
    assert manager.get_existing_created_time_metadata([]) == {}
    es.mget.assert_not_called()


def test_search_similar_names_escapes_wildcards_and_adds_author_clause():
    manager, _ = _manager()
    captured: dict = {}

    def fake_search(query, sort=None, max_result_count=-1, source_fields=None):
        captured["query"] = query
        return []

    manager._search = fake_search

    manager.search_similar_names("a*b?c\\d", author="저자")

    should = captured["query"]["bool"]["should"]
    assert should[1]["wildcard"]["file_path"]["value"] == "*a\\*b\\?c\\\\d*"
    assert should[-1]["match"]["author"]["query"] == "저자"


def test_search_similar_names_without_author_has_no_author_clause():
    manager, _ = _manager()
    captured: dict = {}

    def fake_search(query, sort=None, max_result_count=-1, source_fields=None):
        captured["query"] = query
        return []

    manager._search = fake_search

    manager.search_similar_names("제목")

    assert len(captured["query"]["bool"]["should"]) == 2


def test_search_category_pdf_stats_sums_pdf_documents():
    manager, es = _manager()
    es.search.return_value = {"aggregations": {"pdfs": {"doc_count": 3, "total_pages": {"value": 120.0}, "total_file_size": {"value": None}}}}

    assert manager.search_category_pdf_stats("A") == {"file_count": 3, "page_count": 120, "total_file_size": 0}


def test_search_categories_pdf_stats_returns_empty_for_no_categories():
    manager, es = _manager()
    assert manager.search_categories_pdf_stats([]) == {}
    es.search.assert_not_called()


def test_search_categories_pdf_stats_maps_each_bucket():
    manager, es = _manager()
    es.search.return_value = {"aggregations": {"by_category": {"buckets": {"A": {"pdfs": {"doc_count": 2, "total_pages": {"value": 10}, "total_file_size": {"value": 99}}}, "B": {"pdfs": {"doc_count": 0, "total_pages": {"value": None}, "total_file_size": {"value": None}}}}}}}

    assert manager.search_categories_pdf_stats(["A", "B"]) == {"A": {"file_count": 2, "page_count": 10, "total_file_size": 99}, "B": {"file_count": 0, "page_count": 0, "total_file_size": 0}}


def test_iter_all_category_file_paths_stops_when_last_page_has_no_after_key():
    manager, es = _manager()
    es.search.return_value = {"aggregations": {"paths": {"buckets": [{"key": {"category": None, "file_path": "x.txt"}, "doc_count": 2}]}}}

    assert list(manager.iter_all_category_file_paths()) == [("", "x.txt", 2)]
    assert es.search.call_count == 1


def test_iter_all_category_file_paths_follows_after_key_until_empty_page():
    manager, es = _manager()
    es.search.side_effect = [{"aggregations": {"paths": {"buckets": [{"key": {"category": "A", "file_path": "A/1"}, "doc_count": 1}], "after_key": {"category": "A", "file_path": "A/1"}}}}, {"aggregations": {"paths": {"buckets": []}}}]

    assert list(manager.iter_all_category_file_paths(batch_size=1)) == [("A", "A/1", 1)]
    assert es.search.call_args_list[1].kwargs["aggs"]["paths"]["composite"]["after"] == {"category": "A", "file_path": "A/1"}
