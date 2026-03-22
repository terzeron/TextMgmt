from __future__ import annotations

import json
import tempfile
from pathlib import Path
from typing import List, Tuple

import pytest
from bs4 import BeautifulSoup

from backend.bookstore import (
    AbstractBookstore,
    Yes24Bookstore,
    AladinBookstore,
    RidibooksBookstore,
    NaverShoppingBookstore,
    MunpiaBookstore,
    NaverSeriesBookstore,
)


class DummyBookstore(AbstractBookstore):
    BASE_URL = "https://example.com"
    SUPPORTS_ISBN_SEARCH = True

    def __init__(self):
        super().__init__(verbose=False)
        self.seen: list[str] = []

    def build_search_url(self, keyword: str) -> str:
        self.seen.append(keyword)
        return f"{self.BASE_URL}/search?q={keyword}"

    def extract_search_links(self, soup: BeautifulSoup) -> List[str]:
        return [a["href"] for a in soup.find_all("a", href=True)]

    def extract_book_info(self, soup: BeautifulSoup):
        title = soup.find("h1")
        author = soup.find("span", class_="author")
        category = soup.find("div", class_="cat")
        isbn = soup.find("div", class_="isbn")
        return {
            "title": title.get_text(strip=True) if title else "",
            "author": author.get_text(strip=True) if author else "",
            "category": category.get_text(strip=True) if category else "",
            "isbn": isbn.get_text(strip=True) if isbn else "",
        }


def test_truncate_title_separators():
    assert AbstractBookstore._truncate_title("A - B") == "A"
    assert AbstractBookstore._truncate_title("A－B") == "A"
    assert AbstractBookstore._truncate_title("A：B") == "A"
    assert AbstractBookstore._truncate_title("A-B") == "A"


def test_search_prefers_isbn_then_title_author_then_title():
    store = DummyBookstore()
    store.search_by_isbn = lambda isbn: [("t", "a", "", "", "", isbn)]  # type: ignore[method-assign]
    results, keyword, method = store.search(isbn="123", title="Title", author="Auth")
    assert results
    assert keyword == "123"
    assert method == "isbn"

    store2 = DummyBookstore()
    store2.SUPPORTS_ISBN_SEARCH = False
    store2.search_by_keyword = lambda keyword: [("t", "a", "", "", "", "")]  # type: ignore[method-assign]
    results, keyword, method = store2.search(title="Title", author="Auth")
    assert results
    assert method == "title_author"
    assert keyword == "Title Auth"

    store3 = DummyBookstore()
    store3.SUPPORTS_ISBN_SEARCH = False
    store3.search_by_keyword = lambda keyword: [("t", "a", "", "", "", "")]  # type: ignore[method-assign]
    results, keyword, method = store3.search(title="Title")
    assert results
    assert method == "title"
    assert keyword == "Title"

    store4 = DummyBookstore()
    store4.SUPPORTS_ISBN_SEARCH = False
    store4.search_by_keyword = lambda keyword: []  # type: ignore[method-assign]
    results, keyword, method = store4.search(title="Title", author="Auth")
    assert results == []
    assert method == "title_author"
    assert keyword == "Title Auth"


