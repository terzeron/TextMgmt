#!/usr/bin/env python

import pytest

from backend.category_mapping import CategoryMapping


class TestCategoryMapping:
    """CategoryMapping 클래스 테스트 (MySQL testcontainer 기반)"""

    @pytest.fixture
    def mapping(self, mysql_container):
        """CategoryMapping 인스턴스 생성 (testcontainer MySQL 사용)"""
        mapping = CategoryMapping()
        # 테스트 전 데이터 정리 (book, comic 모두)
        mapping.update_all_mappings({}, content_type="book")
        mapping.update_all_mappings({}, content_type="comic")
        yield mapping
        # 테스트 후 데이터 정리
        mapping.update_all_mappings({}, content_type="book")
        mapping.update_all_mappings({}, content_type="comic")

    # === 초기화 테스트 ===

    def test_init_creates_table(self, mapping):
        """테이블이 생성되는지 테스트"""
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

    def test_delete_category_prefix(self, mapping):
        """prefix=True로 하위 카테고리 키워드도 함께 삭제되는지 테스트"""
        mapping.add_keyword("fiction", "소설")
        mapping.add_keyword("fiction/fantasy", "판타지")
        mapping.add_keyword("fiction/romance", "로맨스")
        mapping.add_keyword("nonfiction", "논픽션")

        result = mapping.delete_category("fiction", prefix=True)
        assert result is True

        assert mapping.get_keywords("fiction") == []
        assert mapping.get_keywords("fiction/fantasy") == []
        assert mapping.get_keywords("fiction/romance") == []
        # 다른 카테고리는 영향 없음
        assert mapping.get_keywords("nonfiction") == ["논픽션"]

    def test_delete_category_prefix_false_exact_only(self, mapping):
        """prefix=False(기본값)는 정확 매칭만 삭제"""
        mapping.add_keyword("fiction", "소설")
        mapping.add_keyword("fiction/fantasy", "판타지")

        result = mapping.delete_category("fiction")
        assert result is True

        assert mapping.get_keywords("fiction") == []
        # 하위 카테고리는 남아있음
        assert mapping.get_keywords("fiction/fantasy") == ["판타지"]

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

    # === content_type 분리 테스트 ===

    def test_content_type_isolation_keywords(self, mapping):
        """book과 comic의 키워드가 독립적으로 관리되는지 테스트"""
        mapping.add_keyword("카테고리A", "키워드1", content_type="book")
        mapping.add_keyword("카테고리A", "키워드2", content_type="comic")

        book_keywords = mapping.get_keywords("카테고리A", content_type="book")
        comic_keywords = mapping.get_keywords("카테고리A", content_type="comic")

        assert book_keywords == ["키워드1"]
        assert comic_keywords == ["키워드2"]

    def test_content_type_isolation_mappings(self, mapping):
        """book과 comic의 전체 매핑이 독립적으로 조회되는지 테스트"""
        mapping.add_keyword("책카테고리", "책키워드", content_type="book")
        mapping.add_keyword("만화카테고리", "만화키워드", content_type="comic")

        book_mappings = mapping.get_all_mappings(content_type="book")
        comic_mappings = mapping.get_all_mappings(content_type="comic")

        assert "책카테고리" in book_mappings
        assert "만화카테고리" not in book_mappings
        assert "만화카테고리" in comic_mappings
        assert "책카테고리" not in comic_mappings

    def test_content_type_same_keyword_both_types(self, mapping):
        """같은 카테고리+키워드가 서로 다른 content_type에 독립 등록 가능"""
        result1 = mapping.add_keyword("공통카테고리", "공통키워드", content_type="book")
        result2 = mapping.add_keyword("공통카테고리", "공통키워드", content_type="comic")

        assert result1 is True
        assert result2 is True

        book_keywords = mapping.get_keywords("공통카테고리", content_type="book")
        comic_keywords = mapping.get_keywords("공통카테고리", content_type="comic")

        assert "공통키워드" in book_keywords
        assert "공통키워드" in comic_keywords

    def test_content_type_remove_keyword_isolation(self, mapping):
        """한쪽 content_type에서 삭제해도 다른 쪽에 영향 없음"""
        mapping.add_keyword("카테고리", "키워드", content_type="book")
        mapping.add_keyword("카테고리", "키워드", content_type="comic")

        mapping.remove_keyword("카테고리", "키워드", content_type="book")

        assert mapping.get_keywords("카테고리", content_type="book") == []
        assert mapping.get_keywords("카테고리", content_type="comic") == ["키워드"]

    def test_content_type_hidden_categories_isolation(self, mapping):
        """book과 comic의 비노출 카테고리가 독립적으로 관리되는지 테스트"""
        mapping.set_hidden("숨김카테고리", True, content_type="book")

        book_hidden = mapping.get_hidden_categories(content_type="book")
        comic_hidden = mapping.get_hidden_categories(content_type="comic")

        assert "숨김카테고리" in book_hidden
        assert "숨김카테고리" not in comic_hidden

    def test_content_type_set_hidden_both_types(self, mapping):
        """같은 카테고리를 양쪽에 독립적으로 비노출 설정"""
        mapping.set_hidden("카테고리", True, content_type="book")
        mapping.set_hidden("카테고리", True, content_type="comic")

        assert "카테고리" in mapping.get_hidden_categories(content_type="book")
        assert "카테고리" in mapping.get_hidden_categories(content_type="comic")

        mapping.set_hidden("카테고리", False, content_type="book")

        assert "카테고리" not in mapping.get_hidden_categories(content_type="book")
        assert "카테고리" in mapping.get_hidden_categories(content_type="comic")

    def test_content_type_is_hidden_isolation(self, mapping):
        """is_hidden이 content_type별로 독립 동작"""
        mapping.set_hidden("숨김", True, content_type="comic")

        assert mapping.is_hidden("숨김", content_type="comic") is True
        assert mapping.is_hidden("숨김", content_type="book") is False

    def test_content_type_delete_category_isolation(self, mapping):
        """delete_category가 content_type별로 독립 동작"""
        mapping.add_keyword("삭제대상", "키워드", content_type="book")
        mapping.add_keyword("삭제대상", "키워드", content_type="comic")

        mapping.delete_category("삭제대상", content_type="book")

        assert mapping.get_keywords("삭제대상", content_type="book") == []
        assert mapping.get_keywords("삭제대상", content_type="comic") == ["키워드"]

    def test_content_type_update_all_mappings_isolation(self, mapping):
        """update_all_mappings가 해당 content_type만 영향"""
        mapping.add_keyword("책카테고리", "책키워드", content_type="book")
        mapping.add_keyword("만화카테고리", "만화키워드", content_type="comic")

        mapping.update_all_mappings({"새책카테고리": ["새키워드"]}, content_type="book")

        book_mappings = mapping.get_all_mappings(content_type="book")
        comic_mappings = mapping.get_all_mappings(content_type="comic")

        assert "책카테고리" not in book_mappings
        assert "새책카테고리" in book_mappings
        assert "만화카테고리" in comic_mappings

    def test_content_type_rename_category_isolation(self, mapping):
        """rename_category가 content_type별로 독립 동작"""
        mapping.add_keyword("원래이름", "키워드", content_type="book")
        mapping.add_keyword("원래이름", "키워드", content_type="comic")

        mapping.rename_category("원래이름", "새이름", content_type="book")

        assert mapping.get_keywords("새이름", content_type="book") == ["키워드"]
        assert mapping.get_keywords("원래이름", content_type="book") == []
        assert mapping.get_keywords("원래이름", content_type="comic") == ["키워드"]

    def test_content_type_default_is_book(self, mapping):
        """content_type 미지정 시 기본값 'book' 동작 확인"""
        mapping.add_keyword("기본카테고리", "기본키워드")

        # 기본값으로 조회
        assert mapping.get_keywords("기본카테고리") == ["기본키워드"]
        # book으로 명시 조회
        assert mapping.get_keywords("기본카테고리", content_type="book") == ["기본키워드"]
        # comic에는 없음
        assert mapping.get_keywords("기본카테고리", content_type="comic") == []


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
