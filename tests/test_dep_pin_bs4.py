#!/usr/bin/env python
"""bs4 (beautifulsoup4) dependency pinning.

backend 사용처:
- backend/book_manager.py: HTML 위생화(decompose, attrs 제거), EPUB 자원 추출
- backend/bookstore.py: 검색 결과/상세 페이지 파싱, isinstance(_, Tag) 분기

여기서 박제하는 API 표면:
- BeautifulSoup(markup, parser) — "html.parser", "lxml"
- Tag(별칭 _Tag) 클래스 import 및 isinstance 분기
- find / find_all / select
- get / get_text(strip=True) / attrs / .name
- find_all(True), find_all([태그명, ...])
- decompose, del tag.attrs[key], attrs.pop
- find_all(string=lambda)
- find_all("a", href=re.compile(...))
"""

import re
import unittest


HTML_BOOKSTORE = """
<html><body>
  <h2 class="gd_name">Test Title</h2>
  <span class="gd_auth"><a href="/author/1">Author Name</a></span>
  <ul id="ulCategory">
    <li><a href="/cat/1">Category 1</a></li>
    <li><a href="/cat/2">Category 2</a></li>
  </ul>
  <a class="gd_name" href="/book/1">Book One</a>
  <a class="gd_name" href="/book/2">Book Two</a>
</body></html>
"""

HTML_FOR_SANITIZE = """
<html>
  <head>
    <meta http-equiv="refresh" content="0; url=http://x" />
    <meta name="ok" content="ok" />
    <link rel="stylesheet" href="style.css" />
    <link rel="alternate" href="http://x" />
  </head>
  <body>
    <script>alert(1)</script>
    <iframe src="http://x"></iframe>
    <frame src="http://x"></frame>
    <object data="x"></object>
    <embed src="x" />
    <form action="x"></form>
    <base href="x" />
    <a href="http://x" onclick="x()">link</a>
    <img src="cover.jpg" />
    <image xlink:href="cover.svg" />
  </body>
</html>
"""


class TestBeautifulSoupImport(unittest.TestCase):
    """backend의 import 형태 박제"""

    def test_import_beautifulsoup(self):
        from bs4 import BeautifulSoup

        self.assertTrue(callable(BeautifulSoup))

    def test_import_tag_with_alias(self):
        """bookstore.py: from bs4 import BeautifulSoup, Tag as _Tag"""
        from bs4 import BeautifulSoup, Tag as _Tag

        self.assertTrue(callable(_Tag))
        soup = BeautifulSoup("<a>x</a>", "html.parser")
        a = soup.find("a")
        self.assertIsInstance(a, _Tag)

    def test_html_parser_available(self):
        from bs4 import BeautifulSoup

        soup = BeautifulSoup("<p>x</p>", "html.parser")
        self.assertIsNotNone(soup.find("p"))

    def test_lxml_parser_available(self):
        from bs4 import BeautifulSoup

        soup = BeautifulSoup("<p>x</p>", "lxml")
        self.assertIsNotNone(soup.find("p"))


class TestBookstoreParsingPatterns(unittest.TestCase):
    """backend/bookstore.py 파싱 패턴 박제"""

    def setUp(self):
        from bs4 import BeautifulSoup

        self.soup = BeautifulSoup(HTML_BOOKSTORE, "html.parser")

    def test_find_with_class(self):
        h2 = self.soup.find("h2", class_="gd_name")
        self.assertIsNotNone(h2)
        self.assertEqual(h2.get_text(strip=True), "Test Title")

    def test_find_all_with_class_and_href_true(self):
        """bookstore.py: soup.find_all("a", class_="gd_name", href=True)"""
        links = self.soup.find_all("a", class_="gd_name", href=True)
        self.assertEqual(len(links), 2)
        for a in links:
            self.assertTrue(a.get("href"))

    def test_select_css(self):
        """bookstore.py: soup.select("...")"""
        anchors = self.soup.select("#ulCategory a")
        self.assertEqual(len(anchors), 2)
        self.assertEqual(anchors[0]["href"], "/cat/1")

    def test_nested_find_with_tag_isinstance_check(self):
        """bookstore.py: a_tag가 _Tag 인스턴스인지 확인 후 .get_text() 호출"""
        from bs4 import Tag as _Tag

        span = self.soup.find("span", class_="gd_auth")
        a_tag = span.find("a")
        self.assertIsInstance(a_tag, _Tag)
        self.assertEqual(a_tag.get_text(strip=True), "Author Name")

    def test_get_text_strip_true(self):
        h2 = self.soup.find("h2", class_="gd_name")
        self.assertEqual(h2.get_text(strip=True), "Test Title")

    def test_get_attribute_via_get(self):
        a = self.soup.find("a", href="/cat/1")
        self.assertEqual(a.get("href"), "/cat/1")

    def test_get_attribute_default(self):
        a = self.soup.find("a")
        self.assertEqual(a.get("nonexistent", ""), "")

    def test_find_all_string_lambda(self):
        matches = self.soup.find_all(string=lambda t: t and "Title" in t)
        self.assertGreaterEqual(len(matches), 1)

    def test_find_all_with_regex_href(self):
        links = self.soup.find_all("a", href=re.compile(r"/cat/"))
        self.assertEqual(len(links), 2)


