import unittest, os, sys
from pathlib import Path
from unittest.mock import patch, MagicMock
from typing import Any, Dict, List

import pytest
from elastic_transport import ConnectionError
from elasticsearch import BadRequestError, NotFoundError
from types import SimpleNamespace

os.environ.setdefault('TM_ES_URL', 'http://localhost:9200')
os.environ.setdefault('TM_ES_BOOK_INDEX', 'test')
os.environ.setdefault('TM_ES_USER', 'test')
os.environ.setdefault('TM_ES_PASSWORD', 'test')
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from backend.es_manager import ESManager
class TestES(unittest.TestCase):
    def test_init(self):
        with patch('backend.es_manager.Elasticsearch'):
            esm = ESManager()
            self.assertIsNotNone(esm)


class DummyIndices:
    def __init__(self):
        self.exists_called = False
        self.created = False
        self.deleted = False
        self.put_mapping_called = False
        self.mapping = {}

    def exists(self, index: str):
        self.exists_called = True
        return self.mapping.get("exists", False)

    def create(self, index: str, body: Dict[str, Any]):
        self.created = True
        return {"acknowledged": True}

    def get_mapping(self, index: str):
        return self.mapping

    def put_mapping(self, index: str, properties: Dict[str, Any]):
        self.put_mapping_called = True
        return {"acknowledged": True}

    def delete(self, index: str):
        self.deleted = True
        return {"acknowledged": True}

    def refresh(self, index: str):
        return {"acknowledged": True}


class DummyES:
    def __init__(self):
        self.indices = DummyIndices()
        self.bulk_calls = 0
        self.search_calls = 0
        self.scroll_calls = 0
        self.cleared_scroll = False

    def info(self):
        return {"ok": True}

    def mget(self, docs: List[Dict[str, Any]], source=False):
        return {"docs": [{"_id": "1", "found": True, "_source": {"file_path": "/a"}}, {"_id": "2", "found": False}]}

    def bulk(self, body, timeout="60s", refresh=False):
        self.bulk_calls += 1
        if self.bulk_calls == 1:
            raise ConnectionError("boom")
        return {"errors": False}

    def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
        self.search_calls += 1
        if body is not None and body.get("aggs"):
            return {"aggregations": {"unique_values": {"buckets": [{"key": "A", "doc_count": 2}]}}}
        if body is not None and body.get("query", {}).get("match_all") is not None:
            return {"_scroll_id": "scroll1", "hits": {"hits": [{"_source": {"category": "A", "file_path": "/a"}}, {"_source": {"category": "A", "file_path": "/b"}}]}}
        if scroll is not None:
            return {"_scroll_id": "scroll1", "hits": {"max_score": 2.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 2.0}]}}
        if size == 1 and query and query.get("bool", {}).get("filter"):
            return {"hits": {"hits": [{"_score": 7.0}]}}
        if track_total_hits:
            return {"hits": {"total": {"value": 2}, "max_score": 5.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 5.0}]}}
        return {"hits": {"max_score": 2.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 2.0}]}}

    def scroll(self, scroll_id: str, scroll: str):
        self.scroll_calls += 1
        return {"_scroll_id": "scroll2", "hits": {"max_score": 2.0, "hits": []}}

    def clear_scroll(self, scroll_id: str):
        self.cleared_scroll = True
        return {"succeeded": True}

    def msearch(self, searches: List[Dict[str, Any]]):
        return {"responses": [{"hits": {"hits": [{"_score": 10.0}]}}, {"hits": {"total": {"value": 1}, "hits": [{"_id": "2", "_source": {"a": 2}, "_score": 5.0}]}}]}

    def get(self, index: str, id: str):
        raise NotFoundError("not found", meta=None, body=None)

    def count(self, index: str, query: Dict[str, Any]):
        return {"count": 3}

    def delete_by_query(self, index: str, body: Dict[str, Any] | None = None, conflicts="abort", refresh=False, query: Dict[str, Any] | None = None):
        return {"deleted": 2, "failures": []}

    def delete_by_query_raises(self, *args, **kwargs):
        raise RuntimeError("fail")

    def update_by_query(self, index: str, query: Dict[str, Any], script: Dict[str, Any], conflicts="abort", refresh=False):
        return {"updated": 4, "failures": []}

    def update(self, index: str, id: str, body: Dict[str, Any], refresh=True):
        return {"_shards": {"failed": 1}}

    def delete(self, index: str, id: str, refresh=True):
        return {"result": "deleted"}


def make_manager(es: DummyES) -> ESManager:
    manager = ESManager.__new__(ESManager)
    manager.es = es
    manager.index_name = "idx"
    return manager


def test_do_exist_index_get_existing_ids_and_paths():
    es = DummyES()
    manager = make_manager(es)
    es.indices.mapping["exists"] = True
    assert manager.do_exist_index() is True
    assert manager.get_existing_ids([1, 2]) == {1}
    assert manager.get_existing_paths([1, 2]) == {1: "/a"}