def test_fetch_search_results_uses_cached_html(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore()

    search_html = "<html><a href='https://example.com/detail/1'>x</a></html>"
    detail_html = "<html><h1>Book</h1><span class='author'>Auth</span><div class='cat'>A > B > C > D</div><div class='isbn'>123</div></html>"

    def fake_get(url, timeout=10, verify=True):
        class Resp:
            status_code = 200
            text = search_html
            encoding = "utf-8"
        return Resp()

    monkeypatch.setattr(store.session, "get", fake_get)
    monkeypatch.setattr(store, "_load_html_from_tmp", lambda url: detail_html)

    results = store._fetch_search_results("https://example.com/search?q=x")
    assert results == [("Book", "Auth", "A > B > C", "https://example.com/detail/1", "https://example.com/search?q=x", "123")]


def test_save_and_load_html_cache(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore()
    monkeypatch.setattr(tempfile, "gettempdir", lambda: str(tmp_path))
    url = "https://example.com/detail/1"
    html = "<html>cached</html>"
    store._save_html_to_tmp(html, url)
    loaded = store._load_html_from_tmp(url)
    assert loaded == html


def test_yes24_extract_search_links_and_category_and_isbn():
    store = Yes24Bookstore(verbose=False)
    html = """
    <html>
      <ul id="yesSchList">
        <li><div class="itemUnit"><div class="item_info"><div class="info_row info_name">
          <a class="gd_name" href="/product/goods/123">A</a>
        </div></div></div></li>
      </ul>
      <div>관련분류
        <a href="/product/category/display/1">국내도서</a>
        <a href="/product/category/display/2">소설</a>
      </div>
      ISBN13 9781234567890
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://www.yes24.com/product/goods/123"]
    assert store._extract_yes24_category(soup) == "국내도서 > 소설"
    assert store._extract_yes24_isbn(soup) == "9781234567890"


def test_yes24_extract_book_info_author_fallback():
    store = Yes24Bookstore(verbose=False)
    html = "<html><h2 class='gd_name'>Title</h2><span class='gd_auth'>Auth</span></html>"
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "Title"
    assert info["author"] == "Auth"


def test_aladin_strip_trailing_number_and_search_author_first(monkeypatch: pytest.MonkeyPatch):
    store = AladinBookstore(verbose=False)
    captured = {}

    def fake_search(keyword):
        captured["keyword"] = keyword
        return [("t", "a", "", "", "", "")]

    monkeypatch.setattr(store, "search_by_keyword", fake_search)
    results, keyword, method = store.search(title="마왕의 딸 3", author="홍길동")
    assert results
    assert method == "title_author"
    assert keyword == "홍길동 마왕의 딸"
    assert captured["keyword"] == "홍길동 마왕의 딸"


def test_aladin_extract_search_links_and_info():
    store = AladinBookstore(verbose=False)
    html = """
    <html>
      <div id="Search3_Result">
        <a href="/shop/wproduct.aspx?ItemId=111">A</a>
        <a href="/shop/wproduct.aspx?ItemId=111">A2</a>
        <a href="/shop/wproduct.aspx?ItemId=222_CommentReview">Review</a>
        <a href="/shop/wproduct.aspx?ItemId=222">B</a>
      </div>
      <title>Title | Author | Aladin</title>
      <meta name="author" content="MetaAuthor" />
      <meta property="og:title" content="MetaTitle" />
      <ul id="ulCategory"><a>국내도서</a><a>소설</a></ul>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == [
        "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=111",
        "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=222",
    ]
    info = store.extract_book_info(soup)
    assert info["title"] == "MetaTitle"
    assert info["author"] == "MetaAuthor"
    assert info["category"] == "국내도서 > 소설"


def test_aladin_extract_isbn_fallback():
    store = AladinBookstore(verbose=False)
    html = """
    <html>
      <div id="Ere_prod_allwrap">
        <div class="Ere_prod_middlewrap">
          <div class="Ere_prod_mconts_R">
            <ul><li>기타</li><li>ISBN 9781234567890</li></ul>
          </div>
        </div>
      </div>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    assert store._extract_aladin_isbn(soup) == "9781234567890"


def test_ridibooks_search_by_keyword_and_extract_info(monkeypatch: pytest.MonkeyPatch):
    store = RidibooksBookstore(verbose=False)

    def fake_get(url, params=None, timeout=10, verify=True):
        class Resp:
            status_code = 200
            def json(self):
                return {
                    "books": [
                        {
                            "b_id": "1",
                            "title": "T",
                            "author": "A",
                            "parent_category_name": "P",
                            "category_name": "C",
                            "parent_category_name2": "P2",
                            "category_name2": "C2",
                        }
                    ]
                }
        return Resp()

    monkeypatch.setattr(store.session, "get", fake_get)
    results = store.search_by_keyword("키워드")
    assert results == [("T", "A", "P > C || P2 > C2", "https://ridibooks.com/books/1", "https://ridibooks.com/search?q=%ED%82%A4%EC%9B%8C%EB%93%9C&adult_exclude=n", "")]

    html = """
    <html>
      <meta property="og:title" content="Title" />
      <div id="ISLANDS__Header"><ul><li><li>홍길동저자</li></li></ul></div>
      <div id="books_contents">
        <section class="detail_body"><ul><li><a href="/category/1">A</a><a href="/category/2">B</a></li></ul></section>
      </div>
      <div>ISBN</div><div>9781234567890</div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "Title"
    assert info["author"] == "홍길동"
    assert info["category"] == "A > B"
    assert info["isbn"] == "9781234567890"


def test_naver_shopping_links_and_info():
    store = NaverShoppingBookstore(verbose=False)
    data = {
        "props": {
            "pageProps": {
                "dehydratedState": {
                    "queries": [
                        {
                            "queryKey": ["SearchAll", "book"],
                            "state": {
                                "data": {
                                    "SearchAll": {
                                        "bookSasResult": {"itemList": [{"id": "123"}]}
                                    }
                                }
                            },
                        }
                    ]
                }
            }
        }
    }
    html = f"<html><script>{json.dumps(data)}</script></html>"
    links = store.extract_search_links(BeautifulSoup(html, "html.parser"))
    assert links == ["https://search.shopping.naver.com/book/catalog/123"]

    detail_html = """
    <html>
      <h2 class="bookTitle_book_name__abc">Title</h2>
      <div class="bookTitle_info_title__1">저자</div>
      <div class="bookTitle_info_content__1">Author</div>
      <div class="bookCatalogTop_breadcrumb__1">국내도서 > 소설</div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(detail_html, "html.parser"))
    assert info["title"] == "Title"
    assert info["author"] == "Author"
    assert info["category"] == "국내도서 > 소설"


def test_munpia_extract_links_and_info():
    store = MunpiaBookstore(verbose=False)
    html = """
    <html>
      <div id="SEARCH-BOX" class="section2">
        <div class="ebook_lists">
          <div class="article_wrap">
            <div class="article">
              <dl class="detail"><dt><a href="/book/1">A</a></dt></dl>
            </div>
          </div>
        </div>
      </div>
      <meta property="og:title" content="Title" />
      <meta property="og:description" content="Author - Desc" />
      <p class="meta-path">장르 > 판타지</p>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://novel.munpia.com/book/1"]
    info = store.extract_book_info(soup)
    assert info["title"] == "Title"
    assert info["author"] == "Author"
    assert info["category"] == "장르 > 판타지"


def test_naver_series_links_and_info():
    store = NaverSeriesBookstore(verbose=False)
    html = """
    <html>
      <ul class="lst_list"><li><a class="N=a:foo" href="/detail/1">A</a></li></ul>
      <div id="_otherProductByPerson"><strong>dummy</strong><strong>작가명</strong></div>
      <div id="content"><ul class="end_info"><li class="info_lst"><ul><li><span><a>장르</a></span></li></ul></li></ul></div>
      <title>Title</title>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://series.naver.com/detail/1"]
    info = store.extract_book_info(soup)
    assert info["title"] == "Title"
    assert info["author"] == "작가명"
    assert info["category"] == "장르"
