import unittest
import os
import sys
from pathlib import Path
from unittest.mock import patch, MagicMock
from typing import Any

import pytest
from elastic_transport import ConnectionError
from elasticsearch import BadRequestError, NotFoundError
from types import SimpleNamespace

os.environ.setdefault("TM_ES_URL", "http://localhost:9200")
os.environ.setdefault("TM_ES_BOOK_INDEX", "test")
os.environ.setdefault("TM_ES_USER", "test")
os.environ.setdefault("TM_ES_PASSWORD", "test")
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from backend.es_manager import ESManager


class TestES(unittest.TestCase):
    def test_init(self):
        with patch("backend.es_manager.Elasticsearch"):
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

    def create(self, index: str, body: dict[str, Any]):
        self.created = True
        return {"acknowledged": True}

    def get_mapping(self, index: str):
        return self.mapping

    def put_mapping(self, index: str, properties: dict[str, Any]):
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

    def mget(self, docs: list[dict[str, Any]], source=False):
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

    def msearch(self, searches: list[dict[str, Any]]):
        return {"responses": [{"hits": {"hits": [{"_score": 10.0}]}}, {"hits": {"total": {"value": 1}, "hits": [{"_id": "2", "_source": {"a": 2}, "_score": 5.0}]}}]}

    def get(self, index: str, id: str):
        raise NotFoundError("not found", meta=None, body=None)

    def count(self, index: str, query: dict[str, Any]):
        return {"count": 3}

    def delete_by_query(self, index: str, body: dict[str, Any] | None = None, conflicts="abort", refresh=False, query: dict[str, Any] | None = None):
        return {"deleted": 2, "failures": []}

    def delete_by_query_raises(self, *args, **kwargs):
        raise RuntimeError("fail")

    def update_by_query(self, index: str, query: dict[str, Any], script: dict[str, Any], conflicts="abort", refresh=False):
        return {"updated": 4, "failures": []}

    def update(self, index: str, id: str, body: dict[str, Any], refresh=True):
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

    def raise_exists(index: str, body: dict[str, Any]):
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
    results, total = manager.search_similar_docs_paged(title="t", author="a", file_size=100, summary="s", size=3, offset=1)
    assert total == 2
    assert results == [(1, {"a": 1}, 10.0)]


def test_search_builders_delegate_to__search():
    es = DummyES()
    manager = make_manager(es)
    captured = []

    def fake_search(query, sort=None, max_result_count=-1):
        captured.append((query, sort, max_result_count))
        return [(1, {"ok": True}, 100.0)]

    manager._search = fake_search
    assert manager.search_by_title("hello", "epub", 123, 5)
    assert manager.search_by_summary("summary", 7)
    assert manager.search_by_category("cat", 9)
    assert manager.search_by_keyword("kw", 11)
    assert manager.search_similar_docs(title="t", author="a", file_size=100, summary="s", exclude_id=3, max_result_count=4)

    assert captured[0][2] == 5
    assert captured[2][1] == ["author.keyword", "title.keyword"]
    assert {"term": {"_id": "3"}} in captured[4][0]["bool"]["must_not"]


def test_search_paged_returns_empty_when_base_score_non_positive():
    es = DummyES()
    manager = make_manager(es)

    def zero_score_search(**kwargs):
        return {"hits": {"total": {"value": 3}, "max_score": 0, "hits": []}}

    es.search = zero_score_search
    assert manager._search_paged({"match_all": {}}, size=10, offset=0) == ([], 3)


def test_count_by_categories_handles_msearch_error():
    es = DummyES()
    manager = make_manager(es)
    es.msearch = lambda searches: {"responses": [{"error": "boom"}, {"hits": {"total": {"value": 2}}}]}
    assert manager.count_by_categories(["A", "B"]) == {"A": 0, "B": 2}


def test_delete_returns_false_when_not_deleted():
    es = DummyES()
    manager = make_manager(es)
    es.delete = lambda index, id, refresh=True: {"result": "noop"}
    assert manager.delete(1) is False


def test_update_returns_true_when_no_failed_shards():
    es = DummyES()
    manager = make_manager(es)
    es.update = lambda index, id, body, refresh=True: {"_shards": {"failed": 0}}
    assert manager.update(1, title="ok") is True


