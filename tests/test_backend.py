#!/usr/bin/env python


import unittest
from unittest.mock import MagicMock, patch
from fastapi.testclient import TestClient
from backend.main import app, get_book_manager
from backend.book import Book
from backend.book_manager import BookManager


class TestBackend(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.bm = BookManager()
        self.bm.es_manager = MagicMock()
        
        def get_book_manager_override():
            return self.bm

        app.dependency_overrides[get_book_manager] = get_book_manager_override
        
        self.client = TestClient(app)
        self.book_dict = {
            "book_id": 1,
            "category": "test_category",
            "title": "test_title",
            "author": "test_author",
            "file_path": "a/b.epub",
            "file_type": "epub",
            "file_size": 123,
            "summary": "summary",
            "updated_time": "2021-01-01T00:00:00.000000",
        }

    def tearDown(self):
        app.dependency_overrides = {}

    async def test_get_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book_dict
        response = self.client.get("/books/1")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "success")
        self.bm.es_manager.search_by_id.assert_called_with(1)

    async def test_search_by_keyword(self):
        self.bm.es_manager.search_by_keyword.return_value = [(1, self.book_dict, 1.0)]
        response = self.client.get("/search/test_title")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "success")

    async def test_update_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book_dict
        self.bm.es_manager.update.return_value = True
        with patch('pathlib.Path.rename'):
            response = self.client.put("/books/1", json=self.book_dict)
            self.assertEqual(response.status_code, 200)
            self.assertEqual(response.json()["status"], "success")

    async def test_delete_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book_dict
        self.bm.es_manager.delete.return_value = True
        with patch('pathlib.Path.unlink'):
            response = self.client.delete("/books/1")
            self.assertEqual(response.status_code, 200)
            self.assertEqual(response.json()["status"], "success")
            
    async def test_get_categories(self):
        self.bm.es_manager.search_and_aggregate_by_category.return_value = ["cat1", "cat2"]
        response = self.client.get("/categories")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["result"], ["cat1", "cat2"])

    async def test_get_books_in_category(self):
        self.bm.es_manager.search_by_category.return_value = [(1, self.book_dict, 1.0)]
        response = self.client.get("/categories/test_category")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "success")

    async def test_download_book(self):
        self.bm.es_manager.search_by_id.return_value = self.book_dict
        with patch('fastapi.responses.FileResponse'):
            response = self.client.get("/download/1")
            self.assertEqual(response.status_code, 200)

    async def test_get_similar_books(self):
        self.bm.es_manager.search_by_id.return_value = self.book_dict
        self.bm.es_manager.search_similar_docs.return_value = [(1, self.book_dict, 1.0)]
        response = self.client.get("/similar/1")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.json()["status"], "success")


if __name__ == "__main__":
    unittest.main()
