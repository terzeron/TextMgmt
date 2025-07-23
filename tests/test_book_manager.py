#!/usr/bin/env python


import unittest
from unittest.mock import MagicMock, patch
import logging.config
from pathlib import Path
from datetime import datetime
from typing import Tuple, Optional
from fastapi.responses import FileResponse
from backend.book import Book
from backend.book_manager import BookManager

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TestBookManager(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.bm = BookManager()
        self.bm.es_manager = MagicMock()
        self.category1 = "_epub"
        self.category2 = "_txt"
        self.book1 = Book(1, {"category": self.category1, "title": "test1", "author": "author1", "file_path": "test1.epub", "file_type": "epub", "file_size": 1, "summary": "summary1", "updated_time": "2021-01-01T00:00:00.000000"})
        self.book2 = Book(2, {"category": self.category2, "title": "test2", "author": "author2", "file_path": "test2.txt", "file_type": "txt", "file_size": 2, "summary": "summary2", "updated_time": "2021-01-01T00:00:00.000000"})

    def inspect_book_info(self, book: Book):
        assert isinstance(book, Book)
        assert isinstance(book.book_id, int)
        assert isinstance(book.category, str)
        assert isinstance(book.title, str)
        assert isinstance(book.author, str)
        assert isinstance(book.file_type, str)
        assert isinstance(book.file_path, Path)
        assert isinstance(book.file_size, int)
        assert isinstance(book.updated_time, datetime)

    async def test_get_categories(self):
        self.bm.es_manager.search_and_aggregate_by_category.return_value = [self.category1, self.category2]
        result, _ = await self.bm.get_categories()
        assert len(result) > 0
        assert isinstance(result[0], str)
        assert self.category1 in result

    async def test_get_books_in_category(self):
        self.bm.es_manager.search_by_category.return_value = [(1, self.book1.dict(), 1.0)]
        book_list, error = await self.bm.get_books_in_category(self.category1)
        assert book_list and len(book_list) > 0
        for book in book_list:
            assert book and not error
            self.inspect_book_info(book)

    async def test_get_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book1.dict()
        book, error = await self.bm.get_book(self.book1.book_id)
        assert book and not error
        self.inspect_book_info(book)
        assert book.book_id == self.book1.book_id

    async def test_search_by_keyword(self):
        self.bm.es_manager.search_by_keyword.return_value = [(1, self.book1.dict(), 1.0)]
        result, error = await self.bm.search_by_keyword("test")
        assert not error
        assert len(result) == 1
        self.inspect_book_info(result[0])

    async def test_search_similar_books(self):
        self.bm.es_manager.search_by_id.return_value = self.book1.dict()
        self.bm.es_manager.search_similar_docs.return_value = [(2, self.book2.dict(), 1.0)]
        result, error = await self.bm.search_similar_books(str(self.book1.book_id))
        assert not error
        assert len(result) == 1
        self.inspect_book_info(result[0])

    async def test_add_book(self):
        self.bm.es_manager.insert.return_value = [3]
        book_id, error = await self.bm.add_book({3: self.book1.dict()})
        assert not error
        assert book_id == 3

    async def test_update_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book1.dict()
        self.bm.es_manager.update.return_value = True
        with patch('pathlib.Path.rename'):
            result, error = await self.bm.update_book(self.book1.book_id, {"category": "new_cat", "title": "new_title", "author": "new_author", "file_type": "txt"})
        assert not error
        assert result == "Ok"

    async def test_get_book_not_found(self):
        self.bm.es_manager.search_by_id.return_value = None
        book, error = await self.bm.get_book(999)
        self.assertIsNone(book)
        self.assertIn("No book found", error)

    @patch('pathlib.Path.rename', side_effect=IOError("Permission denied"))
    async def test_update_book_io_error(self, mock_rename):
        self.bm.es_manager.search_by_id.return_value = self.book1.dict()
        result, error = await self.bm.update_book(self.book1.book_id, {"category": "new_cat", "title": "new_title", "author": "new_author", "file_type": "txt"})
        self.assertEqual(result, "Error")
        self.assertIn("can't move", error)

    async def test_delete_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book1.dict()
        self.bm.es_manager.delete.return_value = True
        with patch('pathlib.Path.unlink'):
            result, error = await self.bm.delete_book(self.book1.book_id)
        assert not error
        assert result == "Ok"


if __name__ == "__main__":
    unittest.main()
