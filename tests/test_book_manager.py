#!/usr/bin/env python

import logging.config
import shutil
from datetime import datetime
from pathlib import Path
from typing import Optional, Tuple

import pytest
from fastapi.responses import FileResponse

from backend.book import Book
from backend.book_manager import BookManager
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

CATEGORY1 = "_epub"
CATEGORY2 = "_txt"


def inspect_book_info(book: Book) -> None:
    """Verify book object has correct types."""
    assert isinstance(book, Book)
    assert isinstance(book.book_id, int)
    assert isinstance(book.category, str)
    assert isinstance(book.title, str)
    assert isinstance(book.author, str)
    assert isinstance(book.file_type, str)
    assert isinstance(book.file_path, Path)
    assert isinstance(book.file_size, int)
    assert isinstance(book.updated_time, datetime)


@pytest.fixture(scope="module")
def book_manager_with_data(es_client, es_index):
    """Create BookManager with test data loaded (공유된 ES 클라이언트 및 인덱스 사용)."""
    # Create BookManager and use shared ES client
    bm = BookManager()
    bm.es_manager.es = es_client

    # Load test data from actual files if available
    epub_path = bm.path_prefix / CATEGORY1
    txt_path = bm.path_prefix / CATEGORY2

    if epub_path.exists():
        data = Loader.read_files(epub_path, num_files=5)
        if data:
            bm.es_manager.insert(data, num_docs=20)
            LOGGER.info("Inserted %d epub documents", len(data))

    if txt_path.exists():
        data = Loader.read_files(txt_path, num_files=5)
        if data:
            bm.es_manager.insert(data, num_docs=20)
            LOGGER.info("Inserted %d txt documents", len(data))

    # Refresh index to make data searchable
    bm.es_manager.refresh()

    yield bm


async def get_one_random_book(bm: BookManager) -> Optional[Book]:
    """Helper to get one random book from the test data."""
    for category in [CATEGORY1, CATEGORY2, "_txt", "test"]:
        book_list, error = await bm.get_books_in_category(category)
        if book_list and not error:
            return book_list[0]
    return None


async def get_two_random_books(bm: BookManager) -> Optional[Tuple[Book, Book]]:
    """Helper to get two random books from different categories."""
    book1 = None
    book2 = None

    book_list, error = await bm.get_books_in_category(CATEGORY1)
    if book_list and not error:
        book1 = book_list[0]

    book_list, error = await bm.get_books_in_category(CATEGORY2)
    if book_list and not error:
        book2 = book_list[0]

    if book1 and book2:
        return book1, book2
    return None


