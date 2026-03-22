import types


from backend.bookstore import AbstractBookstore


class DummyBookstore(AbstractBookstore):
    BASE_URL = "http://example.com"

    def build_search_url(self, keyword: str) -> str:
        return f"{self.BASE_URL}/search?q={keyword}"

    def extract_search_links(self, soup):
        return [
            f"{self.BASE_URL}/detail/1",
            f"{self.BASE_URL}/detail/2",
        ]

    def extract_book_info(self, soup):
        return {
            "title": "T",
            "author": "A",
            "category": "A > B > C > D",
            "isbn": "I",
        }


class FakeResponse:
    def __init__(self, text):
        self.text = text
        self.encoding = "utf-8"


def test_truncate_title():
    bs = DummyBookstore()
    assert bs._truncate_title("A - B") == "A"
    assert bs._truncate_title("A－B") == "A"
    assert bs._truncate_title("A: B") == "A: B"
    assert bs._truncate_title("A：B") == "A"


def test_search_prefers_isbn_when_supported(monkeypatch):
    bs = DummyBookstore()
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
    bs = DummyBookstore()
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
    bs = DummyBookstore()
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
    bs = DummyBookstore()

    monkeypatch.setattr(
        bs.session,
        "get",
        lambda *args, **kwargs: FakeResponse("<html></html>"),
    )

    monkeypatch.setattr(bs, "_load_html_from_tmp", lambda url: "<html></html>")
    results = bs._fetch_search_results(bs.build_search_url("k"))
    assert len(results) == 2
    assert results[0][2] == "A > B > C"


def test_save_and_load_html_from_tmp():
    bs = DummyBookstore()
    url = "http://example.com/detail/123"
    html = "<html>cached</html>"
    bs._save_html_to_tmp(html, url)
    loaded = bs._load_html_from_tmp(url)
    assert loaded == html
