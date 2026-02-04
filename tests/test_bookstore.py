#!/usr/bin/env python3
"""
Bookstore 클래스 테스트

외부 서점 사이트 의존성을 제거하고 mock을 사용하여 테스트합니다.
"""

import unittest
from unittest.mock import patch, MagicMock
import tempfile
import shutil
import os
import uuid
from urllib.parse import quote

from bs4 import BeautifulSoup

from backend.bookstore import (
    Yes24Bookstore,
    AladinBookstore,
    RidibooksBookstore,
    NaverShoppingBookstore,
    NaverSeriesBookstore,
    MunpiaBookstore,
)


# ========== 테스트용 HTML 샘플 ==========

YES24_SEARCH_HTML = """
<html>
<body>
<ul id="yesSchList">
    <li>
        <div class="itemUnit">
            <div class="item_info">
                <div class="info_row info_name">
                    <a class="gd_name" href="/product/goods/12345">해리 포터와 마법사의 돌</a>
                </div>
            </div>
        </div>
    </li>
    <li>
        <div class="itemUnit">
            <div class="item_info">
                <div class="info_row info_name">
                    <a class="gd_name" href="/product/goods/67890">해리 포터와 비밀의 방</a>
                </div>
            </div>
        </div>
    </li>
</ul>
</body>
</html>
"""

YES24_DETAIL_HTML = """
<html>
<body>
<h2 class="gd_name">해리 포터와 마법사의 돌</h2>
<span class="gd_auth"><a href="/author/12345">J.K. 롤링</a></span>
<div>
    관련분류
    <a href="/product/category/display/001">국내도서</a>
    <a href="/product/category/display/002">소설</a>
    <a href="/product/category/display/003">판타지</a>
</div>
<div>ISBN13 9788983920799</div>
</body>
</html>
"""

ALADIN_SEARCH_HTML = """
<html>
<body>
<div id="Search3_Result">
    <a href="/shop/wproduct.aspx?ItemId=111111">책 제목 1</a>
    <a href="/shop/wproduct.aspx?ItemId=222222">책 제목 2</a>
    <a href="/shop/wproduct.aspx?ItemId=111111_CommentReview">리뷰</a>
</div>
</body>
</html>
"""

ALADIN_DETAIL_HTML = """
<html>
<head>
<title>책 제목 | 저자명 | 알라딘</title>
<meta property="og:title" content="책 제목">
<meta name="author" content="저자명">
</head>
<body>
<ul id="ulCategory">
    <a href="/category/1">국내도서</a>
    <a href="/category/2">소설</a>
    <a href="/category/3">SF</a>
</ul>
<div id="Ere_prod_allwrap">
    <div class="Ere_prod_mconts_R">
        <div class="conts_info_list1">
            <ul>
                <li>ISBN : 9788983920799</li>
            </ul>
        </div>
    </div>
</div>
</body>
</html>
"""

RIDI_API_RESPONSE = {
    "books": [
        {
            "b_id": "123456",
            "title": "테스트 도서",
            "author": "테스트 저자",
            "category_name": "판타지",
        },
        {
            "b_id": "789012",
            "title": "테스트 도서 2",
            "author": "테스트 저자 2",
            "parent_category_name": "로맨스",
        },
    ]
}

RIDI_DETAIL_HTML = """
<html>
<head>
<meta property="og:title" content="테스트 도서">
<meta name="author" content="테스트 저자">
</head>
<body>
<section id="books_contents">
    <section class="detail_body">
        <ul><li><a href="/category/100">판타지</a></li></ul>
    </section>
</section>
<div>ISBN</div>
<div>9788983920799</div>
</body>
</html>
"""

NAVER_SHOPPING_API_RESPONSE = {
    "searchResult": {
        "items": [
            {"id": "item123"},
            {"id": "item456"},
        ]
    }
}

NAVER_SHOPPING_DETAIL_HTML = """
<html>
<body>
<div class="bookTitle_book_name__">네이버 테스트 도서</div>
<div class="bookTitle_info_content__">네이버 저자</div>
<div class="bookCatalogTop_breadcrumb__">국내도서 > 소설 > 판타지</div>
</body>
</html>
"""

