#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Tuple, Dict, List, Union, Optional, Any
import chardet
from fastapi.responses import FileResponse
from backend.es_manager import ESManager
from backend.book import Book

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)


class BookManager:
    ROOT_DIRECTORY = "$$rootdir$$"
    MEDIA_TYPES = {
        ".txt": "text/plain",
        ".pdf": "application/pdf",
        ".epub": "application/epub+zip",
        ".doc": "application/msword",
        ".docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".html": "text/html",
        ".jpg": "image/jpeg",
        ".jpeg": "image/jpeg",
        ".png": "image/png",
        ".gif": "image/gif",
        ".webp": "image/webp",
        ".bmp": "image/bmp",
        ".tiff": "image/tiff",
        ".svg": "image/svg+xml",
    }

    def __init__(self) -> None:
        if "TM_WORK_DIR" not in os.environ:
            LOGGER.error("The environment variable TM_WORK_DIR is not set.")
            sys.exit(-1)

        self.path_prefix = Path(os.environ["TM_WORK_DIR"])
        LOGGER.debug(self.path_prefix)
        self.es_manager = ESManager()
        self.es_manager.create_index()

    def __del__(self) -> None:
        del self.es_manager

    async def get_categories(self) -> Tuple[List[str], Optional[str]]:
        LOGGER.debug("# get_categories()")
        categories = self.es_manager.search_and_aggregate_by_category()
        return categories, None

    async def get_books_in_category(self, category: str) -> Tuple[List[Book], Optional[str]]:
        doc_list = self.es_manager.search_by_category(category, max_result_count=50000)
        if doc_list and len(doc_list) > 0:
            return [Book(book_id, doc) for book_id, doc, _score in doc_list], None
        return [], f"No books found in '{category}'"

    async def get_book(self, book_id: int) -> Tuple[Optional[Book], Optional[str]]:
        LOGGER.debug("# get_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            return Book(book_id, doc), None
        return None, f"No book found by '{book_id}'"

    @staticmethod
    def determine_file_content_and_encoding(file_path: Path) -> str:
        LOGGER.debug("# determine_file_content_and_encoding(file_path='%s')", file_path)
        if file_path.suffix != ".txt":
            return "binary"

        encoding = "utf-8"
        with file_path.open("r") as infile:
            content = infile.read(1024 * 100)
            if content:
                encoding_metadata = chardet.detect(content.encode())
                if encoding_metadata["confidence"] > 0.99:
                    encoding = encoding_metadata["encoding"] if encoding_metadata["encoding"] else "utf-8"
        return encoding

    async def get_book_content(self, book_id: int) -> Union[str, FileResponse]:
        LOGGER.debug("# get_book_content(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        book = Book(book_id, doc)
        file_path = book.file_path
        if file_path.is_file():
            media_type = BookManager.MEDIA_TYPES.get(file_path.suffix, "application/octet-stream")
            return FileResponse(path=file_path, media_type=media_type)
        return ""

    async def search_by_keyword(self, keyword: str, max_result_count: int = sys.maxsize) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_by_keyword(keyword='%s', max_result_count=%d)", keyword, max_result_count)
        result_list = self.es_manager.search_by_keyword(keyword, max_result_count)
        if result_list and len(result_list) > 0:
            return [Book(book_id, doc) for book_id, doc, _score in result_list], None
        return [], "No books found"

    async def search_similar_books(self, book_id: int, max_result_count: int = sys.maxsize) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_similar_books(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)
        result_list = self.es_manager.search_similar_docs(doc["category"], doc["title"], doc["author"], doc["file_type"], doc["file_size"], doc["summary"][:3500], max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [Book(doc_id, doc) for doc_id, doc, _score in result_list], None
        return [], "No similar books found"

    async def add_book(self, data: Dict[int, Dict[str, Any]]) -> Tuple[Optional[int], Optional[str]]:
        LOGGER.debug("# add_book(data='%r')", data)
        doc_id_list = self.es_manager.insert(data)
        if doc_id_list and len(doc_id_list) == 1:
            return doc_id_list[0], None
        return None, f"can't add book '{data}' to ElasticSearch"

    async def update_book(self, book_id: int, new_category: str, new_title: str, new_author: str, new_path: Path, new_type: str) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# update_book(book_id=%d, new_category='%s', new_title='%s', new_author='%s', new_path='%r', new_file_type='%s')", book_id, new_category, new_title, new_author, new_path, new_type)
        # rename file
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            book = Book(book_id, doc)
            file_path = book.file_path
            new_path = file_path.parent.parent / new_category / (new_title + "." + new_type)
            try:
                file_path.rename(new_path)
            except IOError as e:
                return "Error", f"can't move '{file_path}' to '{new_path}', {e}"

            # update book info in ElasticSearch
            if self.es_manager.update(book_id, category=new_category, title=new_title, author=new_author, file_path=str(new_path), file_type=new_type):
                return "Ok", None
        return "Error", f"can't update book information of '{book_id}' in ElasticSearch, no such a book"

    async def delete_book(self, book_id: int) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# delete_book(book_id=%d)", book_id)
        doc = self.es_manager.search_by_id(book_id)

        # delete file
        try:
            book = Book(book_id, doc)
            file_path = book.file_path
            file_path.unlink()
        except IOError as e:
            return "Error", f"can't delete a book with '{book_id}', {e}"

        # delete book info from ElasticSearch
        if self.es_manager.delete(book_id):
            return "Ok", None
        return "Error", f"can't delete book information of '{book_id}' from ElasticSearch"