class TestBookManager:

    @pytest.mark.asyncio
    async def test_get_categories(self, book_manager_with_data):
        bm = book_manager_with_data
        result, _ = await bm.get_categories()
        assert isinstance(result, list)
        # May be empty if no data loaded
        if result:
            assert isinstance(result[0], str)

    @pytest.mark.asyncio
    async def test_get_books_in_category(self, book_manager_with_data):
        bm = book_manager_with_data
        for category in [CATEGORY1, CATEGORY2]:
            book_list, error = await bm.get_books_in_category(category)
            if book_list:
                for book in book_list:
                    assert book and not error
                    inspect_book_info(book)
                return
        pytest.skip("No books found in test categories")

    @pytest.mark.asyncio
    async def test_get_book(self, book_manager_with_data):
        bm = book_manager_with_data
        randomly_chosen_book = await get_one_random_book(bm)
        if not randomly_chosen_book:
            pytest.skip("No books available for testing")

        book_id = randomly_chosen_book.book_id
        book, error = await bm.get_book(book_id)
        assert book and not error
        inspect_book_info(book)
        assert book.book_id == randomly_chosen_book.book_id
        assert book.category == randomly_chosen_book.category
        assert book.title == randomly_chosen_book.title

    def test_determine_file_content_and_encoding(self, book_manager_with_data):
        bm = book_manager_with_data
        # Find any epub file for testing
        epub_files = list(bm.path_prefix.glob("**/*.epub"))
        if not epub_files:
            pytest.skip("No epub files available")
        file_path = epub_files[0]
        content = bm.determine_file_content_and_encoding(file_path)
        assert isinstance(content, str)

    @pytest.mark.asyncio
    async def test_get_book_content(self, book_manager_with_data):
        bm = book_manager_with_data
        book = await get_one_random_book(bm)
        if not book:
            pytest.skip("No books available for testing")
        inspect_book_info(book)
        content = await bm.get_book_content(book.book_id)
        assert isinstance(content, (FileResponse, str))

    @pytest.mark.asyncio
    async def test_search_by_keyword(self, book_manager_with_data):
        bm = book_manager_with_data
        # Get a book to use its title as keyword
        book = await get_one_random_book(bm)
        if not book:
            pytest.skip("No books available for testing")

        # Use first word of title as keyword
        keyword = book.title.split()[0] if book.title else "테스트"
        book_list, error = await bm.search_by_keyword(keyword, max_result_count=20)
        assert isinstance(book_list, list)
        # Results may be empty depending on test data

    @pytest.mark.asyncio
    async def test_search_similar_books(self, book_manager_with_data):
        bm = book_manager_with_data
        book = await get_one_random_book(bm)
        if not book:
            pytest.skip("No books available for testing")
        inspect_book_info(book)
        book_list, error = await bm.search_similar_books(book.book_id, max_result_count=20)
        assert isinstance(book_list, list)

    @pytest.mark.asyncio
    async def test_add_book(self, book_manager_with_data):
        bm = book_manager_with_data
        book = await get_one_random_book(bm)
        if not book:
            pytest.skip("No books available for testing")
        inspect_book_info(book)
        title = book.title
        file_type = book.file_type
        file_path = book.file_path

        if not file_path.exists():
            pytest.skip("Source file does not exist")

        # make a copy of a file
        new_file_name = title + ".copy" + "." + file_type
        temp_file_path = file_path.with_name(new_file_name)
        try:
            shutil.copy(file_path, temp_file_path)

            # add the copy
            book_id2, error = await bm.add_book(Loader.read_file(temp_file_path))
            assert book_id2 and not error
            book2, error = await bm.get_book(book_id2)
            assert book2
            inspect_book_info(book2)

            # delete the copy
            result, error = await bm.delete_book(book_id2)
            assert result and not error
        finally:
            if temp_file_path.exists():
                temp_file_path.unlink()

    @pytest.mark.asyncio
    async def test_move_book(self, book_manager_with_data):
        bm = book_manager_with_data
        result = await get_two_random_books(bm)
        if not result:
            pytest.skip("Need two books from different categories for this test")

        book1, book2 = result
        book_id = book1.book_id
        category1 = book1.category
        title1 = book1.title
        author1 = book1.author
        type1 = book1.file_type
        path1 = book1.file_path

        if not path1.is_file():
            pytest.skip("Source file does not exist")

        category2 = book2.category
        title2 = "renamed_" + book1.title
        author2 = book2.author if book2.author else book1.author
        type2 = book2.file_type
        path2 = bm.path_prefix / category2 / (title2 + "." + type2)

        try:
            assert path1.is_file()
            assert not path2.is_file()
            assert await bm.update_book(book_id, category2, title2, author2, path2, type2)
            assert not path1.is_file()
            assert path2.is_file()

            book3, error = await bm.get_book(book_id)
            assert book3 and not error
            inspect_book_info(book3)
            assert book3.category == category2
            assert book3.title == title2
            assert book3.author == author2
            assert book3.file_type == type2
        finally:
            # move back
            if path2.is_file():
                await bm.update_book(book_id, category1, title1, author1, path1, type1)

    @pytest.mark.asyncio
    async def test_delete_book(self, book_manager_with_data):
        bm = book_manager_with_data
        book = await get_one_random_book(bm)
        if not book:
            pytest.skip("No books available for testing")
        inspect_book_info(book)
        book_id = book.book_id
        title = book.title
        file_type = book.file_type
        file_path = book.file_path

        if not file_path.exists():
            pytest.skip("Source file does not exist")

        # make a copy of a file
        new_file_name = title + ".copy" + "." + file_type
        temp_file_path = file_path.with_name(new_file_name)
        shutil.copy(file_path, temp_file_path)

        result2, error = await bm.delete_book(book_id)
        assert result2 and not error

        book2, error = await bm.get_book(book_id)
        assert not book2 and error

        # restore the deleted file
        temp_file_path.rename(file_path)
        book_id3, error = await bm.add_book(Loader.read_file(file_path))
        assert book_id3 and not error


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
