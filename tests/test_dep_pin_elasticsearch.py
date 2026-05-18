#!/usr/bin/env python
"""elasticsearch dependency pinning.

backend 사용처:
- backend/es_manager.py: Elasticsearch 클라이언트, 인덱스 CRUD, 검색, bulk, scroll
- backend/es_manager.py:118: from elasticsearch import BadRequestError

박제 API:
- Elasticsearch(hosts=[...], basic_auth=(...), request_timeout, retry_on_timeout)
- 메서드: info, search, bulk, scroll, clear_scroll, mget, msearch, count, update,
  update_by_query, delete, delete_by_query, get
- indices namespace: exists, create, delete, get_mapping, put_mapping, refresh
- cluster namespace: health
- BadRequestError 예외 클래스
"""

import unittest


class TestElasticsearchImports(unittest.TestCase):
    def test_elasticsearch_class_importable(self):
        from elasticsearch import Elasticsearch

        self.assertTrue(callable(Elasticsearch))

    def test_bad_request_error_importable(self):
        """es_manager.py: from elasticsearch import BadRequestError"""
        from elasticsearch import BadRequestError

        self.assertTrue(issubclass(BadRequestError, Exception))


class TestElasticsearchClientConstructor(unittest.TestCase):
    """es_manager.py:34
    Elasticsearch(hosts=[url], basic_auth=(user, password),
                  request_timeout=10, retry_on_timeout=True)
    """

    def test_constructor_accepts_hosts_basic_auth_timeout_retry(self):
        from elasticsearch import Elasticsearch

        client = Elasticsearch(hosts=["http://localhost:9999"], basic_auth=("u", "p"), request_timeout=10, retry_on_timeout=True)
        self.assertIsNotNone(client)


class TestElasticsearchClientMethods(unittest.TestCase):
    """es_manager.py가 사용하는 메서드의 존재 박제"""

    def setUp(self):
        from elasticsearch import Elasticsearch

        self.Elasticsearch = Elasticsearch

    def test_required_top_level_methods(self):
        for method in ("info", "search", "bulk", "scroll", "clear_scroll", "mget", "msearch", "count", "update", "update_by_query", "delete", "delete_by_query", "get", "index"):
            self.assertTrue(hasattr(self.Elasticsearch, method), f"missing {method}")

    def test_indices_namespace_methods(self):
        from elasticsearch._sync.client.indices import IndicesClient

        for method in ("exists", "create", "delete", "get_mapping", "put_mapping", "refresh"):
            self.assertTrue(hasattr(IndicesClient, method), f"missing indices.{method}")

    def test_cluster_namespace_methods(self):
        from elasticsearch._sync.client.cluster import ClusterClient

        self.assertTrue(hasattr(ClusterClient, "health"))


class TestElasticsearchSearchKeywordArguments(unittest.TestCase):
    """es_manager.py가 search()에 전달하는 keyword argument들이 시그니처에 존재함을 확인.

    es_manager.py 사용 형태:
      es.search(index=..., query=..., sort=..., size=..., track_scores=True)
      es.search(index=..., query=..., sort=..., scroll="10m", track_scores=True, size=...)
      es.search(index=..., query=..., sort=..., from_=offset, size=..., track_scores=True, track_total_hits=True)
      es.search(index=..., body={"query": ...})
    """

    def test_search_signature_accepts_expected_kwargs(self):
        import inspect

        from elasticsearch import Elasticsearch

        sig = inspect.signature(Elasticsearch.search)
        params = set(sig.parameters)
        for kw in ("index", "query", "sort", "size", "from_", "scroll", "track_scores", "track_total_hits", "body"):
            self.assertIn(kw, params, f"search() missing kwarg {kw}")

    def test_bulk_signature_accepts_body_timeout_refresh(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.bulk).parameters)
        for kw in ("body", "timeout", "refresh"):
            self.assertIn(kw, params, f"bulk() missing kwarg {kw}")

    def test_scroll_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.scroll).parameters)
        for kw in ("scroll_id", "scroll"):
            self.assertIn(kw, params, f"scroll() missing kwarg {kw}")

    def test_clear_scroll_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.clear_scroll).parameters)
        self.assertIn("scroll_id", params)

    def test_mget_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.mget).parameters)
        self.assertIn("docs", params)
        # es_manager.py: response = self.es.mget(docs=docs, source=False)
        # es_manager.py: response = self.es.mget(docs=docs, source=["file_path"])
        self.assertIn("source", params)

    def test_msearch_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.msearch).parameters)
        self.assertIn("searches", params)

    def test_count_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.count).parameters)
        for kw in ("index", "query"):
            self.assertIn(kw, params)

    def test_update_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.update).parameters)
        for kw in ("index", "id", "body", "refresh"):
            self.assertIn(kw, params, f"update() missing kwarg {kw}")

    def test_update_by_query_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.update_by_query).parameters)
        for kw in ("index", "query", "script", "conflicts", "refresh"):
            self.assertIn(kw, params, f"update_by_query() missing kwarg {kw}")

    def test_delete_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.delete).parameters)
        for kw in ("index", "id", "refresh"):
            self.assertIn(kw, params, f"delete() missing kwarg {kw}")

    def test_delete_by_query_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.delete_by_query).parameters)
        for kw in ("index", "query", "body", "conflicts", "refresh"):
            self.assertIn(kw, params, f"delete_by_query() missing kwarg {kw}")

    def test_get_signature(self):
        import inspect

        from elasticsearch import Elasticsearch

        params = set(inspect.signature(Elasticsearch.get).parameters)
        for kw in ("index", "id"):
            self.assertIn(kw, params)


class TestIndicesClientSignatures(unittest.TestCase):
    """es_manager.py: self.es.indices.* 호출 시그니처"""

    def test_create_signature(self):
        import inspect

        from elasticsearch._sync.client.indices import IndicesClient

        params = set(inspect.signature(IndicesClient.create).parameters)
        for kw in ("index", "body"):
            self.assertIn(kw, params, f"indices.create missing {kw}")

    def test_exists_signature(self):
        import inspect

        from elasticsearch._sync.client.indices import IndicesClient

        params = set(inspect.signature(IndicesClient.exists).parameters)
        self.assertIn("index", params)

    def test_delete_signature(self):
        import inspect

        from elasticsearch._sync.client.indices import IndicesClient

        params = set(inspect.signature(IndicesClient.delete).parameters)
        self.assertIn("index", params)

    def test_get_mapping_signature(self):
        import inspect

        from elasticsearch._sync.client.indices import IndicesClient

        params = set(inspect.signature(IndicesClient.get_mapping).parameters)
        self.assertIn("index", params)

    def test_put_mapping_signature(self):
        """es_manager.py: indices.put_mapping(index=..., properties=...)"""
        import inspect

        from elasticsearch._sync.client.indices import IndicesClient

        params = set(inspect.signature(IndicesClient.put_mapping).parameters)
        for kw in ("index", "properties"):
            self.assertIn(kw, params, f"indices.put_mapping missing {kw}")

    def test_refresh_signature(self):
        import inspect

        from elasticsearch._sync.client.indices import IndicesClient

        params = set(inspect.signature(IndicesClient.refresh).parameters)
        self.assertIn("index", params)


class TestBadRequestErrorRaiseCatch(unittest.TestCase):
    """es_manager.py:186 except BadRequestError as e"""

    def test_can_raise_and_catch(self):
        from elasticsearch import BadRequestError

        with self.assertRaises(BadRequestError):
            raise BadRequestError(message="test", meta=None, body=None)


if __name__ == "__main__":
    unittest.main()
