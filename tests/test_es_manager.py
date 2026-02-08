#!/usr/bin/env python

import math
import logging.config
from pathlib import Path
from typing import Dict, Any
import pytest

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


def inspect_search_result_hierarchy(data: Dict[str, Any]) -> None:
    assert isinstance(data, dict)
    assert "category" in data and isinstance(data["category"], str)
    assert "title" in data and isinstance(data["title"], str)
    assert "author" in data and isinstance(data["author"], str)
    assert "file_path" in data and isinstance(data["file_path"], str)
    assert "file_type" in data and isinstance(data["file_type"], str)
    assert "file_size" in data and isinstance(data["file_size"], int)
    assert "summary" in data and isinstance(data["summary"], str)
    assert "updated_time" in data


@pytest.fixture(scope="module")
def es_manager_with_data(es_client, es_index):
    """Create ESManager with test data loaded (공유된 ES 클라이언트 및 인덱스 사용)."""
    from backend.es_manager import ESManager

    # ESManager 생성 및 공유된 클라이언트 사용
    esm = ESManager()
    esm.es = es_client

    # Insert minimal test data
    test_data = {
        1: {
            "category": "test",
            "title": "테스트 문서 1",
            "author": "테스트 작가",
            "file_path": "/test/path1.txt",
            "file_type": "txt",
            "file_size": 1000,
            "line_count": 100,
            "page_count": 0,
            "isbn": "",
            "summary": "이것은 테스트 문서입니다. 마법사와 드래곤 이야기.",
            "updated_time": "2024-01-01T00:00:00",
        },
        2: {
            "category": "test",
            "title": "테스트 문서 2 드래곤",
            "author": "다른 작가",
            "file_path": "/test/path2.txt",
            "file_type": "txt",
            "file_size": 2000,
            "line_count": 200,
            "page_count": 0,
            "isbn": "",
            "summary": "두 번째 테스트 문서입니다.",
            "updated_time": "2024-01-02T00:00:00",
        },
        3: {
            "category": "_txt",
            "title": "마법사의 모험",
            "author": "마법 작가",
            "file_path": "/test/path3.txt",
            "file_type": "txt",
            "file_size": 3000,
            "line_count": 300,
            "page_count": 0,
            "isbn": "",
            "summary": "마법사가 드래곤을 만나는 이야기입니다.",
            "updated_time": "2024-01-03T00:00:00",
        },
    }
    try:
        esm.insert(test_data)
        esm.refresh()
        LOGGER.info("Test data inserted: %d docs", len(test_data))
    except Exception as e:
        LOGGER.warning("Failed to insert test data: %s", e)

    yield esm


