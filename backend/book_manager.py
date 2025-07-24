#!/usr/bin/env python

import sys
import os
import logging.config
from pathlib import Path
from typing import Tuple, Dict, List, Union, Optional, Any
import chardet
from fastapi import HTTPException
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
        self.es_manager = ESManager()

    def __del__(self) -> None:
        del self.es_manager

    def is_healthy(self) -> bool:
        return self.es_manager.is_healthy()

    def get_categories(self) -> Tuple[List[str], Optional[str]]:
        LOGGER.debug("# get_categories()")
        categories = self.es_manager.search_and_aggregate_by_category()
        # '$$rootdir$$/' 접두사 제거
        prefix = f"{self.ROOT_DIRECTORY}/"
        cleaned_categories = [cat.replace(prefix, "") for cat in categories]
        return cleaned_categories, None

    def get_books_in_category(self, category: str, page: int = 1, page_size: int = 10) -> Tuple[List[Book], Optional[str]]:
        doc_list = self.es_manager.search_by_category(category, max_result_count=10000)
        LOGGER.info(f"doc_list from ES for category '{category}': {doc_list}")
        if doc_list and len(doc_list) > 0:
            start = (page - 1) * page_size
            end = start + page_size
            paginated_list = doc_list[start:end]
            return [Book(book_id, doc) for book_id, doc, _score in paginated_list], None
        return [], f"No books found in '{category}'"

    def get_book(self, book_id: int) -> Tuple[Optional[Book], Optional[str]]:
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

    def get_book_content(self, book_id: int, file_path: str) -> Union[str, FileResponse]:
        LOGGER.debug("# get_book_content(book_id=%d, file_path=%s)", book_id, file_path)
        doc = self.es_manager.search_by_id(book_id)
        if not doc:
            raise HTTPException(status_code=404, detail="Book not found")

        book = Book(book_id, doc)
        
        # URL에서 받은 file_path는 디코딩되었을 수 있으므로, book.file_path와 비교하기 전에 정규화가 필요할 수 있습니다.
        # 여기서는 단순 문자열 비교를 가정합니다.
        # Path(file_path)와 book.file_path.relative_to(self.path_prefix)를 비교해야 할 수도 있습니다.
        if str(book.file_path.relative_to(self.path_prefix)) != file_path:
            raise HTTPException(status_code=403, detail="File path does not match")
            
        if book.file_path.is_file():
            media_type = BookManager.MEDIA_TYPES.get(book.file_path.suffix, "application/octet-stream")
            return FileResponse(path=book.file_path, media_type=media_type)
        
        raise HTTPException(status_code=404, detail="File not found")

    def search_by_keyword(self, keyword: str, max_result_count: int = sys.maxsize) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_by_keyword(keyword='%s', max_result_count=%d)", keyword, max_result_count)
        result_list = self.es_manager.search_by_keyword(keyword, max_result_count)
        if result_list and len(result_list) > 0:
            return [Book(book_id, doc) for book_id, doc, _score in result_list], None
        return [], "No books found"

    def search_similar_books(self, book_id: str, max_result_count: int = sys.maxsize) -> Tuple[List[Book], Optional[str]]:
        LOGGER.debug("# search_similar_books(book_id=%s)", book_id)
        doc = self.es_manager.search_by_id(int(book_id))
        result_list = self.es_manager.search_similar_docs(book_id, doc["category"], doc["title"], doc["author"], doc["file_type"], doc["file_size"], doc["summary"][:3500], max_result_count=max_result_count)
        if result_list and len(result_list) > 0:
            return [Book(doc_id, doc) for doc_id, doc, _score in result_list], None
        return [], "No similar books found"

    def add_book(self, data: Dict[int, Dict[str, Any]]) -> Tuple[Optional[int], Optional[str]]:
        LOGGER.debug("# add_book(data='%r')", data)
        doc_id_list = self.es_manager.insert(data)
        if doc_id_list and len(doc_id_list) == 1:
            return doc_id_list[0], None
        return None, f"can't add book '{data}' to ElasticSearch"

    def update_book(self, book_id: int, book_data: Dict[str, Any]) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# update_book(book_id=%d, book_data=%r)", int(book_id), book_data)
        # rename file
        doc = self.es_manager.search_by_id(book_id)
        if doc:
            book = Book(book_id, doc)
            file_path = book.file_path
            new_path = file_path.parent.parent / book_data["category"] / (book_data["title"] + "." + book_data["file_type"])
            try:
                file_path.rename(new_path)
            except IOError as e:
                return "Error", f"can't move '{file_path}' to '{new_path}', {e}"

            book_data["file_path"] = str(new_path)
            # update book info in ElasticSearch
            if self.es_manager.update(book_id, **book_data):
                return "Ok", None
        return "Error", f"can't update book information of '{book_id}' in ElasticSearch, no such a book"

    def delete_book(self, book_id: int) -> Tuple[str, Optional[str]]:
        LOGGER.debug("# delete_book(book_id=%d)", int(book_id))
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
