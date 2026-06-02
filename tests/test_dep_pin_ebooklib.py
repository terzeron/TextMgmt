#!/usr/bin/env python
"""ebooklib dependency pinning.

utils 사용처:
- utils/loader.py:20-21,133,145  EPUB 읽기 및 본문 추출
    import ebooklib
    from ebooklib import epub
    book = epub.read_epub(file_path)
    for doc in book.get_items_of_type(ebooklib.ITEM_DOCUMENT):
        ... doc.get_body_content()

박제 API:
- ebooklib.ITEM_DOCUMENT 상수
- epub.read_epub(path) -> book
- book.get_items_of_type(ITEM_DOCUMENT) -> 반복 가능
- item.get_body_content() -> bytes
"""

import tempfile
import unittest
from pathlib import Path


class TestEbooklibDependencyPinning(unittest.TestCase):
    def _make_epub(self, path: Path) -> None:
        from ebooklib import epub

        book = epub.EpubBook()
        book.set_identifier("pin-test-id")
        book.set_title("Pin Test Book")
        book.set_language("ko")
        book.add_author("Pin Author")

        chapter = epub.EpubHtml(title="Chapter 1", file_name="chap01.xhtml", lang="ko")
        chapter.content = b"<html><body><h1>Chapter 1</h1><p>Hello</p></body></html>"
        book.add_item(chapter)
        book.spine = ["nav", chapter]
        book.add_item(epub.EpubNav())
        book.add_item(epub.EpubNcx())
        epub.write_epub(str(path), book)

    def test_item_document_constant_exists(self):
        """loader.py: ebooklib.ITEM_DOCUMENT"""
        import ebooklib

        self.assertTrue(hasattr(ebooklib, "ITEM_DOCUMENT"))

    def test_read_epub_and_iterate_documents(self):
        """epub.read_epub() + get_items_of_type(ITEM_DOCUMENT) + get_body_content()"""
        import ebooklib
        from ebooklib import epub

        with tempfile.NamedTemporaryFile(suffix=".epub", delete=False) as f:
            epub_path = Path(f.name)
        try:
            self._make_epub(epub_path)
            book = epub.read_epub(str(epub_path))
            docs = list(book.get_items_of_type(ebooklib.ITEM_DOCUMENT))
            self.assertGreaterEqual(len(docs), 1)
            body = docs[0].get_body_content()
            self.assertIsInstance(body, bytes)
            self.assertGreater(len(body), 0)
        finally:
            epub_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
