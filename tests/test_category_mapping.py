#!/usr/bin/env python

import os
import tempfile
import pytest

from backend.category_mapping import CategoryMapping


class TestCategoryMapping:
    """CategoryMapping 클래스 테스트"""

    @pytest.fixture
    def temp_db(self):
        """임시 데이터베이스 파일 생성"""
        fd, path = tempfile.mkstemp(suffix=".db")
        os.close(fd)
        yield path
        # 테스트 후 파일 삭제
        if os.path.exists(path):
            os.remove(path)

    @pytest.fixture
    def mapping(self, temp_db):
        """CategoryMapping 인스턴스 생성"""
        return CategoryMapping(db_path=temp_db)

    # === 초기화 테스트 ===

    def test_init_creates_database(self, temp_db):
        """데이터베이스 파일이 생성되는지 테스트"""
        mapping = CategoryMapping(db_path=temp_db)
        assert os.path.exists(temp_db)

    def test_init_creates_table(self, mapping):
        """테이블이 생성되는지 테스트"""
        # 테이블에 쿼리 실행 가능 여부로 확인
        result = mapping.get_all_mappings()
        assert isinstance(result, dict)

    # === add_keyword 테스트 ===

    def test_add_keyword_success(self, mapping):
        """키워드 추가 성공 테스트"""
        result = mapping.add_keyword("4_심리학뇌과학", "심리학")
        assert result is True

    def test_add_keyword_duplicate(self, mapping):
        """중복 키워드 추가 시 실패 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        result = mapping.add_keyword("4_심리학뇌과학", "심리학")
        assert result is False

    def test_add_keyword_empty(self, mapping):
        """빈 키워드 추가 시 실패 테스트"""
        result = mapping.add_keyword("4_심리학뇌과학", "")
        assert result is False

    def test_add_keyword_whitespace_only(self, mapping):
        """공백만 있는 키워드 추가 시 실패 테스트"""
        result = mapping.add_keyword("4_심리학뇌과학", "   ")
        assert result is False

    def test_add_keyword_strips_whitespace(self, mapping):
        """키워드 앞뒤 공백 제거 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "  심리학  ")
        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert "심리학" in keywords

    def test_add_multiple_keywords_same_category(self, mapping):
        """같은 카테고리에 여러 키워드 추가 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")
        mapping.add_keyword("4_심리학뇌과학", "인지과학")

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert len(keywords) == 3
        assert "심리학" in keywords
        assert "뇌과학" in keywords
        assert "인지과학" in keywords

    def test_add_same_keyword_different_categories(self, mapping):
        """서로 다른 카테고리에 같은 키워드 추가 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리")
        mapping.add_keyword("5_철학종교", "심리")

        keywords1 = mapping.get_keywords("4_심리학뇌과학")
        keywords2 = mapping.get_keywords("5_철학종교")

        assert "심리" in keywords1
        assert "심리" in keywords2

    # === get_keywords 테스트 ===

    def test_get_keywords_empty_category(self, mapping):
        """키워드가 없는 카테고리 조회 테스트"""
        keywords = mapping.get_keywords("nonexistent")
        assert keywords == []

    def test_get_keywords_returns_sorted(self, mapping):
        """키워드가 정렬되어 반환되는지 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "인지과학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")
        mapping.add_keyword("4_심리학뇌과학", "심리학")

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert keywords == sorted(keywords)

    # === remove_keyword 테스트 ===

    def test_remove_keyword_success(self, mapping):
        """키워드 삭제 성공 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        result = mapping.remove_keyword("4_심리학뇌과학", "심리학")
        assert result is True

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert "심리학" not in keywords

    def test_remove_keyword_not_found(self, mapping):
        """존재하지 않는 키워드 삭제 테스트"""
        result = mapping.remove_keyword("4_심리학뇌과학", "nonexistent")
        assert result is False

    def test_remove_keyword_only_affects_target(self, mapping):
        """삭제 시 다른 키워드에 영향 없는지 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")
        mapping.remove_keyword("4_심리학뇌과학", "심리학")

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert "심리학" not in keywords
        assert "뇌과학" in keywords

    # === set_keywords 테스트 ===

    def test_set_keywords_replaces_existing(self, mapping):
        """set_keywords가 기존 키워드를 대체하는지 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")

        mapping.set_keywords("4_심리학뇌과학", ["인지과학", "행동과학"])

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert len(keywords) == 2
        assert "인지과학" in keywords
        assert "행동과학" in keywords
        assert "심리학" not in keywords
        assert "뇌과학" not in keywords

    def test_set_keywords_empty_list(self, mapping):
        """빈 리스트로 set_keywords 호출 시 모든 키워드 삭제 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.set_keywords("4_심리학뇌과학", [])

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert keywords == []

    def test_set_keywords_removes_duplicates(self, mapping):
        """set_keywords가 중복을 제거하는지 테스트"""
        mapping.set_keywords("4_심리학뇌과학", ["심리학", "심리학", "뇌과학"])

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert len(keywords) == 2

    def test_set_keywords_strips_whitespace(self, mapping):
        """set_keywords가 공백을 제거하는지 테스트"""
        mapping.set_keywords("4_심리학뇌과학", ["  심리학  ", "뇌과학  "])

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert "심리학" in keywords
        assert "뇌과학" in keywords

    def test_set_keywords_ignores_empty_strings(self, mapping):
        """set_keywords가 빈 문자열을 무시하는지 테스트"""
        mapping.set_keywords("4_심리학뇌과학", ["심리학", "", "  ", "뇌과학"])

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert len(keywords) == 2

    # === get_all_mappings 테스트 ===

    def test_get_all_mappings_empty(self, mapping):
        """매핑이 없을 때 빈 딕셔너리 반환 테스트"""
        result = mapping.get_all_mappings()
        assert result == {}

    def test_get_all_mappings_single_category(self, mapping):
        """단일 카테고리 매핑 조회 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")

        result = mapping.get_all_mappings()
        assert len(result) == 1
        assert "4_심리학뇌과학" in result
        assert len(result["4_심리학뇌과학"]) == 2

    def test_get_all_mappings_multiple_categories(self, mapping):
        """여러 카테고리 매핑 조회 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("5_철학종교", "철학")
        mapping.add_keyword("6_역사", "역사")

        result = mapping.get_all_mappings()
        assert len(result) == 3
        assert "4_심리학뇌과학" in result
        assert "5_철학종교" in result
        assert "6_역사" in result

    # === update_all_mappings 테스트 ===

    def test_update_all_mappings_replaces_all(self, mapping):
        """update_all_mappings가 전체를 대체하는지 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("5_철학종교", "철학")

        new_mappings = {
            "6_역사": ["역사", "세계사"],
            "7_과학": ["물리", "화학"]
        }
        mapping.update_all_mappings(new_mappings)

        result = mapping.get_all_mappings()
        assert "4_심리학뇌과학" not in result
        assert "5_철학종교" not in result
        assert "6_역사" in result
        assert "7_과학" in result

    def test_update_all_mappings_empty_dict(self, mapping):
        """빈 딕셔너리로 update_all_mappings 호출 시 모든 데이터 삭제 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.update_all_mappings({})

        result = mapping.get_all_mappings()
        assert result == {}

    # === delete_category 테스트 ===

    def test_delete_category_success(self, mapping):
        """카테고리 삭제 성공 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")

        result = mapping.delete_category("4_심리학뇌과학")
        assert result is True

        keywords = mapping.get_keywords("4_심리학뇌과학")
        assert keywords == []

    def test_delete_category_not_found(self, mapping):
        """존재하지 않는 카테고리 삭제 테스트"""
        result = mapping.delete_category("nonexistent")
        assert result is False

    def test_delete_category_only_affects_target(self, mapping):
        """삭제 시 다른 카테고리에 영향 없는지 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("5_철학종교", "철학")

        mapping.delete_category("4_심리학뇌과학")

        assert mapping.get_keywords("4_심리학뇌과학") == []
        assert mapping.get_keywords("5_철학종교") == ["철학"]

    # === get_categories_with_keywords 테스트 ===

    def test_get_categories_with_keywords_empty(self, mapping):
        """키워드가 없을 때 빈 리스트 반환 테스트"""
        result = mapping.get_categories_with_keywords()
        assert result == []

    def test_get_categories_with_keywords(self, mapping):
        """키워드가 있는 카테고리 목록 조회 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("5_철학종교", "철학")

        result = mapping.get_categories_with_keywords()
        assert len(result) == 2
        assert "4_심리학뇌과학" in result
        assert "5_철학종교" in result

    # === search_by_keyword 테스트 ===

    def test_search_by_keyword_exact_match(self, mapping):
        """정확히 일치하는 키워드 검색 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("5_철학종교", "철학")

        result = mapping.search_by_keyword("심리학")
        assert "4_심리학뇌과학" in result
        assert "5_철학종교" not in result

    def test_search_by_keyword_partial_match(self, mapping):
        """부분 일치 키워드 검색 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "사회심리학")

        result = mapping.search_by_keyword("심리")
        assert "4_심리학뇌과학" in result

    def test_search_by_keyword_no_match(self, mapping):
        """일치하는 키워드가 없을 때 빈 리스트 반환 테스트"""
        mapping.add_keyword("4_심리학뇌과학", "심리학")

        result = mapping.search_by_keyword("역사")
        assert result == []

    # === 동시성 및 데이터 무결성 테스트 ===

    def test_multiple_operations_sequence(self, mapping):
        """여러 작업을 연속으로 수행하는 통합 테스트"""
        # 추가
        mapping.add_keyword("4_심리학뇌과학", "심리학")
        mapping.add_keyword("4_심리학뇌과학", "뇌과학")
        mapping.add_keyword("5_철학종교", "철학")

        # 조회
        all_mappings = mapping.get_all_mappings()
        assert len(all_mappings) == 2

        # 수정
        mapping.set_keywords("4_심리학뇌과학", ["인지과학"])

        # 삭제
        mapping.remove_keyword("5_철학종교", "철학")

        # 최종 확인
        all_mappings = mapping.get_all_mappings()
        assert len(all_mappings) == 1
        assert all_mappings["4_심리학뇌과학"] == ["인지과학"]

    def test_special_characters_in_keyword(self, mapping):
        """특수 문자가 포함된 키워드 테스트"""
        mapping.add_keyword("test", "키워드 (테스트)")
        mapping.add_keyword("test", "키워드/슬래시")
        mapping.add_keyword("test", "키워드'따옴표")

        keywords = mapping.get_keywords("test")
        assert len(keywords) == 3

    def test_unicode_category_and_keyword(self, mapping):
        """유니코드 카테고리 및 키워드 테스트"""
        mapping.add_keyword("한글카테고리", "한글키워드")
        mapping.add_keyword("日本語カテゴリ", "日本語キーワード")

        assert mapping.get_keywords("한글카테고리") == ["한글키워드"]
        assert mapping.get_keywords("日本語カテゴリ") == ["日本語キーワード"]