def test_get_mappings_when_index_missing():
    es = DummyES()
    manager = make_manager(es)
    manager.do_exist_index = lambda: False
    assert manager.get_mappings() == {}


def test_search_max_score_none_returns_empty():
    class ESNoScore(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            return {"hits": {"max_score": None, "hits": []}}

    manager = make_manager(ESNoScore())
    assert manager._search({"match": {"title": "a"}}, max_result_count=10) == []


def test_search_paged_base_score_zero():
    class ESZeroScore(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            return {"hits": {"total": {"value": 3}, "max_score": 0.0, "hits": []}}

    manager = make_manager(ESZeroScore())
    results, total = manager._search_paged({"match": {"title": "a"}}, size=10, offset=0)
    assert results == []
    assert total == 3


def test_search_paged_with_self_score_errors():
    class ESBad(DummyES):
        def msearch(self, searches: list[dict[str, Any]]):
            return {"responses": [{"error": {"reason": "bad"}}, {"error": {"reason": "bad2"}}]}

    manager = make_manager(ESBad())
    results, total = manager._search_paged_with_self_score([{"match": {"title": {"query": "a"}}}], exclude_id=1, size=10, offset=0)
    assert results == []
    assert total == 0

    class ESBaseZero(DummyES):
        def msearch(self, searches: list[dict[str, Any]]):
            return {"responses": [{"hits": {"hits": []}}, {"hits": {"total": {"value": 2}, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 5.0}]}}]}

    manager = make_manager(ESBaseZero())
    results, total = manager._search_paged_with_self_score([{"match": {"title": {"query": "a"}}}], exclude_id=1, size=10, offset=0)
    assert results == []
    assert total == 2


def test_count_by_categories_error_response():
    class ESError(DummyES):
        def msearch(self, searches: list[dict[str, Any]]):
            return {"responses": [{"error": {"reason": "bad"}}, {"hits": {"total": {"value": 5}}}]}

    manager = make_manager(ESError())
    result = manager.count_by_categories(["A", "B"])
    assert result == {"A": 0, "B": 5}


def test_ensure_category_nori_subfield_exception():
    class ESBad(DummyES):
        def __init__(self):
            super().__init__()
            self.indices = DummyIndices()

    es = ESBad()
    manager = make_manager(es)

    def raise_mapping(index: str):
        raise RuntimeError("boom")

    es.indices.get_mapping = raise_mapping  # type: ignore[assignment]
    manager._ensure_category_nori_subfield()


def test_delete_by_file_paths_with_exclude_ids():
    class ESDelete(DummyES):
        def delete_by_query(self, index: str, body: dict[str, Any] | None = None, conflicts="abort", refresh=False, query: dict[str, Any] | None = None):
            self.last_body = body
            return {"deleted": 3}

    es = ESDelete()
    manager = make_manager(es)
    deleted = manager.delete_by_file_paths(["/a", "/b"], exclude_ids=[1, 2])
    assert deleted == 3
    must_not = es.last_body["query"]["bool"]["must_not"]
    assert {"ids": {"values": ["1", "2"]}} in must_not


def test_search_scroll_max_score_none():
    class ESScroll(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            return {"_scroll_id": "scroll1", "hits": {"max_score": 1.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 1.0}]}}

        def scroll(self, scroll_id: str, scroll: str):
            return {"_scroll_id": "scroll1", "hits": {"max_score": None, "hits": [{"_id": "2", "_source": {"a": 2}, "_score": 1.0}]}}

    es = ESScroll()
    manager = make_manager(es)
    assert manager._search({"match": {"title": "a"}}, max_result_count=20000) == []


