#!/usr/bin/env python


import unittest
import logging.config
from pathlib import Path
from typing import Dict, Any
from datetime import datetime, timedelta
from backend.book import Book


logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TestBook(unittest.TestCase):
    def test_init(self):
        book_id = 3
        info: Dict[str, Any] = {
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "summary": "summary1",
            "file_path": Book.path_prefix / "category1" / "[anonymous] any book.epub",
            "file_type": "file_type1",
            "file_size": 100,
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        book = Book(book_id, info)
        assert book
        assert book.category == "category1"
        assert book.title == "title1"
        assert book.author == "author1"
        assert book.summary == "summary1"
        assert book.file_path == Book.path_prefix / "category1" / "[anonymous] any book.epub"
        assert book.file_type == "file_type1"
        assert book.file_size == 100

    def test_dict(self):
        book_id = 3
        info: Dict[str, Any] = {
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "summary": "summary1",
            "file_path": Book.path_prefix / "category1" / "[anonymous] any book.epub",
            "file_type": "file_type1",
            "file_size": 100,
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        book = Book(book_id, info)
        assert book.dict() == {
            "book_id": 3,
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "summary": "summary1",
            "file_path": "category1/[anonymous] any book.epub",
            "file_type": "file_type1",
            "file_size": 100,
            "updated_time": "2021-01-01T00:00:00.000000",
        }

    def test_json(self):
        book_id = 3
        info: Dict[str, Any] = {
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "file_path": Book.path_prefix / "category1" / "[anonymous] any book.epub",
            "file_type": "file_type1",
            "file_size": 100,
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        book = Book(book_id, info)
        self.assertIn('"book_id": 3', book.json())
        self.assertIn('"title": "title1"', book.json())

    def test_str(self):
        book_id = 3
        info: Dict[str, Any] = {
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "file_path": Book.path_prefix / "category1" / "[anonymous] any book.epub",
            "file_type": "file_type1",
            "file_size": 100,
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        book = Book(book_id, info)
        self.assertIn("title: title1", str(book))
        self.assertIn("author: author1", str(book))

    def test_init_with_missing_key(self):
        with self.assertRaises(KeyError):
            Book(4, {"category": "c"})

    def test_init_with_invalid_date_format(self):
        info: Dict[str, Any] = {
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "file_path": Book.path_prefix / "category1" / "file.epub",
            "file_type": "epub",
            "file_size": 100,
            "updated_time": "invalid-date-format",
        }
        book = Book(5, info)
        self.assertTrue(datetime.now() - book.updated_time < timedelta(seconds=1))


if __name__ == "__main__":
    unittest.main()