def test_bulk_update_paths_retries(monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(es)
    updates = {1: {"file_path": "/a"}, 2: {"file_path": "/b"}}
    assert manager.bulk_update_paths(updates, max_retries=2) == 2
    assert es.bulk_calls == 2


def test_create_index_paths(monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(es)

    called = {"ensure": 0}

    def fake_ensure():
        called["ensure"] += 1

    monkeypatch.setattr(manager, "_ensure_category_nori_subfield", fake_ensure)
    monkeypatch.setattr(manager, "do_exist_index", lambda: True)
    assert manager.create_index()["acknowledged"] is True
    assert called["ensure"] == 1

    def raise_exists(index: str, body: Dict[str, Any]):
        raise BadRequestError("resource_already_exists_exception", meta=SimpleNamespace(status=400), body=None)

    monkeypatch.setattr(manager, "do_exist_index", lambda: False)
    monkeypatch.setattr(es.indices, "create", raise_exists)
    assert manager.create_index()["acknowledged"] is True
    assert called["ensure"] == 2


def test_ensure_category_nori_subfield_adds():
    es = DummyES()
    manager = make_manager(es)
    es.indices.mapping = {"idx": {"mappings": {"properties": {"category": {}}}}}
    manager._ensure_category_nori_subfield()
    assert es.indices.put_mapping_called is True


def test_search_and_scroll_and_paged():
    es = DummyES()
    manager = make_manager(es)
    results = manager._search({"match": {"title": "a"}}, max_result_count=5)
    assert results == [(1, {"a": 1}, 100.0)]

    results = manager._search({"match": {"title": "a"}}, max_result_count=20000)
    assert results
    assert es.cleared_scroll is True

    paged, total = manager._search_paged({"match": {"title": "a"}}, size=10, offset=0)
    assert total == 2
    assert paged[0][2] == 100.0


def test_search_paged_with_self_score_and_get_self_score():
    es = DummyES()
    manager = make_manager(es)
    results, total = manager._search_paged_with_self_score([{"match": {"title": {"query": "a"}}}], exclude_id=1, size=10, offset=0)
    assert total == 1
    assert results[0][2] == 50.0
    assert manager._get_self_score([{"match": {"title": {"query": "a"}}}], doc_id=1) == 7.0


def test_search_by_id_not_found():
    es = DummyES()
    manager = make_manager(es)
    assert manager.search_by_id(1) == {}


def test_aggregate_grouped_delete_insert_update_counts():
    es = DummyES()
    manager = make_manager(es)
    assert manager.search_and_aggregate_by_category() == {"A": 2}
    grouped = manager.get_all_file_paths_grouped()
    assert grouped == {"A": {"/a", "/b"}}
    assert manager.delete_by_file_paths(["/a"], exclude_ids=[1]) == 2

    def raise_delete(*args, **kwargs):
        raise RuntimeError("fail")

    es.delete_by_query = raise_delete
    assert manager.delete_by_file_paths(["/a"]) == 0
    es.delete_by_query = DummyES().delete_by_query

    data = {1: {"file_path": "/a"}, 2: {"file_path": "/b"}}
    assert manager.insert(data, max_retries=2) == [1, 2]

    assert manager.update(1, title="t") is False
    assert manager.count_by_category("A", prefix=True) == 3
    assert manager.count_by_categories(["A"]) == {"A": 3}
    assert manager.rename_category("A", "B") == {"updated": 4, "failures": []}
    assert manager.delete_by_category("A", prefix=True) == {"deleted": 2, "failures": []}
    assert manager.delete(1) is True


def test_search_by_keyword_paged_with_exclude_categories():
    es = DummyES()
    manager = make_manager(es)

    called = {}

    def fake_search_paged(query, size=10, offset=0, sort=None, ref_score=0.0):
        called["query"] = query
        return ([(1, {"a": 1}, 10.0)], 1)

    manager._search_paged = fake_search_paged
    results, total = manager.search_by_keyword_paged("kw", size=5, offset=1, exclude_categories=["A", "B"])
    assert total == 1
    assert results
    must_not = called["query"]["bool"]["must_not"]
    assert {"prefix": {"category": "A"}} in must_not
    assert {"prefix": {"category": "B"}} in must_not


def test_search_similar_docs_paged_without_exclude():
    es = DummyES()
    manager = make_manager(es)

    def fake_search_paged(query, size=10, offset=0, sort=None, ref_score=0.0):
        return ([(1, {"a": 1}, 10.0)], 2)

    manager._search_paged = fake_search_paged
    results, total = manager.search_similar_docs_paged(title="t", author="a", summary="s", file_size=10, exclude_id=None, size=10, offset=0)
    assert total == 2
    assert results


def test_search_paged_with_zero_score():
    es = DummyES()
    manager = make_manager(es)

    class DummyZeroScoreES(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            return {"hits": {"total": {"value": 1}, "max_score": 0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 0}]}}

    manager.es = DummyZeroScoreES()
    results, total = manager._search_paged({"match": {"title": "a"}}, size=10, offset=0)
    assert results == []
    assert total == 1


def test_get_existing_ids_and_paths_empty():
    es = DummyES()
    manager = make_manager(es)
    assert manager.get_existing_ids([]) == set()
    assert manager.get_existing_paths([]) == {}


def test_delete_index_when_exists(monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(es)
    monkeypatch.setattr(manager, "do_exist_index", lambda: True)
    manager.delete_index()
    assert es.indices.deleted is True


def test_get_mappings_when_missing(monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(es)
    monkeypatch.setattr(manager, "do_exist_index", lambda: False)
    assert manager.get_mappings() == {}


def test_update_success_and_delete_by_file_paths_empty():
    es = DummyES()

    class DummySuccessES(DummyES):
        def update(self, index: str, id: str, body: Dict[str, Any], refresh=True):
            return {"_shards": {"failed": 0}}

    manager = make_manager(es)
    manager.es = DummySuccessES()
    assert manager.update(1, title="t") is True
    assert manager.delete_by_file_paths([]) == 0


def test_count_by_categories_single(monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(es)
    monkeypatch.setattr(manager, "count_by_category", lambda category, prefix=False: 7)
    assert manager.count_by_categories(["A"]) == {"A": 7}