def test_search_scroll_clear_scroll_error():
    class ESScroll(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            return {"_scroll_id": "scroll1", "hits": {"max_score": 1.0, "hits": []}}

        def clear_scroll(self, scroll_id: str):
            raise RuntimeError("boom")

    es = ESScroll()
    manager = make_manager(es)
    assert manager._search({"match": {"title": "a"}}, max_result_count=20000) == []


def test_search_by_title_summary_category_defaults():
    es = DummyES()
    manager = make_manager(es)
    assert manager.search_by_title("t")  # default max_result_count path
    assert manager.search_by_summary("s")
    assert manager.search_by_category("c")


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
        def update(self, index: str, id: str, body: dict[str, Any], refresh=True):
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


# ---- coverage: uncovered lines ----


def test_init_missing_env_var(monkeypatch: pytest.MonkeyPatch):
    """Lines 26-27: missing env var → sys.exit"""
    monkeypatch.delenv("TM_ES_BOOK_INDEX", raising=False)
    with pytest.raises(SystemExit):
        ESManager()


def test_bulk_update_paths_empty():
    """Line 85: empty updates → return 0"""
    es = DummyES()
    manager = make_manager(es)
    assert manager.bulk_update_paths({}) == 0


def test_bulk_update_paths_final_failure(monkeypatch: pytest.MonkeyPatch):
    """Lines 110-111: bulk_update_paths retries exhausted → raise"""

    class ESAlwaysFail(DummyES):
        def bulk(self, body, timeout="60s", refresh=False):
            raise ConnectionError("persistent failure")

    es = ESAlwaysFail()
    manager = make_manager(es)
    monkeypatch.setattr("backend.es_manager.time.sleep", lambda _: None)
    with pytest.raises(ConnectionError):
        manager.bulk_update_paths({1: {"file_path": "/a"}}, max_retries=2)


def test_create_index_non_exists_bad_request():
    """Line 191: BadRequestError without resource_already_exists → re-raise"""
    es = DummyES()
    manager = make_manager(es)

    def raise_other(index: str, body: dict[str, Any]):
        raise BadRequestError("some_other_error", meta=SimpleNamespace(status=400), body=None)

    manager.do_exist_index = lambda: False
    es.indices.create = raise_other
    with pytest.raises(BadRequestError):
        manager.create_index()


def test_get_mappings_when_exists():
    """Line 214: get_mappings when index exists"""
    es = DummyES()
    manager = make_manager(es)
    es.indices.mapping = {"exists": True, "idx": {"mappings": {"properties": {"title": {"type": "text"}}}}}
    manager.do_exist_index = lambda: True
    result = manager.get_mappings()
    assert "properties" in result


def test_search_default_max_result_count():
    """Line 220: _search with max_result_count < 0 uses default"""
    es = DummyES()
    manager = make_manager(es)
    results = manager._search({"match": {"title": "a"}})
    assert len(results) <= manager.DEFAULT_MAX_RESULT_COUNT


def test_search_scroll_early_exit_on_max():
    """Lines 248, 255, 264-268: scroll path with result_count >= max_result_count"""

    class ESScrollMany(DummyES):
        def __init__(self):
            super().__init__()
            self._call = 0

        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            if scroll:
                hits = [{"_id": str(i), "_source": {"a": i}, "_score": 2.0} for i in range(5)]
                return {"_scroll_id": "s1", "hits": {"max_score": 2.0, "hits": hits}}
            return super().search(index, query, sort, size, track_scores, track_total_hits, from_, body, scroll)

        def scroll(self, scroll_id: str, scroll: str):
            self._call += 1
            hits = [{"_id": str(i + 10), "_source": {"a": i}, "_score": 2.0} for i in range(5)]
            return {"_scroll_id": "s2", "hits": {"max_score": 2.0, "hits": hits}}

    es = ESScrollMany()
    manager = make_manager(es)
    # max_result_count > 10000 forces scroll, but cap at 3
    results = manager._search({"match": {"title": "a"}}, max_result_count=10001)
    assert len(results) > 0


def test_search_by_title_negative_max():
    """Lines 295-299: search_by_title with negative max_result_count"""
    es = DummyES()
    manager = make_manager(es)
    results = manager.search_by_title("t", max_result_count=-1)
    assert isinstance(results, list)


def test_search_by_summary_negative_max():
    """Lines 302-306: search_by_summary with negative max_result_count"""
    es = DummyES()
    manager = make_manager(es)
    results = manager.search_by_summary("s", max_result_count=-1)
    assert isinstance(results, list)


def test_search_by_category_negative_max():
    """Line 310: search_by_category with negative max_result_count"""
    es = DummyES()
    manager = make_manager(es)
    results = manager.search_by_category("c", max_result_count=-1)
    assert isinstance(results, list)


def test_search_by_keyword_negative_max():
    """Line 319: search_by_keyword with negative max_result_count"""
    es = DummyES()
    manager = make_manager(es)
    results = manager.search_by_keyword("k", max_result_count=-1)
    assert isinstance(results, list)


def test_search_similar_docs_negative_max():
    """Line 335: search_similar_docs with negative max_result_count"""
    es = DummyES()
    manager = make_manager(es)

    def fake_search(query, sort=None, max_result_count=-1):
        return [(1, {"a": 1}, 100.0)]

    manager._search = fake_search
    manager._get_self_score = lambda *a, **kw: 0.0
    results = manager.search_similar_docs(title="t", author="a", summary="s", file_size=10, max_result_count=-1)
    assert isinstance(results, list)


def test_get_self_score_none_doc_id():
    """Line 395: _get_self_score with doc_id=None"""
    es = DummyES()
    manager = make_manager(es)
    assert manager._get_self_score([{"match": {"title": "a"}}], doc_id=None) == 0.0


def test_get_self_score_no_hits():
    """Line 401: _get_self_score with no hits"""

    class ESNoHits(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            if size == 1 and query and query.get("bool", {}).get("filter"):
                return {"hits": {"hits": []}}
            return super().search(index, query, sort, size, track_scores, track_total_hits, from_, body, scroll)

    manager = make_manager(ESNoHits())
    assert manager._get_self_score([{"match": {"title": "a"}}], doc_id=999) == 0.0


def test_get_all_file_paths_grouped_clear_scroll_error():
    """Lines 446-447: clear_scroll raises exception during get_all_file_paths_grouped"""

    class ESScrollClearFail(DummyES):
        def search(self, index: str, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            if body is not None and body.get("query", {}).get("match_all") is not None:
                return {"_scroll_id": "scroll1", "hits": {"hits": [{"_source": {"category": "A", "file_path": "/a"}}]}}
            return super().search(index, query, sort, size, track_scores, track_total_hits, from_, body, scroll)

        def scroll(self, scroll_id: str, scroll: str):
            return {"_scroll_id": "scroll2", "hits": {"hits": []}}

        def clear_scroll(self, scroll_id: str):
            raise RuntimeError("clear scroll failed")

    es = ESScrollClearFail()
    manager = make_manager(es)
    result = manager.get_all_file_paths_grouped()
    assert result == {"A": {"/a"}}


def test_insert_final_failure(monkeypatch: pytest.MonkeyPatch):
    """Lines 502-503: insert bulk retries exhausted → raise"""

    class ESAlwaysFail(DummyES):
        def bulk(self, body, timeout="60s", refresh=False):
            raise ConnectionError("persistent failure")

    es = ESAlwaysFail()
    manager = make_manager(es)
    monkeypatch.setattr("backend.es_manager.time.sleep", lambda _: None)
    with pytest.raises(ConnectionError):
        manager.insert({1: {"file_path": "/a"}}, max_retries=2)


def test_insert_num_docs_limit():
    """Line 507: insert stops when data_count >= num_docs"""

    class ESBulkOk(DummyES):
        def bulk(self, body, timeout="60s", refresh=False):
            return {"errors": False}

    es = ESBulkOk()
    manager = make_manager(es)
    data = {i: {"file_path": f"/{i}"} for i in range(10)}
    result = manager.insert(data, num_docs=3)
    # Should stop after processing first chunk that hits num_docs
    assert len(result) <= 10


def test_update_with_file_size_and_summary():
    """Lines 529, 531: update with file_size and summary"""

    class ESUpdateCapture(DummyES):
        def __init__(self):
            super().__init__()
            self.last_body = None

        def update(self, index: str, id: str, body: dict[str, Any], refresh=True):
            self.last_body = body
            return {"_shards": {"failed": 0}}

    es = ESUpdateCapture()
    manager = make_manager(es)
    result = manager.update(1, title="t", file_size=1024, summary="test summary")
    assert result is True
    doc = es.last_body["doc"]
    assert doc["file_size"] == 1024
    assert doc["summary"] == "test summary"


def test_delete_by_category_no_prefix():
    """Line 612: delete_by_category with prefix=False"""

    class ESDeleteCapture(DummyES):
        def __init__(self):
            super().__init__()
            self.last_query = None

        def delete_by_query(self, index: str, body: dict[str, Any] | None = None, conflicts="abort", refresh=False, query: dict[str, Any] | None = None):
            self.last_query = query
            return {"deleted": 1, "failures": []}

    es = ESDeleteCapture()
    manager = make_manager(es)
    result = manager.delete_by_category("A", prefix=False)
    assert result["deleted"] == 1
    assert es.last_query == {"term": {"category": "A"}}


def test_delete_returns_false():
    """Line 622: delete when result != 'deleted'"""

    class ESDeleteFail(DummyES):
        def delete(self, index: str, id: str, refresh=True):
            return {"result": "not_found"}

    manager = make_manager(ESDeleteFail())
    assert manager.delete(1) is False


# ---- merged from test_es_retry_check.py ----
import pytest
from elasticsearch import Elasticsearch
from elastic_transport import ConnectionError as ESConnectionError


def _set_es_env(monkeypatch):
    monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_success_first_try(mock_es_init, mock_info, monkeypatch):
    _set_es_env(monkeypatch)
    mock_info.return_value = {"cluster_name": "test"}

    from backend.es_manager import ESManager

    ESManager()

    mock_info.assert_called_once()


@patch("backend.es_manager.time.sleep")
@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_retry_then_success(mock_es_init, mock_info, mock_sleep, monkeypatch):
    _set_es_env(monkeypatch)
    mock_info.side_effect = [ESConnectionError("connection refused"), ESConnectionError("connection refused"), {"cluster_name": "test"}]

    from backend.es_manager import ESManager

    ESManager()

    assert mock_info.call_count == 3
    assert mock_sleep.call_count == 2
    mock_sleep.assert_any_call(2)
    mock_sleep.assert_any_call(4)


@patch("backend.es_manager.time.sleep")
@patch.object(Elasticsearch, "info")
@patch.object(Elasticsearch, "__init__", return_value=None)
def test_init_all_retries_fail(mock_es_init, mock_info, mock_sleep, monkeypatch):
    _set_es_env(monkeypatch)
    mock_info.side_effect = ESConnectionError("connection refused")

    from backend.es_manager import ESManager

    with pytest.raises(ESConnectionError):
        ESManager()

    assert mock_info.call_count == 5
    assert mock_sleep.call_count == 4


# ---- merged from test_get_all_file_paths_grouped_check.py ----
from unittest.mock import patch
import pytest
from elasticsearch import Elasticsearch


def _set_es_env(monkeypatch):
    monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_index")
    monkeypatch.setenv("TM_ES_URL", "http://localhost:9200")
    monkeypatch.setenv("TM_ES_USER", "elastic")
    monkeypatch.setenv("TM_ES_PASSWORD", "changeme")


def _make_es_manager(monkeypatch):
    _set_es_env(monkeypatch)
    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={"cluster_name": "test"}):
        from backend.es_manager import ESManager

        return ESManager()


def test_basic_grouping(monkeypatch):
    mgr = _make_es_manager(monkeypatch)

    first_page = {"hits": {"hits": [{"_source": {"category": "소설", "file_path": "소설/book1.txt"}}, {"_source": {"category": "소설", "file_path": "소설/book2.txt"}}, {"_source": {"category": "만화", "file_path": "만화/comic1.pdf"}}]}, "_scroll_id": "scroll_1"}

    empty_page = {"hits": {"hits": []}, "_scroll_id": "scroll_2"}

    with patch.object(Elasticsearch, "search", return_value=first_page), patch.object(Elasticsearch, "scroll", return_value=empty_page), patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert result == {"소설": {"소설/book1.txt", "소설/book2.txt"}, "만화": {"만화/comic1.pdf"}}


def test_empty_index(monkeypatch):
    mgr = _make_es_manager(monkeypatch)

    empty_page = {"hits": {"hits": []}, "_scroll_id": "scroll_1"}

    with patch.object(Elasticsearch, "search", return_value=empty_page), patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert result == {}


def test_multi_scroll_pages(monkeypatch):
    mgr = _make_es_manager(monkeypatch)

    page1 = {"hits": {"hits": [{"_source": {"category": "A", "file_path": "A/1.txt"}}]}, "_scroll_id": "s1"}
    page2 = {"hits": {"hits": [{"_source": {"category": "A", "file_path": "A/2.txt"}}, {"_source": {"category": "B", "file_path": "B/1.txt"}}]}, "_scroll_id": "s2"}
    empty = {"hits": {"hits": []}, "_scroll_id": "s3"}

    with patch.object(Elasticsearch, "search", return_value=page1), patch.object(Elasticsearch, "scroll", side_effect=[page2, empty]), patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert result == {"A": {"A/1.txt", "A/2.txt"}, "B": {"B/1.txt"}}


def test_missing_category_skipped(monkeypatch):
    mgr = _make_es_manager(monkeypatch)

    page = {"hits": {"hits": [{"_source": {"category": "", "file_path": "orphan.txt"}}, {"_source": {"category": "X", "file_path": "X/ok.txt"}}]}, "_scroll_id": "s1"}
    empty = {"hits": {"hits": []}, "_scroll_id": "s2"}

    with patch.object(Elasticsearch, "search", return_value=page), patch.object(Elasticsearch, "scroll", return_value=empty), patch.object(Elasticsearch, "clear_scroll"):
        result = mgr.get_all_file_paths_grouped()

    assert "" not in result
    assert result == {"X": {"X/ok.txt"}}


def test_clear_scroll_called_on_success(monkeypatch):
    mgr = _make_es_manager(monkeypatch)

    empty = {"hits": {"hits": []}, "_scroll_id": "s1"}
    mock_clear = MagicMock()

    with patch.object(Elasticsearch, "search", return_value=empty), patch.object(Elasticsearch, "clear_scroll", mock_clear):
        mgr.get_all_file_paths_grouped()

    mock_clear.assert_called_once_with(scroll_id="s1")


def test_clear_scroll_called_on_error(monkeypatch):
    mgr = _make_es_manager(monkeypatch)

    page = {"hits": {"hits": [{"_source": {"category": "A", "file_path": "A/1.txt"}}]}, "_scroll_id": "s1"}
    mock_clear = MagicMock()

    with patch.object(Elasticsearch, "search", return_value=page), patch.object(Elasticsearch, "scroll", side_effect=Exception("network error")), patch.object(Elasticsearch, "clear_scroll", mock_clear):
        with pytest.raises(Exception, match="network error"):
            mgr.get_all_file_paths_grouped()

    mock_clear.assert_called_once_with(scroll_id="s1")


# ---- coverage: search.py and update.py CLI tests ----


def test_search_main(monkeypatch):
    """utils/search.py main() with mocked ESManager"""
    _set_es_env(monkeypatch)
    monkeypatch.setattr("sys.argv", ["search", "test_title"])

    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={}):
        from utils.search import main

        mock_es = MagicMock()
        mock_es.search_by_title.return_value = [(1, {"summary": "short summary text"}, 1.0)]
        mock_es.search_similar_docs.return_value = []
        monkeypatch.setattr("utils.search.ESManager", lambda: mock_es)
        result = main()
        assert result == 0


def test_search_main_with_content(monkeypatch):
    _set_es_env(monkeypatch)
    monkeypatch.setattr("sys.argv", ["search", "title", "content", "txt", "100"])

    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={}):
        from utils.search import main

        mock_es = MagicMock()
        mock_es.search_by_title.return_value = []
        mock_es.search_similar_docs.return_value = [(2, {"summary": "s"}, 1.0)]
        monkeypatch.setattr("utils.search.ESManager", lambda: mock_es)
        result = main()
        assert result == 0


