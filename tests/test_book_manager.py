#!/usr/bin/env python


import unittest
import logging.config
import shutil
from pathlib import Path
from datetime import datetime
from typing import Tuple, Optional
from fastapi.responses import FileResponse
from backend.book import Book
from backend.book_manager import BookManager
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TestBookManager(unittest.IsolatedAsyncioTestCase):
    bm = BookManager()
    category1 = "_epub"
    category2 = "_txt"

    @classmethod
    def setUpClass(cls):
        cls.bm.es_manager.create_index()
        data = Loader.read_files(cls.bm.path_prefix / cls.category1, 10000)
        cls.bm.es_manager.insert(data)
        data = Loader.read_files(cls.bm.path_prefix / cls.category2, 10000)
        cls.bm.es_manager.insert(data)

    @classmethod
    def tearDownClass(cls):
        cls.bm.es_manager.delete_index()
        del cls.bm

    @classmethod
    async def get_one_random_book(cls) -> Optional[Book]:
        book_list, error = await cls.bm.get_books_in_category(cls.category1)
        if book_list and not error:
            return book_list[0]
        return None

    @classmethod
    async def get_two_random_books(cls) -> Optional[Tuple[Book, Book]]:
        book_list, error = await cls.bm.get_books_in_category(cls.category1)
        if book_list and not error:
            book1 = book_list[0]
            if not book1:
                return None

            book_list, error = await cls.bm.get_books_in_category(cls.category2)
            if book_list and not error:
                book2 = book_list[0]
                if not book2:
                    return None

                return book1, book2

        return None

    @staticmethod
    def inspect_book_info(book: Book):
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
        result, _ = await self.bm.get_categories()
        assert len(result) > 0
        assert isinstance(result[0], str)
        assert self.category1 in result

    async def test_get_books_in_category(self):
        book_list, error = await self.bm.get_books_in_category(self.category1)
        assert book_list and len(book_list) > 0
        for book in book_list:
            assert book and not error
            self.inspect_book_info(book)

    async def test_get_book(self):
        randomly_chosen_book = await TestBookManager.get_one_random_book()
        assert randomly_chosen_book
        book_id = randomly_chosen_book.book_id
        book, error = await self.bm.get_book(book_id)
        assert book and not error
        self.inspect_book_info(book)
        assert book.book_id == randomly_chosen_book.book_id
        assert book.category == randomly_chosen_book.category
        assert book.title == randomly_chosen_book.title
        assert book.author == randomly_chosen_book.author
        assert book.file_type == randomly_chosen_book.file_type
        assert book.file_path == randomly_chosen_book.file_path
        assert book.file_size == randomly_chosen_book.file_size
        assert book.updated_time == randomly_chosen_book.updated_time

    def test_determine_file_content_and_encoding(self):
        file_path = self.bm.path_prefix / "epub" / "[J. R. R. 톨킨] 실마릴리온 1.epub"
        content = self.bm.determine_file_content_and_encoding(file_path)
        assert isinstance(content, str)

    async def test_get_book_content(self):
        book = await TestBookManager.get_one_random_book()
        assert book
        self.inspect_book_info(book)
        content = await self.bm.get_book_content(book.book_id)
        assert isinstance(content, (FileResponse, str))

    async def test_search_by_keyword(self):
        keyword = "검왕"
        book_list, error = await self.bm.search_by_keyword(keyword, max_result_count=20)
        assert book_list and not error
        assert isinstance(book_list, list)
        assert len(book_list) > 0

        match_count = 0
        for book in book_list:
            assert isinstance(book, Book)
            if keyword in book.title:
                match_count += 1
        assert match_count > 10
        assert match_count / len(book_list) > 0.1

    async def test_search_similar_books(self):
        book = await TestBookManager.get_one_random_book()
        assert book
        self.inspect_book_info(book)
        book_list, error = await self.bm.search_similar_books(book.book_id, max_result_count=20)
        assert book_list and not error
        assert isinstance(book_list, list)
        assert len(book_list) > 0

    async def test_add_book(self):
        book = await TestBookManager.get_one_random_book()
        assert book
        self.inspect_book_info(book)
        title = book.title
        file_type = book.file_type
        file_path = book.file_path

        # make a copy of a file
        new_file_name = title + ".copy" + "." + file_type
        temp_file_path = file_path.with_name(new_file_name)
        shutil.copy(file_path, temp_file_path)

        # add the copy
        book_id2, error = await self.bm.add_book(Loader.read_file(temp_file_path))
        assert book_id2 and not error
        book2, error = await self.bm.get_book(book_id2)
        assert book2
        self.inspect_book_info(book2)

        # delete the copy
        result, error = await self.bm.delete_book(book_id2)
        assert result and not error

    async def test_move_book(self):
        result = await TestBookManager.get_two_random_books()
        assert result
        book1, book2 = result
        book_id = book1.book_id
        category1 = book1.category
        title1 = book1.title
        author1 = book1.author
        type1 = book1.file_type
        path1 = book1.file_path

        category2 = book2.category
        title2 = "renamed_" + book1.title
        author2 = book2.author
        type2 = book2.file_type
        path2 = self.bm.path_prefix / category2 / (title2 + "." + type2)

        assert path1.is_file()
        assert not path2.is_file()
        assert await self.bm.update_book(book_id, category2, title2, author2, path2, type2)
        assert not path1.is_file()
        assert path2.is_file()

        book3, error = await self.bm.get_book(book_id)
        assert book3 and not error
        self.inspect_book_info(book3)
        assert book3.category == category2
        assert book3.title == title2
        assert book3.author == author2
        assert book3.file_type == type2

        # move back
        await self.bm.update_book(book_id, category1, title1, author1, path1, type1)

    async def test_delete_book(self):
        book = await TestBookManager.get_one_random_book()
        assert book
        self.inspect_book_info(book)
        book_id = book.book_id
        title = book.title
        file_type = book.file_type
        file_path = book.file_path

        # make a copy of a file
        new_file_name = title + ".copy" + "." + file_type
        temp_file_path = file_path.with_name(new_file_name)
        shutil.copy(file_path, temp_file_path)

        result2, error = await self.bm.delete_book(book_id)
        assert result2 and not error

        book2, error = await self.bm.get_book(book_id)
        assert not book2 and error

        # restore the deleted file
        temp_file_path.rename(file_path)
        book_id3, error = await self.bm.add_book(Loader.read_file(file_path))
        assert book_id3 and not error


if __name__ == "__main__":
    unittest.main()
