import unittest, sys, os
from pathlib import Path
from unittest.mock import patch, MagicMock
from bs4 import BeautifulSoup
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from backend.bookstore import (
    AbstractBookstore,
    Yes24Bookstore,
    AladinBookstore,
    RidibooksBookstore,
    NaverShoppingBookstore,
    NaverSeriesBookstore,
    MunpiaBookstore,
)

class TestBookstore(unittest.TestCase):
    def test_yes24_extract(self):
        store = Yes24Bookstore(verbose=False)
        html = "<html><h2 class='gd_name'>Title</h2><span class='gd_auth'>Author</span></html>"
        info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
        self.assertEqual(info["title"], "Title")
    def test_aladin_extract(self):
        store = AladinBookstore(verbose=False)
        html = "<html><title>Title | Author | Aladin</title></html>"
        info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
        self.assertEqual(info["title"], "Title")


# ---- merged from test_bookstore_extra_branches.py ----
import json

import pytest
from bs4 import BeautifulSoup

from backend.bookstore import (
    Yes24Bookstore,
    AladinBookstore,
    RidibooksBookstore,
    NaverShoppingBookstore,
    NaverSeriesBookstore,
    MunpiaBookstore,
)


def test_yes24_fallback_links_and_no_category_isbn():
    store = Yes24Bookstore(verbose=False)
    html = '<html><a class="gd_name" href="/product/goods/1">X</a></html>'
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://www.yes24.com/product/goods/1"]
    assert store._extract_yes24_category(soup) == ""
    assert store._extract_yes24_isbn(soup) == ""


def test_aladin_meta_fallbacks_and_no_isbn():
    store = AladinBookstore(verbose=False)
    html = """
    <html>
      <title>Title | Author | Aladin</title>
      <meta property="og:title" content="OT" />
      <meta property="og:author" content="OA" />
      <div id="Search3_Result"></div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "OT"
    assert info["author"] == "OA"
    assert store._extract_aladin_isbn(BeautifulSoup("<html></html>", "html.parser")) == ""


def test_ridibooks_search_failure_and_author_fallbacks(monkeypatch: pytest.MonkeyPatch):
    store = RidibooksBookstore(verbose=False)

    class Resp:
        status_code = 500

    monkeypatch.setattr(store.session, "get", lambda *a, **k: Resp())
    assert store.search_by_keyword("k") == []

    html = """
    <html>
      <h1>HT</h1>
      <li>홍길동저자</li>
      <a href="/author/1">Auth</a>
      <meta name="author" content="MA" />
      <meta property="og:description" content="저자: MD, 기타" />
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "HT"
    assert info["author"] != ""


def test_naver_shopping_json_missing_and_bad():
    store = NaverShoppingBookstore(verbose=False)
    soup = BeautifulSoup("<html></html>", "html.parser")
    assert store.extract_search_links(soup) == []

    soup2 = BeautifulSoup("<html><script>{bad</script></html>", "html.parser")
    assert store.extract_search_links(soup2) == []


def test_naver_series_meta_description_fallback():
    store = NaverSeriesBookstore(verbose=False)
    html = """
    <html>
      <meta name="description" content="작가: 홍길동, 기타" />
      <div id="content"><ul class="end_info"><li class="info_lst"><ul><li><span><a>장르</a></span></li></ul></li></ul></div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "홍길동"


def test_munpia_fallback_author():
    store = MunpiaBookstore(verbose=False)
    html = """
    <html>
      <meta name="title" content="T" />
      <a href="/writer/1">Writer</a>
      <p class="meta-path">A > B</p>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "Writer"


class DummyBookstore(AbstractBookstore):
    BASE_URL = "https://example.com"
    SUPPORTS_ISBN_SEARCH = True
    AUTHOR_FIRST_SEARCH = True

    def build_search_url(self, keyword: str) -> str:
        return f"{self.BASE_URL}/search?q={keyword}"

    def build_isbn_search_url(self, isbn: str) -> str:
        return f"{self.BASE_URL}/isbn/{isbn}"

    def extract_search_links(self, soup: BeautifulSoup):
        return ["https://example.com/detail/1", "https://example.com/detail/2"]

    def extract_book_info(self, soup: BeautifulSoup):
        title = soup.find("h1").text if soup.find("h1") else ""
        author = soup.find("span", class_="author").text if soup.find("span", class_="author") else ""
        category = soup.find("p", class_="category").text if soup.find("p", class_="category") else ""
        isbn = soup.find("p", class_="isbn").text if soup.find("p", class_="isbn") else ""
        return {"title": title, "author": author, "category": category, "isbn": isbn}


def test_truncate_title_variants():
    store = DummyBookstore(verbose=False)
    assert store._truncate_title("A - B") == "A"
    assert store._truncate_title("A－B") == "A"
    assert store._truncate_title("A - B - C") == "A"
    assert store._truncate_title("A：B") == "A"


def test_search_precedence_and_fallbacks(monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore(verbose=False)

    def fake_fetch(url: str):
        if "/isbn/" in url:
            return [("T", "A", "C", url, url, "ISBN")]
        if "author title" in url:
            return [("T2", "A2", "C2", url, url, "")]
        if "title" in url:
            return []
        return []

    monkeypatch.setattr(store, "_fetch_search_results", fake_fetch)

    results, keyword, method = store.search(isbn="123", title="title", author="author")
    assert method == "isbn"
    assert keyword == "123"
    assert results

    results, keyword, method = store.search(isbn="", title="title", author="author")
    assert method == "title_author"
    assert keyword == "author title"
    assert results

    results, keyword, method = store.search(isbn="", title="only", author="")
    assert method == "title"
    assert keyword == "only"
    assert results == []


def test_fetch_search_results_cached_and_trimmed(monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore(verbose=False)

    class Resp:
        def __init__(self, text: str):
            self.text = text
            self.encoding = "utf-8"

    monkeypatch.setattr(store.session, "get", lambda *a, **k: Resp("<html></html>"))

    cached = {
        "https://example.com/detail/1": "<html><h1>Title1</h1><span class='author'>Auth</span><p class='category'>A > B > C > D</p><p class='isbn'>123</p></html>",
        "https://example.com/detail/2": "   ",
    }
    monkeypatch.setattr(store, "_load_html_from_tmp", lambda url: cached.get(url))
    monkeypatch.setattr(store, "_save_html_to_tmp", lambda html, url: None)

    results = store._fetch_search_results("https://example.com/search?q=x")
    assert results == [("Title1", "Auth", "A > B > C", "https://example.com/detail/1", "https://example.com/search?q=x", "123")]


def test_fetch_search_results_request_error(monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore(verbose=False)

    def raise_get(*args, **kwargs):
        raise RuntimeError("boom")

    monkeypatch.setattr(store.session, "get", raise_get)
    assert store._fetch_search_results("https://example.com/search?q=x") == []