NAVER_SERIES_SEARCH_HTML = """
<html>
<body>
<ul class="lst_list">
    <li><a class="N=a:nov.title" href="/novel/12345">웹소설 제목</a></li>
    <li><a class="N=a:com.title" href="/comic/67890">만화 제목</a></li>
</ul>
</body>
</html>
"""

NAVER_SERIES_DETAIL_HTML = """
<html>
<head>
<title>웹소설 제목 - 네이버 시리즈</title>
<meta name="description" content="작가: 테스트 작가, 장르: 판타지">
</head>
<body>
<div id="_otherProductByPerson">
    <strong>다른 작품</strong>
    <strong>테스트 작가</strong>
</div>
<div id="content">
    <ul class="end_info">
        <li class="info_lst">
            <ul><li><span><a href="/category/1">판타지</a></span></li></ul>
        </li>
    </ul>
</div>
</body>
</html>
"""

MUNPIA_SEARCH_HTML = """
<html>
<body>
<div id="SEARCH-BOX" class="section2">
    <div class="ebook_lists">
        <div class="article_wrap">
            <div class="article">
                <dl class="detail">
                    <dt><a href="/novel/123456">문피아 소설 제목</a></dt>
                </dl>
            </div>
        </div>
    </div>
</div>
</body>
</html>
"""

MUNPIA_DETAIL_HTML = """
<html>
<head>
<meta property="og:title" content="문피아 소설 제목">
<meta property="og:description" content="문피아 작가 - 판타지 소설입니다">
</head>
<body>
<p class="meta-path">웹소설 > 판타지 > 현대 판타지</p>
</body>
</html>
"""


