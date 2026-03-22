#!/usr/bin/env python

import logging.config
import shutil
import io
import json
import os
import sys
import time
import zipfile
from datetime import datetime
from pathlib import Path
from typing import Optional, Tuple

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


# ---- merged from test_book_manager_extra.py ----
import io
import time
import zipfile
from pathlib import Path

import pytest

from backend.book_manager import BookManager
from backend.book import Book


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
    return {
        "category": "A",
        "title": "T",
        "author": "U",
        "file_path": rel_path,
        "file_type": file_type,
        "file_size": 1,
        "updated_time": "2024-01-01T00:00:00.000000",
        "summary": "S",
    }


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

    missing = tmp_path / "A" / "none.txt"
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
    inode = (tmp_path / "A" / "dup.txt")
    inode.write_text("z")
    inode_id = inode.stat().st_ino
    es.category_docs = [
        (inode_id, make_doc("A/dup.txt"), 1.0),
        (999999, make_doc("A/dup.txt"), 1.0),
    ]
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
        zf.writestr("OEBPS/content.opf", """<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
          <spine><itemref idref="c1"/></spine>
        </package>""")
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
        zf.writestr("content.opf", """<package xmlns="http://www.idpf.org/2007/opf">
          <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
        </package>""")
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
        data = {
            "messages": [],
            "checker": {"nFatal": 0, "nError": 0, "nWarning": 0, "nUsage": 0, "nInfo": 0},
        }
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
        zf.writestr(
            "META-INF/container.xml",
            '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>',
        )
        zf.writestr("OEBPS/ch1.xhtml", "<html><body>Hi</body></html>")
        zf.writestr("OEBPS/toc.ncx", "<ncx/>")
        zf.writestr(
            "OEBPS/styles.css",
            "@font-face{font-family:'X';src:url('fonts/missing.ttf'),url('fonts/f.ttf');}",
        )
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