class TestBookManagerSanitizePatterns(unittest.TestCase):
    """backend/book_manager.py HTML 위생화 패턴 박제"""

    def setUp(self):
        from bs4 import BeautifulSoup

        self.soup = BeautifulSoup(HTML_FOR_SANITIZE, "html.parser")

    def test_find_all_with_list_of_tag_names(self):
        """book_manager.py: find_all(["script", "iframe", ...])"""
        tags = self.soup.find_all(["script", "iframe", "frame", "object", "embed", "form", "base"])
        self.assertGreaterEqual(len(tags), 7)

    def test_decompose_removes_element(self):
        script = self.soup.find("script")
        self.assertIsNotNone(script)
        script.decompose()
        self.assertIsNone(self.soup.find("script"))

    def test_find_all_meta_and_inspect_attr(self):
        """book_manager.py: meta.get('http-equiv')로 분기"""
        metas = self.soup.find_all("meta")
        with_http_equiv = [m for m in metas if m.get("http-equiv")]
        self.assertEqual(len(with_http_equiv), 1)

    def test_find_all_true_iterates_all_tags(self):
        """book_manager.py: for tag in soup.find_all(True)"""
        all_tags = self.soup.find_all(True)
        self.assertTrue(all(hasattr(t, "name") for t in all_tags))
        self.assertGreater(len(all_tags), 5)

    def test_attrs_is_dict_like(self):
        a = self.soup.find("a")
        self.assertIn("href", a.attrs)
        keys = list(a.attrs.keys())
        self.assertIn("href", keys)

    def test_del_tag_attrs_key(self):
        """book_manager.py: del tag.attrs[attr]"""
        a = self.soup.find("a")
        a.attrs["data-x"] = "y"
        del a.attrs["data-x"]
        self.assertNotIn("data-x", a.attrs)

    def test_attrs_pop(self):
        """book_manager.py: tag.attrs.pop(attr_name, None)"""
        a = self.soup.find("a")
        a.attrs["onclick"] = "evil()"
        a.attrs.pop("onclick", None)
        self.assertNotIn("onclick", a.attrs)
        a.attrs.pop("not-there", None)  # 기본값 None 동작 확인

    def test_get_rel_returns_list(self):
        """book_manager.py: tag.get('rel', []) 후 list comprehension"""
        link = self.soup.find("link", rel="stylesheet")
        rel = link.get("rel", [])
        self.assertIsInstance(rel, list)
        self.assertIn("stylesheet", [str(r).lower() for r in rel])

    def test_xlink_href_attribute(self):
        """book_manager.py: image.get('xlink:href') or image.get('href', '')"""
        image = self.soup.find("image")
        self.assertEqual(image.get("xlink:href"), "cover.svg")


class TestBeautifulSoupBytesInput(unittest.TestCase):
    """book_manager.py: BeautifulSoup(content, 'html.parser')에서 content는 bytes일 수 있음"""

    def test_parse_bytes_input(self):
        from bs4 import BeautifulSoup

        soup = BeautifulSoup(b"<html><body><img src='a.jpg'/></body></html>", "html.parser")
        self.assertIsNotNone(soup.find("img"))
        self.assertEqual(soup.find("img").get("src"), "a.jpg")


if __name__ == "__main__":
    unittest.main()