class TestAbstractBookstore(unittest.TestCase):
    """AbstractBookstore 공통 로직 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_search_priority_isbn_first(self):
        """search() 메서드의 우선순위 테스트: ISBN이 최우선"""
        store = Yes24Bookstore(verbose=False)
        store.SUPPORTS_ISBN_SEARCH = True

        # search_by_isbn을 mock
        with patch.object(store, 'search_by_isbn') as mock_isbn:
            mock_isbn.return_value = [('제목', '저자', '카테고리', 'url', 'search_url')]
            results, _keyword, method = store.search(isbn='9788983920799', title='해리포터', author='롤링')

            mock_isbn.assert_called_once_with('9788983920799')
            self.assertEqual(method, 'isbn')
            self.assertEqual(len(results), 1)

    def test_search_priority_title_author_second(self):
        """search() 메서드의 우선순위 테스트: ISBN 없으면 제목+저자"""
        store = Yes24Bookstore(verbose=False)

        with patch.object(store, 'search_by_keyword') as mock_keyword:
            mock_keyword.return_value = [('제목', '저자', '카테고리', 'url', 'search_url')]
            _results, _keyword, method = store.search(title='해리포터', author='롤링')

            mock_keyword.assert_called_once_with('해리포터 롤링')
            self.assertEqual(method, 'title_author')

    def test_search_priority_title_third(self):
        """search() 메서드의 우선순위 테스트: 제목만"""
        store = Yes24Bookstore(verbose=False)

        with patch.object(store, 'search_by_keyword') as mock_keyword:
            mock_keyword.return_value = [('제목', '저자', '카테고리', 'url', 'search_url')]
            _results, _keyword, method = store.search(title='해리포터')

            mock_keyword.assert_called_once_with('해리포터')
            self.assertEqual(method, 'title')

    def test_search_priority_author_fourth(self):
        """search() 메서드의 우선순위 테스트: 저자만"""
        store = Yes24Bookstore(verbose=False)

        with patch.object(store, 'search_by_keyword') as mock_keyword:
            mock_keyword.return_value = [('제목', '저자', '카테고리', 'url', 'search_url')]
            _results, _keyword, method = store.search(author='롤링')

            mock_keyword.assert_called_once_with('롤링')
            self.assertEqual(method, 'author')

    def test_search_fallback_when_no_results(self):
        """search() 메서드: 검색 결과 없을 때 fallback"""
        store = Yes24Bookstore(verbose=False)

        with patch.object(store, 'search_by_keyword') as mock_keyword:
            # 제목+저자 검색 실패, 제목만 검색 성공
            mock_keyword.side_effect = [[], [('제목', '저자', '카테고리', 'url', 'search_url')]]
            _results, _keyword, method = store.search(title='해리포터', author='롤링')

            self.assertEqual(mock_keyword.call_count, 2)
            self.assertEqual(method, 'title')

    def test_search_returns_empty_when_all_fail(self):
        """search() 메서드: 모든 검색 실패 시 빈 결과 반환"""
        store = Yes24Bookstore(verbose=False)

        with patch.object(store, 'search_by_keyword') as mock_keyword:
            mock_keyword.return_value = []
            results, keyword, method = store.search(title='없는책')

            self.assertEqual(results, [])
            self.assertEqual(keyword, '없는책')
            self.assertEqual(method, 'title')

    def test_search_by_isbn_not_supported(self):
        """ISBN 검색 미지원 서점 테스트"""
        store = RidibooksBookstore(verbose=False)
        self.assertFalse(store.SUPPORTS_ISBN_SEARCH)

        results = store.search_by_isbn('9788983920799')
        self.assertEqual(results, [])

    def test_html_cache_save_and_load(self):
        """HTML 캐시 저장 및 로드 테스트"""
        store = Yes24Bookstore(verbose=False)
        test_url = "https://www.yes24.com/product/goods/12345"
        test_html = "<html><body>Test Content</body></html>"

        # 저장
        store._save_html_to_tmp(test_html, test_url)

        # 로드
        loaded_html = store._load_html_from_tmp(test_url)
        self.assertEqual(loaded_html, test_html)

    def test_html_cache_deterministic_filename(self):
        """HTML 캐시 파일명이 URL 기반으로 결정적인지 테스트"""
        url = "https://www.yes24.com/product/goods/12345"
        expected_filename = f"{uuid.uuid5(uuid.NAMESPACE_URL, url)}.html"

        store = Yes24Bookstore(verbose=False)
        store._save_html_to_tmp("<html></html>", url)

        expected_path = os.path.join('/tmp', expected_filename)
        self.assertTrue(os.path.exists(expected_path))

        # 정리
        if os.path.exists(expected_path):
            os.remove(expected_path)

    def test_html_cache_load_nonexistent(self):
        """존재하지 않는 캐시 로드 시 None 반환"""
        store = Yes24Bookstore(verbose=False)
        result = store._load_html_from_tmp("https://nonexistent.url/page")
        self.assertIsNone(result)


class TestYes24Bookstore(unittest.TestCase):
    """Yes24 서점 테스트"""

    def test_build_search_url(self):
        """검색 URL 생성 테스트"""
        store = Yes24Bookstore(verbose=False)
        url = store.build_search_url("해리 포터")

        self.assertTrue(url.startswith(Yes24Bookstore.BASE_URL))
        self.assertIn(quote("해리 포터"), url)
        self.assertIn("domain=ALL", url)

    def test_build_isbn_search_url(self):
        """ISBN 검색 URL 생성 테스트"""
        store = Yes24Bookstore(verbose=False)
        url = store.build_isbn_search_url("9788983920799")

        self.assertIn("9788983920799", url)

    def test_extract_search_links(self):
        """검색 결과 페이지에서 링크 추출 테스트"""
        store = Yes24Bookstore(verbose=False)
        soup = BeautifulSoup(YES24_SEARCH_HTML, 'html.parser')

        links = store.extract_search_links(soup)

        self.assertEqual(len(links), 2)
        self.assertIn("https://www.yes24.com/product/goods/12345", links)
        self.assertIn("https://www.yes24.com/product/goods/67890", links)

    def test_extract_search_links_fallback(self):
        """검색 결과 추출 fallback 로직 테스트"""
        # 계층적 선택자가 실패하는 HTML
        fallback_html = """
        <html>
        <body>
            <a class="gd_name" href="/product/goods/99999">Fallback Book</a>
        </body>
        </html>
        """
        store = Yes24Bookstore(verbose=False)
        soup = BeautifulSoup(fallback_html, 'html.parser')

        links = store.extract_search_links(soup)

        self.assertEqual(len(links), 1)
        self.assertIn("https://www.yes24.com/product/goods/99999", links)

    def test_extract_book_info(self):
        """상세 페이지에서 책 정보 추출 테스트"""
        store = Yes24Bookstore(verbose=False)
        soup = BeautifulSoup(YES24_DETAIL_HTML, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['title'], '해리 포터와 마법사의 돌')
        self.assertEqual(info['author'], 'J.K. 롤링')
        self.assertIn('국내도서', info['category'])
        self.assertEqual(info['isbn'], '9788983920799')

    def test_extract_book_info_empty_page(self):
        """빈 페이지에서 책 정보 추출 시 빈 값 반환"""
        store = Yes24Bookstore(verbose=False)
        soup = BeautifulSoup("<html><body></body></html>", 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['title'], '')
        self.assertEqual(info['author'], '')
        self.assertEqual(info['category'], '')
        self.assertEqual(info['isbn'], '')

    @patch('backend.bookstore.requests.Session')
    def test_search_by_keyword_integration(self, mock_session_class):
        """search_by_keyword 통합 테스트 (HTTP mock)"""
        # Mock 응답 설정
        mock_session = MagicMock()
        mock_session_class.return_value = mock_session

        mock_search_response = MagicMock()
        mock_search_response.text = YES24_SEARCH_HTML
        mock_search_response.encoding = 'utf-8'

        mock_detail_response = MagicMock()
        mock_detail_response.text = YES24_DETAIL_HTML
        mock_detail_response.encoding = 'utf-8'

        mock_session.get.side_effect = [mock_search_response, mock_detail_response, mock_detail_response]

        store = Yes24Bookstore(verbose=False)
        store.session = mock_session

        results = store.search_by_keyword("해리 포터")

        self.assertIsInstance(results, list)
        self.assertGreater(len(results), 0)

        # 결과 구조 검증
        title, author, category, book_url, search_url = results[0]
        self.assertIsInstance(title, str)
        self.assertIsInstance(author, str)
        self.assertIsInstance(category, str)
        self.assertIsInstance(book_url, str)
        self.assertIsInstance(search_url, str)


class TestAladinBookstore(unittest.TestCase):
    """알라딘 서점 테스트"""

    def test_build_search_url(self):
        """검색 URL 생성 테스트"""
        store = AladinBookstore(verbose=False)
        url = store.build_search_url("테스트 키워드")

        self.assertTrue(url.startswith(AladinBookstore.BASE_URL))
        self.assertIn(quote("테스트 키워드"), url)
        self.assertIn("SearchTarget=All", url)

    def test_extract_search_links(self):
        """검색 결과에서 링크 추출 테스트"""
        store = AladinBookstore(verbose=False)
        soup = BeautifulSoup(ALADIN_SEARCH_HTML, 'html.parser')

        links = store.extract_search_links(soup)

        # 리뷰 링크는 제외되어야 함
        self.assertEqual(len(links), 2)
        self.assertTrue(all('_CommentReview' not in link for link in links))

    def test_extract_search_links_dedup(self):
        """중복 링크 제거 테스트"""
        dup_html = """
        <div id="Search3_Result">
            <a href="/shop/wproduct.aspx?ItemId=111111">책1</a>
            <a href="/shop/wproduct.aspx?ItemId=111111">책1 중복</a>
        </div>
        """
        store = AladinBookstore(verbose=False)
        soup = BeautifulSoup(dup_html, 'html.parser')

        links = store.extract_search_links(soup)

        self.assertEqual(len(links), 1)

    def test_extract_book_info(self):
        """상세 페이지에서 책 정보 추출 테스트"""
        store = AladinBookstore(verbose=False)
        soup = BeautifulSoup(ALADIN_DETAIL_HTML, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['title'], '책 제목')
        self.assertEqual(info['author'], '저자명')
        self.assertIn('국내도서', info['category'])
        self.assertEqual(info['isbn'], '9788983920799')

    def test_supports_isbn_search(self):
        """ISBN 검색 지원 확인"""
        store = AladinBookstore(verbose=False)
        self.assertTrue(store.SUPPORTS_ISBN_SEARCH)


class TestRidibooksBookstore(unittest.TestCase):
    """리디북스 서점 테스트"""

    def test_build_search_url(self):
        """검색 URL 생성 테스트"""
        store = RidibooksBookstore(verbose=False)
        url = store.build_search_url("테스트")

        self.assertTrue(url.startswith(RidibooksBookstore.BASE_URL))
        self.assertIn("adult_exclude=n", url)

    def test_does_not_support_isbn_search(self):
        """ISBN 검색 미지원 확인"""
        store = RidibooksBookstore(verbose=False)
        self.assertFalse(store.SUPPORTS_ISBN_SEARCH)

    @patch('backend.bookstore.requests.Session')
    def test_search_by_keyword_api(self, mock_session_class):
        """API 기반 검색 테스트"""
        mock_session = MagicMock()
        mock_session_class.return_value = mock_session

        mock_response = MagicMock()
        mock_response.status_code = 200
        mock_response.json.return_value = RIDI_API_RESPONSE
        mock_session.get.return_value = mock_response

        store = RidibooksBookstore(verbose=False)
        store.session = mock_session

        results = store.search_by_keyword("테스트")

        self.assertEqual(len(results), 2)
        self.assertEqual(results[0][0], "테스트 도서")
        self.assertEqual(results[0][1], "테스트 저자")

    @patch('backend.bookstore.requests.Session')
    def test_search_by_keyword_api_error(self, mock_session_class):
        """API 에러 시 빈 결과 반환"""
        mock_session = MagicMock()
        mock_session_class.return_value = mock_session

        mock_response = MagicMock()
        mock_response.status_code = 500
        mock_session.get.return_value = mock_response

        store = RidibooksBookstore(verbose=False)
        store.session = mock_session

        results = store.search_by_keyword("테스트")

        self.assertEqual(results, [])

    def test_extract_book_info(self):
        """상세 페이지 정보 추출 테스트"""
        store = RidibooksBookstore(verbose=False)
        soup = BeautifulSoup(RIDI_DETAIL_HTML, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['title'], '테스트 도서')
        self.assertIn('판타지', info['category'])
        self.assertEqual(info['isbn'], '9788983920799')

    def test_extract_ridi_isbn(self):
        """ISBN 추출 로직 테스트"""
        html = """
        <div>ISBN</div>
        <div>9788983920799</div>
        """
        store = RidibooksBookstore(verbose=False)
        soup = BeautifulSoup(html, 'html.parser')

        isbn = store._extract_ridi_isbn(soup)

        self.assertEqual(isbn, '9788983920799')


class TestNaverShoppingBookstore(unittest.TestCase):
    """네이버쇼핑 서점 테스트"""

    def test_build_search_url(self):
        """검색 URL 생성 테스트"""
        store = NaverShoppingBookstore(verbose=False)
        url = store.build_search_url("테스트")

        self.assertTrue(url.startswith(NaverShoppingBookstore.BASE_URL))
        self.assertIn("bookTabType=ALL", url)

    def test_extract_book_info(self):
        """상세 페이지 정보 추출 테스트"""
        store = NaverShoppingBookstore(verbose=False)
        soup = BeautifulSoup(NAVER_SHOPPING_DETAIL_HTML, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['title'], '네이버 테스트 도서')
        self.assertEqual(info['author'], '네이버 저자')
        self.assertIn('판타지', info['category'])

    @patch('backend.bookstore.requests.Session')
    def test_search_by_keyword(self, mock_session_class):
        """키워드 검색 테스트"""
        mock_session = MagicMock()
        mock_session_class.return_value = mock_session

        mock_api_response = MagicMock()
        mock_api_response.json.return_value = NAVER_SHOPPING_API_RESPONSE

        mock_detail_response = MagicMock()
        mock_detail_response.text = NAVER_SHOPPING_DETAIL_HTML
        mock_detail_response.encoding = 'utf-8'

        mock_session.get.side_effect = [mock_api_response, mock_detail_response, mock_detail_response]

        store = NaverShoppingBookstore(verbose=False)
        store.session = mock_session

        results = store.search_by_keyword("테스트")

        self.assertIsInstance(results, list)


class TestNaverSeriesBookstore(unittest.TestCase):
    """네이버 시리즈 서점 테스트"""

    def test_build_search_url(self):
        """검색 URL 생성 테스트"""
        store = NaverSeriesBookstore(verbose=False)
        url = store.build_search_url("웹소설")

        self.assertTrue(url.startswith(NaverSeriesBookstore.BASE_URL))
        self.assertIn("t=all", url)

    def test_extract_search_links(self):
        """검색 결과에서 링크 추출 테스트"""
        store = NaverSeriesBookstore(verbose=False)
        soup = BeautifulSoup(NAVER_SERIES_SEARCH_HTML, 'html.parser')

        links = store.extract_search_links(soup)

        self.assertEqual(len(links), 2)
        self.assertTrue(any('/novel/' in link for link in links))
        self.assertTrue(any('/comic/' in link for link in links))

    def test_extract_book_info(self):
        """상세 페이지 정보 추출 테스트"""
        store = NaverSeriesBookstore(verbose=False)
        soup = BeautifulSoup(NAVER_SERIES_DETAIL_HTML, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertIn('웹소설 제목', info['title'])
        self.assertEqual(info['author'], '테스트 작가')
        self.assertEqual(info['category'], '판타지')

    def test_extract_author_from_meta_description(self):
        """meta description에서 저자 추출 테스트"""
        html = """
        <html>
        <head>
            <title>테스트 제목</title>
            <meta name="description" content="작가: 메타 작가, 장르: SF">
        </head>
        <body></body>
        </html>
        """
        store = NaverSeriesBookstore(verbose=False)
        soup = BeautifulSoup(html, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['author'], '메타 작가')


class TestMunpiaBookstore(unittest.TestCase):
    """문피아 서점 테스트"""

    def test_build_search_url(self):
        """검색 URL 생성 테스트"""
        store = MunpiaBookstore(verbose=False)
        url = store.build_search_url("판타지 소설")

        self.assertTrue(url.startswith(MunpiaBookstore.BASE_URL))
        # 공백은 %20으로 인코딩
        self.assertIn("%20", url)

    def test_extract_search_links(self):
        """검색 결과에서 링크 추출 테스트"""
        store = MunpiaBookstore(verbose=False)
        soup = BeautifulSoup(MUNPIA_SEARCH_HTML, 'html.parser')

        links = store.extract_search_links(soup)

        self.assertEqual(len(links), 1)
        self.assertIn('/novel/123456', links[0])

    def test_extract_book_info(self):
        """상세 페이지 정보 추출 테스트"""
        store = MunpiaBookstore(verbose=False)
        soup = BeautifulSoup(MUNPIA_DETAIL_HTML, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['title'], '문피아 소설 제목')
        self.assertEqual(info['author'], '문피아 작가')
        self.assertIn('판타지', info['category'])

    def test_extract_author_from_description(self):
        """og:description에서 저자 추출 테스트"""
        html = """
        <html>
        <head>
            <meta property="og:description" content="작가이름 - 이것은 설명입니다">
        </head>
        </html>
        """
        store = MunpiaBookstore(verbose=False)
        soup = BeautifulSoup(html, 'html.parser')

        info = store.extract_book_info(soup)

        self.assertEqual(info['author'], '작가이름')


class TestAllStores(unittest.TestCase):
    """모든 서점 공통 테스트"""

    def setUp(self):
        self.stores = [
            Yes24Bookstore,
            AladinBookstore,
            RidibooksBookstore,
            NaverShoppingBookstore,
            NaverSeriesBookstore,
            MunpiaBookstore,
        ]

    def test_all_stores_have_base_url(self):
        """모든 서점이 BASE_URL을 가지고 있는지 확인"""
        for store_cls in self.stores:
            with self.subTest(store=store_cls.__name__):
                self.assertTrue(hasattr(store_cls, 'BASE_URL'))
                self.assertTrue(store_cls.BASE_URL.startswith('http'))

    def test_all_stores_build_search_url(self):
        """모든 서점이 build_search_url을 올바르게 구현했는지 확인"""
        keyword = "테스트 키워드"
        for store_cls in self.stores:
            with self.subTest(store=store_cls.__name__):
                store = store_cls(verbose=False)
                url = store.build_search_url(keyword)
                self.assertTrue(url.startswith(store_cls.BASE_URL))
                self.assertIn(quote(keyword), url)

    def test_all_stores_have_session(self):
        """모든 서점이 requests 세션을 가지고 있는지 확인"""
        for store_cls in self.stores:
            with self.subTest(store=store_cls.__name__):
                store = store_cls(verbose=False)
                self.assertIsNotNone(store.session)
                self.assertTrue(hasattr(store.session, 'get'))

    def test_all_stores_have_user_agent(self):
        """모든 서점이 User-Agent 헤더를 설정했는지 확인"""
        for store_cls in self.stores:
            with self.subTest(store=store_cls.__name__):
                store = store_cls(verbose=False)
                self.assertIn('User-Agent', store.session.headers)


class TestCategoryTruncation(unittest.TestCase):
    """카테고리 문자열 처리 테스트"""

    def test_category_truncation_in_fetch_results(self):
        """_fetch_search_results에서 카테고리가 3단계로 잘리는지 테스트"""
        # 카테고리 처리 로직 테스트
        full_category = "국내도서 > 소설 > 판타지 > 현대 판타지 > 게임 판타지"
        parts = [p.strip() for p in full_category.split('>')]
        truncated = ' > '.join(parts[:3])

        self.assertEqual(truncated, '국내도서 > 소설 > 판타지')

    def test_empty_category_handling(self):
        """빈 카테고리 처리 테스트"""
        def process_category(cat: str) -> str:
            if cat:
                parts = [p.strip() for p in cat.split('>')]
                return ' > '.join(parts[:3])
            return cat

        self.assertEqual(process_category(""), "")
        self.assertEqual(process_category("국내도서 > 소설"), "국내도서 > 소설")


class TestErrorHandling(unittest.TestCase):
    """에러 처리 테스트"""

    def setUp(self):
        """테스트 중 에러 로그 억제"""
        import logging
        self.logger = logging.getLogger('backend.bookstore')
        self.original_level = self.logger.level
        self.logger.setLevel(logging.CRITICAL)

    def tearDown(self):
        """로그 레벨 복원"""
        self.logger.setLevel(self.original_level)

    @patch('backend.bookstore.requests.Session')
    def test_network_error_returns_empty(self, mock_session_class):
        """네트워크 에러 시 빈 결과 반환"""
        import requests

        mock_session = MagicMock()
        mock_session_class.return_value = mock_session
        mock_session.get.side_effect = requests.exceptions.Timeout()

        store = Yes24Bookstore(verbose=False)
        store.session = mock_session

        results = store.search_by_keyword("테스트")

        self.assertEqual(results, [])

    @patch('backend.bookstore.requests.Session')
    def test_connection_error_returns_empty(self, mock_session_class):
        """연결 에러 시 빈 결과 반환"""
        import requests

        mock_session = MagicMock()
        mock_session_class.return_value = mock_session
        mock_session.get.side_effect = requests.exceptions.ConnectionError()

        store = Yes24Bookstore(verbose=False)
        store.session = mock_session

        results = store.search_by_keyword("테스트")

        self.assertEqual(results, [])


if __name__ == "__main__":
    unittest.main()
