#!/usr/bin/env python3

import unittest
from unittest import skip
import asyncio
import sys
import os

# 프로젝트 루트 디렉토리를 Python 경로에 추가
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from backend.bookstore import (
    Yes24Bookstore,
    AladinBookstore,
    RidibooksBookstore,
    NaverShoppingBookstore,
    NaverSeriesBookstore,
    MunpiaBookstore
)
from bs4 import BeautifulSoup


class TestBookstore(unittest.IsolatedAsyncioTestCase):
    """서점 검색 관련 테스트 클래스"""

    def setUp(self):
        """테스트 설정: 각 서점 클래스 목록 준비"""
        self.stores = [
            Yes24Bookstore,
            AladinBookstore,
            RidibooksBookstore,
            NaverShoppingBookstore,
            NaverSeriesBookstore,
            MunpiaBookstore,
        ]

    def inspect_book_info(self, book):
        """도서 정보 검사 헬퍼 메서드"""
        print(f"Book ID: {book.id}")
        print(f"Title: {book.title}")
        print(f"Author: {book.author}")
        print(f"Category: {book.category}")
        print(f"File Path: {book.file_path}")
        print(f"File Type: {book.file_type}")
        print(f"File Size: {book.file_size}")
        print(f"Created At: {book.created_at}")
        print(f"Updated At: {book.updated_at}")

    @skip("skip category extraction test due to external dependency instability")
    def test_bookstore_category_extraction(self):
        """Bookstore 클래스의 카테고리 추출 기능 테스트"""
        bookstore = Yes24Bookstore(base_dir=".", verbose=False)
        test_books = [
            {"keyword": "해리포터", "expected": "국내도서 > 소설/시/희곡 > 장르소설 > 판타지"},
            {"keyword": "어린 왕자", "expected": "국내도서 > 소설/시/희곡 > 프랑스소설"},
            {"keyword": "드래곤라자", "expected": "국내도서 > 소설/시/희곡 > 장르소설 > 판타지"},
            {"keyword": "마검크루세이더", "expected": "국내도서 > 소설/시/희곡 > 장르소설 > 판타지"},
            {"keyword": "1984", "expected": "국내도서 > 소설/시/희곡 > 영미소설 > 영미 장편소설"}
        ]
        success_count = 0
        total_count = len(test_books)
        for i, book in enumerate(test_books, 1):
            keyword = book["keyword"]
            expected = book["expected"]
            try:
                books = bookstore.search_by_keyword(keyword)
                assert isinstance(books, list) and len(books) >= 2, f"검색 결과가 2개 미만입니다: {len(books)}"
                title, _, category, book_url, search_url = books[0]
                assert title, f"제목이 비어있음: {keyword}"
                assert book_url, f"URL이 비어있음: {keyword}"
                assert search_url, f"검색 URL이 비어있음: {keyword}"
                if category == expected:
                    success_count += 1
                else:
                    print(f"❌ 카테고리 불일치 - 키워드: {keyword}")
                    print(f"  기대값: {expected}")
                    print(f"  실제값: {category}")
            except Exception as e:
                print(f"❌ 검색 중 오류 발생 - 키워드: {keyword}, 오류: {e}")
        success_rate = (success_count / total_count) * 100
        assert success_rate >= 80.0, f"카테고리 추출 성공률이 80% 미만입니다: {success_rate:.1f}%"
        print(f"✅ 카테고리 추출 테스트 완료: {success_count}/{total_count} 성공 ({success_rate:.1f}%)")

    def test_bookstore_integration(self):
        """BookManager와 Bookstore 통합 테스트"""
        import re

        # 실제 도서가 없으므로 Bookstore만 테스트
        print("=== Bookstore 통합 테스트 ===")

        # 테스트할 키워드
        keyword = "해리 포터와 마법사의 돌"

        # 제목 정리
        clean_title = re.sub(r'\s*\d+\s*', ' ', keyword)
        clean_title = re.sub(r'\s+', ' ', clean_title).strip()

        print(f"원본 제목: {keyword}")
        print(f"정리된 제목: {clean_title}")

        # Bookstore로 검색
        # 단일 서점 테스트는 필요 시 개별적으로 수행하세요
        # 이 통합 테스트는 기존대로 Yes24Bookstore 기본값으로 실행합니다
        bookstore = Yes24Bookstore(base_dir=".", verbose=False)
        # 새로운 search_by_keyword 사용
        books = bookstore.search_by_keyword(clean_title)
        assert isinstance(books, list) and books, f"검색 결과가 없습니다: {clean_title}"
        title, author, category, book_url, search_url = books[0]

        print("검색 결과:")
        print(f"  제목: {title}")
        print(f"  저자: {author}")
        print(f"  카테고리: {category}")
        print(f"  URL: {book_url}")

        # 결과 검증
        assert title, "제목이 비어있습니다"
        assert author, "저자가 비어있습니다"
        assert category, "카테고리가 비어있습니다"
        assert book_url, "URL이 비어있습니다"
        assert search_url, "검색 URL이 비어있습니다"

        print("✅ Bookstore 통합 테스트 성공")

    def test_build_search_url_for_all_stores(self):
        """모든 서점 클래스의 build_search_url이 키워드를 올바르게 인코딩하는지 확인"""
        from urllib.parse import quote

        keyword = "테스트 키워드"
        for store_cls in self.stores:
            with self.subTest(store=store_cls.__name__):
                bs = store_cls(base_dir=".", verbose=False)
                url = bs.build_search_url(keyword)
                # BASE_URL로 시작하고, 인코딩된 키워드를 포함해야 함
                self.assertTrue(url.startswith(store_cls.BASE_URL))
                self.assertIn(quote(keyword), url)
    def test_search_by_keyword_result_structure(self):
        """search_by_keyword 결과가 길이 5 튜플 리스트인지 검증"""
        keyword = "요괴"
        for store_cls in self.stores:
            with self.subTest(store=store_cls.__name__):
                bs = store_cls(base_dir=".", verbose=False)
                results = bs.search_by_keyword(keyword)
                self.assertIsInstance(results, list)
                if not results:
                    self.skipTest(f"{store_cls.__name__} 서점에서 결과가 없습니다")
                for item in results:
                    self.assertIsInstance(item, tuple)
                    self.assertEqual(len(item), 5)
                    title, author, category, book_url, search_url = item
                    self.assertIsInstance(title, str)
                    self.assertIsInstance(author, str)
                    self.assertIsInstance(category, str)
                    self.assertIsInstance(book_url, str)
                    self.assertIsInstance(search_url, str)
    def test_extract_book_info_fields(self):
        """extract_book_info가 필드별로 문자열을 반환하는지 확인"""
        bs = Yes24Bookstore(base_dir=".", verbose=False)
        # 검색 결과에서 첫 번째 상세 페이지로 이동
        results = bs.search_by_keyword("요괴")
        self.assertTrue(results, "search_by_keyword 결과가 비어있습니다")
        _, _, _, detail_url, _ = results[0]
        resp = bs.session.get(detail_url, timeout=10, verify=False)
        resp.encoding = 'utf-8'
        soup = BeautifulSoup(resp.text, 'html.parser')
        info = bs.extract_book_info(soup)
        # 필수 키 확인
        for key in ['title', 'author', 'category']:
            self.assertIn(key, info)
            self.assertIsInstance(info[key], str)
        # ISBN 필드도 문자열로 존재해야 함
        self.assertIn('isbn', info)
        self.assertIsInstance(info['isbn'], str)
