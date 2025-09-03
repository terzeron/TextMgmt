#!/usr/bin/env python3

import unittest
from unittest import skip
import asyncio
import sys
import os

# 프로젝트 루트 디렉토리를 Python 경로에 추가
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from backend.bookstore import Bookstore


class TestBookstore(unittest.IsolatedAsyncioTestCase):
    """서점 검색 관련 테스트 클래스"""

    def setUp(self):
        """테스트 설정"""
        self.bookstore = Bookstore(base_dir=".", verbose=False)

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
    async def test_bookstore_category_extraction(self):
        """Bookstore 클래스의 카테고리 추출 기능 테스트"""
        bookstore = Bookstore(base_dir=".", verbose=False)
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
                books = bookstore.search_yes24_by_keyword(keyword)
                assert isinstance(books, list) and len(books) >= 2, f"검색 결과가 2개 미만입니다: {len(books)}"
                title, author, category, book_url, search_url = books[0]
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

    async def test_bookstore_integration(self):
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
        bookstore = Bookstore(base_dir=".", verbose=False)
        # 새로운 search_by_keyword 사용
        books = bookstore.search_by_keyword(clean_title)
        assert isinstance(books, list) and books, f"검색 결과가 없습니다: {clean_title}"
        title, author, category, book_url, search_url = books[0]

        print(f"검색 결과:")
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
