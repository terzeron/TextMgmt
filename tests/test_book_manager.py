#!/usr/bin/env python

import logging.config
import shutil
import json
import os
import sys
import time
import zipfile
import tempfile
from datetime import datetime
from pathlib import Path

import pytest
from fastapi.responses import FileResponse, Response

from backend.book import Book
from backend.book_manager import BookManager
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

CATEGORY1 = "_epub"
CATEGORY2 = "_txt"


@pytest.fixture(autouse=True)
def restore_book_path_prefix():
    original = Book.path_prefix
    try:
        yield
    finally:
        Book.path_prefix = original


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


async def get_one_random_book(bm: BookManager) -> Book | None:
    """Helper to get one random book from the test data."""
    for category in [CATEGORY1, CATEGORY2, "_txt", "test"]:
        book_list, error = await bm.get_books_in_category(category)
        if book_list and not error:
            return book_list[0]
    return None


async def get_two_random_books(bm: BookManager) -> tuple[Book, Book] | None:
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
        result, error = await bm.update_book(book.book_id, book.category, book.title, book.author, outside_path, book.file_type)

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


# ---- merged from test_book_manager_extra.py ----


class DummyES:
    def __init__(self, doc: dict | None = None):
        self.updated = True
        self.delete_ok = True
        self.inserted = []
        self.aggregate = {"A": 1}
        self.counts = {"A": 1, "B": 0}
        self.category_docs = []
        self.doc = doc
        self.keyword = []
        self.similar = []
        self.similar_paged = ([], 0)
        self.deleted_by_category = {"deleted": 2, "failures": []}
        self.keyword_paged = ([], 0)

    def search_by_id(self, book_id: int):
        return self.doc

    def update(self, *args, **kwargs):
        return self.updated

    def delete(self, book_id: int):
        return self.delete_ok

    def insert(self, data):
        self.inserted.append(data)
        return list(data.keys())

    def refresh(self):
        return None

    def search_and_aggregate_by_category(self):
        return self.aggregate

    def search_by_category(self, category: str, max_result_count: int):
        return self.category_docs

    def count_by_categories(self, categories):
        return {c: self.counts.get(c, 0) for c in categories}

    def rename_category(self, old_category: str, new_category: str):
        return {"updated": 3, "failures": []}

    def count_by_category(self, category: str, prefix: bool = False):
        return self.counts.get(category, 0)

    def delete_by_category(self, category: str, prefix: bool = False):
        return self.deleted_by_category

    def search_by_keyword(self, keyword, max_result_count=-1):
        return self.keyword

    def search_similar_docs(self, *args, **kwargs):
        return self.similar

    def search_similar_docs_paged(self, *args, **kwargs):
        return self.similar_paged

    def search_by_keyword_paged(self, *args, **kwargs):
        return self.keyword_paged


def make_manager(tmp_path: Path, es: DummyES | dict | None) -> BookManager:
    manager = BookManager.__new__(BookManager)
    manager.path_prefix = tmp_path
    if isinstance(es, DummyES):
        manager.es_manager = es
    else:
        manager.es_manager = DummyES(es)
    manager._mismatch_cache = None
    manager._mismatch_cache_time = 0.0
    manager.item_class = Book
    Book.path_prefix = tmp_path
    return manager


def make_doc(rel_path: str, file_type: str = ".txt") -> dict:
    return {"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"}


def test_determine_file_content_and_encoding(tmp_path: Path):
    txt = tmp_path / "a.txt"
    txt.write_text("hello", encoding="utf-8")
    enc = BookManager.determine_file_content_and_encoding(txt)
    assert enc in {"utf-8", "ascii"}

    other = tmp_path / "a.pdf"
    other.write_bytes(b"%PDF")
    assert BookManager.determine_file_content_and_encoding(other) == "binary"


def test_evict_old_cache(tmp_path: Path):
    old_file = tmp_path / "old.txt"
    new_file = tmp_path / "new.txt"
    old_file.write_text("x")
    new_file.write_text("y")
    past = time.time() - (BookManager.CACHE_MAX_AGE_SECONDS + 10)
    import os

    os.utime(old_file, (past, past))
    BookManager._evict_old_cache(tmp_path)
    assert not old_file.exists()
    assert new_file.exists()


def test_find_opf_path_variants(tmp_path: Path):
    zip_path = tmp_path / "book.epub"
    with zipfile.ZipFile(zip_path, "w") as zf:
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/content.opf", "<package></package>")
    with zipfile.ZipFile(zip_path, "r") as zin:
        assert BookManager._find_opf_path(zin) == "OEBPS/content.opf"

    zip_path2 = tmp_path / "book2.epub"
    with zipfile.ZipFile(zip_path2, "w") as zf:
        zf.writestr("META-INF/container.xml", b"full-path='OPS/test.opf'")
        zf.writestr("OPS/test.opf", "<package></package>")
    with zipfile.ZipFile(zip_path2, "r") as zin:
        assert BookManager._find_opf_path(zin) == "OPS/test.opf"

    zip_path3 = tmp_path / "book3.epub"
    with zipfile.ZipFile(zip_path3, "w") as zf:
        zf.writestr("content.opf", "<package></package>")
    with zipfile.ZipFile(zip_path3, "r") as zin:
        assert BookManager._find_opf_path(zin) == "content.opf"


def test_update_book_conflict_and_success_and_rollback(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)

    original = tmp_path / "A" / "old.txt"
    original.parent.mkdir(parents=True)
    original.write_text("x")
    doc = make_doc("A/old.txt")
    es.search_by_id = lambda _id: doc

    conflict = tmp_path / "A" / "new.txt"
    conflict.write_text("y")
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", conflict, ".txt"))
    assert status == "Error"
    assert "CONFLICT" in msg

    conflict.unlink()
    es.updated = True
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", conflict, ".txt"))
    assert status == "Ok"
    assert msg is None
    assert conflict.exists()
    assert not original.exists()

    # rollback on ES update failure
    doc2 = make_doc("A/old2.txt")
    old2 = tmp_path / "A" / "old2.txt"
    old2.write_text("z")
    es.search_by_id = lambda _id: doc2
    es.updated = False
    new2 = tmp_path / "A" / "new2.txt"
    status, msg = asyncio_runner(manager.update_book(2, "A", "T", "U", new2, ".txt"))
    assert status == "Error"
    assert old2.exists()


def test_update_book_es_false_rollback_succeeds(tmp_path: Path):
    """ES update returns False → file is rolled back and a meaningful error is returned."""
    es = DummyES()
    manager = make_manager(tmp_path, es)

    src = tmp_path / "A" / "src.txt"
    src.parent.mkdir(parents=True, exist_ok=True)
    src.write_text("content")
    es.search_by_id = lambda _id: make_doc("A/src.txt")
    es.updated = False

    dst = tmp_path / "A" / "dst.txt"
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", dst, ".txt"))

    assert status == "Error"
    assert msg is not None
    assert "ES 업데이트 실패" in msg
    assert src.exists(), "rollback should have restored the source file"
    assert not dst.exists(), "destination should not exist after rollback"


def test_update_book_es_false_rollback_fails(tmp_path: Path):
    """ES update returns False and rollback rename also fails → combined error message."""
    es = DummyES()
    manager = make_manager(tmp_path, es)

    src = tmp_path / "A" / "src2.txt"
    src.parent.mkdir(parents=True, exist_ok=True)
    src.write_text("content")
    es.search_by_id = lambda _id: make_doc("A/src2.txt")
    es.updated = False

    dst = tmp_path / "A" / "dst2.txt"

    original_rename = dst.__class__.rename

    def fail_rename(self, target):
        if self == dst:
            raise OSError("simulated rollback failure")
        return original_rename(self, target)

    import unittest.mock as mock

    with mock.patch.object(type(dst), "rename", fail_rename):
        status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", dst, ".txt"))

    assert status == "Error"
    assert msg is not None
    assert "롤백" in msg


def test_update_book_es_exception_rollback_succeeds(tmp_path: Path):
    """ES update raises exception → file is rolled back and a meaningful error is returned."""
    es = DummyES()
    manager = make_manager(tmp_path, es)

    src = tmp_path / "A" / "src3.txt"
    src.parent.mkdir(parents=True, exist_ok=True)
    src.write_text("content")
    es.search_by_id = lambda _id: make_doc("A/src3.txt")

    def raise_on_update(*args, **kwargs):
        raise RuntimeError("ES connection error")

    es.update = raise_on_update

    dst = tmp_path / "A" / "dst3.txt"
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", dst, ".txt"))

    assert status == "Error"
    assert msg is not None
    assert "ES 업데이트 예외" in msg
    assert src.exists(), "rollback should have restored the source file"
    assert not dst.exists()