class TestCategoryMappingEdgeCases:
    """CategoryMapping 엣지 케이스 테스트"""

    @pytest.fixture
    def temp_db(self):
        fd, path = tempfile.mkstemp(suffix=".db")
        os.close(fd)
        yield path
        if os.path.exists(path):
            os.remove(path)

    def test_very_long_keyword(self, temp_db):
        """매우 긴 키워드 테스트"""
        mapping = CategoryMapping(db_path=temp_db)
        long_keyword = "a" * 1000
        result = mapping.add_keyword("test", long_keyword)
        assert result is True

        keywords = mapping.get_keywords("test")
        assert long_keyword in keywords

    def test_very_long_category(self, temp_db):
        """매우 긴 카테고리명 테스트"""
        mapping = CategoryMapping(db_path=temp_db)
        long_category = "category_" + "a" * 1000
        result = mapping.add_keyword(long_category, "keyword")
        assert result is True

        keywords = mapping.get_keywords(long_category)
        assert "keyword" in keywords

    def test_large_number_of_keywords(self, temp_db):
        """많은 수의 키워드 테스트"""
        mapping = CategoryMapping(db_path=temp_db)

        keywords_to_add = [f"keyword_{i}" for i in range(100)]
        for keyword in keywords_to_add:
            mapping.add_keyword("test", keyword)

        keywords = mapping.get_keywords("test")
        assert len(keywords) == 100

    def test_large_number_of_categories(self, temp_db):
        """많은 수의 카테고리 테스트"""
        mapping = CategoryMapping(db_path=temp_db)

        for i in range(100):
            mapping.add_keyword(f"category_{i}", "keyword")

        all_mappings = mapping.get_all_mappings()
        assert len(all_mappings) == 100


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
