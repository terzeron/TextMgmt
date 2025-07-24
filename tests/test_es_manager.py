#!/usr/bin/env python


import unittest
import logging.config
import math
from pathlib import Path
from unittest.mock import MagicMock
from typing import Dict, Any
from backend.es_manager import ESManager
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class ESManagerTest(unittest.TestCase):
    def setUp(self):
        self.esm = ESManager()
        self.esm.es = MagicMock()
        self.doc = {
            "category": "_txt",
            "title": "마법사와 드래곤",
            "author": "작가",
            "file_path": "/path/to/file.txt",
            "file_type": "txt",
            "file_size": 1000,
            "summary": "마법사와 드래곤은 무엇일까?",
            "updated_time": "2021-01-01T00:00:00.000000",
        }

    def inspect_search_result_hierarchy(self, data: Dict[str, Any]):
        assert isinstance(data, dict)
        assert "category" in data
        assert isinstance(data["category"], str)
        assert "title" in data
        assert isinstance(data["title"], str)
        assert "author" in data
        assert isinstance(data["author"], str)
        assert "file_path" in data
        assert isinstance(data["file_path"], str)
        assert "file_type" in data
        assert isinstance(data["file_type"], str)
        assert "file_size" in data
        assert isinstance(data["file_size"], int)
        assert "summary" in data
        assert isinstance(data["summary"], str)
        assert "updated_time" in data

    def test_search_by_keyword(self):
        self.esm.es.search.return_value = {"hits": {"hits": [{"_id": "1", "_source": self.doc, "_score": 1.0}], "max_score": 1.0}}
        result = self.esm.search_by_keyword("마법", max_result_count=1)
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0][1]["title"], self.doc["title"])

    def test_search_similar_docs(self):
        self.esm.es.search.return_value = {"hits": {"hits": [{"_id": "1", "_source": self.doc, "_score": 1.0}], "max_score": 1.0}}
        result = self.esm.search_similar_docs(
            self.doc["category"], self.doc["title"], self.doc["author"],
            self.doc["file_type"], self.doc["file_size"], self.doc["summary"]
        )
        self.assertEqual(len(result), 1)

    def test_insert_update_delete(self):
        self.esm.es.bulk.return_value = {"items": [{"index": {"_id": "1", "status": 201}}]}
        ids = self.esm.insert({1: self.doc})
        self.assertEqual(ids, [1])

        self.esm.es.update.return_value = {"result": "updated"}
        update_doc = self.doc.copy()
        del update_doc["updated_time"]
        self.assertTrue(self.esm.update(1, **update_doc))

        self.esm.es.delete.return_value = {"result": "deleted"}
        self.assertTrue(self.esm.delete(1))

    def test_create_delete_exists_index(self):
        self.esm.es.indices.exists.return_value = False
        self.esm.es.indices.create.return_value = {"acknowledged": True}
        self.esm.create_index()
        self.esm.es.indices.create.assert_called_once()

        self.esm.es.indices.exists.return_value = True
        self.esm.es.indices.delete.return_value = {"acknowledged": True}
        self.esm.delete_index()
        self.esm.es.indices.delete.assert_called_once()

    def test_search_query_structure(self):
        keyword = "test"
        self.esm.search_by_keyword(keyword)
        self.esm.es.search.assert_called_with(
            index=self.esm.index_name,
            body={
                "query": {
                    "bool": {
                        "should": [
                            {"match": {"title": {"query": keyword, "boost": 10}}},
                            {"match": {"author": {"query": keyword, "boost": 5}}},
                            {"match": {"summary": {"query": keyword, "boost": 1}}},
                        ],
                        "minimum_should_match": 1,
                    }
                }
            },
            size=unittest.mock.ANY
        )

    def test_search_with_request_error(self):
        self.esm.es.search.side_effect = Exception("ES error")
        result = self.esm._search(query={})
        self.assertEqual(result, [])

if __name__ == "__main__":
    unittest.main()