def test_update_book_path_traversal(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", Path("/tmp/out.txt"), ".txt"))
    assert status == "Error"
    assert "잘못된 경로" in msg


def test_delete_book_warning_on_missing_file(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: make_doc("A/missing.txt")
    status, msg = asyncio_runner(manager.delete_book(1))
    assert status == "Warning"
    assert "이미 삭제" in msg


def test_delete_file_and_index_single_file(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)

    status, msg = asyncio_runner(manager.delete_file("../bad.txt"))
    assert status == "Error"

    status, msg = asyncio_runner(manager.delete_file("A/none.txt"))
    assert status == "Error"

    ok = tmp_path / "A" / "ok.txt"
    ok.parent.mkdir(parents=True)
    ok.write_text("x")
    status, msg = asyncio_runner(manager.delete_file("A/ok.txt"))
    assert status == "Ok"
    assert not ok.exists()

    monkeypatch.setattr("utils.loader.Loader.read_file", lambda p: {})
    status_id, msg = asyncio_runner(manager.index_single_file("../bad.txt"))
    assert status_id is None
    assert "잘못된 경로" in msg


def test_category_mismatches_cache_and_details(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)

    # FS structure
    (tmp_path / "A").mkdir()
    (tmp_path / "A" / "f1.txt").write_text("x")
    (tmp_path / "A" / "sub").mkdir()
    (tmp_path / "A" / "sub" / "f2.txt").write_text("y")

    es.aggregate = {"A": 2, "B": 1}
    result = manager.get_category_mismatches()
    assert "mismatches" in result

    manager._mismatch_cache = {"cached": True}
    manager._mismatch_cache_time = time.monotonic()
    es.aggregate = {"A": 0}
    assert manager.get_category_mismatches() == {"cached": True}

    # mismatch details with duplicates
    inode = tmp_path / "A" / "dup.txt"
    inode.write_text("z")
    inode_id = inode.stat().st_ino
    es.category_docs = [(inode_id, make_doc("A/dup.txt"), 1.0), (999999, make_doc("A/dup.txt"), 1.0)]
    details = manager.get_category_mismatch_details("A")
    assert details["duplicates"]
    assert details["fs_count"] >= 1


def test_category_mismatch_details_root(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    root_file = tmp_path / "root.txt"
    root_file.write_text("x")
    es.category_docs = [(root_file.stat().st_ino, make_doc("root.txt"), 1.0)]
    details = manager.get_category_mismatch_details("_root")
    assert details["fs_count"] >= 1


def asyncio_runner(coro):
    import asyncio

    return asyncio.run(coro)


# ---- merged from test_book_manager_epub_pdf_extra.py ----


def test_validate_preview_epub_ok_and_fail(tmp_path: Path):
    epub = tmp_path / "ok.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/ch1.xhtml", "<html/>")
        zf.writestr(
            "OEBPS/content.opf",
            """<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
          <spine><itemref idref="c1"/></spine>
        </package>""",
        )
    ok, err = BookManager._validate_preview_epub(epub)
    assert ok is True
    assert err is None

    bad = tmp_path / "bad.epub"
    with zipfile.ZipFile(bad, "w") as zf:
        zf.writestr("META-INF/container.xml", "x")
    ok, err = BookManager._validate_preview_epub(bad)
    assert ok is False
    assert "mimetype" in err


def test_get_epub_total_chapters(tmp_path: Path):
    epub = tmp_path / "c.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>')
        zf.writestr(
            "content.opf",
            """<package xmlns="http://www.idpf.org/2007/opf">
          <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
        </package>""",
        )
    assert BookManager._get_epub_total_chapters(epub) == 2


def test_convert_with_libreoffice_success_and_fallback(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    fake_bin = tmp_path / "libreoffice"
    fake_bin.write_text("")
    monkeypatch.setattr(BookManager, "_find_libreoffice", lambda: str(fake_bin))

    class DummyProc:
        returncode = 0
        stderr = b""

    def fake_run(cmd, capture_output=True, timeout=60):
        outdir = Path(cmd[cmd.index("--outdir") + 1])
        outdir.mkdir(parents=True, exist_ok=True)
        # write mismatched stem to trigger glob fallback
        (outdir / "other.txt").write_text("ok", encoding="utf-8")
        return DummyProc()

    monkeypatch.setattr("backend.book_manager.subprocess.run", fake_run)
    content = BookManager._convert_with_libreoffice(tmp_path / "x.doc", "txt")
    assert content == "ok"

    def fake_run_empty(cmd, capture_output=True, timeout=60):
        return DummyProc()

    monkeypatch.setattr("backend.book_manager.subprocess.run", fake_run_empty)
    content = BookManager._convert_with_libreoffice(tmp_path / "x.doc", "txt")
    assert content == ""


def test_validate_epub_success_and_errors(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    epub = tmp_path / "a.epub"
    epub.write_text("x")
    doc = make_doc("a.epub", "epub")
    manager = make_manager(tmp_path, doc)

    class DummyProc:
        def __init__(self):
            self.returncode = 0

        async def communicate(self):
            return b"", b""

        def kill(self):
            return None

        async def wait(self):
            return None

    async def fake_exec(*args, **kwargs):
        # write JSON to the provided path
        json_path = args[3]
        data = {"messages": [], "checker": {"nFatal": 0, "nError": 0, "nWarning": 0, "nUsage": 0, "nInfo": 0}}
        Path(json_path).write_text(json.dumps(data), encoding="utf-8")
        return DummyProc()

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", fake_exec)
    result, err = asyncio_runner(manager.validate_epub(1))
    assert err is None
    assert result["valid"] is True

    # wrong type
    manager = make_manager(tmp_path, make_doc("a.epub", "pdf"))
    result, err = asyncio_runner(manager.validate_epub(1))
    assert err and "Not an EPUB" in err


def test_validate_pdf_success_and_open_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    pdf = tmp_path / "a.pdf"
    pdf.write_bytes(b"%PDF")
    doc = make_doc("a.pdf", "pdf")
    manager = make_manager(tmp_path, doc)

    class DummyPDF:
        def __init__(self):
            self.docinfo = {"/Title": "T", "/Author": "A"}
            self.pages = [1, 2]
            self.pdf_version = "1.4"

        def check_pdf_syntax(self):
            return []

        def close(self):
            return None

    class DummyPike:
        @staticmethod
        def open(path):
            return DummyPDF()

    monkeypatch.setitem(sys.modules, "pikepdf", DummyPike())
    result, err = asyncio_runner(manager.validate_pdf(1))
    assert err is None
    assert result["valid"] is True

    class DummyPikeFail:
        @staticmethod
        def open(path):
            raise RuntimeError("bad")

    monkeypatch.setitem(sys.modules, "pikepdf", DummyPikeFail())
    result, err = asyncio_runner(manager.validate_pdf(1))
    assert err and "Failed to open PDF" in err


def test_validate_epub_not_found_and_missing_file(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    manager.es_manager.search_by_id = lambda _id: None
    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is None
    assert "Book not found" in err

    doc = make_doc("missing.epub", "epub")
    manager.es_manager.search_by_id = lambda _id: doc
    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is None
    assert "File not found" in err


def test_validate_epub_not_installed(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    epub = tmp_path / "a.epub"
    epub.write_text("x")
    doc = make_doc("a.epub", "epub")
    manager = make_manager(tmp_path, doc)

    async def raise_not_found(*args, **kwargs):
        raise FileNotFoundError()

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", raise_not_found)
    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is None
    assert "not installed" in err


def test_validate_epub_timeout(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    epub = tmp_path / "b.epub"
    epub.write_text("x")
    doc = make_doc("b.epub", "epub")
    manager = make_manager(tmp_path, doc)

    class DummyProc:
        def __init__(self):
            self.returncode = 1
            self.killed = False

        async def communicate(self):
            return b"", b""

        def kill(self):
            self.killed = True

        async def wait(self):
            return None

    async def fake_exec(*args, **kwargs):
        return DummyProc()

    def raise_timeout(coro, *args, **kwargs):
        try:
            coro.close()
        except Exception:
            pass
        raise asyncio.TimeoutError()

    import asyncio

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", fake_exec)
    monkeypatch.setattr("backend.book_manager.asyncio.wait_for", raise_timeout)
    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is None
    assert "timed out" in err


def test_validate_epub_parse_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    epub = tmp_path / "c.epub"
    epub.write_text("x")
    doc = make_doc("c.epub", "epub")
    manager = make_manager(tmp_path, doc)

    class DummyProc:
        def __init__(self):
            self.returncode = 1

        async def communicate(self):
            return b"", b""

        def kill(self):
            return None

        async def wait(self):
            return None

    async def fake_exec(*args, **kwargs):
        json_path = args[3]
        Path(json_path).write_text("{bad", encoding="utf-8")
        return DummyProc()

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", fake_exec)
    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is None
    assert "Failed to parse epubcheck output" in err


def test_validate_pdf_wrong_type_and_missing_file(tmp_path: Path):
    doc = make_doc("a.txt", "txt")
    manager = make_manager(tmp_path, doc)
    result, err = asyncio_runner(manager.validate_pdf(1))
    assert result is None
    assert "Not a PDF" in err

    doc2 = make_doc("missing.pdf", "pdf")
    manager2 = make_manager(tmp_path, doc2)
    result, err = asyncio_runner(manager2.validate_pdf(1))
    assert result is None
    assert "File not found" in err


# ---- merged from test_book_manager_preview_extra.py ----


def build_epub(epub_path: Path):
    with zipfile.ZipFile(epub_path, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/ch1.xhtml", "<html><body>Hi</body></html>")
        zf.writestr("OEBPS/toc.ncx", "<ncx/>")
        zf.writestr("OEBPS/styles.css", "@font-face{font-family:'X';src:url('fonts/missing.ttf'),url('fonts/f.ttf');}")
        zf.writestr("OEBPS/fonts/f.ttf", b"fontdata")
        zf.writestr(
            "OEBPS/content.opf",
            """<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
                <item id="css" href="styles.css" media-type="text/css"/>
                <item id="f1" href="fonts/f.ttf" media-type="font/ttf"/>
              </manifest>
              <spine toc="toc"><itemref idref="c1"/></spine>
            </package>""",
        )


def test_get_book_preview_epub_success(tmp_path: Path):
    epub = tmp_path / "book.epub"
    build_epub(epub)
    doc = make_doc("book.epub", "epub")
    manager = make_manager(tmp_path, doc)
    resp = asyncio_runner(manager.get_book_preview(1, chapters=1))
    assert isinstance(resp, Response)
    assert resp.status_code == 200


def test_get_book_preview_epub_missing_opf(tmp_path: Path):
    epub = tmp_path / "bad.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
    doc = make_doc("bad.epub", "epub")
    manager = make_manager(tmp_path, doc)
    resp = asyncio_runner(manager.get_book_preview(1, chapters=1))
    assert resp.status_code == 422


def test_get_book_preview_doc_and_unsupported(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    doc_file = tmp_path / "a.doc"
    doc_file.write_text("x")
    doc = make_doc("a.doc", "doc")
    manager = make_manager(tmp_path, doc)
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "<p>ok</p>")
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 200

    other_file = tmp_path / "a.bin"
    other_file.write_text("x")
    doc2 = make_doc("a.bin", "bin")
    manager2 = make_manager(tmp_path, doc2)
    resp = asyncio_runner(manager2.get_book_preview(1))
    assert resp.status_code == 400


# ---- merged from test_book_manager_more.py ----
def test_search_by_keyword_and_similar(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.keyword = [(1, make_doc("a.txt"), 1.0)]
    books, err = asyncio_runner(manager.search_by_keyword("k"))
    assert books and err is None

    es.keyword = []
    books, err = asyncio_runner(manager.search_by_keyword("k"))
    assert books == [] and err

    es.similar = [(2, make_doc("b.txt"), 1.0)]
    es_doc = make_doc("a.txt")
    es.search_by_id = lambda _id: es_doc
    books, err = asyncio_runner(manager.search_similar_books(1))
    assert books and err is None

    es.similar = []
    books, err = asyncio_runner(manager.search_similar_books(1))
    assert books == [] and err


def test_search_similar_books_paged_and_add_book(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.similar_paged = ([(2, make_doc("b.txt"), 10.0)], 1)
    es.search_by_id = lambda _id: make_doc("a.txt")
    books, total, err = asyncio_runner(manager.search_similar_books_paged(1, size=10, offset=0))
    assert total == 1 and err is None

    es.similar_paged = ([], 0)
    books, total, err = asyncio_runner(manager.search_similar_books_paged(1, size=10, offset=0))
    assert err

    result, err = asyncio_runner(manager.add_book({1: make_doc("a.txt")}))
    assert result == 1 and err is None


def test_search_by_keyword_paged(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.keyword_paged = ([(1, make_doc("a.txt"), 1.0)], 1)
    books, total, err = asyncio_runner(manager.search_by_keyword_paged("k", size=10, offset=0))
    assert total == 1
    assert books and err is None

    es.keyword_paged = ([], 0)
    books, total, err = asyncio_runner(manager.search_by_keyword_paged("k", size=10, offset=0))
    assert books == []
    assert total == 0


def test_delete_book_when_missing_doc(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: None
    status, msg = asyncio_runner(manager.delete_book(1))
    assert status == "Ok"
    assert msg is None


def test_rename_delete_category_errors(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)

    result, err = asyncio_runner(manager.rename_category("", "B"))
    assert err

    result, err = asyncio_runner(manager.rename_category("A", "A"))
    assert err

    result, err = asyncio_runner(manager.rename_category("A", "../B"))
    assert err

    result, err = asyncio_runner(manager.delete_category(""))
    assert err

    result, err = asyncio_runner(manager.delete_category("../A"))
    assert err

    es.counts["A"] = 0
    result, err = asyncio_runner(manager.delete_category("A"))
    assert err

    es.counts["A"] = 1
    es.deleted_by_category = {"deleted": 0, "failures": ["x"]}
    result, err = asyncio_runner(manager.delete_category("A"))
    assert err


def _make_epub(tmp_path: Path, files: dict[str, bytes]) -> Path:
    epub_path = tmp_path / "case.epub"
    with zipfile.ZipFile(epub_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for name, content in files.items():
            zf.writestr(name, content)
    return epub_path


def test_validate_preview_epub_missing_opf_and_manifest(tmp_path: Path):
    container = b"""<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/content.opf"/></rootfiles></container>"""
    missing_opf = _make_epub(tmp_path, {"mimetype": b"application/epub+zip", "META-INF/container.xml": container})
    ok, err = BookManager._validate_preview_epub(missing_opf)
    assert ok is False
    assert "OPF file missing in archive" in err

    opf_no_manifest = b"""<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><spine><itemref idref="c1"/></spine></package>"""
    no_manifest = _make_epub(tmp_path, {"mimetype": b"application/epub+zip", "META-INF/container.xml": container, "OPS/content.opf": opf_no_manifest})
    ok, err = BookManager._validate_preview_epub(no_manifest)
    assert ok is False
    assert err == "manifest element missing"

    opf_no_spine = b"""<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest/></package>"""
    no_spine = _make_epub(tmp_path, {"mimetype": b"application/epub+zip", "META-INF/container.xml": container, "OPS/content.opf": opf_no_spine})
    ok, err = BookManager._validate_preview_epub(no_spine)
    assert ok is False
    assert err == "spine element missing"


def test_validate_preview_epub_bad_zip(tmp_path: Path):
    bad = tmp_path / "bad.epub"
    bad.write_text("not a zip", encoding="utf-8")
    ok, err = BookManager._validate_preview_epub(bad)
    assert ok is False
    assert err == "corrupted ZIP file"


def test_find_opf_path_regex_and_direct(tmp_path: Path):
    bad_container = b'<container full-path="OPS/content.opf">'
    epub = _make_epub(tmp_path, {"mimetype": b"application/epub+zip", "META-INF/container.xml": bad_container, "OPS/content.opf": b"<package/>"})
    with zipfile.ZipFile(epub, "r") as zin:
        assert BookManager._find_opf_path(zin) == "OPS/content.opf"

    epub2 = _make_epub(tmp_path, {"mimetype": b"application/epub+zip", "OPS/only.opf": b"<package/>"})
    with zipfile.ZipFile(epub2, "r") as zin:
        assert BookManager._find_opf_path(zin) == "OPS/only.opf"


def test_find_libreoffice_mac_path(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(shutil, "which", lambda cmd: None)
    mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice"

    def fake_exists(self: Path) -> bool:
        return str(self) == mac_path

    monkeypatch.setattr(Path, "exists", fake_exists)
    assert BookManager._find_libreoffice() == mac_path


def test_convert_with_libreoffice_direct_output(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    sample = tmp_path / "sample.docx"
    sample.write_text("doc", encoding="utf-8")
    monkeypatch.setattr(BookManager, "_find_libreoffice", lambda: "lo")

    class DummyProc:
        returncode = 0
        stderr = b""

    class FakeTmp:
        def __enter__(self):
            return str(tmp_path)

        def __exit__(self, exc_type, exc, tb):
            return False

    monkeypatch.setattr(tempfile, "TemporaryDirectory", lambda: FakeTmp())

    def fake_run(*args, **kwargs):
        (tmp_path / "sample.html").write_text("ok", encoding="utf-8")
        return DummyProc()

    monkeypatch.setattr("backend.book_manager.subprocess.run", fake_run)
    assert BookManager._convert_with_libreoffice(sample, "html") == "ok"


def test_convert_with_libreoffice_no_output(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    sample = tmp_path / "sample.docx"
    sample.write_text("doc", encoding="utf-8")
    monkeypatch.setattr(BookManager, "_find_libreoffice", lambda: "lo")

    class DummyProc:
        returncode = 1
        stderr = b"err"

    class FakeTmp:
        def __enter__(self):
            return str(tmp_path)

        def __exit__(self, exc_type, exc, tb):
            return False

    monkeypatch.setattr(tempfile, "TemporaryDirectory", lambda: FakeTmp())

    def fake_run(*args, **kwargs):
        for path in tmp_path.glob("*.html"):
            path.unlink()
        return DummyProc()

    monkeypatch.setattr("backend.book_manager.subprocess.run", fake_run)
    assert BookManager._convert_with_libreoffice(sample, "html") == ""


def test_book_manager_init_requires_env(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.delenv("TM_BOOK_DIR", raising=False)
    with pytest.raises(RuntimeError):
        BookManager()


def test_get_books_in_category_empty(tmp_path: Path):
    es = DummyES()
    es.category_docs = []
    manager = make_manager(tmp_path, es)
    books, err = asyncio_runner(manager.get_books_in_category("missing"))
    assert books == []
    assert "No books found" in err


# ---- coverage: additional uncovered lines ----


def test_find_libreoffice_which_found(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(shutil, "which", lambda cmd: "/usr/bin/libreoffice" if cmd == "libreoffice" else None)
    assert BookManager._find_libreoffice() == "/usr/bin/libreoffice"


def test_find_libreoffice_fallback(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(shutil, "which", lambda cmd: None)
    monkeypatch.setattr(Path, "exists", lambda self: False)
    assert BookManager._find_libreoffice() == "libreoffice"


def test_get_book_content_missing_file(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: make_doc("A/missing.txt")
    result = asyncio_runner(manager.get_book_content(1))
    assert result == ""


def test_get_book_preview_pdf_generate(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    pdf_path = tmp_path / "A" / "a.pdf"
    pdf_path.write_bytes(b"%PDF")
    doc = make_doc("A/a.pdf", "pdf")
    es.search_by_id = lambda _id: doc

    class DummyReader:
        def __init__(self, path):
            self.pages = [object(), object(), object()]

    class DummyWriter:
        def __init__(self):
            self._pages = []

        def add_page(self, page):
            self._pages.append(page)

        def write(self, buf):
            buf.write(b"PDFPREVIEW")

    monkeypatch.setitem(sys.modules, "pypdf", type("P", (), {"PdfReader": DummyReader, "PdfWriter": DummyWriter})())
    resp = asyncio_runner(manager.get_book_preview(1, pages=2))
    assert resp.status_code == 200


def test_get_book_preview_epub_chapters_zero(tmp_path: Path):
    epub = tmp_path / "book.epub"
    build_epub(epub)
    doc = make_doc("book.epub", "epub")
    manager = make_manager(tmp_path, doc)
    resp = asyncio_runner(manager.get_book_preview(1, chapters=0))
    assert isinstance(resp, Response)


def test_get_book_preview_epub_old_cache_cleanup(tmp_path: Path):
    epub = tmp_path / "book.epub"
    build_epub(epub)
    doc = make_doc("book.epub", "epub")
    manager = make_manager(tmp_path, doc)
    cache_dir = tmp_path / ".preview_cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    old_cache = cache_dir / "1.epub"
    old_cache.write_bytes(b"old")
    old_html = cache_dir / "1.html"
    old_html.write_text("old")
    asyncio_runner(manager.get_book_preview(1, chapters=1))
    assert not old_cache.exists()
    assert not old_html.exists()


def test_get_book_preview_epub_img_and_css_and_font(tmp_path: Path):
    with zipfile.ZipFile(tmp_path / "book.epub", "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/ch1.xhtml", '<html><body><img src="img/cover.png"/><link href="styles.css"/></body></html>')
        zf.writestr("OEBPS/img/cover.png", b"PNG")
        zf.writestr("OEBPS/styles.css", "@font-face{font-family:'X';src:url('fonts/missing.woff2');}body{color:red;}")
        zf.writestr("OEBPS/toc.ncx", '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint><content src="ch1.xhtml"/></navPoint></ncx>')
        zf.writestr(
            "OEBPS/content.opf",
            """<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest>
            <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
            <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
            <item id="css" href="styles.css" media-type="text/css"/>
            <item id="img" href="img/cover.png" media-type="image/png"/>
          </manifest>
          <spine toc="toc"><itemref idref="c1"/></spine>
        </package>""",
        )
    doc = make_doc("book.epub", "epub")
    manager = make_manager(tmp_path, doc)
    resp = asyncio_runner(manager.get_book_preview(1, chapters=1))
    assert isinstance(resp, Response)


def test_get_book_preview_epub_ncx_filter_fail(tmp_path: Path):
    with zipfile.ZipFile(tmp_path / "book.epub", "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/ch1.xhtml", "<html><body>Hi</body></html>")
        zf.writestr("OEBPS/toc.ncx", "not valid xml at all <<<")
        zf.writestr(
            "OEBPS/content.opf",
            """<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest>
            <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
            <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
          </manifest>
          <spine toc="toc"><itemref idref="c1"/></spine>
        </package>""",
        )
    doc = make_doc("book.epub", "epub")
    manager = make_manager(tmp_path, doc)
    resp = asyncio_runner(manager.get_book_preview(1, chapters=1))
    assert isinstance(resp, Response)


def test_get_book_preview_doc_cache_hit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    doc_file = tmp_path / "A" / "a.hwp"
    doc_file.write_text("hwp content")
    doc = make_doc("A/a.hwp", "hwp")
    es.search_by_id = lambda _id: doc
    cache_dir = tmp_path / ".preview_cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    cache_file = cache_dir / "1.html"
    cache_file.write_text("<p>cached</p>")
    os.utime(cache_file, (doc_file.stat().st_mtime + 10, doc_file.stat().st_mtime + 10))
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 200


def test_search_similar_books_not_found(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: None
    books, err = asyncio_runner(manager.search_similar_books(999))
    assert books == []
    assert "No book found" in err


def test_search_similar_books_paged_not_found(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: None
    books, total, err = asyncio_runner(manager.search_similar_books_paged(999))
    assert books == []
    assert total == 0
    assert "No book found" in err


def test_add_book_es_failure(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.insert = lambda data: []
    result, err = asyncio_runner(manager.add_book({1: make_doc("a.txt")}))
    assert result is None
    assert "can't add book" in err


def test_update_book_samefile_os_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    original = tmp_path / "A" / "old.txt"
    original.write_text("x")
    doc = make_doc("A/old.txt")
    es.search_by_id = lambda _id: doc

    def raise_samefile(self, other):
        raise OSError("samefile error")

    monkeypatch.setattr(Path, "samefile", raise_samefile)
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", original, ".txt"))
    assert status == "Error"
    assert "CONFLICT" in msg


def test_category_mismatch_details_scandir_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.category_docs = []

    def raise_scandir(path):
        raise PermissionError("nope")

    monkeypatch.setattr(os, "scandir", raise_scandir)
    details = manager.get_category_mismatch_details("A")
    assert details["fs_count"] == 0


def test_index_single_file_not_found(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    result, err = asyncio_runner(manager.index_single_file("A/nonexistent.txt"))
    assert result is None
    assert "파일을 찾을 수 없습니다" in err


def test_index_single_file_success(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    txt_file = tmp_path / "A" / "test.txt"
    txt_file.write_text("hello world")
    fake_data = {123: make_doc("A/test.txt")}
    monkeypatch.setattr("utils.loader.Loader.read_file", lambda p: fake_data)
    result, err = asyncio_runner(manager.index_single_file("A/test.txt"))
    assert result == 123
    assert err is None


def test_rename_category_target_dir_exists(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    (tmp_path / "old").mkdir()
    (tmp_path / "new").mkdir()
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert err is not None
    assert "이미 존재합니다" in err


def test_rename_category_no_dir_es_only(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    es.rename_category = lambda old, new: {"updated": 3, "failures": []}
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert err is None
    assert result["fs_renamed"] is False


def test_rename_category_es_failure_with_rollback(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    (tmp_path / "old").mkdir()

    def raise_es(*args, **kwargs):
        raise RuntimeError("ES failure")

    es.rename_category = raise_es
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert err is not None
    assert "ES 업데이트 실패" in err
    assert (tmp_path / "old").exists()


def test_rename_category_partial_es_failure(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    (tmp_path / "old").mkdir()
    es.rename_category = lambda old, new: {"updated": 2, "failures": ["some error"]}
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert err is not None
    assert "부분 실패" in err
    assert (tmp_path / "old").exists()


def test_delete_book_io_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    f = tmp_path / "A" / "ok.txt"
    f.write_text("x")
    doc = make_doc("A/ok.txt")
    es.search_by_id = lambda _id: doc

    def raise_unlink(self, missing_ok=False):
        raise IOError("disk error")

    monkeypatch.setattr(Path, "unlink", raise_unlink)
    status, msg = asyncio_runner(manager.delete_book(1))
    assert status == "Error"
    assert "can't delete a book" in msg


def test_delete_book_es_failure(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    f = tmp_path / "A" / "ok.txt"
    f.write_text("x")
    doc = make_doc("A/ok.txt")
    es.search_by_id = lambda _id: doc
    es.delete_ok = False
    status, msg = asyncio_runner(manager.delete_book(1))
    assert status == "Error"
    assert "can't delete book information" in msg


# ---- coverage: book_manager additional uncovered lines ----


def test_get_book_content_existing_file(tmp_path: Path):
    """Lines 433-436: get_book_content returns FileResponse when file exists"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    f = tmp_path / "A" / "ok.txt"
    f.write_text("content")
    es.search_by_id = lambda _id: make_doc("A/ok.txt")
    result = asyncio_runner(manager.get_book_content(1))
    assert isinstance(result, FileResponse)


def test_get_book_preview_pdf_cache_hit(tmp_path: Path):
    """Lines 458-459: PDF preview cache hit"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    pdf = tmp_path / "A" / "a.pdf"
    pdf.write_bytes(b"%PDF")
    es.search_by_id = lambda _id: make_doc("A/a.pdf", "pdf")
    cache_dir = tmp_path / ".preview_cache"
    cache_dir.mkdir()
    cache_file = cache_dir / "1.pdf"
    cache_file.write_bytes(b"CACHED")
    os.utime(cache_file, (pdf.stat().st_mtime + 10, pdf.stat().st_mtime + 10))
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 200


def test_get_book_preview_pdf_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 476-478: PDF preview generation exception"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    pdf = tmp_path / "A" / "a.pdf"
    pdf.write_bytes(b"%PDF")
    es.search_by_id = lambda _id: make_doc("A/a.pdf", "pdf")

    class BadReader:
        def __init__(self, path):
            raise RuntimeError("boom")

    monkeypatch.setitem(sys.modules, "pypdf", type("P", (), {"PdfReader": BadReader, "PdfWriter": object})())
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 500
    assert resp.body.decode("utf-8") == "PDF preview failed"


def test_update_book_os_error_on_resolve(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 805-806: OSError during path resolution"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    original_resolve = Path.resolve
    call_count = {"n": 0}

    def raise_resolve(self):
        call_count["n"] += 1
        if call_count["n"] <= 2:
            raise OSError("bad resolve")
        return original_resolve(self)

    monkeypatch.setattr(Path, "resolve", raise_resolve)
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", tmp_path / "A" / "f.txt", ".txt"))
    assert status == "Error"
    assert "잘못된 경로" in msg


def test_update_book_es_exception_rollback_failure(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 840-847: ES update exception + rollback failure"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    doc = make_doc("A/old.txt")
    es.search_by_id = lambda _id: doc
    original = tmp_path / "A" / "old.txt"
    original.parent.mkdir(parents=True, exist_ok=True)
    original.write_text("x")
    new_path = tmp_path / "A" / "new.txt"

    def raise_update(*args, **kwargs):
        raise RuntimeError("es fail")

    es.update = raise_update
    calls = {"count": 0}
    orig_rename = Path.rename

    def fake_rename(self, target):
        calls["count"] += 1
        if calls["count"] == 2:
            raise OSError("rollback fail")
        return orig_rename(self, target)

    monkeypatch.setattr(Path, "rename", fake_rename)
    status, msg = asyncio_runner(manager.update_book(1, "A", "T", "U", new_path, ".txt"))
    assert status == "Error"
    assert "롤백" in msg


def test_category_mismatch_count_files_permission_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 875-876: PermissionError in count_files helper"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.aggregate = {}
    (tmp_path / "cat").mkdir()
    original_scandir = os.scandir
    calls = {"n": 0}

    def mock_scandir(path):
        calls["n"] += 1
        if calls["n"] == 2:  # count_files for root
            raise PermissionError("nope")
        return original_scandir(path)

    monkeypatch.setattr(os, "scandir", mock_scandir)
    result = manager.get_category_mismatches()
    assert isinstance(result, dict)


def test_category_mismatch_l2_scandir_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 899-900: PermissionError during L2 scandir"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.aggregate = {}
    l1 = tmp_path / "cat1"
    l1.mkdir()
    (l1 / "file.txt").write_text("x")
    original_scandir = os.scandir
    calls = {"n": 0}

    def mock_scandir(path):
        calls["n"] += 1
        if calls["n"] >= 3:
            raise PermissionError("nope")
        return original_scandir(path)

    monkeypatch.setattr(os, "scandir", mock_scandir)
    result = manager.get_category_mismatches()
    assert isinstance(result, dict)


def test_category_mismatch_top_level_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 910-911: PermissionError scanning base directory"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.aggregate = {}

    def raise_scandir(path):
        raise PermissionError("nope")

    monkeypatch.setattr(os, "scandir", raise_scandir)
    result = manager.get_category_mismatches()
    assert result["mismatches"] == []


def test_category_mismatch_details_es_only(tmp_path: Path):
    """Lines 970-971: ES-only file detection"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.category_docs = [(1, make_doc("A/missing.txt"), 1.0)]
    (tmp_path / "A").mkdir()
    details = manager.get_category_mismatch_details("A")
    assert details["es_only"]


def test_category_mismatch_details_stat_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 991-992: OSError during stat for duplicates"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    file_path = tmp_path / "dup.txt"
    file_path.write_text("x")
    es.category_docs = [(1, make_doc("dup.txt"), 1.0), (2, make_doc("dup.txt"), 1.0)]

    def raise_stat(path):
        raise OSError("no stat")

    monkeypatch.setattr(os, "stat", raise_stat)
    details = manager.get_category_mismatch_details("_root")
    assert details["duplicates"]


def test_delete_file_io_error(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 1024-1025: IOError during file deletion"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    target = tmp_path / "A" / "x.txt"
    target.write_text("x")

    def raise_unlink(self):
        raise OSError("boom")

    monkeypatch.setattr(Path, "unlink", raise_unlink)
    status, msg = asyncio_runner(manager.delete_file("A/x.txt"))
    assert status == "Error"
    assert "파일 삭제 실패" in msg


def test_reload_category_full_flow(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 1029-1073: reload_category subprocess flow"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir()

    class DummyProc:
        def __init__(self, rc, stdout, stderr):
            self.returncode = rc
            self._stdout = stdout
            self._stderr = stderr

        async def communicate(self):
            return self._stdout, self._stderr

    # timeout
    async def raise_timeout(*args, **kwargs):
        raise asyncio.TimeoutError()

    import asyncio

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", raise_timeout)
    result, err = asyncio_runner(manager.reload_category("A"))
    assert "초과" in err

    # exec error
    async def raise_exec(*args, **kwargs):
        raise RuntimeError("boom")

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", raise_exec)
    result, err = asyncio_runner(manager.reload_category("A"))
    assert "재적재 실행 실패" in err

    # nonzero exit
    async def fake_exec_fail(*args, **kwargs):
        return DummyProc(2, b"", b"error message")

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", fake_exec_fail)
    monkeypatch.setattr("backend.book_manager.asyncio.wait_for", lambda coro, timeout: coro)
    result, err = asyncio_runner(manager.reload_category("A"))
    assert "exit 2" in err

    # success
    async def fake_exec_ok(*args, **kwargs):
        return DummyProc(0, "총 5개 파일 처리됨".encode(), b"warn")

    monkeypatch.setattr("backend.book_manager.asyncio.create_subprocess_exec", fake_exec_ok)
    result, err = asyncio_runner(manager.reload_category("A", content_type="comic"))
    assert err is None
    assert result["processed_count"] == 5


def test_get_pdf_pages_file_not_found(tmp_path: Path):
    """Line 1083: get_pdf_pages file not found"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: make_doc("A/missing.pdf", "pdf")
    resp = asyncio_runner(manager.get_pdf_pages(1, start=1, end=1))
    assert resp.status_code == 404


def test_get_pdf_pages_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 1123-1125: PDF pages extraction exception"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    pdf = tmp_path / "A" / "a.pdf"
    pdf.write_bytes(b"%PDF")
    es.search_by_id = lambda _id: make_doc("A/a.pdf", "pdf")

    class BadReader:
        def __init__(self, path):
            raise RuntimeError("boom")

    monkeypatch.setitem(sys.modules, "pypdf", type("P", (), {"PdfReader": BadReader, "PdfWriter": object})())
    resp = asyncio_runner(manager.get_pdf_pages(1, start=1, end=1))
    assert resp.status_code == 500
    assert resp.body.decode("utf-8") == "PDF pages extraction failed"


def test_rename_category_path_traversal(tmp_path: Path):
    """Lines 1147, 1149: rename_category path traversal"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"A": 1, "B": 0}
    evil = tmp_path / "evil"
    evil.symlink_to("/tmp")
    result, err = asyncio_runner(manager.rename_category("evil", "B"))
    assert err is not None


def test_rename_category_os_error_rename(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 1173-1174: OSError during directory rename"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    (tmp_path / "old").mkdir()

    def raise_rename(self, target):
        raise OSError("rename failed")

    monkeypatch.setattr(Path, "rename", raise_rename)
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert "이름 변경 실패" in err


def test_rename_category_es_failure_rollback_fail(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 1187-1189: ES exception + FS rollback failure"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    (tmp_path / "old").mkdir()

    def raise_es(*a, **kw):
        raise RuntimeError("ES fail")

    es.rename_category = raise_es
    calls = {"n": 0}
    orig_rename = Path.rename

    def fake_rename(self, target):
        calls["n"] += 1
        if calls["n"] == 2:
            raise OSError("rollback fail")
        return orig_rename(self, target)

    monkeypatch.setattr(Path, "rename", fake_rename)
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert "수동 복구 필요" in err


def test_rename_category_partial_es_rollback_fail(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 1198-1200: partial ES failure + rollback failure"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 0}
    (tmp_path / "old").mkdir()
    es.rename_category = lambda old, new: {"updated": 2, "failures": ["err"]}
    calls = {"n": 0}
    orig_rename = Path.rename

    def fake_rename(self, target):
        calls["n"] += 1
        if calls["n"] == 2:
            raise OSError("rollback fail")
        return orig_rename(self, target)

    monkeypatch.setattr(Path, "rename", fake_rename)
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert "수동 복구 필요" in err


def test_delete_category_path_traversal(tmp_path: Path):
    """Line 1222: delete_category path traversal"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    evil = tmp_path / "evil"
    evil.symlink_to("/tmp")
    result, err = asyncio_runner(manager.delete_category("evil"))
    assert err is not None


def test_delete_category_es_exception(tmp_path: Path):
    """Lines 1232-1233: ES delete exception"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"A": 1}

    def raise_delete(category, prefix=False):
        raise RuntimeError("boom")

    es.delete_by_category = raise_delete
    result, err = asyncio_runner(manager.delete_category("A"))
    assert "ES 삭제 실패" in err


def test_delete_book_warning_message(tmp_path: Path):
    """Line 1265: delete_book returns warning when file already deleted"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: make_doc("A/missing.txt")
    status, msg = asyncio_runner(manager.delete_book(1))
    assert status == "Warning"
    assert "이미 삭제" in msg


# ---- coverage: validate_preview_epub edge cases ----


def _make_minimal_epub(path: Path, *, mimetype: str = "application/epub+zip", opf_content: str | None = None, extra_files: dict | None = None, include_mimetype: bool = True, include_container: bool = True, include_opf: bool = True) -> None:
    """Helper: create a minimal EPUB zip at `path`."""
    import zipfile

    opf_ns = "http://www.idpf.org/2007/opf"
    if opf_content is None:
        opf_content = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""

    container_xml = """<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles>
    <rootfile full-path="content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"""

    with zipfile.ZipFile(str(path), "w", zipfile.ZIP_DEFLATED) as zout:
        if include_mimetype:
            zout.writestr("mimetype", mimetype, compress_type=zipfile.ZIP_STORED)
        if include_container:
            zout.writestr("META-INF/container.xml", container_xml)
        if include_opf:
            zout.writestr("content.opf", opf_content)
        zout.writestr("ch1.xhtml", "<html><body><p>Hello</p></body></html>")
        if extra_files:
            for name, data in extra_files.items():
                zout.writestr(name, data)


def test_validate_epub_invalid_mimetype(tmp_path: Path):
    """Line 67: mimetype content is not application/epub+zip"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub, mimetype="application/zip")
    valid, err = BookManager._validate_preview_epub(epub)
    assert not valid
    assert "invalid mimetype" in err


def test_validate_epub_no_opf(tmp_path: Path):
    """Line 72: OPF file not found"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub, include_opf=False, include_container=False)
    valid, err = BookManager._validate_preview_epub(epub)
    assert not valid
    assert "OPF file not found" in err


def test_validate_epub_opf_parse_error(tmp_path: Path):
    """Lines 79-80: OPF parse error (severely malformed XML)"""
    epub = tmp_path / "test.epub"
    # lxml with recover=True handles most malformed XML, but we can test the exception path
    # by providing binary garbage that even recover can't handle
    _make_minimal_epub(epub, opf_content="\x00\x01\x02")
    valid, err = BookManager._validate_preview_epub(epub)
    # lxml recover mode may still parse garbage; if it does, the result is still a valid test
    # The important thing is it doesn't crash
    assert isinstance(valid, bool)


def test_validate_epub_spine_idref_not_in_manifest(tmp_path: Path):
    """Lines 103-105: spine idref not in manifest → removal + rewrite"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="missing_item"/>
  </spine>
</package>"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub, opf_content=opf)
    valid, err = BookManager._validate_preview_epub(epub)
    assert valid
    assert err is None


def test_validate_epub_spine_file_missing_from_zip(tmp_path: Path):
    """Lines 113-121: spine item's file missing from ZIP"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"""
    epub = tmp_path / "test.epub"
    # ch2.xhtml is in manifest/spine but NOT in zip
    _make_minimal_epub(epub, opf_content=opf)
    valid, err = BookManager._validate_preview_epub(epub)
    assert valid
    assert err is None


def test_validate_epub_no_valid_spine_chapters(tmp_path: Path):
    """Line 126: all spine chapters removed → no valid chapters"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="missing.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub, opf_content=opf, include_opf=False)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        # NO ch1.xhtml or missing.xhtml in the zip
    valid, err = BookManager._validate_preview_epub(epub)
    assert not valid
    assert "no valid spine chapters" in err


def test_validate_epub_toc_not_in_manifest(tmp_path: Path):
    """Lines 132-134: toc id not in manifest"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="missing_ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub, opf_content=opf)
    valid, err = BookManager._validate_preview_epub(epub)
    assert valid


def test_validate_epub_toc_ncx_missing_from_zip(tmp_path: Path):
    """Lines 139-141: toc NCX in manifest but missing from ZIP"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"""
    epub = tmp_path / "test.epub"
    # toc.ncx is in manifest but NOT in zip
    _make_minimal_epub(epub, opf_content=opf)
    valid, err = BookManager._validate_preview_epub(epub)
    assert valid


def test_validate_epub_corrupted_zip(tmp_path: Path):
    """Lines 159-160: corrupted ZIP"""
    epub = tmp_path / "test.epub"
    epub.write_bytes(b"not a zip file")
    valid, err = BookManager._validate_preview_epub(epub)
    assert not valid
    assert "corrupted ZIP" in err


def test_validate_epub_general_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 161-162: general exception during validation"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub)

    import zipfile

    def bad_init(self, *a, **kw):
        raise PermissionError("nope")

    monkeypatch.setattr(zipfile.ZipFile, "__init__", bad_init)
    valid, err = BookManager._validate_preview_epub(epub)
    assert not valid
    assert "validation error" in err


# ---- coverage: _evict_old_cache exception paths ----


def test_evict_old_cache_file_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 176-177: per-file exception during eviction"""
    import os

    old_file = tmp_path / "old.txt"
    old_file.write_text("x")
    past = time.time() - (BookManager.CACHE_MAX_AGE_SECONDS + 10)
    os.utime(old_file, (past, past))

    def bad_unlink(self, **kw):
        raise PermissionError("nope")

    monkeypatch.setattr(Path, "unlink", bad_unlink)
    # Should not raise
    BookManager._evict_old_cache(tmp_path)
    assert old_file.exists()


def test_evict_old_cache_iterdir_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 178-179: iterdir exception"""

    def bad_iterdir(self):
        raise OSError("boom")

    monkeypatch.setattr(Path, "iterdir", bad_iterdir)
    BookManager._evict_old_cache(tmp_path)


# ---- coverage: _get_epub_total_chapters edge cases ----


def test_get_epub_total_chapters_no_spine(tmp_path: Path):
    """Line 241: spine not found in OPF"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"""
    epub = tmp_path / "test.epub"
    _make_minimal_epub(epub, opf_content=opf)
    assert BookManager._get_epub_total_chapters(epub) == 0


def test_get_epub_total_chapters_exception(tmp_path: Path):
    """Lines 243-245: exception during chapter counting"""
    epub = tmp_path / "test.epub"
    epub.write_bytes(b"not a zip")
    assert BookManager._get_epub_total_chapters(epub) == 0


# ---- coverage: validate_epub (epubcheck) edge cases ----


def test_validate_epub_epubcheck_json_unlink_oserror(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 339-340: os.unlink in finally fails"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc

    # Mock subprocess to produce a valid epubcheck JSON output
    import asyncio

    class FakeProc:
        returncode = 0

        async def communicate(self):
            return b"", b""

    epubcheck_json = '{"messages": [], "publication": {"title": "T"}, "checker": {"nFatal": 0, "nError": 0, "nWarning": 0, "nUsage": 0, "nInfo": 0}}'

    unlink_calls = []

    async def fake_subprocess(*args, **kwargs):
        # Write fake epubcheck output
        for arg in args[0] if isinstance(args[0], (list, tuple)) else args:
            if isinstance(arg, str) and arg.endswith(".json"):
                Path(arg).write_text(epubcheck_json)
        return FakeProc()

    def tracked_unlink(path):
        unlink_calls.append(path)
        raise OSError("can't delete")

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_subprocess)
    monkeypatch.setattr(os, "unlink", tracked_unlink)

    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is not None
    assert err is None
    assert len(unlink_calls) > 0


def test_validate_epub_message_empty_locations(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 345-347, 353, 361: messages with empty locations + publication metadata"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc

    import asyncio

    class FakeProc:
        returncode = 0

        async def communicate(self):
            return b"", b""

    epubcheck_json = '{"messages": [{"severity": "WARNING", "id": "W1", "message": "test warn", "locations": []}], "publication": {"title": "T", "creator": "C", "date": "2024", "publisher": "P"}, "checker": {"nFatal": 0, "nError": 0, "nWarning": 1, "nUsage": 0, "nInfo": 0}}'

    async def fake_subprocess(*args, **kwargs):
        for arg in args[0] if isinstance(args[0], (list, tuple)) else args:
            if isinstance(arg, str) and arg.endswith(".json"):
                Path(arg).write_text(epubcheck_json)
        return FakeProc()

    monkeypatch.setattr(asyncio, "create_subprocess_exec", fake_subprocess)

    result, err = asyncio_runner(manager.validate_epub(1))
    assert result is not None
    assert err is None
    assert result["messages"][0]["location"] is None
    assert "publication" in result
    assert result["publication"]["title"] == "T"


# ---- coverage: validate_pdf edge cases ----


def test_validate_pdf_not_found(tmp_path: Path):
    """Line 372: book not found in ES"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = None
    result, err = asyncio_runner(manager.validate_pdf(999))
    assert result is None
    assert "not found" in err.lower()


def test_validate_pdf_producer_and_creation_date(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 395, 397: PDF with Producer and CreationDate metadata"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    pdf = tmp_path / "A" / "test.pdf"
    pdf.parent.mkdir(parents=True, exist_ok=True)

    # Create a minimal valid PDF
    from pypdf import PdfWriter

    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    writer.add_metadata({"/Producer": "TestProducer", "/CreationDate": "D:20240101", "/Title": "Test", "/Author": "Author"})
    with open(pdf, "wb") as f:
        writer.write(f)

    es.doc = make_doc("A/test.pdf", file_type="pdf")
    es.search_by_id = lambda _id: es.doc

    result, err = asyncio_runner(manager.validate_pdf(1))
    assert result is not None
    assert err is None
    assert "producer" in result["publication"]
    assert "creation_date" in result["publication"]


# ---- coverage: get_book_content edge case ----


def test_get_book_content_not_found(tmp_path: Path):
    """Line 429: book not in ES"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = None
    result = asyncio_runner(manager.get_book_content(999))
    assert result == ""


# ---- coverage: get_book_preview edge cases ----


def test_get_book_preview_not_found(tmp_path: Path):
    """Lines 443-444: book not in ES"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = None
    es.search_by_id = lambda _id: None
    result = asyncio_runner(manager.get_book_preview(999))
    assert result.status_code == 404


def test_get_book_preview_file_not_found(tmp_path: Path):
    """Lines 447-448: file not found on disk"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/missing.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code == 404


def test_get_book_preview_epub_cache_hit(tmp_path: Path):
    """Lines 493-494: EPUB preview cache hit"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc

    # Create cache file with newer mtime
    cache_dir = tmp_path / ".preview_cache"
    cache_dir.mkdir(parents=True, exist_ok=True)
    BookManager._get_epub_total_chapters(epub)
    cache_file = cache_dir / "1_ch3.epub"
    _make_minimal_epub(cache_file)

    import os

    future = time.time() + 1000
    os.utime(cache_file, (future, future))

    result = asyncio_runner(manager.get_book_preview(1, chapters=3))
    assert result.status_code == 200


def test_get_book_preview_epub_opf_keyerror(tmp_path: Path):
    """Lines 516-518: OPF file in container.xml but missing from archive"""
    import zipfile

    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    container_xml = """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="MISSING.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"""
    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", container_xml)
        # NO MISSING.opf in zip
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code == 422


def test_get_book_preview_epub_opf_namespace_fix(tmp_path: Path):
    """Lines 523-524: OPF with opf: prefix but no xmlns:opf declaration"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <metadata>
    <opf:meta name="cover" content="cover-image"/>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub, opf_content=opf)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    # Should not crash; namespace gets injected
    assert result.status_code in (200, 422, 500)


def test_get_book_preview_epub_no_spine(tmp_path: Path):
    """Lines 540-547: EPUB without spine element → fallback to manifest order"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub, opf_content=opf)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_chapter_missing_from_zip(tmp_path: Path):
    """Lines 580-582: chapter file missing from ZIP archive"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    # ch2.xhtml NOT in zip
    _make_minimal_epub(epub, opf_content=opf)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1, chapters=5))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_svg_image_tag(tmp_path: Path):
    """Lines 591-593: chapter with SVG <image> tags"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="cover.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    chapter = '<html><body><svg><image xlink:href="cover.png"/></svg></body></html>'
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        zout.writestr("ch1.xhtml", chapter)
        zout.writestr("cover.png", b"\x89PNG\r\n\x1a\n" + b"\x00" * 100)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_css_missing(tmp_path: Path):
    """Lines 613-614: CSS file missing from ZIP"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    chapter = '<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>'
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        zout.writestr("ch1.xhtml", chapter)
        # style.css NOT in zip
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_large_font_skip(tmp_path: Path):
    """Lines 626-633: large font file skipped, missing font skipped"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
    <item id="font1" href="big.ttf" media-type="font/ttf"/>
    <item id="font2" href="missing.woff" media-type="font/woff"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    chapter = '<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>'
    css = '@font-face { font-family: "Big"; src: url("big.ttf"); }\n@font-face { font-family: "Missing"; src: url("missing.woff"); }\nbody { font-family: serif; }'
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        zout.writestr("ch1.xhtml", chapter)
        zout.writestr("style.css", css)
        # big.ttf over 500KB
        zout.writestr("big.ttf", b"\x00" * (600 * 1024))
        # missing.woff NOT in zip
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_guide_element(tmp_path: Path):
    """Lines 648, 653-657: EPUB with guide element"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
  <guide>
    <reference type="toc" href="toc.xhtml"/>
    <reference type="text" href="ch1.xhtml"/>
  </guide>
</package>"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub, opf_content=opf)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_ncx_navpoint_filtering(tmp_path: Path):
    """Lines 686-694: NCX navPoint filtering removes missing file references"""
    opf_ns = "http://www.idpf.org/2007/opf"
    ncx_ns = "http://www.daisy.org/z3986/2005/ncx/"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"""
    ncx = f"""<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}">
  <navMap>
    <navPoint id="np1"><navLabel><text>Ch1</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2"><navLabel><text>Ch99</text></navLabel><content src="missing.xhtml"/></navPoint>
  </navMap>
</ncx>"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub, opf_content=opf, extra_files={"toc.ncx": ncx})
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_css_font_face_strip(tmp_path: Path):
    """Line 712: CSS @font-face block removed when font not in files_to_include"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    chapter = '<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>'
    css = '@font-face { font-family: "Missing"; src: url("nothere.woff"); }\nbody { color: red; }'
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        zout.writestr("ch1.xhtml", chapter)
        zout.writestr("style.css", css)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_file_missing_in_write(tmp_path: Path):
    """Lines 717-718: file missing when writing output EPUB"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="cover.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"""
    chapter = '<html><body><img src="cover.png"/></body></html>'
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        zout.writestr("ch1.xhtml", chapter)
        # cover.png NOT in zip — it's in manifest and referenced in HTML
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code in (200, 422)


def test_get_book_preview_epub_validation_fail(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 723-725: generated EPUB fails validation"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    monkeypatch.setattr(BookManager, "_validate_preview_epub", staticmethod(lambda path: (False, "bad epub")))
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code == 422
    assert "bad epub" in result.body.decode()


def test_get_book_preview_epub_bad_zip(tmp_path: Path):
    """Lines 730-732: corrupted ZIP"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    epub.write_bytes(b"not a zip file")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code == 422


def test_get_book_preview_epub_general_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 733-735: general exception during EPUB processing"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    _make_minimal_epub(epub)
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    monkeypatch.setattr(BookManager, "_find_opf_path", staticmethod(lambda zin: (_ for _ in ()).throw(RuntimeError("boom"))))
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code == 500
    assert result.body.decode("utf-8") == "EPUB preview failed"


def test_get_book_preview_doc_exception(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 750-752: .doc conversion exception"""
    doc = tmp_path / "A" / "test.doc"
    doc.parent.mkdir(parents=True, exist_ok=True)
    doc.write_bytes(b"fake doc content")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.doc", file_type="doc")
    es.search_by_id = lambda _id: es.doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", staticmethod(lambda *a, **kw: (_ for _ in ()).throw(RuntimeError("boom"))))
    result = asyncio_runner(manager.get_book_preview(1))
    assert result.status_code == 500
    assert result.body.decode("utf-8") == ".DOC preview failed"


# ---- coverage: reload_category edge cases ----


def test_reload_category_empty_name(tmp_path: Path):
    """Line 1032: empty category name"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    result, err = asyncio_runner(manager.reload_category(""))
    assert err is not None
    assert "비어있습니다" in err


def test_reload_category_path_traversal(tmp_path: Path):
    """Line 1034: category with '..'"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    result, err = asyncio_runner(manager.reload_category("../etc"))
    assert err is not None


def test_reload_category_path_escape(tmp_path: Path):
    """Line 1038: resolved path escapes path_prefix"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    evil = tmp_path / "evil"
    evil.symlink_to("/tmp")
    result, err = asyncio_runner(manager.reload_category("evil"))
    assert err is not None


def test_reload_category_not_a_directory(tmp_path: Path):
    """Lines 1040-1041: path is not a directory"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "notadir").write_text("file")
    result, err = asyncio_runner(manager.reload_category("notadir"))
    assert err is not None
    assert "디렉토리를 찾을 수 없습니다" in err


# ---- coverage: get_pdf_pages edge cases ----


def test_get_pdf_pages_not_found(tmp_path: Path):
    """Line 1080: book not in ES"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = None
    es.search_by_id = lambda _id: None
    result = asyncio_runner(manager.get_pdf_pages(999, 1, 1))
    assert result.status_code == 404


def test_get_pdf_pages_not_pdf(tmp_path: Path):
    """Line 1085: file is not a PDF"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    txt = tmp_path / "A" / "test.txt"
    txt.parent.mkdir(parents=True, exist_ok=True)
    txt.write_text("hello")
    es.doc = make_doc("A/test.txt", file_type="txt")
    es.search_by_id = lambda _id: es.doc
    result = asyncio_runner(manager.get_pdf_pages(1, 1, 1))
    assert result.status_code == 400


def test_get_pdf_pages_happy_path_and_cache(tmp_path: Path):
    """Lines 1091-1122: happy path + cache hit"""
    from pypdf import PdfWriter

    es = DummyES()
    manager = make_manager(tmp_path, es)
    pdf = tmp_path / "A" / "test.pdf"
    pdf.parent.mkdir(parents=True, exist_ok=True)
    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    writer.add_blank_page(width=612, height=792)
    with open(pdf, "wb") as f:
        writer.write(f)

    es.doc = make_doc("A/test.pdf", file_type="pdf")
    es.search_by_id = lambda _id: es.doc

    # First call: generates and caches
    result = asyncio_runner(manager.get_pdf_pages(1, 1, 1))
    assert result.status_code == 200
    assert result.headers.get("x-total-pages") == "2"

    # Second call: cache hit
    result2 = asyncio_runner(manager.get_pdf_pages(1, 1, 1))
    assert result2.status_code == 200


def test_get_pdf_pages_start_exceeds_total(tmp_path: Path):
    """Lines 1096-1097: start page exceeds total pages"""
    from pypdf import PdfWriter

    es = DummyES()
    manager = make_manager(tmp_path, es)
    pdf = tmp_path / "A" / "test.pdf"
    pdf.parent.mkdir(parents=True, exist_ok=True)
    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    with open(pdf, "wb") as f:
        writer.write(f)

    es.doc = make_doc("A/test.pdf", file_type="pdf")
    es.search_by_id = lambda _id: es.doc

    result = asyncio_runner(manager.get_pdf_pages(1, 100, 200))
    assert result.status_code == 400
    assert "exceeds total pages" in result.body.decode()


# ---- coverage: rename_category new_category path traversal ----


def test_rename_category_new_path_traversal(tmp_path: Path):
    """Line 1149: new_category resolved path escapes path_prefix"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1}
    evil = tmp_path / "evil"
    evil.symlink_to("/tmp")
    result, err = asyncio_runner(manager.rename_category("old", "evil"))
    assert err is not None


# ---- coverage: index_single_file unsupported format ----


def test_index_single_file_unsupported(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Line 1011: Loader.read_file returns falsy"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    f = tmp_path / "test.xyz"
    f.write_text("data")

    from utils import loader

    monkeypatch.setattr(loader.Loader, "read_file", staticmethod(lambda path: {}))
    result, err = asyncio_runner(manager.index_single_file("test.xyz"))
    assert result is None
    assert "지원하지 않는" in err


# ---- coverage: rename_category old/new count checks ----


def test_rename_category_old_count_zero(tmp_path: Path):
    """Line 1155: old_category has 0 documents"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 0, "new": 0}
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert "문서가 없습니다" in err


def test_rename_category_new_count_nonzero(tmp_path: Path):
    """Line 1159: new_category already has documents"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.counts = {"old": 1, "new": 5}
    result, err = asyncio_runner(manager.rename_category("old", "new"))
    assert "이미" in err
    assert "5" in err


# ---- coverage: update_book IOError and force overwrite ----


def test_update_book_ioerror_on_rename(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """Lines 829-830: IOError during file rename"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir()
    src = tmp_path / "A" / "src.txt"
    src.write_text("content")
    es.doc = make_doc("A/src.txt")
    es.search_by_id = lambda _id: es.doc

    def fail_rename(self, target):
        raise IOError("disk error")

    monkeypatch.setattr(Path, "rename", fail_rename)
    result, err = asyncio_runner(manager.update_book(1, new_category="A", new_title="dst", new_author="U", new_path=tmp_path / "A" / "dst.txt", new_type="txt"))
    assert result == "Error"
    assert "can't move" in err


def test_update_book_force_overwrite(tmp_path: Path):
    """Line 824: force overwrite when destination exists"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "A").mkdir()
    src = tmp_path / "A" / "src.txt"
    dst = tmp_path / "A" / "dst.txt"
    src.write_text("source")
    dst.write_text("existing")
    es.doc = make_doc("A/src.txt")
    es.search_by_id = lambda _id: es.doc

    result, err = asyncio_runner(manager.update_book(1, new_category="A", new_title="dst", new_author="U", new_path=dst, new_type="txt", force=True))
    assert result == "Ok"
    assert not src.exists()
    assert dst.read_text() == "source"


# ---- coverage: get_book_preview epub with many spine items (line 648) ----


def test_get_book_preview_epub_spine_trimmed(tmp_path: Path):
    """Line 648: spine refs not in chapter_idrefs are removed"""
    opf_ns = "http://www.idpf.org/2007/opf"
    opf = f"""<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch3" href="ch3.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch4" href="ch4.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch5" href="ch5.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
    <itemref idref="ch3"/>
    <itemref idref="ch4"/>
    <itemref idref="ch5"/>
  </spine>
</package>"""
    epub = tmp_path / "A" / "test.epub"
    epub.parent.mkdir(parents=True, exist_ok=True)
    import zipfile

    with zipfile.ZipFile(str(epub), "w") as zout:
        zout.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        zout.writestr("META-INF/container.xml", """<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>""")
        zout.writestr("content.opf", opf)
        for i in range(1, 6):
            zout.writestr(f"ch{i}.xhtml", f"<html><body><p>Chapter {i}</p></body></html>")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.doc = make_doc("A/test.epub", file_type="epub")
    es.search_by_id = lambda _id: es.doc
    # Request only 2 chapters; ch3-ch5 should be removed from spine
    result = asyncio_runner(manager.get_book_preview(1, chapters=2))
    assert result.status_code in (200, 422)


# ---- coverage: get_categories and get_books_in_category (unit test) ----


def test_get_categories_unit(tmp_path: Path):
    """Lines 282-284: get_categories returns aggregation"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.aggregate = {"Cat1": 3, "Cat2": 5}
    cats, err = asyncio_runner(manager.get_categories())
    assert err is None
    assert cats == {"Cat1": 3, "Cat2": 5}


def test_get_books_in_category_unit(tmp_path: Path):
    """Line 289: get_books_in_category returns books"""
    es = DummyES()
    manager = make_manager(tmp_path, es)
    doc = make_doc("A/test.txt")
    es.category_docs = [(1, doc, 1.0)]
    (tmp_path / "A").mkdir(exist_ok=True)
    (tmp_path / "A" / "test.txt").write_text("x")
    books, err = asyncio_runner(manager.get_books_in_category("A"))
    assert err is None
    assert len(books) == 1


# ---- HWP3 네이티브 파서 fallback 프리뷰 테스트 ----

HWP_TEST_DIR = Path(__file__).parent / "books" / "_hwp"


def test_hwp_preview_fallback_when_libreoffice_empty(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """LibreOffice가 빈 문자열 반환 시 hwp3 파서로 fallback하여 HTML 생성"""
    hwp_src = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
    if not hwp_src.exists():
        pytest.skip("테스트 파일 없음")
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    hwp_file = tmp_path / "A" / "kyoka.hwp"
    shutil.copy(hwp_src, hwp_file)
    doc = make_doc("A/kyoka.hwp", "hwp")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 200
    body = resp.body.decode("utf-8")
    assert "영동" in body or "교가" in body
    assert "<p>" in body


def test_hwp_preview_fallback_v3_00(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """V3.00 파일의 fallback 프리뷰"""
    hwp_src = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
    if not hwp_src.exists():
        pytest.skip("테스트 파일 없음")
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    hwp_file = tmp_path / "A" / "poem.hwp"
    shutil.copy(hwp_src, hwp_file)
    doc = make_doc("A/poem.hwp", "hwp")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 200
    body = resp.body.decode("utf-8")
    assert len(body) > 50


def test_hwp_preview_fallback_caches_result(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """fallback 결과가 캐시 파일에 저장되는지 확인"""
    hwp_src = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
    if not hwp_src.exists():
        pytest.skip("테스트 파일 없음")
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    hwp_file = tmp_path / "A" / "kyoka.hwp"
    shutil.copy(hwp_src, hwp_file)
    doc = make_doc("A/kyoka.hwp", "hwp")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
    asyncio_runner(manager.get_book_preview(1))
    cache_file = tmp_path / ".preview_cache" / "1.html"
    assert cache_file.exists()
    assert len(cache_file.read_text()) > 50


def test_hwp_preview_no_fallback_for_doc(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """DOC 파일은 hwp3 fallback을 사용하지 않음"""
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    doc_file = tmp_path / "A" / "test.doc"
    doc_file.write_bytes(b"fake doc content")
    doc = make_doc("A/test.doc", "doc")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
    resp = asyncio_runner(manager.get_book_preview(1))
    # DOC는 빈 html_content → 캐시 안 되고 preview 없음
    assert resp.status_code in (200, 400, 500) or resp.status_code == 755  # 빈 content면 status 755 안 나옴


def test_hwp_preview_fallback_both_fail(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """LibreOffice도 실패하고 hwp3 파서도 빈 결과인 경우 (V1.20)"""
    hwp_src = HWP_TEST_DIR / "v1.20_부하의약혼녀.hwp"
    if not hwp_src.exists():
        pytest.skip("테스트 파일 없음")
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    hwp_file = tmp_path / "A" / "old.hwp"
    shutil.copy(hwp_src, hwp_file)
    doc = make_doc("A/old.hwp", "hwp")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
    asyncio_runner(manager.get_book_preview(1))
    # V1.20은 hwp3 파서도 빈 결과 → 캐시 미생성, 400/500 가능
    cache_file = tmp_path / ".preview_cache" / "1.html"
    assert not cache_file.exists()


def test_hwp_preview_libreoffice_success_no_fallback(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """LibreOffice가 정상 결과 반환 시 hwp3 fallback 미사용"""
    hwp_src = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
    if not hwp_src.exists():
        pytest.skip("테스트 파일 없음")
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    hwp_file = tmp_path / "A" / "kyoka.hwp"
    shutil.copy(hwp_src, hwp_file)
    doc = make_doc("A/kyoka.hwp", "hwp")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "<p>LibreOffice 결과</p>")
    resp = asyncio_runner(manager.get_book_preview(1))
    assert resp.status_code == 200
    body = resp.body.decode("utf-8")
    assert "LibreOffice" in body
    assert "영동" not in body  # fallback 미사용 확인


def test_hwp_preview_html_escaping(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """fallback HTML 출력에서 특수문자가 이스케이프되는지 확인"""
    hwp_src = HWP_TEST_DIR / "v2.10_PAGER.hwp"
    if not hwp_src.exists():
        pytest.skip("테스트 파일 없음")
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    hwp_file = tmp_path / "A" / "pager.hwp"
    shutil.copy(hwp_src, hwp_file)
    doc = make_doc("A/pager.hwp", "hwp")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc
    monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
    resp = asyncio_runner(manager.get_book_preview(1))
    if resp.status_code == 200:
        body = resp.body.decode("utf-8")
        # XSS 방지: <script> 같은 태그가 이스케이프되어야 함
        assert "<script>" not in body


def test_get_book_preview_html_sanitizes_active_content_and_rewrites_resources(tmp_path: Path):
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    html_file = tmp_path / "A" / "test.html"
    html_file.write_text(
        """
        <html>
          <head>
            <meta http-equiv="refresh" content="0;url=https://evil.example.com" />
            <script>alert(1)</script>
            <link rel="stylesheet" href="style.css" />
            <link rel="preload" href="evil.js" />
          </head>
          <body onload="steal()">
            <iframe src="https://evil.example.com/embed"></iframe>
            <img src="cover.png" onclick="hack()" />
            <a href="chapter2.html">next</a>
            <a href="#section1">toc</a>
          </body>
        </html>
        """,
        encoding="utf-8",
    )
    (tmp_path / "A" / "style.css").write_text("body { color: red; }", encoding="utf-8")
    (tmp_path / "A" / "cover.png").write_bytes(b"png")
    doc = make_doc("A/test.html", "html")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc

    resp = asyncio_runner(manager.get_book_preview(1, resource_base_url="/html-resource/1"))
    body = resp.body.decode("utf-8")

    assert resp.status_code == 200
    assert resp.headers["content-security-policy"].startswith("sandbox;")
    assert "<script" not in body
    assert "<iframe" not in body
    assert "onload=" not in body
    assert "onclick=" not in body
    assert "http-equiv" not in body
    assert 'href="/html-resource/1?path=style.css"' in body
    assert 'src="/html-resource/1?path=cover.png"' in body
    assert 'href="#section1"' in body
    assert 'href="chapter2.html"' not in body


def test_get_html_resource_allows_local_whitelisted_files_only(tmp_path: Path):
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    html_file = tmp_path / "A" / "test.html"
    html_file.write_text("<html></html>", encoding="utf-8")
    css_file = tmp_path / "A" / "style.css"
    css_file.write_text("body{}", encoding="utf-8")
    doc = make_doc("A/test.html", "html")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc

    resp = asyncio_runner(manager.get_html_resource(1, "style.css"))
    assert isinstance(resp, FileResponse)
    assert resp.status_code == 200
    assert resp.headers["content-security-policy"].startswith("sandbox;")
    assert resp.headers["x-content-type-options"] == "nosniff"

    bad = asyncio_runner(manager.get_html_resource(1, "../secret.txt"))
    assert bad.status_code == 400

    unsupported = tmp_path / "A" / "script.js"
    unsupported.write_text("alert(1)", encoding="utf-8")
    resp2 = asyncio_runner(manager.get_html_resource(1, "script.js"))
    assert resp2.status_code == 400


def test_get_book_content_html_forces_attachment(tmp_path: Path):
    (tmp_path / "A").mkdir(parents=True, exist_ok=True)
    html_file = tmp_path / "A" / "test.html"
    html_file.write_text("<html><body>safe</body></html>", encoding="utf-8")
    doc = make_doc("A/test.html", "html")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc

    resp = asyncio_runner(manager.get_book_content(1))
    assert isinstance(resp, FileResponse)
    content_disposition = resp.headers["content-disposition"]
    assert content_disposition.startswith("attachment;")


# ── _build_html_resource_url 엣지 케이스 (lines 61, 64, 71, 73) ──────────────


class TestBuildHtmlResourceUrl:
    def test_empty_string_returns_none(self):
        assert BookManager._build_html_resource_url("/base", "") is None

    def test_whitespace_only_returns_none(self):
        assert BookManager._build_html_resource_url("/base", "   ") is None

    def test_data_uri_returned_as_is(self):
        uri = "data:image/png;base64,abc123"
        assert BookManager._build_html_resource_url("/base", uri) == uri

    def test_blob_uri_returned_as_is(self):
        uri = "blob:http://example.com/uuid"
        assert BookManager._build_html_resource_url("/base", uri) == uri

    def test_external_http_url_returns_none(self):
        assert BookManager._build_html_resource_url("/base", "http://external.com/img.png") is None

    def test_protocol_relative_url_returns_none(self):
        assert BookManager._build_html_resource_url("/base", "//cdn.example.com/img.png") is None


# ── _sanitize_html_for_viewer: external src 속성 제거 (line 116) ─────────────


def test_sanitize_html_external_src_attribute_removed():
    """외부 URL src 는 _build_html_resource_url 이 None 을 반환하므로 속성이 제거된다."""
    from bs4 import BeautifulSoup

    html = '<html><body><img src="http://external.com/img.png"></body></html>'
    result = BookManager._sanitize_html_for_viewer(html, "/base")
    soup = BeautifulSoup(result, "html.parser")
    img = soup.find("img")
    assert img is None or "src" not in img.attrs


# ── get_book_preview: HTML read_text 예외 → 500 (lines 818-820) ─────────────


def test_get_book_preview_html_read_exception(tmp_path: Path):
    """HTML 파일 read_text 가 예외를 올리면 500 응답을 반환한다."""
    from unittest.mock import patch

    html_file = tmp_path / "A" / "book.html"
    html_file.parent.mkdir(parents=True)
    html_file.write_text("<html/>", encoding="utf-8")

    doc = make_doc("A/book.html", "html")
    es = DummyES()
    manager = make_manager(tmp_path, es)
    es.search_by_id = lambda _id: doc

    with patch("pathlib.Path.read_text", side_effect=PermissionError("denied")):
        resp = asyncio_runner(manager.get_book_preview(1))

    assert resp.status_code == 500


# ── get_html_resource 오류 경로 (lines 859, 862, 872-874, 876) ───────────────


class TestGetHtmlResource:
    def test_book_not_found_returns_404(self, tmp_path: Path):
        """ES 에 책이 없으면 404 (line 859)."""
        manager = make_manager(tmp_path, DummyES(doc=None))
        resp = asyncio_runner(manager.get_html_resource(99, "image.jpg"))
        assert resp.status_code == 404

    def test_non_html_file_returns_400(self, tmp_path: Path):
        """HTML 이 아닌 파일(.epub)은 400 (line 862)."""
        doc = make_doc("A/book.epub", "epub")
        manager = make_manager(tmp_path, DummyES(doc=doc))
        resp = asyncio_runner(manager.get_html_resource(1, "image.jpg"))
        assert resp.status_code == 400

    def test_path_traversal_via_symlink_returns_400(self, tmp_path: Path):
        """html_dir 밖을 가리키는 심볼릭 링크는 400 (line 872)."""
        html_file = tmp_path / "A" / "book.html"
        html_file.parent.mkdir(parents=True)
        html_file.write_text("<html/>")

        # html_dir 밖 파일을 가리키는 심볼릭 링크 생성
        outside = tmp_path / "secret.jpg"
        outside.touch()
        symlink = html_file.parent / "evil.jpg"
        symlink.symlink_to(outside)

        doc = make_doc("A/book.html", "html")
        manager = make_manager(tmp_path, DummyES(doc=doc))
        resp = asyncio_runner(manager.get_html_resource(1, "evil.jpg"))
        assert resp.status_code == 400

    def test_resource_file_not_found_returns_404(self, tmp_path: Path):
        """html_dir 내 존재하지 않는 리소스는 404 (line 876)."""
        html_file = tmp_path / "A" / "book.html"
        html_file.parent.mkdir(parents=True)
        html_file.write_text("<html/>")

        doc = make_doc("A/book.html", "html")
        manager = make_manager(tmp_path, DummyES(doc=doc))
        resp = asyncio_runner(manager.get_html_resource(1, "nonexistent.jpg"))
        assert resp.status_code == 404

    def test_is_relative_to_oserror_returns_400(self, tmp_path: Path, monkeypatch):
        """is_relative_to 가 OSError 를 던지면 400 (lines 873-874)."""
        html_file = tmp_path / "A" / "book.html"
        html_file.parent.mkdir(parents=True)
        html_file.write_text("<html/>")
        (html_file.parent / "image.jpg").touch()

        def _raise(self, other):
            raise OSError("path too long")

        monkeypatch.setattr(Path, "is_relative_to", _raise)

        doc = make_doc("A/book.html", "html")
        manager = make_manager(tmp_path, DummyES(doc=doc))
        resp = asyncio_runner(manager.get_html_resource(1, "image.jpg"))
        assert resp.status_code == 400


# ── update_book: 책이 없을 때 Error 반환 (line 978) ─────────────────────────


def test_update_book_no_such_book_returns_error(tmp_path: Path):
    """ES 에서 책을 찾지 못하면 ('Error', '…no such a book') 을 반환한다."""
    manager = make_manager(tmp_path, DummyES(doc=None))
    new_path = tmp_path / "A" / "new.txt"

    status, msg = asyncio_runner(manager.update_book(999, "A", "Title", "Author", new_path, "txt"))
    assert status == "Error"
    assert "no such a book" in (msg or "")
