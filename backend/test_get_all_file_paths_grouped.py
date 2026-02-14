#!/usr/bin/env python

from unittest.mock import patch, MagicMock
import pytest
from elasticsearch import Elasticsearch


@pytest.fixture
def env_vars(monkeypatch):
    monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


def _make_es_manager(env_vars):
    with patch.object(Elasticsearch, "__init__", return_value=None), \
         patch.object(Elasticsearch, "info", return_value={"cluster_name": "test"}):
        from es_manager import ESManager
        return ESManager()


def test_basic_grouping(env_vars):
    """카테고리별로 file_path가 올바르게 그룹핑되는지 확인"""
    mgr = _make_es_manager(env_vars)

    first_page = {"hits": {"hits": [
        {"_source": {"category": "소설", "file_path": "소설/book1.txt"}},
        {"_source": {"category": "소설", "file_path": "소설/book2.txt"}},
        {"_source": {"category": "만화", "file_path": "만화/comic1.pdf"}},
    ]}, "_scroll_id": "scroll_1"}

    empty_page = {"hits": {"hits": []}, "_scroll_id": "scroll_2"}

    with patch.object(Elasticsearch, "search", return_value=first_page), \
         patch.object(Elasticsearch, "scroll", return_value=empty_page), \
         patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert result == {
        "소설": {"소설/book1.txt", "소설/book2.txt"},
        "만화": {"만화/comic1.pdf"},
    }


def test_empty_index(env_vars):
    """빈 인덱스에서 빈 딕셔너리 반환"""
    mgr = _make_es_manager(env_vars)

    empty_page = {"hits": {"hits": []}, "_scroll_id": "scroll_1"}

    with patch.object(Elasticsearch, "search", return_value=empty_page), \
         patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert result == {}


def test_multi_scroll_pages(env_vars):
    """여러 scroll 페이지를 올바르게 병합하는지 확인"""
    mgr = _make_es_manager(env_vars)

    page1 = {"hits": {"hits": [
        {"_source": {"category": "A", "file_path": "A/1.txt"}},
    ]}, "_scroll_id": "s1"}
    page2 = {"hits": {"hits": [
        {"_source": {"category": "A", "file_path": "A/2.txt"}},
        {"_source": {"category": "B", "file_path": "B/1.txt"}},
    ]}, "_scroll_id": "s2"}
    empty = {"hits": {"hits": []}, "_scroll_id": "s3"}

    with patch.object(Elasticsearch, "search", return_value=page1), \
         patch.object(Elasticsearch, "scroll", side_effect=[page2, empty]), \
         patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert result == {"A": {"A/1.txt", "A/2.txt"}, "B": {"B/1.txt"}}


def test_missing_category_skipped(env_vars):
    """category가 빈 문자열인 문서는 건너뜀"""
    mgr = _make_es_manager(env_vars)

    page = {"hits": {"hits": [
        {"_source": {"category": "", "file_path": "orphan.txt"}},
        {"_source": {"category": "X", "file_path": "X/ok.txt"}},
    ]}, "_scroll_id": "s1"}
    empty = {"hits": {"hits": []}, "_scroll_id": "s2"}

    with patch.object(Elasticsearch, "search", return_value=page), \
         patch.object(Elasticsearch, "scroll", return_value=empty), \
         patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert "" not in result
    assert result == {"X": {"X/ok.txt"}}


def test_clear_scroll_called_on_success(env_vars):
    """정상 완료 후 clear_scroll이 호출되는지 확인"""
    mgr = _make_es_manager(env_vars)

    empty = {"hits": {"hits": []}, "_scroll_id": "s1"}
    mock_clear = MagicMock()

    with patch.object(Elasticsearch, "search", return_value=empty), \
         patch.object(Elasticsearch, "clear_scroll", mock_clear):
        mgr.get_all_file_paths_grouped()

    mock_clear.assert_called_once_with(scroll_id="s1")


def test_clear_scroll_called_on_error(env_vars):
    """예외 발생 시에도 clear_scroll이 호출되는지 확인"""
    mgr = _make_es_manager(env_vars)

    page = {"hits": {"hits": [
        {"_source": {"category": "A", "file_path": "A/1.txt"}},
    ]}, "_scroll_id": "s1"}
    mock_clear = MagicMock()

    with patch.object(Elasticsearch, "search", return_value=page), \
         patch.object(Elasticsearch, "scroll", side_effect=Exception("network error")), \
         patch.object(Elasticsearch, "clear_scroll", mock_clear):
        with pytest.raises(Exception, match="network error"):
            mgr.get_all_file_paths_grouped()

    mock_clear.assert_called_once_with(scroll_id="s1")