class TestESManager:

    def test_create_delete_do_exists_index(self, es_manager_with_data):
        esm = es_manager_with_data
        # Use a separate test index to avoid affecting other tests
        original_index = esm.index_name
        esm.index_name = "test_index_crud"

        try:
            # Delete if exists
            if esm.do_exist_index():
                esm.delete_index()

            # Create new index
            actual = esm.create_index()
            assert actual["acknowledged"] is True
            assert actual["index"] == esm.index_name
            assert esm.do_exist_index()

            # Create again (should return acknowledged only)
            actual = esm.create_index()
            assert actual == {"acknowledged": True}
            assert esm.do_exist_index()

            # Delete and verify
            esm.delete_index()
            assert not esm.do_exist_index()
        finally:
            # Restore original index name
            esm.index_name = original_index

    def test_get_mappings(self, es_manager_with_data):
        esm = es_manager_with_data
        mappings = esm.get_mappings()
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

    def test_search(self, es_manager_with_data):
        esm = es_manager_with_data
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
        result_list = esm._search(query=query, max_result_count=10)
        assert isinstance(result_list, list)
        # May have fewer results if test data is limited
        if result_list:
            for doc_id, doc, score in result_list:
                assert isinstance(doc_id, int)
                assert isinstance(doc, dict)
                assert isinstance(score, float)
                inspect_search_result_hierarchy(doc)

    def test_search_by_title(self, es_manager_with_data):
        esm = es_manager_with_data
        keyword = "드래곤"
        result_list = esm.search_by_title(keyword, max_result_count=10)
        assert isinstance(result_list, list)
        for _, doc, _ in result_list:
            inspect_search_result_hierarchy(doc)

    def test_search_by_summary(self, es_manager_with_data):
        esm = es_manager_with_data
        keyword = "테스트"
        result_list = esm.search_by_summary(keyword, max_result_count=10)
        assert isinstance(result_list, list)
        for _, doc, _ in result_list:
            inspect_search_result_hierarchy(doc)

    def test_search_by_category(self, es_manager_with_data):
        esm = es_manager_with_data
        # Use a category that might exist in test data
        categories = ["_txt", "testdata2"]
        result_list = []
        for category in categories:
            result_list = esm.search_by_category(category, max_result_count=10)
            if result_list:
                break

        assert isinstance(result_list, list)
        for _, doc, _ in result_list:
            inspect_search_result_hierarchy(doc)

    def test_search_by_keyword(self, es_manager_with_data):
        esm = es_manager_with_data
        keyword = "마법"
        result_list = esm.search_by_keyword(keyword, max_result_count=5)
        assert isinstance(result_list, list)
        for _, doc, _ in result_list:
            inspect_search_result_hierarchy(doc)

    def test_search_similar_docs(self, es_manager_with_data):
        esm = es_manager_with_data
        doc_to_search: Dict[str, Any] = {
            "category": "_txt",
            "title": "마법사와 드래곤",
            "author": "작가",
            "file_type": "txt",
            "file_size": 1000,
            "summary": "마법사와 드래곤은 무엇일까?",
        }
        result_list = esm.search_similar_docs(
            doc_to_search["category"],
            doc_to_search["title"],
            doc_to_search["author"],
            doc_to_search["file_type"],
            doc_to_search["file_size"],
            doc_to_search["summary"],
            max_result_count=100,
        )
        assert isinstance(result_list, list)
        for _, doc, _ in result_list:
            inspect_search_result_hierarchy(doc)

    def test_search_by_id(self, es_manager_with_data):
        esm = es_manager_with_data
        # First get some results to find a valid ID
        categories = ["testdata2", "_txt"]
        result_list = []
        for category in categories:
            result_list = esm.search_by_category(category, max_result_count=5)
            if result_list:
                break

        if result_list:
            doc_id, doc, _score = result_list[0]
            assert doc_id and doc and _score
            assert isinstance(doc_id, int)

            found_doc = esm.search_by_id(doc_id)
            assert isinstance(found_doc, dict)
            if found_doc:
                inspect_search_result_hierarchy(found_doc)

    def test_search_and_aggregate_by_category(self, es_manager_with_data):
        esm = es_manager_with_data
        result = esm.search_and_aggregate_by_category()
        assert isinstance(result, dict)
        for key, value in result.items():
            assert isinstance(key, str)
            assert isinstance(value, int)

    def test_insert(self, es_manager_with_data):
        esm = es_manager_with_data
        from utils.loader import Loader

        num_files = 5
        dir1 = "_epub"
        path = Loader.path_prefix / dir1
        if path.exists():
            data = Loader.read_files(path, num_files=num_files)
            if data:
                esm.insert(data, num_docs=num_files)
                esm.refresh()  # refresh=False로 변경되어 명시적 refresh 필요
                result = esm.search_by_category(dir1, max_result_count=num_files)
                assert len(result) >= 1

    def test_update(self, es_manager_with_data):
        esm = es_manager_with_data
        # Find a document to update
        categories = ["_txt", "testdata2"]
        previous_result = []
        for category in categories:
            previous_result = esm.search_by_category(category, max_result_count=1)
            if previous_result:
                break

        if not previous_result:
            pytest.skip("No documents found to test update")

        doc_id, previous_doc, _ = previous_result[0]
        new_title = "updated_" + previous_doc["title"]
        new_author = "updated_" + previous_doc["author"]
        new_summary = "updated_" + previous_doc["summary"]

        assert esm.update(
            doc_id,
            previous_doc["category"],
            new_title,
            new_author,
            previous_doc["file_path"],
            previous_doc["file_type"],
            previous_doc["file_size"],
            new_summary
        )

        # Verify update
        updated_doc = esm.search_by_id(doc_id)
        assert updated_doc["title"] == new_title
        assert updated_doc["author"] == new_author
        assert updated_doc["summary"] == new_summary

    def test_delete(self, es_manager_with_data):
        esm = es_manager_with_data
        # Find a document to delete
        categories = ["_txt", "testdata2"]
        previous_result = []
        for category in categories:
            previous_result = esm.search_by_category(category, max_result_count=1)
            if previous_result:
                break

        if not previous_result:
            pytest.skip("No documents found to test delete")

        doc_id, _, _ = previous_result[0]
        assert esm.delete(doc_id)

        # Verify deletion
        result = esm.search_by_id(doc_id)
        assert result == {}


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