def test_search_print_usage(monkeypatch):
    monkeypatch.setattr("sys.argv", ["search"])
    from utils.search import print_usage

    with pytest.raises(SystemExit):
        print_usage("search")


def test_update_main(monkeypatch):
    """utils/update.py main() with mocked ESManager"""
    _set_es_env(monkeypatch)

    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={}):
        from utils.update import main

        mock_es = MagicMock()
        mock_es.update.return_value = True
        mock_es.search_by_id.return_value = {"title": "t"}
        mock_es.delete.return_value = True
        monkeypatch.setattr("utils.update.ESManager", lambda: mock_es)
        result = main()
        assert result == 0


def test_update_main_failure(monkeypatch):
    _set_es_env(monkeypatch)

    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={}):
        from utils.update import main

        mock_es = MagicMock()
        mock_es.update.return_value = False
        monkeypatch.setattr("utils.update.ESManager", lambda: mock_es)
        result = main()
        assert result == -1


def test_update_main_delete_failure(monkeypatch):
    """Lines 23-24: update succeeds but delete fails"""
    _set_es_env(monkeypatch)

    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={}):
        from utils.update import main

        mock_es = MagicMock()
        mock_es.update.return_value = True
        mock_es.search_by_id.return_value = {"title": "t"}
        mock_es.delete.return_value = False
        monkeypatch.setattr("utils.update.ESManager", lambda: mock_es)
        result = main()
        assert result == -1


