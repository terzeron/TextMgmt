#!/usr/bin/env python


import unittest
import logging.config
from pathlib import Path
from typing import Dict, Any
from backend.book import Book


logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TestBook(unittest.TestCase):
    def test_init(self):
        book_id = 3
        info: Dict[str, Any] = {"category": "category1", "title": "title1", "author": "author1", "file_path": Book.path_prefix / "category1" / "[anonymous] any book.epub", "file_type": "file_type1", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

        book = Book(book_id, info)
        assert book
        assert book.category == "category1"
        assert book.title == "title1"
        assert book.author == "author1"
        assert book.file_path == Book.path_prefix / "category1" / "[anonymous] any book.epub"
        assert book.file_type == "file_type1"
        assert book.file_size == 100

    def test_dict(self):
        book_id = 3
        info: Dict[str, Any] = {"category": "category1", "title": "title1", "author": "author1", "file_path": Book.path_prefix / "category1" / "[anonymous] any book.epub", "file_type": "file_type1", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

        book = Book(book_id, info)
        assert book.dict() == {"book_id": 3, "category": "category1", "title": "title1", "author": "author1", "file_path": "category1/[anonymous] any book.epub", "file_type": "file_type1", "file_size": 100, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2021-01-01T00:00:00.000000", "score": 0.0}

    def test_json_and_str(self):
        book_id = 4
        info: Dict[str, Any] = {"category": "category1", "title": "title1", "author": "author1", "file_path": Book.path_prefix / "category1" / "book.txt", "file_type": "txt", "file_size": 10, "updated_time": "2021-01-01T00:00:00.000000"}
        book = Book(book_id, info)
        assert '"title": "title1"' in book.json()
        assert "category: category1" in str(book)

    def test_init_uses_optional_defaults_and_score(self):
        info: Dict[str, Any] = {
            "category": "category2",
            "title": "title2",
            "author": "author2",
            "file_path": "category2/book2.pdf",
            "file_type": "pdf",
            "file_size": 200,
            "updated_time": "2022-02-02T00:00:00.000000",
        }
        book = Book(5, info, score=12.5)
        assert book.line_count == 0
        assert book.page_count == 0
        assert book.isbn == ""
        assert book.summary == ""
        assert book.score == 12.5
        assert book.dict()["file_path"] == "category2/book2.pdf"


if __name__ == "__main__":
    unittest.main()


# ---- merged from test_book_env_guard.py ----


def test_book_requires_book_dir():
    import importlib, os, sys

    prev = os.environ.pop("TM_BOOK_DIR", None)
    try:
        if "backend.book" in sys.modules:
            del sys.modules["backend.book"]
        import pytest

        with pytest.raises(SystemExit):
            importlib.import_module("backend.book")
    finally:
        if prev is not None:
            os.environ["TM_BOOK_DIR"] = prev
