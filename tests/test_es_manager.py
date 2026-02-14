#!/usr/bin/env python

import os
import math
import logging.config
from pathlib import Path
from typing import Dict, Any
from unittest.mock import patch
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

    def test_search_by_keyword_no_category_overlap(self, es_manager_with_data):
        """카테고리와 제목+저자에 겹치는 글자가 없는 문서를 삽입하고 키워드 검색한다"""
        esm = es_manager_with_data
        doc_id = 435
        test_doc = {
            doc_id: {
                "category": "1_한국고전국역총서",
                "title": "열하일기",
                "author": "박지원",
                "file_path": "1_한국고전국역총서/열하일기.epub",
                "file_type": "epub",
                "file_size": 5000,
                "line_count": 0,
                "page_count": 200,
                "isbn": "",
                "summary": "청나라 여행을 통해 조선 사회의 문제점을 비판한 기행문학의 걸작.",
                "updated_time": "2024-06-01T00:00:00",
            },
        }
        esm.insert(test_doc)
        esm.refresh()

        try:
            # 제목으로 검색
            result_list = esm.search_by_keyword("열하일기", max_result_count=5)
            assert len(result_list) >= 1
            found_ids = [doc_id_ for doc_id_, _, _ in result_list]
            assert doc_id in found_ids, f"doc {doc_id}이 검색 결과에 없음: {found_ids}"

            # 저자로 검색
            result_list = esm.search_by_keyword("박지원", max_result_count=5)
            assert len(result_list) >= 1
            found_ids = [doc_id_ for doc_id_, _, _ in result_list]
            assert doc_id in found_ids, f"doc {doc_id}이 저자 검색 결과에 없음: {found_ids}"

            # 결과 구조 검증
            for _, doc, score in result_list:
                inspect_search_result_hierarchy(doc)
                assert score > 0
        finally:
            esm.delete(doc_id)
            esm.refresh()

    def test_search_by_partial_tokens(self, es_manager_with_data):
        """카테고리/제목/저자의 일부 토큰으로 검색하면 해당 문서가 검색된다"""
        esm = es_manager_with_data
        doc_id = 435
        test_doc = {
            doc_id: {
                "category": "1_한국고전국역총서",
                "title": "열하일기",
                "author": "박지원",
                "file_path": "1_한국고전국역총서/열하일기.epub",
                "file_type": "epub",
                "file_size": 5000,
                "line_count": 0,
                "page_count": 200,
                "isbn": "",
                "summary": "청나라 여행을 통해 조선 사회의 문제점을 비판한 기행문학의 걸작.",
                "updated_time": "2024-06-01T00:00:00",
            },
        }
        esm.insert(test_doc)
        esm.refresh()

        try:
            # 제목 일부 토큰 "열하"로 검색 (nori가 "열하일기" → "열하"+"일기" 분리 기대)
            result_list = esm.search_by_keyword("열하", max_result_count=10)
            found_ids = [did for did, _, _ in result_list]
            assert doc_id in found_ids, f"제목 부분 토큰 '열하'로 검색 실패: {found_ids}"

            # 제목 일부 토큰 "일기"로 검색
            result_list = esm.search_by_keyword("일기", max_result_count=10)
            found_ids = [did for did, _, _ in result_list]
            assert doc_id in found_ids, f"제목 부분 토큰 '일기'로 검색 실패: {found_ids}"

            # 저자 토큰 "박지원"으로 검색
            result_list = esm.search_by_keyword("박지원", max_result_count=10)
            found_ids = [did for did, _, _ in result_list]
            assert doc_id in found_ids, f"저자 토큰 '박지원'으로 검색 실패: {found_ids}"

            # 카테고리 일부 토큰 "한국고전"으로 키워드 검색
            # category 필드가 text(nori) 타입이므로 search_by_keyword로 부분 매칭 가능
            result_list = esm.search_by_keyword("한국고전", max_result_count=10)
            found_ids = [did for did, _, _ in result_list]
            assert doc_id in found_ids, f"카테고리 부분 토큰 '한국고전'으로 검색 실패: {found_ids}"
        finally:
            esm.delete(doc_id)
            esm.refresh()

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

    # ── _get_self_score ──

    def test_get_self_score_returns_positive(self, es_manager_with_data):
        """존재하는 문서의 self_score는 양수다"""
        esm = es_manager_with_data
        should_clauses = [
            {"match": {"title": {"query": "테스트 문서 1", "boost": 20}}},
            {"match": {"author": {"query": "테스트 작가", "boost": 15}}},
            {"match": {"summary": {"query": "이것은 테스트 문서입니다.", "boost": 3}}},
        ]
        score = esm._get_self_score(should_clauses, doc_id=1)
        assert score > 0

    def test_get_self_score_with_none_returns_zero(self, es_manager_with_data):
        """doc_id가 None이면 0.0을 반환한다"""
        esm = es_manager_with_data
        should_clauses = [
            {"match": {"title": {"query": "테스트", "boost": 20}}},
        ]
        score = esm._get_self_score(should_clauses, doc_id=None)
        assert score == 0.0

    def test_get_self_score_nonexistent_doc(self, es_manager_with_data):
        """존재하지 않는 문서의 self_score는 0.0이다"""
        esm = es_manager_with_data
        should_clauses = [
            {"match": {"title": {"query": "테스트", "boost": 20}}},
        ]
        score = esm._get_self_score(should_clauses, doc_id=999999)
        assert score == 0.0

    # ── _search_paged ref_score 정규화 ──

    def test_search_paged_without_ref_score_first_is_100(self, es_manager_with_data):
        """ref_score 없이 호출하면 1등이 100점이다 (기존 동작)"""
        esm = es_manager_with_data
        query = {"bool": {"should": [{"match": {"title": {"query": "테스트 문서", "boost": 1}}}]}}
        results, total = esm._search_paged(query, size=10)
        if results:
            assert results[0][2] == 100.0

    def test_search_paged_with_ref_score_normalizes_correctly(self, es_manager_with_data):
        """ref_score를 지정하면 해당 값 기준으로 정규화된다"""
        esm = es_manager_with_data
        query = {"bool": {"should": [{"match": {"title": {"query": "테스트 문서", "boost": 1}}}]}}

        # ref_score 없이 → 1등이 100점
        results_default, _ = esm._search_paged(query, size=10)
        # ref_score를 1등 raw_score의 2배로 설정 → 모든 점수가 절반으로
        if results_default:
            # 1등의 원래 점수를 역산 (default에서 100점이므로 raw_score = max_score)
            response = esm.es.search(index=esm.index_name, query=query, size=1, track_scores=True)
            raw_max = response['hits']['max_score']
            ref_double = raw_max * 2

            results_ref, _ = esm._search_paged(query, size=10, ref_score=ref_double)
            if results_ref:
                # ref_score가 2배이므로 1등 점수는 약 50점
                assert abs(results_ref[0][2] - 50.0) < 0.1

    def test_search_paged_with_ref_score_caps_at_100(self, es_manager_with_data):
        """ref_score보다 높은 raw_score가 있어도 100을 초과하지 않는다"""
        esm = es_manager_with_data
        query = {"bool": {"should": [{"match": {"title": {"query": "테스트 문서", "boost": 1}}}]}}
        # 아주 작은 ref_score를 지정
        results, _ = esm._search_paged(query, size=10, ref_score=0.001)
        for _, _, score in results:
            assert score <= 100.0

    # ── search_by_keyword_paged exclude_categories ──

    def test_keyword_paged_exclude_categories_filters_results(self, es_manager_with_data):
        """exclude_categories로 특정 카테고리를 제외하면 해당 카테고리 결과가 없다"""
        esm = es_manager_with_data
        # "마법" 키워드는 category="test"(doc1,2)와 category="_txt"(doc3) 모두 매칭 가능
        results_all, total_all = esm.search_by_keyword_paged("마법", size=10)
        results_filtered, total_filtered = esm.search_by_keyword_paged("마법", size=10, exclude_categories=["test"])

        # 필터링된 결과에는 "test" 카테고리 문서가 없어야 함
        for _, doc, _ in results_filtered:
            assert doc["category"] != "test", f"제외된 카테고리 'test' 문서가 포함됨: {doc['title']}"

        # 필터링된 total이 전체보다 작거나 같아야 함
        assert total_filtered <= total_all

    def test_keyword_paged_exclude_categories_prefix_match(self, es_manager_with_data):
        """exclude_categories는 prefix 매칭으로 하위 카테고리도 제외한다"""
        esm = es_manager_with_data
        # "test"로 시작하는 카테고리를 가진 임시 데이터 삽입
        extra_data = {
            200: {
                "category": "test/sub", "title": "하위 카테고리 마법 문서",
                "author": "테스트", "file_path": "/test/sub.txt", "file_type": "txt",
                "file_size": 500, "line_count": 50, "page_count": 0,
                "isbn": "", "summary": "하위 카테고리 테스트.",
                "updated_time": "2024-01-05T00:00:00",
            },
        }
        esm.insert(extra_data)
        esm.refresh()

        try:
            results, _ = esm.search_by_keyword_paged("마법", size=10, exclude_categories=["test"])
            for _, doc, _ in results:
                assert not doc["category"].startswith("test"), \
                    f"'test' prefix 카테고리 문서가 포함됨: {doc['category']}"
        finally:
            esm.delete(200)
            esm.refresh()

    def test_keyword_paged_no_exclude_returns_all(self, es_manager_with_data):
        """exclude_categories가 None이면 모든 결과를 반환한다"""
        esm = es_manager_with_data
        results_none, total_none = esm.search_by_keyword_paged("테스트", size=10, exclude_categories=None)
        results_empty, total_empty = esm.search_by_keyword_paged("테스트", size=10, exclude_categories=[])
        assert total_none == total_empty

    # ── search_similar_docs_paged 유사도 정규화 ──

    def test_similar_docs_paged_no_perfect_100(self, es_manager_with_data):
        """유사 검색에서 원본과 다른 문서는 100점이 되지 않는다"""
        esm = es_manager_with_data
        # Doc 1 정보로 유사 검색 (Doc 1 자체는 제외)
        results, total = esm.search_similar_docs_paged(
            category="test",
            title="테스트 문서 1",
            author="테스트 작가",
            file_type="txt",
            file_size=1000,
            summary="이것은 테스트 문서입니다. 마법사와 드래곤 이야기.",
            exclude_id=1,
            size=10,
        )
        if results:
            for doc_id, doc, score in results:
                assert score < 100, f"Doc {doc_id} ({doc['title']})의 점수 {score}이 100이면 안 됨"

    def test_similar_docs_paged_scores_in_range(self, es_manager_with_data):
        """유사 검색 점수는 0~100 범위다"""
        esm = es_manager_with_data
        results, _ = esm.search_similar_docs_paged(
            category="test",
            title="테스트 문서 1",
            author="테스트 작가",
            file_type="txt",
            file_size=1000,
            summary="이것은 테스트 문서입니다.",
            exclude_id=1,
            size=10,
        )
        for doc_id, _, score in results:
            assert 0 <= score <= 100, f"Doc {doc_id} 점수 {score}가 범위 밖"

    def test_similar_docs_paged_same_author_different_title_below_100(self, es_manager_with_data):
        """같은 작가의 다른 제목 문서는 100점 미만이다 (세네카 사례)"""
        esm = es_manager_with_data
        # 추가 테스트 데이터 삽입: 같은 작가, 다른 제목
        extra_data = {
            100: {
                "category": "test", "title": "세네카의 말",
                "author": "루키우스 안나이우스 세네카",
                "file_path": "/test/seneca1.epub", "file_type": "epub",
                "file_size": 5000, "line_count": 0, "page_count": 100,
                "isbn": "", "summary": "세네카의 명언과 철학을 담은 책.",
                "updated_time": "2024-01-10T00:00:00",
            },
            101: {
                "category": "test", "title": "세네카의 행복론",
                "author": "루키우스 안나이우스 세네카",
                "file_path": "/test/seneca2.epub", "file_type": "epub",
                "file_size": 6000, "line_count": 0, "page_count": 120,
                "isbn": "", "summary": "행복에 관한 세네카의 철학 에세이.",
                "updated_time": "2024-01-11T00:00:00",
            },
        }
        esm.insert(extra_data)
        esm.refresh()

        try:
            results, _ = esm.search_similar_docs_paged(
                category="test",
                title="세네카의 말",
                author="루키우스 안나이우스 세네카",
                file_type="epub",
                file_size=5000,
                summary="세네카의 명언과 철학을 담은 책.",
                exclude_id=100,
                size=10,
            )
            # "세네카의 행복론"은 같은 작가지만 다른 책이므로 100점 미만
            seneca_results = [(did, doc, s) for did, doc, s in results if did == 101]
            assert seneca_results, "세네카의 행복론이 결과에 포함되어야 함"
            score = seneca_results[0][2]
            assert score < 100, f"같은 작가의 다른 책이 {score}점으로 100점이면 안 됨"
            assert score > 0, "같은 작가이므로 0점보다는 높아야 함"
        finally:
            # 테스트 데이터 정리
            esm.delete(100)
            esm.delete(101)
            esm.refresh()

    # ── count_by_category / rename_category ──

    def test_count_by_category(self, es_manager_with_data):
        """count_by_category가 문서 수를 정확히 반환한다"""
        esm = es_manager_with_data
        count = esm.count_by_category("test")
        assert isinstance(count, int)
        assert count >= 2  # test 카테고리에 doc 1, 2가 존재

    def test_count_by_category_empty(self, es_manager_with_data):
        """존재하지 않는 카테고리는 0을 반환한다"""
        esm = es_manager_with_data
        count = esm.count_by_category("nonexistent_category_xyz")
        assert count == 0

    def test_rename_category(self, es_manager_with_data):
        """rename_category로 category와 file_path가 변경된다"""
        esm = es_manager_with_data
        # 테스트 데이터 삽입
        test_data = {
            300: {
                "category": "rename_src",
                "title": "Rename Test Doc",
                "author": "Author",
                "file_path": "rename_src/test.txt",
                "file_type": "txt",
                "file_size": 100,
                "line_count": 10,
                "page_count": 0,
                "isbn": "",
                "summary": "rename test",
                "updated_time": "2024-01-01T00:00:00",
            },
        }
        esm.insert(test_data)
        esm.refresh()

        try:
            result = esm.rename_category("rename_src", "rename_dst")
            assert result["updated"] == 1
            assert result["failures"] == []

            # 변경 확인
            doc = esm.search_by_id(300)
            assert doc["category"] == "rename_dst"
            assert doc["file_path"] == "rename_dst/test.txt"

            # old_category에 문서가 없어야 함
            assert esm.count_by_category("rename_src") == 0
            assert esm.count_by_category("rename_dst") == 1
        finally:
            esm.delete(300)
            esm.refresh()

    def test_rename_category_empty(self, es_manager_with_data):
        """존재하지 않는 카테고리 rename 시 updated=0"""
        esm = es_manager_with_data
        result = esm.rename_category("empty_category_xyz", "new_empty")
        assert result["updated"] == 0

    # ── delete_by_category ──

    def test_delete_by_category(self, es_manager_with_data):
        """delete_by_category로 카테고리의 모든 문서가 삭제된다"""
        esm = es_manager_with_data
        # 테스트 데이터 삽입
        test_data = {
            310: {
                "category": "delete_target",
                "title": "Delete Test 1",
                "author": "Author",
                "file_path": "delete_target/test1.txt",
                "file_type": "txt",
                "file_size": 100,
                "line_count": 10,
                "page_count": 0,
                "isbn": "",
                "summary": "delete test",
                "updated_time": "2024-01-01T00:00:00",
            },
            311: {
                "category": "delete_target",
                "title": "Delete Test 2",
                "author": "Author",
                "file_path": "delete_target/test2.txt",
                "file_type": "txt",
                "file_size": 200,
                "line_count": 20,
                "page_count": 0,
                "isbn": "",
                "summary": "delete test 2",
                "updated_time": "2024-01-01T00:00:00",
            },
        }
        esm.insert(test_data)
        esm.refresh()

        try:
            assert esm.count_by_category("delete_target") == 2

            result = esm.delete_by_category("delete_target")
            assert result["deleted"] == 2
            assert result["failures"] == []

            # 삭제 확인
            assert esm.count_by_category("delete_target") == 0
            assert esm.search_by_id(310) == {}
            assert esm.search_by_id(311) == {}
        finally:
            # 혹시 남아있으면 정리
            for doc_id in [310, 311]:
                try:
                    esm.delete(doc_id)
                except Exception:
                    pass
            esm.refresh()

    def test_delete_by_category_empty(self, es_manager_with_data):
        """존재하지 않는 카테고리 delete 시 deleted=0"""
        esm = es_manager_with_data
        result = esm.delete_by_category("nonexistent_del_xyz")
        assert result["deleted"] == 0
        assert result["failures"] == []

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


