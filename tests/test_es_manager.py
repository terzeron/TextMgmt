#!/usr/bin/env python


import unittest
import logging.config
import math
from pathlib import Path
from typing import Dict, Any
from backend.es_manager import ESManager
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class ESManagerTest(unittest.TestCase):
    esm = ESManager()

    @classmethod
    def setUpClass(cls) -> None:
        try:
            cls.esm.create_index()
        except Exception:
            pass

        path1 = Loader.path_prefix / "testdata2"
        data = Loader.read_files(path1, num_files=1000)
        LOGGER.info("%d files read from %s", len(data), path1)
        cls.esm.insert(data)

        path2 = Loader.path_prefix / "_txt"
        data = Loader.read_files(path2, num_files=1000)
        LOGGER.info("%d files read from %s", len(data), path2)
        cls.esm.insert(data)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.esm.delete_index()
        del cls.esm

    def inspect_search_result_hierarchy(self, data: Dict[str, Any]) -> None:
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

    def test_99_create_delete_do_exists_index(self) -> None:
        self.esm.delete_index()
        actual = self.esm.create_index()
        assert actual == {"acknowledged": True, "index": self.esm.index_name, "shards_acknowledged": True}
        assert self.esm.do_exist_index()

        actual = self.esm.create_index()
        assert actual == {"acknowledged": True}
        assert self.esm.do_exist_index()

        self.esm.delete_index()
        assert not self.esm.do_exist_index()

    def test_00_get_mappings(self) -> None:
        mappings = self.esm.get_mappings()
        assert isinstance(mappings, dict)
        assert "properties" in mappings
        assert isinstance(mappings["properties"], dict)
        assert "category" in mappings["properties"]
        assert "title" in mappings["properties"]
        assert "author" in mappings["properties"]
        assert "file_path" in mappings["properties"]
        assert "file_type" in mappings["properties"]
        assert "file_size" in mappings["properties"]
        assert "summary" in mappings["properties"]
        assert "updated_time" in mappings["properties"]

    def test_01_search(self) -> None:
        keyword = "마법사 드래곤"
        file_type = "txt"
        query = {
            "bool": {
                "should": [
                    {"match": {"title": {"query": keyword, "boost": 1.2 + math.log2(len(keyword.split(" ")))}}},
                    {"match": {"file_type": {"query": file_type, "boost": 1}}},
                ]
            }
        }
        result_list = self.esm._search(query=query, max_result_count=10)
        assert isinstance(result_list, list)
        assert len(result_list) == 10
        for doc_id, doc, score in result_list:
            assert isinstance(doc_id, int)
            assert isinstance(doc, dict)
            assert isinstance(score, float)
            self.inspect_search_result_hierarchy(doc)

        first_doc = result_list[0][1]
        match_count = 0
        for word in keyword.split(" "):
            match_count += word in first_doc["title"]
        assert match_count >= 1
        assert match_count / len(keyword.split(" ")) >= 0.1

    def test_02_search_by_title(self) -> None:
        keyword = "드래곤"
        result_list = self.esm.search_by_title(keyword, max_result_count=10)
        assert isinstance(result_list, list)
        assert len(result_list) == 10
        for _, doc, _ in result_list:
            self.inspect_search_result_hierarchy(doc)
            assert keyword in doc["title"] or keyword in doc["summary"] or keyword in doc["file_path"]

    def test_03_search_by_summary(self) -> None:
        keyword = "리오에 의해 거인이 팔뚝에 장비 부숴지듯 산산조각이 나고"
        result_list = self.esm.search_by_summary(keyword, max_result_count=10)
        assert isinstance(result_list, list)
        assert len(result_list) == 10
        for _, doc, _ in result_list:
            self.inspect_search_result_hierarchy(doc)

        first_doc = result_list[0][1]
        match_count = 0
        for word in keyword.split(" "):
            match_count += word in first_doc["summary"]
        assert match_count >= 1
        assert match_count / len(keyword.split(" ")) >= 0.1

    def test_04_search_by_category(self) -> None:
        category = "_txt"
        result_list = self.esm.search_by_category(category, max_result_count=10)
        assert isinstance(result_list, list)
        assert len(result_list) == 10
        for _, doc, _ in result_list:
            self.inspect_search_result_hierarchy(doc)
            assert doc["category"] == category

    def test_05_search_by_keyword(self) -> None:
        keyword = "마법"
        result_list = self.esm.search_by_keyword(keyword, max_result_count=5)
        assert isinstance(result_list, list)
        assert len(result_list) == 5
        for _, doc, _ in result_list:
            self.inspect_search_result_hierarchy(doc)
            assert keyword in doc["title"] or keyword in doc["author"] or keyword in doc["summary"]

    def test_06_search_similar_docs(self) -> None:
        doc_to_search: Dict[str, Any] = {
            "category": "_txt",
            "title": "마법사와 드래곤",
            "author": "작가",
            "file_type": "txt",
            "file_size": 1000,
            "summary": "마법사와 드래곤은 무엇일까?",
        }
        result_list = self.esm.search_similar_docs(
            doc_to_search["category"],
            doc_to_search["title"],
            doc_to_search["author"],
            doc_to_search["file_type"],
            doc_to_search["file_size"],
            doc_to_search["summary"],
            max_result_count=100,
        )
        assert isinstance(result_list, list)
        assert len(result_list) > 0

        # The first result should be the document itself.
        _, first_doc, _ = result_list[0]
        self.inspect_search_result_hierarchy(first_doc)

        # For the other results, we only check the hierarchy.
        for _, other_doc, _ in result_list[1:]:
            self.inspect_search_result_hierarchy(other_doc)

    def test_07_search_by_id(self) -> None:
        result_list = self.esm.search_by_category("testdata2", max_result_count=5)
        assert result_list
        assert isinstance(result_list, list)
        assert len(result_list) > 0

        doc_id, doc, _score = result_list[0]
        assert doc_id and doc and _score
        assert isinstance(doc_id, int)
        assert isinstance(doc, dict)
        assert isinstance(_score, float)
        assert len(doc) > 0

        doc = self.esm.search_by_id(doc_id)
        assert isinstance(doc, dict)
        assert len(doc) >= 1
        self.inspect_search_result_hierarchy(doc)

    def test_06_search_and_aggregate_by_category(self) -> None:
        result = self.esm.search_and_aggregate_by_category()
        assert isinstance(result, list)
        assert len(result) >= 1
        assert isinstance(result[0], str)
        assert isinstance(result[1], str)

    def test_11_insert(self) -> None:
        num_files = 20
        dir1 = "_epub"
        data = Loader.read_files(Loader.path_prefix / dir1, num_files=num_files)
        self.esm.insert(data, num_docs=num_files)
        result = self.esm.search_by_category(dir1, max_result_count=num_files)
        assert len(result) >= num_files * 0.9

        dir2 = "testdata3"
        data = Loader.read_files(Loader.path_prefix / dir2, num_files=num_files)
        self.esm.insert(data, num_docs=num_files)
        result = self.esm.search_by_category(dir2, max_result_count=num_files)
        assert len(result) >= num_files * 0.9

    def test_12_update(self) -> None:
        previous_result = self.esm.search_by_category("_txt", 1)
        assert isinstance(previous_result, list)
        assert len(previous_result) > 0
        doc_id, previous_doc, _ = previous_result[0]
        category = "_epub"
        title = "renamed_" + previous_doc["title"]
        author = "anonymous_" + previous_doc["author"]
        file_type = "epub"
        file_path = previous_doc["file_path"]
        summary = "modified_" + previous_doc["summary"]
        file_size = previous_doc["file_size"]

        assert self.esm.update(doc_id, category, title, author, file_path, file_type, file_size, summary)

        doc = self.esm.search_by_id(doc_id)
        category = doc["category"]
        title = doc["title"]
        author = doc["author"]
        file_type = doc["file_type"]
        file_path = doc["file_path"]
        summary = doc["summary"]
        file_size = doc["file_size"]

        assert doc_id == previous_result[0][0]
        assert category == "_epub"
        assert title == "renamed_" + previous_doc["title"]
        assert author == "anonymous_" + previous_doc["author"]
        assert file_type == "epub"
        assert file_path == previous_doc["file_path"]
        assert summary == "modified_" + previous_doc["summary"]
        assert file_size == previous_doc["file_size"]

    def test_13_delete(self) -> None:
        previous_result = self.esm.search_by_category("_txt", 1)
        assert isinstance(previous_result, list)
        assert len(previous_result) == 1
        doc_id, _, _ = previous_result[0]

        assert self.esm.delete(doc_id)

        result = self.esm.search_by_id(doc_id)
        assert result == {}


if __name__ == "__main__":
    unittest.main()
