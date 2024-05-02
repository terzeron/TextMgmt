#!/usr/bin/env python


import logging.config
import shutil
import unittest
from pathlib import Path
from fastapi.testclient import TestClient
from backend.main import app
from backend.book_manager import BookManager
from backend.book import Book
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)

client = TestClient(app)
bm = BookManager()


class TestBackend(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUpClass(cls) -> None:
        data = Loader.read_files(Book.path_prefix / "_epub", 1000)
        bm.es_manager.insert(data)

    async def asyncSetUp(self) -> None:
        epub_file_path = list(Book.path_prefix.glob("*/*.epub"))[0]
        temp_file_path = Book.path_prefix / epub_file_path.parent / ("temp_" + epub_file_path.name)
        shutil.copy(epub_file_path, temp_file_path)
        data = Loader.read_file(temp_file_path)

        book_id, error = await bm.add_book(data)
        assert book_id and not error

        book, error = await bm.get_book(book_id)
        assert book and not error
        self.book = book

    def tearDown(self) -> None:
        response = client.delete(f"/books/{self.book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": "Ok"}
        del self.book

    def test_move_file(self) -> None:
        doc = {
            "book_id": self.book.book_id,
            "category": self.book.category,
            "title": "renamed_" + self.book.title,
            "author": "anonymous_" + self.book.author,
            "file_path": self.book.category + "/renamed_" + self.book.title + "." + self.book.file_type,
            "file_type": self.book.file_type,
            "file_size": 100,
            "summary": "summary1",
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        response = client.put(f"/books/{self.book.book_id}", json=doc)
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": "Ok"}

    def test_delete_file(self):
        None

    def test_get_file_content(self):
        media_types = {
            ".txt": "text/plain",
            ".pdf": "application/pdf",
            ".epub": "application/epub+zip",
            ".doc": "application/msword",
            ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ".html": "text/html",
        }

        response = client.get(f"/download/{self.book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.content
        assert len(response.content) > 1024
        media_type = media_types.get(self.book.file_path.suffix, "application/octet-stream")
        assert response.headers["Content-Type"].split(";")[0] == media_type

    def test_get_book(self):
        response = client.get(f"/books/{self.book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.json() == {
            "status": "success",
            "result": self.book.dict()
        }

    def test_get_books_in_category(self):
        response = client.get(f"/categories/{self.book.category}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert self.book.book_id in [book["book_id"] for book in books]

    def test_get_categories(self):
        response = client.get("/categories")
        assert response
        assert response.status_code == 200
        assert response.json()["status"] == "success"
        assert self.book.category in response.json()["result"]

    def test_get_similar_book_list(self):
        response = client.get(f"/similar/{self.book.book_id}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert self.book.book_id in [book["book_id"] for book in books]

    def test_search_by_keyword(self):
        keyword = self.book.title + " " + " ".join(self.book.summary.split(" ")[:5])
        response = client.get(f"/search/{keyword}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert self.book.book_id in [book["book_id"] for book in books]


if __name__ == "__main__":
    unittest.main()