class TestESManagerEnvVars:
    """환경 변수 이름 변경 (TM_ES_INDEX → TM_ES_BOOK_INDEX) 관련 테스트."""

    REQUIRED_ENVS = {
        "TM_ES_BOOK_INDEX": "test_idx",
        "TM_ES_URL": "http://localhost:9200",
        "TM_ES_USER": "elastic",
        "TM_ES_PASSWORD": "",
    }

    def test_uses_tm_es_book_index_env(self):
        """TM_ES_BOOK_INDEX 환경 변수로 기본 인덱스명을 결정한다."""
        with patch.dict(os.environ, self.REQUIRED_ENVS, clear=False):
            with patch("backend.es_manager.Elasticsearch"):
                from backend.es_manager import ESManager
                esm = ESManager()
                assert esm.index_name == "test_idx"

    def test_explicit_index_name_overrides_env(self):
        """index_name 인자가 주어지면 환경 변수보다 우선한다."""
        with patch.dict(os.environ, self.REQUIRED_ENVS, clear=False):
            with patch("backend.es_manager.Elasticsearch"):
                from backend.es_manager import ESManager
                esm = ESManager(index_name="custom_index")
                assert esm.index_name == "custom_index"

    def test_old_tm_es_index_not_used(self):
        """이전 환경 변수 TM_ES_INDEX만 설정하면 KeyError가 발생한다."""
        env = {
            "TM_ES_INDEX": "old_name",
            "TM_ES_URL": "http://localhost:9200",
            "TM_ES_USER": "elastic",
            "TM_ES_PASSWORD": "",
        }
        # TM_ES_BOOK_INDEX가 없으므로 KeyError
        cleaned = {k: v for k, v in os.environ.items() if k != "TM_ES_BOOK_INDEX"}
        cleaned.update(env)
        with patch.dict(os.environ, cleaned, clear=True):
            from backend.es_manager import ESManager
            with pytest.raises(KeyError):
                ESManager()

    def test_comics_manager_uses_tm_es_comics_index(self):
        """ComicsManager는 TM_ES_COMICS_INDEX 환경 변수를 사용한다."""
        env = {
            **self.REQUIRED_ENVS,
            "TM_COMICS_DIR": "/tmp",
            "TM_ES_COMICS_INDEX": "my_comics",
        }
        with patch.dict(os.environ, env, clear=False):
            with patch("backend.es_manager.Elasticsearch"):
                from backend.comics_manager import ComicsManager
                cm = ComicsManager()
                assert cm.es_manager.index_name == "my_comics"

    def test_comics_manager_default_index(self):
        """TM_ES_COMICS_INDEX 미설정 시 기본값 tm_comics를 사용한다."""
        env = {**self.REQUIRED_ENVS, "TM_COMICS_DIR": "/tmp"}
        cleaned = {k: v for k, v in os.environ.items() if k != "TM_ES_COMICS_INDEX"}
        cleaned.update(env)
        with patch.dict(os.environ, cleaned, clear=True):
            with patch("backend.es_manager.Elasticsearch"):
                from backend.comics_manager import ComicsManager
                cm = ComicsManager()
                assert cm.es_manager.index_name == "tm_comics"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
