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
        assert isinstance(result, dict)
        for key, value in result.items():
            assert isinstance(key, str)
            assert isinstance(value, int)

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

    def test_determine_file_content_and_encoding(self, book_manager_with_data, tmp_path):
        bm = book_manager_with_data
        # txt 파일: 인코딩 감지 경로 테스트
        txt_file = tmp_path / "test.txt"
        txt_file.write_text("Hello, world! 안녕하세요", encoding="utf-8")
        encoding = bm.determine_file_content_and_encoding(txt_file)
        assert isinstance(encoding, str)

        # 비-txt 파일: "binary" 반환 경로 테스트
        epub_file = tmp_path / "test.epub"
        epub_file.write_bytes(b"PK\x03\x04")
        assert bm.determine_file_content_and_encoding(epub_file) == "binary"

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
    async def test_update_book_rejects_path_traversal(self, book_manager_with_data, tmp_path):
        bm = book_manager_with_data
        book = await get_one_random_book(bm)
        if not book:
            pytest.skip("No books found for this test")

        if not book.file_path.is_file():
            pytest.skip("Source file does not exist")

        # 경로 탈출 시도: path_prefix 외부
        outside_path = tmp_path / "outside.txt"
        result, error = await bm.update_book(
            book.book_id,
            book.category,
            book.title,
            book.author,
            outside_path,
            book.file_type,
        )

        assert result == "Error"
        assert error == "잘못된 경로입니다"
        assert book.file_path.is_file()

    @pytest.mark.asyncio
    async def test_get_category_mismatches(self, book_manager_with_data, tmp_path):
        bm = book_manager_with_data
        # 프로덕션 디렉토리 전체 스캔 방지: 임시 디렉토리로 교체
        original_prefix = bm.path_prefix
        try:
            # 테스트용 디렉토리 구조 생성
            (tmp_path / "_epub").mkdir()
            (tmp_path / "_txt").mkdir()
            (tmp_path / "_epub" / "test.epub").write_bytes(b"test")
            (tmp_path / "_txt" / "test.txt").write_text("test")

            bm.path_prefix = tmp_path
            result = bm.get_category_mismatches()
        finally:
            bm.path_prefix = original_prefix

        # 반환 구조 검증
        assert isinstance(result, dict)
        assert "mismatches" in result
        assert "es_only" in result
        assert "fs_only" in result
        assert isinstance(result["mismatches"], list)
        assert isinstance(result["es_only"], list)
        assert isinstance(result["fs_only"], list)

        # mismatches 항목 구조 검증
        for item in result["mismatches"]:
            assert "category" in item
            assert "es_count" in item
            assert "fs_count" in item
            assert "diff" in item
            assert isinstance(item["category"], str)
            assert isinstance(item["es_count"], int)
            assert isinstance(item["fs_count"], int)
            assert item["diff"] > 0

        # mismatches가 diff 절대값 내림차순 정렬인지 검증
        diffs = [abs(item["diff"]) for item in result["mismatches"]]
        assert diffs == sorted(diffs, reverse=True)

        # es_only 항목 구조 검증
        for item in result["es_only"]:
            assert "category" in item
            assert "es_count" in item
            assert isinstance(item["category"], str)
            assert isinstance(item["es_count"], int)

        # fs_only 항목 구조 검증
        for item in result["fs_only"]:
            assert "category" in item
            assert "fs_count" in item
            assert isinstance(item["category"], str)
            assert isinstance(item["fs_count"], int)

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
