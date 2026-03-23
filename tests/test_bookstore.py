import unittest, sys, os
from pathlib import Path
from unittest.mock import patch, MagicMock
from bs4 import BeautifulSoup
import backend.bookstore as bookstore
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from backend.bookstore import AbstractBookstore, Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, NaverSeriesBookstore, MunpiaBookstore


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

from backend.bookstore import Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, NaverSeriesBookstore, MunpiaBookstore


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


class DummyNoIsbnBookstore(DummyBookstore):
    SUPPORTS_ISBN_SEARCH = False


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

    cached = {"https://example.com/detail/1": "<html><h1>Title1</h1><span class='author'>Auth</span><p class='category'>A > B > C > D</p><p class='isbn'>123</p></html>", "https://example.com/detail/2": "   "}
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


def test_search_isbn_not_supported(monkeypatch: pytest.MonkeyPatch):
    store = DummyNoIsbnBookstore(verbose=False)
    monkeypatch.setattr(store, "search_by_keyword", lambda keyword: [("t", "a", "c", "u", "s", "i")])
    results, keyword, method = store.search(isbn="123", title="title", author="author")
    assert method == "title_author"
    assert keyword == "author title"
    assert results


def test_save_and_load_html_cache(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore(verbose=False)
    monkeypatch.setattr("tempfile.gettempdir", lambda: str(tmp_path))
    url = "https://example.com/detail/1"
    html = "<html>ok</html>"
    store._save_html_to_tmp(html, url)
    loaded = store._load_html_from_tmp(url)
    assert loaded == html


class BaseOnlyBookstore(AbstractBookstore):
    BASE_URL = "https://base.example.com"

    def build_search_url(self, keyword: str) -> str:
        return f"{self.BASE_URL}/search?q={keyword}"

    def extract_search_links(self, soup: BeautifulSoup):
        return []

    def extract_book_info(self, soup: BeautifulSoup):
        return {}


def test_base_build_isbn_search_and_empty_search(monkeypatch: pytest.MonkeyPatch):
    store = BaseOnlyBookstore(verbose=True)
    assert store.build_isbn_search_url("123") == store.build_search_url("123")
    results, keyword, method = store.search()
    assert results == []
    assert keyword == ""
    assert method == "unknown"
    assert store.search_by_isbn("123") == []


def test_base_class_abstract_methods_are_callable():
    store = BaseOnlyBookstore(verbose=False)
    assert AbstractBookstore.build_search_url(store, "x") is None
    assert AbstractBookstore.extract_search_links(store, BeautifulSoup("<html></html>", "html.parser")) is None
    assert AbstractBookstore.extract_book_info(store, BeautifulSoup("<html></html>", "html.parser")) is None


def test_fetch_search_results_threaded_cache_miss(monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore(verbose=True)
    store.MAX_RESULTS = 2

    class SearchResp:
        def __init__(self, text: str):
            self.text = text
            self.encoding = "utf-8"

    def fake_search_get(*args, **kwargs):
        return SearchResp("<html></html>")

    monkeypatch.setattr(store.session, "get", fake_search_get)
    monkeypatch.setattr(store, "extract_search_links", lambda soup: ["https://example.com/detail/1", "https://example.com/detail/2"])
    monkeypatch.setattr(store, "_load_html_from_tmp", lambda url: None)

    saved = {"count": 0}

    def fake_save(html: str, url: str):
        saved["count"] += 1

    monkeypatch.setattr(store, "_save_html_to_tmp", fake_save)

    class DetailResp:
        def __init__(self, text: str):
            self.text = text
            self.encoding = "utf-8"

    class DetailSession:
        def __init__(self):
            self.headers = {}
            self.cookies = {}

        def get(self, url, timeout=10, verify=True):
            if url.endswith("/1"):
                return DetailResp("<html><h1>T</h1><span class='author'>A</span><p class='category'>A > B > C > D</p><p class='isbn'>I</p></html>")
            raise RuntimeError("boom")

    monkeypatch.setattr(bookstore.requests, "Session", DetailSession)

    results = store._fetch_search_results("https://example.com/search?q=x")
    assert results == [("T", "A", "A > B > C", "https://example.com/detail/1", "https://example.com/search?q=x", "I")]
    assert saved["count"] == 1


def test_fetch_search_results_empty_links_and_blank_html(monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstore(verbose=True)

    class Resp:
        def __init__(self, text: str):
            self.text = text
            self.encoding = "utf-8"

    monkeypatch.setattr(store.session, "get", lambda *a, **k: Resp("<html></html>"))
    monkeypatch.setattr(store, "extract_search_links", lambda soup: [])
    assert store._fetch_search_results("https://example.com/search?q=x") == []

    monkeypatch.setattr(store, "extract_search_links", lambda soup: ["https://example.com/detail/1"])
    monkeypatch.setattr(store, "_load_html_from_tmp", lambda url: "   ")
    assert store._fetch_search_results("https://example.com/search?q=x") == []


def test_save_and_load_html_cache_errors(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    store = DummyBookstore(verbose=True)
    monkeypatch.setattr("tempfile.gettempdir", lambda: str(tmp_path))

    def bad_open(*args, **kwargs):
        raise OSError("nope")

    monkeypatch.setattr("builtins.open", bad_open)
    store._save_html_to_tmp("<html></html>", "https://example.com/detail/1")

    def bad_exists(*args, **kwargs):
        raise OSError("nope")

    monkeypatch.setattr("os.path.exists", bad_exists)
    assert store._load_html_from_tmp("https://example.com/detail/1") is None


def test_yes24_primary_links_and_info_extraction():
    store = Yes24Bookstore(verbose=True)
    html = """
    <html>
      <ul id="yesSchList">
        <li><div class="itemUnit"><div class="item_info"><div class="info_row info_name">
          <a class="gd_name" href="/product/goods/999">Book</a>
        </div></div></div></li>
      </ul>
      <h2 class="gd_name">Yes Title</h2>
      <span class="gd_auth"><a>Yes Author</a></span>
      <div>관련분류 <a href="/product/category/display/1">A</a> <a href="/product/category/display/2">B</a></div>
      <div>ISBN13 1234567890123</div>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://www.yes24.com/product/goods/999"]
    info = store.extract_book_info(soup)
    assert info["title"] == "Yes Title"
    assert info["author"] == "Yes Author"
    assert info["category"] == "A > B"
    assert info["isbn"] == "1234567890123"
    assert "query=hello" in store.build_search_url("hello")
    assert "query=999" in store.build_isbn_search_url("999")


def test_yes24_author_text_and_isbn10():
    store = Yes24Bookstore(verbose=False)
    html = """
    <html>
      <h2 class="gd_name">Yes Title</h2>
      <span class="gd_auth">Yes Author</span>
      <div>ISBN10 1234567890</div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "Yes Author"
    assert info["isbn"] == "1234567890"


def test_yes24_category_and_isbn_exception_paths():
    store = Yes24Bookstore(verbose=False)

    class BadSoup:
        def find_all(self, *args, **kwargs):
            raise RuntimeError("boom")

    class BadText:
        def get_text(self, *args, **kwargs):
            raise RuntimeError("boom")

    assert store._extract_yes24_category(BadSoup()) == ""
    assert store._extract_yes24_isbn(BadText()) == ""


def test_aladin_extract_links_and_isbn_paths():
    store = AladinBookstore(verbose=True)
    html = """
    <html>
      <div id="Search3_Result">
        <a href="/shop/wproduct.aspx?ItemId=1">A</a>
        <a href="/shop/wproduct.aspx?ItemId=1">A</a>
        <a href="/shop/wproduct.aspx?ItemId=2_CommentReview">R</a>
        <a href="/shop/wproduct.aspx?ItemId=3">B</a>
      </div>
      <div id="Ere_prod_allwrap">
        <div class="Ere_prod_mconts_R">
          <div class="conts_info_list1">
            <ul><li>ISBN 9781234567890</li></ul>
          </div>
        </div>
      </div>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=1", "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=3"]
    assert store._extract_aladin_isbn(soup) == "9781234567890"


def test_aladin_strip_trailing_number_and_search_order(monkeypatch: pytest.MonkeyPatch):
    store = AladinBookstore(verbose=False)
    assert store._strip_trailing_number("마왕의 딸 3") == "마왕의 딸"

    def fake_fetch(url: str):
        return [("T", "A", "C", url, url, "")]

    monkeypatch.setattr(store, "_fetch_search_results", fake_fetch)
    results, keyword, method = store.search(isbn="", title="마왕의 딸 3", author="작가")
    assert method == "title_author"
    assert keyword == "작가 마왕의 딸"
    assert results


def test_ridibooks_extract_fallbacks_meta_and_author_link():
    store = RidibooksBookstore(verbose=False)
    html = """
    <html>
      <meta name="title" content="Meta Title" />
      <a href="/author/1">Link Author</a>
      <meta name="description" content="저자: Desc Author, 기타" />
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "Meta Title"
    assert info["author"] == "Link Author"


def test_ridibooks_header_typo_and_og_author():
    store = RidibooksBookstore(verbose=False)
    html = """
    <html>
      <div id="iSLANDS__Header"><ul><li><li>저자 작가</li></li></ul></div>
      <meta property="og:author" content="OG Author" />
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "작가"


def test_naver_shopping_extract_fallback_author_and_title():
    store = NaverShoppingBookstore(verbose=False)
    html = """
    <html>
      <div class="bookTitle_book_name__x">Title</div>
      <div class="bookTitle_info_content__y">Author</div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "Title"
    assert info["author"] == "Author"


def test_munpia_meta_description_author():
    store = MunpiaBookstore(verbose=False)
    html = """
    <html>
      <meta property="og:title" content="T" />
      <meta property="og:description" content="홍길동 - 소개" />
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "홍길동"


def test_aladin_isbn_fallback_and_exception():
    store = AladinBookstore(verbose=False)
    html = """
    <html>
      <div id="Ere_prod_allwrap">
        <div class="Ere_prod_middlewrap">
          <div class="Ere_prod_mconts_R">
            <ul><li>기타 ISBN 1234567890123</li></ul>
          </div>
        </div>
      </div>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    assert store._extract_aladin_isbn(soup) == "1234567890123"

    class BadSoup:
        def select(self, *args, **kwargs):
            raise RuntimeError("boom")

    assert store._extract_aladin_isbn(BadSoup()) == ""


def test_ridibooks_search_empty_and_exception(monkeypatch: pytest.MonkeyPatch):
    store = RidibooksBookstore(verbose=True)

    class Resp:
        status_code = 200

        def json(self):
            return {"books": []}

    monkeypatch.setattr(store.session, "get", lambda *a, **k: Resp())
    assert store.search_by_keyword("k") == []

    def raise_get(*args, **kwargs):
        raise RuntimeError("boom")

    monkeypatch.setattr(store.session, "get", raise_get)
    assert store.search_by_keyword("k") == []


def test_ridibooks_search_success_and_extract():
    store = RidibooksBookstore(verbose=True)

    class Resp:
        status_code = 200

        def json(self):
            return {
                "books": [
                    {"b_id": "1", "title": "T1", "author": "A1", "parent_category_name": "P1", "category_name": "C1", "parent_category_name2": "P2", "category_name2": "C2"},
                    {"b_id": "2", "title": "T2", "author": "A2", "parent_category_name": "", "category_name": "C3", "parent_category_name2": "", "category_name2": "C4"},
                ]
            }

    store.session.get = lambda *a, **k: Resp()
    results = store.search_by_keyword("keyword")
    assert results == [("T1", "A1", "P1 > C1 || P2 > C2", "https://ridibooks.com/books/1", store.build_search_url("keyword"), ""), ("T2", "A2", "C3 || C4", "https://ridibooks.com/books/2", store.build_search_url("keyword"), "")]

    html = """
    <html>
      <meta property="og:title" content="R Title" />
      <div id="ISLANDS__Header"><ul><li><li>저자 홍길동</li></li></ul></div>
      <section id="books_contents"><section class="detail_body"><ul>
        <li><a href="/category/1">A</a><a href="/category/2">B</a></li>
      </ul></section></section>
      <div>ISBN</div><div>1234567890123</div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["title"] == "R Title"
    assert info["author"] == "홍길동"
    assert info["category"] == "A > B"
    assert info["isbn"] == "1234567890123"


def test_ridibooks_isbn_exception():
    store = RidibooksBookstore(verbose=False)

    class BadSoup:
        def find_all(self, *args, **kwargs):
            raise RuntimeError("boom")

    assert store._extract_ridi_isbn(BadSoup()) == ""


def test_naver_shopping_links_and_extract():
    store = NaverShoppingBookstore(verbose=True)
    data = {"props": {"pageProps": {"dehydratedState": {"queries": [{"queryKey": ["SearchAll"], "state": {"data": {"SearchAll": {"bookSasResult": {"itemList": [{"id": "1"}, {"id": "2"}]}}}}}]}}}}
    html = f"<html><script>{json.dumps(data)}</script></html>"
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://search.shopping.naver.com/book/catalog/1", "https://search.shopping.naver.com/book/catalog/2"]

    detail_html = """
    <html>
      <h2 class="bookTitle_book_name__abc">N Title</h2>
      <div class="bookTitle_info_title__x">저자</div>
      <div class="bookTitle_info_content__y">N Author</div>
      <div class="bookCatalogTop_breadcrumb__z">A > B</div>
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(detail_html, "html.parser"))
    assert info["title"] == "N Title"
    assert info["author"] == "N Author"
    assert info["category"] == "A > B"


def test_naver_shopping_items_not_list():
    store = NaverShoppingBookstore(verbose=False)
    data = {"props": {"pageProps": {"dehydratedState": {"queries": [{"queryKey": ["SearchAll"], "state": {"data": {"SearchAll": {"bookSasResult": {"itemList": {}}}}}}]}}}}
    html = f"<html><script>{json.dumps(data)}</script></html>"
    soup = BeautifulSoup(html, "html.parser")
    assert store.extract_search_links(soup) == []


def test_munpia_links_and_meta_desc():
    store = MunpiaBookstore(verbose=True)
    html = """
    <html>
      <div id="SEARCH-BOX" class="section2">
        <div class="ebook_lists">
          <div class="article_wrap">
            <div class="article">
              <dl class="detail"><dt><a href="/view/1">A</a></dt></dl>
              <dl class="detail"><dt><a>Missing</a></dt></dl>
              <dl class="detail"><dt><a href="https://novel.munpia.com/view/2">B</a></dt></dl>
            </div>
          </div>
        </div>
      </div>
      <meta property="og:description" content="홍길동 - 설명" />
      <p class="meta-path">A > B</p>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://novel.munpia.com/view/1", "https://novel.munpia.com/view/2"]
    info = store.extract_book_info(soup)
    assert info["author"] == "홍길동"
    assert info["category"] == "A > B"
    assert store.build_search_url("a b").endswith("a%20b/order/search_result")


def test_naver_series_links_and_authors():
    store = NaverSeriesBookstore(verbose=True)
    html = """
    <html>
      <title>Series Title</title>
      <ul class="lst_list">
        <li><a class="N=a:nov.title" href="/novel/1">A</a></li>
        <li><a class="N=a:com.title" href="/comic/2">B</a></li>
      </ul>
      <div id="_otherProductByPerson"><strong>작가</strong><strong>홍길동</strong></div>
      <div id="content"><ul class="end_info"><li class="info_lst"><ul><li><span><a>장르</a></span></li></ul></li></ul></div>
    </html>
    """
    soup = BeautifulSoup(html, "html.parser")
    links = store.extract_search_links(soup)
    assert links == ["https://series.naver.com/novel/1", "https://series.naver.com/comic/2"]
    info = store.extract_book_info(soup)
    assert info["title"] == "Series Title"
    assert info["author"] == "홍길동"
    assert info["category"] == "장르"
    assert store.build_search_url("hello").endswith("q=hello")


def test_naver_series_meta_og_fallback():
    store = NaverSeriesBookstore(verbose=False)
    html = """
    <html>
      <meta property="og:description" content="작가: 김작가」" />
    </html>
    """
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "김작가"


# ---- coverage: uncovered lines ----


def test_yes24_extract_search_links_fallback_verbose():
    """Line 238: verbose fallback logging when hierarchical selector fails"""
    store = Yes24Bookstore(verbose=True)
    html = '<html><a class="gd_name" href="/product/goods/12345">Book</a></html>'
    links = store.extract_search_links(BeautifulSoup(html, "html.parser"))
    assert len(links) == 1
    assert "/product/goods/12345" in links[0]


def test_yes24_extract_book_info_exception():
    """Lines 287-288: exception during book info extraction"""
    store = Yes24Bookstore(verbose=False)
    html = "<html><h2 class='gd_name'>Title</h2></html>"
    soup = BeautifulSoup(html, "html.parser")

    original_find = soup.find

    def raise_on_author(*args, **kwargs):
        if args and args[0] == "span":
            raise RuntimeError("parse error")
        return original_find(*args, **kwargs)

    soup.find = raise_on_author
    info = store.extract_book_info(soup)
    # Should not raise, returns partial info
    assert isinstance(info, dict)


def test_aladin_build_isbn_search_url():
    """Line 383: AladinBookstore.build_isbn_search_url"""
    store = AladinBookstore(verbose=False)
    url = store.build_isbn_search_url("9781234567890")
    assert "9781234567890" in url
    assert "SearchWord" in url


def test_aladin_extract_search_links_break_at_max():
    """Line 394: break when len(links) >= MAX_RESULTS"""
    store = AladinBookstore(verbose=False)
    # Generate HTML with many results (MAX_RESULTS is 2)
    items = ""
    for i in range(5):
        items += f'<a href="/shop/wproduct.aspx?ItemId={i}">Book {i}</a>'
    html = f'<html><div id="Search3_Result">{items}</div></html>'
    links = store.extract_search_links(BeautifulSoup(html, "html.parser"))
    assert len(links) <= store.MAX_RESULTS


def test_aladin_extract_isbn():
    """Line 442: Aladin ISBN extraction"""
    store = AladinBookstore(verbose=False)
    html = """<html>
    <title>Title | Author | Aladin</title>
    <div id="Ere_prod_allwrap">
      <div class="Ere_prod_mconts_R">
        <div class="conts_info_list1">
          <ul><li>ISBN13 : 9781234567890</li></ul>
        </div>
      </div>
    </div>
    </html>"""
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["isbn"] == "9781234567890"


def test_ridibooks_api_failure_verbose():
    """Line 516: RIDI API response failure with verbose"""
    store = RidibooksBookstore(verbose=True)

    class Resp:
        status_code = 500
        text = "error"

    store.session.get = lambda *a, **k: Resp()
    results = store.search_by_keyword("test")
    assert results == []


def test_ridibooks_extract_search_links():
    """Line 564: RidibooksBookstore.extract_search_links returns []"""
    store = RidibooksBookstore(verbose=False)
    html = "<html><body>nothing</body></html>"
    links = store.extract_search_links(BeautifulSoup(html, "html.parser"))
    assert links == []


def test_ridibooks_author_meta_fallback():
    """Lines 600-602: Ridibooks meta author fallback"""
    store = RidibooksBookstore(verbose=False)
    html = """<html>
    <meta property="og:title" content="My Book"/>
    <meta name="author" content="Meta Author"/>
    </html>"""
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "Meta Author"


def test_ridibooks_author_description_fallback():
    """Lines 605-610: Ridibooks meta description author extraction"""
    store = RidibooksBookstore(verbose=False)
    html = """<html>
    <meta property="og:title" content="My Book"/>
    <meta property="og:description" content="저자: 설명작가, 기타정보"/>
    </html>"""
    info = store.extract_book_info(BeautifulSoup(html, "html.parser"))
    assert info["author"] == "설명작가"


def test_naver_shopping_build_search_url():
    """Lines 631-632: NaverShoppingBookstore.build_search_url"""
    store = NaverShoppingBookstore(verbose=False)
    url = store.build_search_url("테스트 검색")
    assert "search.shopping.naver.com" in url
    assert "query=" in url


def test_naver_shopping_json_not_found_verbose():
    """Line 646: NaverShopping JSON script not found with verbose"""
    store = NaverShoppingBookstore(verbose=True)
    html = "<html><body>no scripts here</body></html>"
    links = store.extract_search_links(BeautifulSoup(html, "html.parser"))
    assert links == []


def test_naver_shopping_json_parse_error_verbose():
    """Lines 651-654: NaverShopping JSON parse error with verbose"""
    store = NaverShoppingBookstore(verbose=True)
    html = '<html><script>{invalid json "SearchAll"</script></html>'
    links = store.extract_search_links(BeautifulSoup(html, "html.parser"))
    assert links == []


def test_naver_shopping_extract_book_info_exception():
    """Lines 708-709: NaverShopping extract_book_info exception"""
    store = NaverShoppingBookstore(verbose=False)
    html = "<html><body>normal</body></html>"
    soup = BeautifulSoup(html, "html.parser")

    original_find = soup.find

    def raise_on_h2(*args, **kwargs):
        if args and args[0] == "h2":
            raise RuntimeError("parse error")
        return original_find(*args, **kwargs)

    soup.find = raise_on_h2
    info = store.extract_book_info(soup)
    assert isinstance(info, dict)


import types


from backend.bookstore import AbstractBookstore


class DummyBookstoreUnit(AbstractBookstore):
    BASE_URL = "http://example.com"

    def build_search_url(self, keyword: str) -> str:
        return f"{self.BASE_URL}/search?q={keyword}"

    def extract_search_links(self, soup):
        return [f"{self.BASE_URL}/detail/1", f"{self.BASE_URL}/detail/2"]

    def extract_book_info(self, soup):
        return {"title": "T", "author": "A", "category": "A > B > C > D", "isbn": "I"}


class FakeResponse:
    def __init__(self, text):
        self.text = text
        self.encoding = "utf-8"


def test_truncate_title():
    bs = DummyBookstoreUnit()
    assert bs._truncate_title("A - B") == "A"
    assert bs._truncate_title("A－B") == "A"
    assert bs._truncate_title("A: B") == "A: B"
    assert bs._truncate_title("A：B") == "A"


def test_search_prefers_isbn_when_supported(monkeypatch):
    bs = DummyBookstoreUnit()
    bs.SUPPORTS_ISBN_SEARCH = True
    called = []

    def fake_fetch(url):
        called.append(url)
        return [("t", "a", "c", "d", "s", "i")]

    monkeypatch.setattr(bs, "_fetch_search_results", fake_fetch)
    results, keyword, method = bs.search(isbn="123", title="t", author="a")
    assert results
    assert keyword == "123"
    assert method == "isbn"
    assert called and "123" in called[0]


def test_search_title_author_order(monkeypatch):
    bs = DummyBookstoreUnit()
    bs.SUPPORTS_ISBN_SEARCH = False
    called = []

    def fake_fetch(url):
        called.append(url)
        return [("t", "a", "c", "d", "s", "i")]

    monkeypatch.setattr(bs, "_fetch_search_results", fake_fetch)
    results, keyword, method = bs.search(title="T", author="A")
    assert results
    assert keyword == "T A"
    assert method == "title_author"

    bs.AUTHOR_FIRST_SEARCH = True
    results, keyword, method = bs.search(title="T", author="A")
    assert keyword == "A T"
    assert method == "title_author"


def test_search_fallbacks(monkeypatch):
    bs = DummyBookstoreUnit()
    monkeypatch.setattr(bs, "_fetch_search_results", lambda url: [])

    results, keyword, method = bs.search(title="T", author="A")
    assert results == []
    assert keyword == "T A"
    assert method == "title_author"

    results, keyword, method = bs.search(title="T")
    assert results == []
    assert keyword == "T"
    assert method == "title"

    bs.SUPPORTS_ISBN_SEARCH = True
    results, keyword, method = bs.search(isbn="123")
    assert results == []
    assert keyword == "123"
    assert method == "isbn"


def test_fetch_search_results_uses_cached_html(monkeypatch):
    bs = DummyBookstoreUnit()

    monkeypatch.setattr(bs.session, "get", lambda *args, **kwargs: FakeResponse("<html></html>"))

    monkeypatch.setattr(bs, "_load_html_from_tmp", lambda url: "<html></html>")
    results = bs._fetch_search_results(bs.build_search_url("k"))
    assert len(results) == 2
    assert results[0][2] == "A > B > C"


def test_save_and_load_html_from_tmp():
    bs = DummyBookstoreUnit()
    url = "http://example.com/detail/123"
    html = "<html>cached</html>"
    bs._save_html_to_tmp(html, url)
    loaded = bs._load_html_from_tmp(url)
    assert loaded == html


# ---- merged from test_bookstore_unit.py (above) ----


# ---- merged from test_bookstore_parsing.py ----
import tempfile

from backend.bookstore import AbstractBookstore, Yes24Bookstore, AladinBookstore, RidibooksBookstore, NaverShoppingBookstore, MunpiaBookstore, NaverSeriesBookstore


class DummyBookstoreParsing(AbstractBookstore):
    BASE_URL = "https://example.com"
    SUPPORTS_ISBN_SEARCH = True

    def __init__(self):
        super().__init__(verbose=False)
        self.seen: list[str] = []

    def build_search_url(self, keyword: str) -> str:
        self.seen.append(keyword)
        return f"{self.BASE_URL}/search?q={keyword}"

    def extract_search_links(self, soup: BeautifulSoup) -> list[str]:
        return [a["href"] for a in soup.find_all("a", href=True)]

    def extract_book_info(self, soup: BeautifulSoup):
        title = soup.find("h1")
        author = soup.find("span", class_="author")
        category = soup.find("div", class_="cat")
        isbn = soup.find("div", class_="isbn")
        return {"title": title.get_text(strip=True) if title else "", "author": author.get_text(strip=True) if author else "", "category": category.get_text(strip=True) if category else "", "isbn": isbn.get_text(strip=True) if isbn else ""}


def test_truncate_title_separators():
    assert AbstractBookstore._truncate_title("A - B") == "A"
    assert AbstractBookstore._truncate_title("A－B") == "A"
    assert AbstractBookstore._truncate_title("A：B") == "A"
    assert AbstractBookstore._truncate_title("A-B") == "A"


def test_search_prefers_isbn_then_title_author_then_title():
    store = DummyBookstoreParsing()
    store.search_by_isbn = lambda isbn: [("t", "a", "", "", "", isbn)]  # type: ignore[method-assign]
    results, keyword, method = store.search(isbn="123", title="Title", author="Auth")
    assert results
    assert keyword == "123"
    assert method == "isbn"

    store2 = DummyBookstoreParsing()
    store2.SUPPORTS_ISBN_SEARCH = False
    store2.search_by_keyword = lambda keyword: [("t", "a", "", "", "", "")]  # type: ignore[method-assign]
    results, keyword, method = store2.search(title="Title", author="Auth")
    assert results
    assert method == "title_author"
    assert keyword == "Title Auth"

    store3 = DummyBookstoreParsing()
    store3.SUPPORTS_ISBN_SEARCH = False
    store3.search_by_keyword = lambda keyword: [("t", "a", "", "", "", "")]  # type: ignore[method-assign]
    results, keyword, method = store3.search(title="Title")
    assert results
    assert method == "title"
    assert keyword == "Title"

    store4 = DummyBookstoreParsing()
    store4.SUPPORTS_ISBN_SEARCH = False
    store4.search_by_keyword = lambda keyword: []  # type: ignore[method-assign]
    results, keyword, method = store4.search(title="Title", author="Auth")
    assert results == []
    assert method == "title_author"
    assert keyword == "Title Auth"


def test_fetch_search_results_uses_cached_html_parsing(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    store = DummyBookstoreParsing()

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
    store = DummyBookstoreParsing()
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
    assert links == ["https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=111", "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=222"]
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
                return {"books": [{"b_id": "1", "title": "T", "author": "A", "parent_category_name": "P", "category_name": "C", "parent_category_name2": "P2", "category_name2": "C2"}]}

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
    data = {"props": {"pageProps": {"dehydratedState": {"queries": [{"queryKey": ["SearchAll", "book"], "state": {"data": {"SearchAll": {"bookSasResult": {"itemList": [{"id": "123"}]}}}}}]}}}}
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