def test_search_main_no_args(monkeypatch):
    """Line 29: main() with no args calls print_usage"""
    _set_es_env(monkeypatch)
    monkeypatch.setattr("sys.argv", ["search"])

    with patch.object(Elasticsearch, "__init__", return_value=None), patch.object(Elasticsearch, "info", return_value={}):
        from utils.search import main

        with pytest.raises(SystemExit):
            main()


# ---- coverage: _search scroll edge cases ----


def test_search_scroll_initial_max_score_none():
    """Line 248: initial scroll response has max_score=None (requires max_result_count > 10000)"""

    class ScrollNullES(DummyES):
        def search(self, index, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            if scroll is not None:
                return {"_scroll_id": "s1", "hits": {"max_score": None, "hits": [{"_id": "1", "_source": {}, "_score": 0}]}}
            return super().search(index=index, query=query, sort=sort, size=size, track_scores=track_scores, track_total_hits=track_total_hits, from_=from_, body=body, scroll=scroll)

    es = ScrollNullES()
    manager = make_manager(es)
    # max_result_count > 10000 triggers scroll path
    result = manager._search({"match_all": {}}, max_result_count=20000)
    assert result == []


def test_search_scroll_early_exit_first_batch():
    """Line 255: result_count >= max_result_count in first scroll batch"""

    class ScrollBigBatchES(DummyES):
        def search(self, index, query=None, sort=None, size=10, track_scores=True, track_total_hits=False, from_=0, body=None, scroll=None):
            if scroll is not None:
                hits = [{"_id": str(i), "_source": {"a": i}, "_score": 5.0} for i in range(10)]
                return {"_scroll_id": "s1", "hits": {"max_score": 5.0, "hits": hits}}
            return super().search(index=index, query=query, sort=sort, size=size, track_scores=track_scores, track_total_hits=track_total_hits, from_=from_, body=body, scroll=scroll)

    es = ScrollBigBatchES()
    manager = make_manager(es)
    # max_result_count > 10000 but small enough to early exit in first batch
    result = manager._search({"match_all": {}}, max_result_count=10001)
    assert len(result) == 10


# ---- coverage: update with author field ----


def test_update_with_author():
    """Line 523: update with author field"""

    class UpdateOkES(DummyES):
        def update(self, index, id, body, refresh=True):
            return {"_shards": {"failed": 0}}

    es = UpdateOkES()
    manager = make_manager(es)
    result = manager.update(1, category="A", title="T", author="Author", file_path="a.txt", file_type="txt")
    assert result is True


def test_search_scroll_early_return_at_max_result_count():
    """scroll 경로에서 max_result_count 에 도달하면 즉시 반환한다 (es_manager.py:255)."""
    from unittest.mock import MagicMock

    es = MagicMock()
    manager = make_manager(es)

    # max_result_count > 10000 이어야 scroll 경로에 진입
    max_rc = 10001
    hits = [{"_id": str(i), "_source": {"a": i}, "_score": 1.0} for i in range(max_rc)]
    es.search.return_value = {"_scroll_id": "scroll1", "hits": {"max_score": 1.0, "hits": hits}}
    es.clear_scroll.return_value = {"succeeded": True}

    result = manager.search_by_title("test", max_result_count=max_rc)
    assert len(result) == max_rc
    # finally 블록에서 clear_scroll 호출 확인
    es.clear_scroll.assert_called_once_with(scroll_id="scroll1")
