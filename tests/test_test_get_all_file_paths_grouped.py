import pytest


def _set_es_env(monkeypatch):
    monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


def test_test_get_all_file_paths_grouped(monkeypatch):
    _set_es_env(monkeypatch)
    from backend import test_get_all_file_paths_grouped as tgp

    tgp.test_basic_grouping(None)
    tgp.test_empty_index(None)
    tgp.test_multi_scroll_pages(None)
    tgp.test_missing_category_skipped(None)
    tgp.test_clear_scroll_called_on_success(None)
    tgp.test_clear_scroll_called_on_error(None)
