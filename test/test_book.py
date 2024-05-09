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
        assert book
        assert book.category == "category1"
        assert book.title == "title1"
        assert book.author == "author1"
        assert book.file_path == Book.path_prefix / "category1" / "[anonymous] any book.epub"
        assert book.file_type == "file_type1"
        assert book.file_size == 100

    def test_dict(self):
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
        assert book.dict() == {
            "book_id": 3,
            "category": "category1",
            "title": "title1",
            "author": "author1",
            "file_path": "category1/[anonymous] any book.epub",
            "file_type": "file_type1",
            "file_size": 100,
            "updated_time": "2021-01-01T00:00:00.000000",
        }
