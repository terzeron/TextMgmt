#!/usr/bin/env python

import logging.config
import shutil
from pathlib import Path
from fastapi.responses import Response

import pytest
import types
from fastapi.testclient import TestClient
import backend.main as main

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

CATEGORY = "_epub"


@pytest.fixture(scope="module")
def backend_test_setup(es_client, es_index, admin_auth_cookies, mysql_container):
    """Create BookManager and TestClient with test data loaded (공유된 ES 클라이언트 및 인덱스 사용)."""
    from fastapi.testclient import TestClient
    from backend.main import app, book_manager as main_bm, comics_manager as main_cm
    from backend.book_manager import BookManager
    from backend.comics_manager import ComicsManager
    from utils.loader import Loader
    import os

    # Create BookManager and use shared ES client
    bm = BookManager()
    bm.es_manager.es = es_client
    bm.es_manager.index_name = es_index

    cm = ComicsManager()
    cm.es_manager.es = es_client
    # comics_manager might use a different index, but for this test we might want to stick to what it has or sync if needed.
    # Given TM_ES_COMICS_INDEX is also usually set in conftest, we should check it.
    if "TM_ES_COMICS_INDEX" in os.environ:
        cm.es_manager.index_name = os.environ["TM_ES_COMICS_INDEX"]

    original_bm_instance = getattr(main_bm, "_instance", None)
    original_cm_instance = getattr(main_cm, "_instance", None)
    main_bm._instance = bm
    main_cm._instance = cm

    # Load test data from actual files if available
    epub_path = bm.path_prefix / CATEGORY
    if epub_path.exists():
        data = Loader.read_files(epub_path, num_files=5)
        if data:
            bm.es_manager.insert(data, num_docs=20)
            LOGGER.info("Inserted %d epub documents", len(data))

    # Refresh index to make data searchable
    bm.es_manager.refresh()

    client = TestClient(app, cookies=admin_auth_cookies)

    try:
        yield {"bm": bm, "client": client}
    finally:
        main_bm._instance = original_bm_instance
        main_cm._instance = original_cm_instance


@pytest.fixture(scope="class")
def test_book(backend_test_setup):
    """Create a temporary test book shared across tests in a class (read-only use)."""
    import asyncio
    from backend.book import Book
    from utils.loader import Loader

    bm = backend_test_setup["bm"]
    client = backend_test_setup["client"]

    epub_files = list(Book.path_prefix.glob(f"{CATEGORY}/*.epub"))
    if not epub_files:
        pytest.skip("No epub files available for testing")

    epub_file_path = epub_files[0]
    temp_file_path = Book.path_prefix / epub_file_path.parent.name / ("temp_" + epub_file_path.name)
    shutil.copy(epub_file_path, temp_file_path)
    data = Loader.read_file(temp_file_path)

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    try:
        book_id, error = loop.run_until_complete(bm.add_book(data))
        assert book_id and not error

        book, error = loop.run_until_complete(bm.get_book(book_id))
        assert book and not error

        yield {"book": book, "bm": bm, "client": client}

        # Cleanup: delete the test book and temp file
        try:
            client.delete(f"/books/{book.book_id}")
        except Exception:
            pass
        try:
            temp_file_path.unlink(missing_ok=True)
        except Exception:
            pass
    finally:
        loop.close()


class TestBackend:
    @pytest.mark.asyncio
    async def test_update_book(self, backend_test_setup):
        from backend.book import Book
        from utils.loader import Loader

        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_files = list(Book.path_prefix.glob(f"{CATEGORY}/*.epub"))
        if not epub_files:
            pytest.skip("No epub files available for testing")

        epub_file_path = epub_files[0]
        temp_file_path = Book.path_prefix / epub_file_path.parent.name / ("update_temp_" + epub_file_path.name)
        shutil.copy(epub_file_path, temp_file_path)
        data = Loader.read_file(temp_file_path)

        book_id, error = await bm.add_book(data)
        assert book_id and not error

        book, error = await bm.get_book(book_id)
        assert book and not error

        try:
            doc = {
                "book_id": book.book_id,
                "category": book.category,
                "title": "renamed_" + book.title,
                "author": "anonymous_" + book.author,
                "file_path": book.category + "/renamed_" + book.title + "." + book.file_type,
                "file_type": book.file_type,
                "file_size": 100,
                "summary": "summary1",
                "updated_time": "2021-01-01T00:00:00.000000",
            }

            response = client.put(f"/books/{book.book_id}", json=doc)
            assert response
            assert response.status_code == 200

            assert response.json() == {"status": "success", "result": "Ok"}
        finally:
            try:
                client.delete(f"/books/{book.book_id}")
            except Exception:
                pass
            temp_file_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_delete_book(self, backend_test_setup):
        from backend.book import Book
        from utils.loader import Loader

        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_files = list(Book.path_prefix.glob(f"{CATEGORY}/*.epub"))
        if not epub_files:
            pytest.skip("No epub files available for testing")

        epub_file_path = epub_files[0]
        temp_file_path = Book.path_prefix / epub_file_path.parent.name / ("to_be_deleted_" + epub_file_path.name)
        shutil.copy(epub_file_path, temp_file_path)
        data = Loader.read_file(temp_file_path)

        book_id, error = await bm.add_book(data)
        assert book_id and not error

        book, error = await bm.get_book(book_id)
        assert book and not error

        try:
            response = client.delete(f"/books/{book.book_id}")
            assert response
            assert response.status_code == 200
            assert response.json() == {"status": "success", "result": "Ok"}
        finally:
            temp_file_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_get_file_content(self, test_book):
        from backend.book_manager import BookManager

        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/download/{book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.content
        assert len(response.content) > 1024
        media_type = BookManager.MEDIA_TYPES.get(book.file_path.suffix, "application/octet-stream")
        assert response.headers["Content-Type"].split(";")[0] == media_type

    @pytest.mark.asyncio
    async def test_get_book(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/books/{book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": book.dict()}

    @pytest.mark.asyncio
    async def test_get_books_in_category(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/categories/{book.category}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert book.book_id in [b["book_id"] for b in books]

    @pytest.mark.asyncio
    async def test_get_books_in_category_paged(self, test_book):
        """limit을 주면 커서 페이지 응답(total, next_cursor)을 돌려준다."""
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/categories/{book.category}?limit=1")
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        assert len(response_data["result"]) <= 1
        assert response_data["total"] >= 1
        assert "next_cursor" in response_data

        # limit 없이 호출하면 기존(비페이지) 응답 형태를 유지한다
        legacy = client.get(f"/categories/{book.category}").json()
        assert legacy["status"] == "success"
        assert "next_cursor" not in legacy

    @pytest.mark.asyncio
    async def test_get_books_in_category_paged_invalid_cursor(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/categories/{book.category}?limit=1&cursor=not-a-cursor")
        assert response.status_code == 200
        assert response.json() == {"status": "failure", "error": "invalid cursor"}

    @pytest.mark.asyncio
    async def test_get_categories(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get("/categories")
        assert response
        assert response.status_code == 200
        assert response.json()["status"] == "success"
        assert book.category in response.json()["result"]

    @pytest.mark.asyncio
    async def test_get_similar_book_list(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        response = client.get(f"/similar/{book.book_id}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0

    @pytest.mark.asyncio
    async def test_get_category_mismatches(self, backend_test_setup):
        client = backend_test_setup["client"]

        response = client.get("/category-mismatches")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"

        result = response_data["result"]
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
            assert item["es_count"] != item["fs_count"]

    @pytest.mark.asyncio
    async def test_search_by_keyword(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        keyword = book.title
        response = client.get(f"/search/{keyword}")
        assert response
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"
        books = response_data["result"]
        assert books and len(books) > 0
        assert book.book_id in [b["book_id"] for b in books]

    @pytest.mark.asyncio
    async def test_search_by_keyword_with_exclude_categories(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        keyword = book.title
        # exclude_categories에 해당 책의 카테고리를 넣으면 결과에서 제외됨
        response = client.get(f"/search/{keyword}?exclude_categories={book.category}")
        assert response.status_code == 200
        response_data = response.json()
        if response_data["status"] == "success" and response_data.get("result"):
            excluded_ids = [b["book_id"] for b in response_data["result"] if b["category"] == book.category]
            assert len(excluded_ids) == 0, f"제외된 카테고리 '{book.category}'의 책이 포함됨"

    @pytest.mark.asyncio
    async def test_search_by_keyword_without_exclude_categories(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        keyword = book.title
        # exclude_categories 없이 검색하면 결과 포함
        response = client.get(f"/search/{keyword}")
        assert response.status_code == 200
        response_data = response.json()
        assert response_data["status"] == "success"

        # 빈 exclude_categories도 동일하게 동작
        response2 = client.get(f"/search/{keyword}?exclude_categories=")
        assert response2.status_code == 200
        response_data2 = response2.json()
        assert response_data2["status"] == "success"
        assert response_data2.get("total", 0) == response_data.get("total", 0)


class TestRenameCategory:
    """카테고리 일괄 변경 API 테스트."""

    @staticmethod
    def _make_test_dir(path: Path) -> None:
        path.mkdir(parents=True, exist_ok=True)

    @staticmethod
    async def _insert_docs(bm, category: str, count: int, start_id: int = 400) -> list:
        """테스트 문서를 ES에 삽입"""
        from datetime import datetime

        ids = []
        for i in range(count):
            doc_id = start_id + i
            data = {
                doc_id: {
                    "category": category,
                    "title": f"Rename Test {i}",
                    "author": "Test Author",
                    "file_path": f"{category}/rename_test_{i}.txt",
                    "file_type": "txt",
                    "file_size": 100,
                    "line_count": 10,
                    "page_count": 0,
                    "isbn": "",
                    "summary": "rename test doc",
                    "updated_time": datetime.now().strftime("%Y-%m-%dT%H:%M:%S.%f"),
                }
            }
            book_id, error = await bm.add_book(data)
            assert book_id and not error
            ids.append(book_id)
        return ids

    @pytest.mark.asyncio
    async def test_rename_category(self, backend_test_setup):
        """정상 rename: ES + FS 모두 변경"""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        old_cat = "_rename_test_src"
        new_cat = "_rename_test_dst"
        old_dir = bm.path_prefix / old_cat
        new_dir = bm.path_prefix / new_cat

        # 준비: 디렉토리 + ES 문서
        self._make_test_dir(old_dir)
        (old_dir / "rename_test_0.txt").write_text("test content")
        doc_ids = await self._insert_docs(bm, old_cat, 1)

        try:
            response = client.put("/categories/rename", json={"old_category": old_cat, "new_category": new_cat})
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success"
            assert data["result"]["old_category"] == old_cat
            assert data["result"]["new_category"] == new_cat
            assert data["result"]["updated_count"] == 1
            assert data["result"]["fs_renamed"] is True
        finally:
            # 정리
            for doc_id in doc_ids:
                bm.es_manager.delete(doc_id)
            bm.es_manager.refresh()
            if new_dir.exists():
                shutil.rmtree(new_dir)
            if old_dir.exists():
                shutil.rmtree(old_dir)

    @pytest.mark.asyncio
    async def test_rename_category_conflict(self, backend_test_setup):
        """대상 카테고리에 문서가 이미 존재하면 에러"""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        old_cat = "_rename_conflict_src"
        new_cat = "_rename_conflict_dst"

        # 양쪽 모두에 문서 삽입
        old_ids = await self._insert_docs(bm, old_cat, 1, start_id=500)
        new_ids = await self._insert_docs(bm, new_cat, 1, start_id=600)

        try:
            response = client.put("/categories/rename", json={"old_category": old_cat, "new_category": new_cat})
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "failure"
            assert "이미" in data["error"]
        finally:
            for doc_id in old_ids + new_ids:
                bm.es_manager.delete(doc_id)
            bm.es_manager.refresh()

    @pytest.mark.asyncio
    async def test_rename_category_not_found(self, backend_test_setup):
        """없는 카테고리 rename 시 에러"""
        client = backend_test_setup["client"]

        response = client.put("/categories/rename", json={"old_category": "_nonexistent_cat_xyz", "new_category": "_new_cat_xyz"})
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "failure"
        assert "문서가 없습니다" in data["error"]


class TestDeleteCategory:
    """카테고리 일괄 삭제 API 테스트."""

    @staticmethod
    async def _insert_docs(bm, category: str, count: int, start_id: int = 700) -> list:
        """테스트 문서를 ES에 삽입"""
        from datetime import datetime

        ids = []
        for i in range(count):
            doc_id = start_id + i
            data = {
                doc_id: {
                    "category": category,
                    "title": f"Delete Test {i}",
                    "author": "Test Author",
                    "file_path": f"{category}/delete_test_{i}.txt",
                    "file_type": "txt",
                    "file_size": 100,
                    "line_count": 10,
                    "page_count": 0,
                    "isbn": "",
                    "summary": "delete test doc",
                    "updated_time": datetime.now().strftime("%Y-%m-%dT%H:%M:%S.%f"),
                }
            }
            book_id, error = await bm.add_book(data)
            assert book_id and not error
            ids.append(book_id)
        return ids

    @pytest.mark.asyncio
    async def test_delete_category(self, backend_test_setup):
        """정상 삭제: ES + FS 모두 삭제"""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        cat = "_delete_test_cat"
        cat_dir = bm.path_prefix / cat

        # 준비: 디렉토리 + 파일 + ES 문서
        cat_dir.mkdir(parents=True, exist_ok=True)
        (cat_dir / "delete_test_0.txt").write_text("test content")
        (cat_dir / "delete_test_1.txt").write_text("test content 2")
        doc_ids = await self._insert_docs(bm, cat, 2)

        try:
            response = client.post("/categories/delete", json={"category": cat})
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success"
            assert data["result"]["category"] == cat
            assert data["result"]["deleted_count"] == 2

            # ES에서 삭제 확인
            assert bm.es_manager.count_by_category(cat) == 0
        finally:
            # 혹시 남아있으면 정리
            for doc_id in doc_ids:
                try:
                    bm.es_manager.delete(doc_id)
                except Exception:
                    pass
            bm.es_manager.refresh()
            if cat_dir.exists():
                shutil.rmtree(cat_dir)

    @pytest.mark.asyncio
    async def test_delete_category_not_found(self, backend_test_setup):
        """없는 카테고리 삭제 시 에러"""
        client = backend_test_setup["client"]

        response = client.post("/categories/delete", json={"category": "_nonexistent_del_xyz"})
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "failure"
        assert "문서가 없습니다" in data["error"]

    @pytest.mark.asyncio
    async def test_delete_category_with_subcategories(self, backend_test_setup):
        """삭제 시 하위 카테고리 문서도 함께 삭제"""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        cat = "_delete_sub_test"
        sub_cat = f"{cat}/sub"
        cat_dir = bm.path_prefix / cat
        sub_dir = bm.path_prefix / sub_cat

        # 준비: 디렉토리 + 파일 + ES 문서
        cat_dir.mkdir(parents=True, exist_ok=True)
        sub_dir.mkdir(parents=True, exist_ok=True)
        (cat_dir / "root.txt").write_text("root content")
        (sub_dir / "child.txt").write_text("child content")
        root_ids = await self._insert_docs(bm, cat, 1, start_id=850)
        sub_ids = await self._insert_docs(bm, sub_cat, 1, start_id=860)

        try:
            response = client.post("/categories/delete", json={"category": cat})
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success"
            assert data["result"]["deleted_count"] == 2

            # ES에서 하위 카테고리도 삭제 확인
            assert bm.es_manager.count_by_category(cat, prefix=True) == 0
        finally:
            for doc_id in root_ids + sub_ids:
                try:
                    bm.es_manager.delete(doc_id)
                except Exception:
                    pass
            bm.es_manager.refresh()
            if cat_dir.exists():
                shutil.rmtree(cat_dir)

    @pytest.mark.asyncio
    async def test_delete_category_es_only(self, backend_test_setup):
        """ES만 있는 경우에도 정상 삭제"""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        cat = "_delete_no_fs_cat"
        doc_ids = await self._insert_docs(bm, cat, 1, start_id=800)

        try:
            response = client.post("/categories/delete", json={"category": cat})
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success"
            assert data["result"]["deleted_count"] == 1
        finally:
            for doc_id in doc_ids:
                try:
                    bm.es_manager.delete(doc_id)
                except Exception:
                    pass
            bm.es_manager.refresh()


class TestUpdateBookConflict:
    """update_book 충돌 감지 및 force 덮어쓰기 테스트."""

    @staticmethod
    def _make_test_file(path: Path, content: bytes = b"test content") -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)

    @staticmethod
    async def _register_book(bm, file_path: Path, category: str = CATEGORY) -> int:
        from datetime import datetime

        rel = file_path.relative_to(bm.path_prefix)
        now = datetime.now().strftime("%Y-%m-%dT%H:%M:%S.%f")
        data = {file_path.stat().st_ino: {"category": category, "title": file_path.stem, "author": "Test Author", "file_path": str(rel), "file_type": file_path.suffix.lstrip("."), "file_size": file_path.stat().st_size, "line_count": 0, "page_count": 0, "isbn": "", "summary": "conflict test", "updated_time": now}}
        book_id, error = await bm.add_book(data)
        assert book_id and not error, f"Failed to register: {error}"
        return book_id

    @pytest.mark.asyncio
    async def test_conflict_detected_when_destination_exists(self, backend_test_setup):
        """대상 경로에 다른 파일이 이미 존재하면 CONFLICT: 에러를 반환한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        src_path = bm.path_prefix / CATEGORY / "conflict_src.epub"
        dst_path = bm.path_prefix / CATEGORY / "conflict_dst.epub"
        self._make_test_file(src_path, b"source")
        self._make_test_file(dst_path, b"existing")

        book_id = await self._register_book(bm, src_path)

        try:
            doc = {"book_id": book_id, "category": CATEGORY, "title": "conflict_dst", "author": "Test Author", "file_path": f"{CATEGORY}/conflict_dst.epub", "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "failure"
            assert "CONFLICT:" in data["error"]
            assert src_path.exists(), "Source file should not have been moved"
            assert dst_path.read_bytes() == b"existing", "Destination should be unchanged"
        finally:
            try:
                client.delete(f"/books/{book_id}")
            except Exception:
                pass
            src_path.unlink(missing_ok=True)
            dst_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_conflict_error_contains_relative_path(self, backend_test_setup):
        """CONFLICT 에러 메시지에 상대 경로가 포함된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        src_path = bm.path_prefix / CATEGORY / "conflict_relpath_src.epub"
        dst_path = bm.path_prefix / CATEGORY / "conflict_relpath_dst.epub"
        self._make_test_file(src_path)
        self._make_test_file(dst_path)

        book_id = await self._register_book(bm, src_path)

        try:
            doc = {"book_id": book_id, "category": CATEGORY, "title": "conflict_relpath_dst", "author": "Test Author", "file_path": f"{CATEGORY}/conflict_relpath_dst.epub", "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

            response = client.put(f"/books/{book_id}", json=doc)
            data = response.json()
            error_msg = data.get("error", "")
            assert f"{CATEGORY}/conflict_relpath_dst.epub" in error_msg, f"CONFLICT error should contain relative path, got: {error_msg}"
        finally:
            try:
                client.delete(f"/books/{book_id}")
            except Exception:
                pass
            src_path.unlink(missing_ok=True)
            dst_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_force_overwrite_succeeds(self, backend_test_setup):
        """force=true이면 대상 경로의 기존 파일을 덮어쓰고 성공한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        src_path = bm.path_prefix / CATEGORY / "force_src.epub"
        dst_path = bm.path_prefix / CATEGORY / "force_dst.epub"
        self._make_test_file(src_path, b"source content")
        self._make_test_file(dst_path, b"existing content")

        book_id = await self._register_book(bm, src_path)

        try:
            doc = {"book_id": book_id, "category": CATEGORY, "title": "force_dst", "author": "Test Author", "file_path": f"{CATEGORY}/force_dst.epub", "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

            response = client.put(f"/books/{book_id}?force=true", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success"
            assert data["result"] == "Ok"

            assert not src_path.exists(), "Source should have been moved"
            assert dst_path.exists(), "Destination should exist"
            assert dst_path.read_bytes() == b"source content"
        finally:
            try:
                client.delete(f"/books/{book_id}")
            except Exception:
                pass
            src_path.unlink(missing_ok=True)
            dst_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_same_file_no_conflict(self, backend_test_setup):
        """동일 파일 경로에 대한 메타데이터 변경 시 충돌이 발생하지 않는다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        file_path = bm.path_prefix / CATEGORY / "samefile_test.epub"
        self._make_test_file(file_path)

        book_id = await self._register_book(bm, file_path)

        try:
            doc = {"book_id": book_id, "category": CATEGORY, "title": "samefile_test", "author": "Changed Author Name", "file_path": f"{CATEGORY}/samefile_test.epub", "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success", f"Same file path should not cause conflict, got: {data}"
            assert file_path.exists(), "File should still exist"
        finally:
            try:
                client.delete(f"/books/{book_id}")
            except Exception:
                pass
            file_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_move_to_new_directory_succeeds(self, backend_test_setup):
        """다른 디렉토리로 이동 시 정상 동작한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        new_dir = "_conflict_test_dir"
        src_path = bm.path_prefix / CATEGORY / "movedir_test.epub"
        dst_path = bm.path_prefix / new_dir / "movedir_test.epub"
        self._make_test_file(src_path)

        book_id = await self._register_book(bm, src_path)

        try:
            doc = {"book_id": book_id, "category": new_dir, "title": "movedir_test", "author": "Test Author", "file_path": f"{new_dir}/movedir_test.epub", "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success"

            assert not src_path.exists(), "Source should have been moved"
            assert dst_path.exists(), "Destination should exist"
        finally:
            try:
                client.delete(f"/books/{book_id}")
            except Exception:
                pass
            src_path.unlink(missing_ok=True)
            dst_path.unlink(missing_ok=True)
            new_dir_path = bm.path_prefix / new_dir
            if new_dir_path.exists() and not any(new_dir_path.iterdir()):
                new_dir_path.rmdir()

    @pytest.mark.asyncio
    async def test_source_missing_returns_error(self, backend_test_setup):
        """원본 파일이 없으면 이동 실패 에러를 반환한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        src_path = bm.path_prefix / CATEGORY / "missing_src.epub"
        self._make_test_file(src_path)

        book_id = await self._register_book(bm, src_path)
        # 등록 후 원본 파일 삭제
        src_path.unlink()

        try:
            doc = {"book_id": book_id, "category": CATEGORY, "title": "missing_renamed", "author": "Test Author", "file_path": f"{CATEGORY}/missing_renamed.epub", "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"}

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "failure"
            assert "can't move" in data.get("error", "").lower() or "error" in data.get("error", "").lower()
        finally:
            try:
                client.delete(f"/books/{book_id}")
            except Exception:
                pass


class TestAuthRefreshEndpoint:
    """POST /auth/refresh 엔드포인트 통합 테스트"""

    @pytest.fixture(autouse=True)
    def setup(self, backend_test_setup):
        import backend.auth as auth_mod

        self.auth_mod = auth_mod
        self.client = backend_test_setup["client"]

    def _get_unauthenticated_client(self, backend_test_setup):
        from fastapi.testclient import TestClient
        from backend.main import app

        return TestClient(app)

    def test_refresh_returns_new_access_token(self, backend_test_setup):
        from backend.main import _issue_auth_tokens

        _, refresh_token = _issue_auth_tokens(email=self.auth_mod.TM_ADMIN_EMAIL, role="admin")
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", cookies={"tm_refresh_token": refresh_token})
        assert response.status_code == 200
        assert "set-cookie" in response.headers

    def test_refresh_with_expired_token_returns_401(self, backend_test_setup):
        import time
        import jwt

        expired_payload = {"type": "refresh", "email": self.auth_mod.TM_ADMIN_EMAIL, "role": "admin", "exp": int(time.time()) - 100, "iat": int(time.time()) - 200}
        from backend.auth import JWT_SECRET, JWT_ALGORITHM

        token = jwt.encode(expired_payload, JWT_SECRET, algorithm=JWT_ALGORITHM)
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", cookies={"tm_refresh_token": token})
        assert response.status_code == 401

    def test_refresh_with_access_token_returns_401(self, backend_test_setup):
        from backend.auth import create_jwt_token

        access_token = create_jwt_token(email=self.auth_mod.TM_ADMIN_EMAIL, role="admin")
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", cookies={"tm_refresh_token": access_token})
        assert response.status_code == 401

    def test_refresh_without_token_returns_400(self, backend_test_setup):
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh")
        assert response.status_code == 400

    def test_refresh_with_unauthorized_email_returns_403(self, backend_test_setup):
        from backend.main import _issue_auth_tokens

        _, refresh_token = _issue_auth_tokens(email="hacker@evil.com", role="admin")
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", cookies={"tm_refresh_token": refresh_token})
        assert response.status_code == 403

    def test_refreshed_token_works_for_api_calls(self, backend_test_setup):
        from backend.main import _issue_auth_tokens

        _, refresh_token = _issue_auth_tokens(email=self.auth_mod.TM_ADMIN_EMAIL, role="admin")
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", cookies={"tm_refresh_token": refresh_token})
        new_access_token = response.cookies.get("tm_access_token")
        # 새 토큰으로 인증 필요한 API 호출
        from fastapi.testclient import TestClient
        from backend.main import app

        auth_client = TestClient(app, cookies={"tm_access_token": new_access_token})
        cat_response = auth_client.get("/categories")
        assert cat_response.status_code == 200


class TestAuthMeLogout:
    def test_auth_me_returns_user_info(self, backend_test_setup):
        from backend.auth import create_jwt_token
        from fastapi.testclient import TestClient
        from backend.main import app

        token = create_jwt_token(email="admin@test.com", role="admin", name="Test", picture="pic")
        client = TestClient(app, cookies={"tm_access_token": token})
        response = client.get("/auth/me")
        assert response.status_code == 200
        data = response.json()
        assert data["status"] == "success"
        assert data["result"]["role"] == "admin"
        assert data["result"]["email"] == "admin@test.com"
        assert data["result"]["name"] == "Test"
        assert data["result"]["picture"] == "pic"

    def test_logout_clears_cookies(self, backend_test_setup):
        from backend.auth import create_jwt_token
        from fastapi.testclient import TestClient
        from backend.main import app

        token = create_jwt_token(email="admin@test.com", role="admin")
        client = TestClient(app, cookies={"tm_access_token": token, "tm_refresh_token": token})
        response = client.post("/auth/logout")
        assert response.status_code == 200
        assert "set-cookie" in response.headers


if __name__ == "__main__":
    pytest.main([__file__, "-v"])


# ---- merged from test_main_api_extra.py ----


@pytest.fixture()
def client():
    def _auth_override():
        return {"email": "user@example.com", "role": "user"}

    def _admin_override():
        return {"email": "admin@example.com", "role": "admin"}

    main.app.dependency_overrides[main.require_auth] = _auth_override
    main.app.dependency_overrides[main.require_admin] = _admin_override
    with TestClient(main.app) as c:
        yield c
    main.app.dependency_overrides.clear()


def test_lazy_proxy_initializes_once():
    calls = {"count": 0}

    def factory():
        calls["count"] += 1
        obj = types.SimpleNamespace(value=1)
        return obj

    proxy = main._LazyProxy(factory, "dummy")
    assert proxy.value == 1
    proxy.value = 2
    assert proxy.value == 2
    assert calls["count"] == 1


def test_custom_jsonable_encoder():
    data = {"korean": "테스트", "list": ["값"], "nested": {"a": "나"}}
    encoded = main.custom_jsonable_encoder(data)
    assert encoded == data


def test_custom_json_response_render_preserves_unicode():
    response = main.CustomJSONResponse({"message": "테스트"})
    assert b"\\u" not in response.body
    assert "테스트".encode("utf-8") in response.body


def test_wake_storage_success_and_failure(client, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("TM_FRONTEND_URL", "http://testserver")
    monkeypatch.setattr(main.os, "listdir", lambda path: ["a", "b"])
    resp = client.get("/wake")
    assert resp.status_code == 200
    assert resp.json() == {"status": "success"}

    def fail_listdir(path):
        raise OSError("boom")

    monkeypatch.setattr(main.os, "listdir", fail_listdir)
    resp = client.get("/wake")
    assert resp.status_code == 503
    assert resp.json() == {"status": "failure"}


def test_wake_storage_rejects_non_frontend_host(client, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("TM_FRONTEND_URL", "https://tm.terzeron.com")
    resp = client.get("/wake")
    assert resp.status_code == 404


def test_search_bookstore_api_errors(client):
    resp = client.get("/search/bookstore/unknown")
    assert resp.status_code == 404

    resp = client.get("/search/bookstore/yes24")
    assert resp.status_code == 400
    assert resp.json()["detail"] == main.ERR_MISSING_INPUT


def test_search_bookstore_api_success(client, monkeypatch: pytest.MonkeyPatch):
    class DummyStore:
        def __init__(self):
            self.called = False

        def search(self, isbn: str = "", title: str = "", author: str = ""):
            self.called = True
            return [("T", "A", "C", "U", "S", "ISBN")], "kw", "title"

        def build_search_url(self, keyword: str) -> str:
            return f"https://example.com?q={keyword}"

    monkeypatch.setattr(main, "Yes24Bookstore", DummyStore)
    resp = client.get("/search/bookstore/yes24?title=Hello")
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "success"
    assert body["search_keyword"] == "kw"
    assert body["result"][0]["isbn"] == "ISBN"


def test_category_mapping_endpoints(client, monkeypatch: pytest.MonkeyPatch):
    class DummyMapping:
        def get_all_mappings(self, content_type="book"):
            return {"A": ["a"]}

        def get_keywords(self, category, content_type="book"):
            return ["k1", "k2"]

        def set_keywords(self, category, keywords, content_type="book"):
            return True

        def add_keyword(self, category, keyword, content_type="book"):
            return keyword != "dup"

        def remove_keyword(self, category, keyword, content_type="book"):
            return keyword == "ok"

        def delete_category(self, category, content_type="book"):
            return category == "ok"

        def update_all_mappings(self, mappings, content_type="book"):
            return True

        def get_hidden_categories(self, content_type="book"):
            return ["A"]

        def set_hidden(self, category, hidden, content_type="book"):
            return True

        def get_latest_excluded_categories(self, content_type="book"):
            return ["B"]

        def set_latest_excluded(self, category, excluded, content_type="book"):
            return True

    monkeypatch.setattr(main, "category_mapping", DummyMapping())

    resp = client.get("/category-mappings")
    assert resp.json()["result"] == {"A": ["a"]}

    resp = client.get("/category-mappings/A")
    assert resp.json()["result"] == ["k1", "k2"]

    resp = client.put("/category-mappings/A", json={"keywords": ["x"]})
    assert resp.json()["status"] == "success"

    resp = client.post("/category-mappings/A/keywords", json={"keyword": ""})
    assert resp.status_code == 400

    resp = client.post("/category-mappings/A/keywords", json={"keyword": "dup"})
    assert resp.json()["status"] == "duplicate"

    resp = client.post("/category-mappings/A/keywords", json={"keyword": "new"})
    assert resp.json()["status"] == "success"

    resp = client.delete("/category-mappings/A/keywords/ok")
    assert resp.json()["status"] == "success"

    resp = client.delete("/category-mappings/A/keywords/miss")
    assert resp.status_code == 404

    resp = client.delete("/category-mappings/ok")
    assert resp.json()["status"] == "success"

    resp = client.delete("/category-mappings/miss")
    assert resp.status_code == 404

    resp = client.put("/category-mappings", json={"mappings": {"A": ["a"]}})
    assert resp.json()["status"] == "success"

    resp = client.get("/hidden-categories")
    assert resp.json()["result"] == ["A"]

    resp = client.post("/hidden-categories/A", json={"hidden": True})
    assert resp.json()["status"] == "success"

    resp = client.get("/latest-excluded-categories")
    assert resp.json()["result"] == ["B"]

    resp = client.post("/latest-excluded-categories/B", json={"excluded": True})
    assert resp.json()["status"] == "success"


# ---- merged from test_main_routes_extra.py ----
class DummyBook:
    def __init__(self, data):
        self._data = data

    def dict(self):
        return self._data

    def __getattr__(self, item):
        if item in self._data:
            return self._data[item]
        raise AttributeError(item)


class DummyManager:
    def __init__(self, tmp_path: Path):
        self.path_prefix = tmp_path
        self.es_manager = type("E", (), {"delete": lambda self, book_id: True})()

    async def update_book(self, *args, **kwargs):
        return "Ok", None

    async def delete_book(self, book_id: int):
        return "Warning", "warn"

    async def get_book_content(self, book_id: int):
        return "content"

    async def get_book_preview(self, *args, **kwargs):
        return Response(content="preview", media_type="text/plain")

    async def get_pdf_pages(self, *args, **kwargs):
        return Response(content="pdf", media_type="application/pdf")

    async def get_book(self, book_id: int):
        data = {"book_id": book_id, "category": "A", "title": "T", "author": "U", "file_path": "a.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0}
        return DummyBook(data), None

    async def validate_epub(self, book_id: int):
        return {"valid": True}, None

    async def validate_pdf(self, book_id: int):
        return {"valid": True}, None

    async def get_books_in_category(self, category: str):
        return [DummyBook({"book_id": 1, "category": category, "title": "T", "author": "U", "file_path": "a.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0})], None

    async def get_categories(self):
        return {"A": 1}, None

    async def search_similar_books_paged(self, book_id: int, size: int, offset: int):
        return [], 0, "No similar books found"

    async def search_similar_books_paged_ok(self, book_id: int, size: int, offset: int):
        return ([DummyBook({"book_id": 2, "category": "A", "title": "T2", "author": "U", "file_path": "b.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 10.0})], 1, None)

    async def search_by_keyword_paged(self, keyword: str, size: int, offset: int, exclude_categories=None):
        return ([DummyBook({"book_id": 3, "category": "A", "title": "T3", "author": "U", "file_path": "c.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 9.0})], 1, None)

    async def get_latest_books(self, size: int = 100, exclude_categories=None):
        self.latest_size = size
        self.latest_exclude_categories = exclude_categories
        return ([DummyBook({"book_id": 4, "category": "A", "title": "T4", "author": "U", "file_path": "d.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "created_time": "2023-12-31T00:00:00.000000", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0})], 1, None)

    def get_category_mismatches(self):
        return {"mismatches": []}

    def get_category_mismatch_details(self, category: str):
        return {"category": category}

    async def index_single_file(self, file_path: str):
        return 1, None

    async def delete_file(self, file_path: str):
        return "Ok", None

    async def reload_category(self, category: str, content_type: str = "book"):
        return {"category": category, "processed_count": 1}, None

    async def reload_category_mismatches(self, content_type: str = "book"):
        return {"content_type": content_type, "category_count": 1, "indexed_count": 2, "deleted_count": 3, "failed_count": 0}, None

    async def reload_category_mismatch_files(self, category: str, content_type: str = "book"):
        return {"content_type": content_type, "category": category, "indexed_count": 1, "deleted_count": 1, "failed_count": 0}, None


class DummyCategoryMapping:
    def get_hidden_categories(self, content_type="book"):
        return []

    def set_hidden(self, category, hidden, content_type="book"):
        return True

    def get_latest_excluded_categories(self, content_type="book"):
        return []

    def set_latest_excluded(self, category, excluded, content_type="book"):
        return True

    def acquire_reload_lock(self, content_type="book"):
        return True, None

    def release_reload_lock(self, content_type="book"):
        return None


@pytest.fixture()
def dummy_client(tmp_path: Path):
    from fastapi.testclient import TestClient
    from backend import main as main_mod

    def _auth_override():
        return {"email": "user@example.com", "role": "user"}

    def _admin_override():
        return {"email": "admin@example.com", "role": "admin"}

    main_mod.app.dependency_overrides[main_mod.require_auth] = _auth_override
    main_mod.app.dependency_overrides[main_mod.require_admin] = _admin_override

    dummy = DummyManager(tmp_path)
    orig_book_instance = getattr(main_mod.book_manager, "_instance", None)
    orig_comics_instance = getattr(main_mod.comics_manager, "_instance", None)
    orig_category_mapping_instance = getattr(main_mod.category_mapping, "_instance", None)
    main_mod.book_manager._instance = dummy  # type: ignore[attr-defined]
    main_mod.comics_manager._instance = dummy  # type: ignore[attr-defined]
    main_mod.category_mapping._instance = DummyCategoryMapping()  # type: ignore[attr-defined]

    with TestClient(main_mod.app) as c:
        yield c

    main_mod.book_manager._instance = orig_book_instance  # type: ignore[attr-defined]
    main_mod.comics_manager._instance = orig_comics_instance  # type: ignore[attr-defined]
    main_mod.category_mapping._instance = orig_category_mapping_instance  # type: ignore[attr-defined]
    main_mod.app.dependency_overrides.clear()


def test_main_routes_basic(dummy_client):
    payload = {"book_id": 1, "category": "A", "title": "T", "author": "U", "file_path": "a.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0}
    resp = dummy_client.put("/books/1", json=payload)
    assert resp.json()["status"] == "success"

    resp = dummy_client.delete("/books/1")
    assert resp.json()["status"] == "success"
    assert resp.json()["warning"]

    resp = dummy_client.get("/download/1")
    assert resp.status_code == 200

    resp = dummy_client.get("/preview/1")
    assert resp.status_code == 200

    resp = dummy_client.get("/pdf-pages/1?start=1&end=1")
    assert resp.status_code == 200

    resp = dummy_client.get("/books/1")
    assert resp.json()["status"] == "success"

    resp = dummy_client.get("/categories/A")
    assert resp.json()["status"] == "success"

    resp = dummy_client.get("/categories")
    assert resp.json()["status"] == "success"


def test_viewer_hidden_category_access_control_routes(dummy_client, monkeypatch):
    from backend import main as main_mod

    main_mod.app.dependency_overrides[main_mod.require_auth] = lambda: {"email": "viewer@example.com", "role": "viewer"}

    class DummyCatMap:
        def get_hidden_categories(self, content_type="book"):
            return ["A"]

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.get("/books/1")
    assert resp.status_code == 403
    assert resp.json()["detail"] == "접근 권한이 없는 카테고리입니다."

    resp = dummy_client.get("/categories")
    assert resp.status_code == 200
    assert resp.json() == {"status": "success", "result": {}}

    resp = dummy_client.get("/hidden-categories")
    assert resp.status_code == 200
    assert resp.json() == {"status": "success", "result": []}


def test_main_search_validate_and_mismatch(dummy_client, monkeypatch):
    from backend import main as main_mod

    dummy = main_mod.book_manager
    resp = dummy_client.get("/similar/1")
    assert resp.json()["status"] == "success"
    assert resp.json()["total"] == 1

    monkeypatch.setattr(dummy, "search_similar_books_paged", dummy.search_similar_books_paged_ok)
    resp = dummy_client.get("/similar/1")
    assert resp.json()["total"] == 1

    resp = dummy_client.get("/search/kw?exclude_categories=A,B")
    assert resp.json()["status"] == "success"

    resp = dummy_client.get("/latest")
    assert resp.json()["status"] == "success"
    assert resp.json()["total"] == 1
    assert resp.json()["result"][0]["created_time"] == "2023-12-31T00:00:00.000000"
    assert dummy._instance.latest_exclude_categories == []

    resp = dummy_client.get("/latest?limit=101")
    assert resp.status_code == 422

    async def get_epub(book_id: int):
        data = (await DummyManager(Path(".")).get_book(book_id))[0]
        data._data["file_type"] = "epub"
        return data, None

    monkeypatch.setattr(dummy, "get_book", get_epub)
    resp = dummy_client.get("/validate/1")
    assert resp.json()["status"] == "success"

    async def get_pdf(book_id: int):
        data = (await DummyManager(Path(".")).get_book(book_id))[0]
        data._data["file_type"] = "pdf"
        return data, None

    monkeypatch.setattr(dummy, "get_book", get_pdf)
    resp = dummy_client.get("/validate/1")
    assert resp.json()["status"] == "success"

    resp = dummy_client.get("/category-mismatches")
    assert resp.json()["status"] == "success"

    resp = dummy_client.post("/category-mismatches/index-file", json={"file_path": "a.txt"})
    assert resp.json()["status"] == "success"

    resp = dummy_client.post("/category-mismatches/delete-file", json={"file_path": "a.txt"})
    assert resp.json()["status"] == "success"

    resp = dummy_client.delete("/category-mismatches/es-doc/1")
    assert resp.json()["status"] == "success"

    resp = dummy_client.post("/category-mismatches/reload", json={"category": "A"})
    assert resp.json()["status"] == "success"

    resp = dummy_client.post("/category-mismatches/reload-mismatches", json={"category": "A"})
    assert resp.json()["status"] == "success"
    assert resp.json()["result"]["category"] == "A"

    resp = dummy_client.post("/category-mismatches/reload-all")
    assert resp.json()["status"] == "success"
    assert resp.json()["result"]["indexed_count"] == 2

    resp = dummy_client.get("/category-mismatches/A")
    assert resp.json()["status"] == "success"


def test_latest_books_uses_latest_excluded_categories(dummy_client, monkeypatch):
    from backend import main as main_mod

    class DummyCatMap:
        def get_hidden_categories(self, content_type="book"):
            return []

        def get_latest_excluded_categories(self, content_type="book"):
            return ["blocked"]

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.get("/latest?limit=5")

    assert resp.status_code == 200
    assert main_mod.book_manager._instance.latest_size == 5
    assert main_mod.book_manager._instance.latest_exclude_categories == ["blocked"]


def test_latest_books_merges_viewer_hidden_and_latest_excluded_categories(dummy_client, monkeypatch):
    from backend import main as main_mod

    main_mod.app.dependency_overrides[main_mod.require_auth] = lambda: {"email": "viewer@example.com", "role": "viewer"}

    class DummyCatMap:
        def get_hidden_categories(self, content_type="book"):
            return ["hidden", "shared"]

        def get_latest_excluded_categories(self, content_type="book"):
            return ["latest", "shared"]

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.get("/latest")

    assert resp.status_code == 200
    assert main_mod.book_manager._instance.latest_exclude_categories == ["hidden", "shared", "latest"]


def test_custom_jsonable_encoder_fallback():
    assert main.custom_jsonable_encoder(1) == 1


def test_lazy_proxy_repr():
    proxy = main._LazyProxy(lambda: {"ok": True}, "dummy")
    assert repr(proxy) == repr({"ok": True})


def test_main_error_branches(dummy_client, monkeypatch):
    from backend import main as main_mod

    async def delete_book_error(book_id: int):
        return ("Error", "boom")

    async def get_book_error(book_id: int):
        return (None, "not found")

    async def get_books_in_category_error(category: str):
        return ([], "nope")

    async def get_categories_error():
        return ({}, "fail")

    async def search_similar_error(book_id: int, size: int, offset: int):
        return ([], 0, "none")

    async def search_keyword_error(keyword: str, size: int, offset: int, exclude_categories=None):
        return ([], 0, "kw")

    async def index_single_file_error(file_path: str):
        return (None, "err")

    async def delete_file_error(file_path: str):
        return ("Error", "bad")

    async def reload_category_error(category: str, content_type: str = "book"):
        return (None, "fail")

    async def reload_category_mismatches_error(content_type: str = "book"):
        return (None, "bulk fail")

    async def reload_category_mismatch_files_error(category: str, content_type: str = "book"):
        return (None, "mismatch fail")

    def get_category_mismatches_error():
        raise RuntimeError("boom")

    def get_category_mismatch_details_error(category: str):
        raise RuntimeError("boom2")

    monkeypatch.setattr(main_mod.book_manager, "delete_book", delete_book_error)
    monkeypatch.setattr(main_mod.book_manager, "get_book", get_book_error)
    monkeypatch.setattr(main_mod.book_manager, "get_books_in_category", get_books_in_category_error)
    monkeypatch.setattr(main_mod.book_manager, "get_categories", get_categories_error)
    monkeypatch.setattr(main_mod.book_manager, "search_similar_books_paged", search_similar_error)
    monkeypatch.setattr(main_mod.book_manager, "search_by_keyword_paged", search_keyword_error)
    monkeypatch.setattr(main_mod.book_manager, "index_single_file", index_single_file_error)
    monkeypatch.setattr(main_mod.book_manager, "delete_file", delete_file_error)
    monkeypatch.setattr(main_mod.book_manager, "reload_category", reload_category_error)
    monkeypatch.setattr(main_mod.book_manager, "reload_category_mismatches", reload_category_mismatches_error)
    monkeypatch.setattr(main_mod.book_manager, "reload_category_mismatch_files", reload_category_mismatch_files_error)
    monkeypatch.setattr(main_mod.book_manager, "get_category_mismatches", get_category_mismatches_error)
    monkeypatch.setattr(main_mod.book_manager, "get_category_mismatch_details", get_category_mismatch_details_error)

    resp = dummy_client.delete("/books/1")
    assert resp.json()["error"] == "boom"

    resp = dummy_client.get("/books/1")
    assert resp.json()["error"] == "not found"

    resp = dummy_client.get("/categories/A")
    assert resp.json()["error"] == "nope"

    resp = dummy_client.get("/categories")
    assert resp.json()["error"] == "fail"

    resp = dummy_client.get("/similar/1")
    assert resp.json()["error"] == "none"

    resp = dummy_client.get("/search/kw")
    assert resp.json()["error"] == "kw"

    resp = dummy_client.get("/category-mismatches")
    assert resp.json()["error"] == main_mod.GENERIC_MISMATCH_ERROR

    resp = dummy_client.post("/category-mismatches/index-file", json={})
    assert resp.status_code == 400

    resp = dummy_client.post("/category-mismatches/index-file", json={"file_path": "a.txt"})
    assert resp.json()["error"] == "err"

    resp = dummy_client.post("/category-mismatches/delete-file", json={})
    assert resp.status_code == 400

    resp = dummy_client.post("/category-mismatches/delete-file", json={"file_path": "a.txt"})
    assert resp.json()["error"] == "bad"

    monkeypatch.setattr(main_mod.book_manager.es_manager, "delete", lambda book_id: False)
    resp = dummy_client.delete("/category-mismatches/es-doc/1")
    assert "ES 문서 삭제 실패" in resp.json()["error"]

    resp = dummy_client.post("/category-mismatches/reload", json={"category": "A"})
    assert resp.json()["error"] == "fail"

    resp = dummy_client.post("/category-mismatches/reload-mismatches", json={"category": "A"})
    assert resp.json()["error"] == "mismatch fail"

    async def reload_category_mismatch_files_raise(category: str, content_type: str = "book"):
        raise RuntimeError("mismatch boom")

    monkeypatch.setattr(main_mod.book_manager, "reload_category_mismatch_files", reload_category_mismatch_files_raise)
    resp = dummy_client.post("/category-mismatches/reload-mismatches", json={"category": "A"})
    assert resp.json()["error"] == main_mod.GENERIC_MISMATCH_ERROR

    resp = dummy_client.post("/category-mismatches/reload-all")
    assert resp.json()["error"] == "bulk fail"

    resp = dummy_client.get("/category-mismatches/A")
    assert resp.json()["error"] == main_mod.GENERIC_MISMATCH_ERROR


def test_wake_storage_error(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("TM_FRONTEND_URL", "http://testserver")
    monkeypatch.setattr(main.os, "listdir", lambda path: (_ for _ in ()).throw(RuntimeError("fail")))
    client = TestClient(main.app)
    resp = client.get("/wake")
    assert resp.status_code == 503
    assert resp.json() == {"status": "failure"}


def test_search_bookstore_api_branches(dummy_client, monkeypatch):
    from backend import main as main_mod

    def fake_search(self, isbn: str = "", title: str = "", author: str = ""):
        return ([], "", "unknown")

    monkeypatch.setattr(main_mod.Yes24Bookstore, "search", fake_search)
    monkeypatch.setattr(main_mod.AladinBookstore, "search", fake_search)
    monkeypatch.setattr(main_mod.RidibooksBookstore, "search", fake_search)
    monkeypatch.setattr(main_mod.NaverShoppingBookstore, "search", fake_search)
    monkeypatch.setattr(main_mod.NaverSeriesBookstore, "search", fake_search)
    monkeypatch.setattr(main_mod.MunpiaBookstore, "search", fake_search)

    assert dummy_client.get("/search/bookstore/yes24?title=t").json()["status"] == "not_found"
    assert dummy_client.get("/search/bookstore/aladin?title=t").json()["status"] == "not_found"
    assert dummy_client.get("/search/bookstore/ridi?title=t").json()["status"] == "not_found"
    assert dummy_client.get("/search/bookstore/naver?title=t").json()["status"] == "not_found"
    assert dummy_client.get("/search/bookstore/naverseries?title=t").json()["status"] == "not_found"
    assert dummy_client.get("/search/bookstore/munpia?title=t").json()["status"] == "not_found"

    resp = dummy_client.get("/search/bookstore/unknown?title=t")
    assert resp.status_code == 404


def test_category_mapping_error_branches(dummy_client, monkeypatch):
    from backend import main as main_mod

    class DummyMapping:
        def get_all_mappings(self, content_type: str = "book"):
            raise RuntimeError("boom")

        def get_keywords(self, category: str, content_type: str = "book"):
            raise RuntimeError("boom2")

        def set_keywords(self, category: str, keywords, content_type: str = "book"):
            return False

        def add_keyword(self, category: str, keyword: str, content_type: str = "book"):
            raise RuntimeError("boom3")

        def remove_keyword(self, category: str, keyword: str, content_type: str = "book"):
            raise RuntimeError("boom4")

        def delete_category(self, category: str, content_type: str = "book", prefix: bool = False):
            raise RuntimeError("boom5")

        def update_all_mappings(self, mappings, content_type: str = "book"):
            raise RuntimeError("boom6")

        def get_hidden_categories(self, content_type: str = "book"):
            raise RuntimeError("boom7")

        def set_hidden(self, category: str, hidden: bool, content_type: str = "book"):
            raise RuntimeError("boom8")

        def get_latest_excluded_categories(self, content_type: str = "book"):
            raise RuntimeError("boom9")

        def set_latest_excluded(self, category: str, excluded: bool, content_type: str = "book"):
            raise RuntimeError("boom10")

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyMapping())

    resp = dummy_client.get("/category-mappings")
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL

    resp = dummy_client.get("/category-mappings/A")
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL

    resp = dummy_client.put("/category-mappings/A", json={"keywords": ["x"]})
    assert resp.status_code == 500
    assert resp.json()["detail"] == "Failed to set keywords"

    resp = dummy_client.post("/category-mappings/A/keywords", json={"keyword": "x"})
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL

    resp = dummy_client.delete("/category-mappings/A/keywords/x")
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL

    resp = dummy_client.delete("/category-mappings/A")
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL

    resp = dummy_client.put("/category-mappings", json={"mappings": {"A": ["x"]}})
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL

    resp = dummy_client.get("/hidden-categories")
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL

    resp = dummy_client.post("/hidden-categories/A", json={"hidden": True})
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL

    resp = dummy_client.get("/latest-excluded-categories")
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_LATEST_EXCLUDED_CATEGORY_ERROR_DETAIL

    resp = dummy_client.post("/latest-excluded-categories/A", json={"excluded": True})
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_LATEST_EXCLUDED_CATEGORY_ERROR_DETAIL


def test_cookie_settings_variants(monkeypatch: pytest.MonkeyPatch):
    from backend import main as main_mod

    monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
    monkeypatch.setenv("TM_COOKIE_SECURE", "false")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "none")
    secure, samesite = main_mod._get_cookie_settings()
    assert secure is False
    assert samesite == "lax"

    monkeypatch.setenv("TM_COOKIE_SECURE", "true")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "strict")
    secure, samesite = main_mod._get_cookie_settings()
    assert secure is True
    assert samesite == "strict"


def test_set_and_clear_auth_cookies(monkeypatch: pytest.MonkeyPatch):
    from backend import main as main_mod
    from fastapi.responses import JSONResponse

    monkeypatch.setenv("TM_COOKIE_SECURE", "true")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "lax")

    resp = JSONResponse({"ok": True})
    main_mod._set_auth_cookies(resp, "access", "refresh")
    headers = resp.headers.get("set-cookie", "")
    assert "tm_access_token" in headers

    resp2 = JSONResponse({"ok": True})
    main_mod._clear_auth_cookies(resp2)
    headers2 = resp2.headers.get("set-cookie", "")
    assert "Max-Age=0" in headers2


def test_cookie_settings_defaults_to_lax_for_unknown_value(monkeypatch: pytest.MonkeyPatch):
    from backend import main as main_mod

    monkeypatch.setenv("TM_FRONTEND_URL", "http://localhost:3000")
    monkeypatch.setenv("TM_COOKIE_SECURE", "false")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "unexpected")
    secure, samesite = main_mod._get_cookie_settings()
    assert secure is False
    assert samesite == "lax"


def test_set_auth_cookies_without_refresh_token(monkeypatch: pytest.MonkeyPatch):
    from backend import main as main_mod
    from fastapi.responses import JSONResponse

    monkeypatch.setenv("TM_COOKIE_SECURE", "false")
    monkeypatch.setenv("TM_COOKIE_SAMESITE", "lax")

    resp = JSONResponse({"ok": True})
    main_mod._set_auth_cookies(resp, "access-only")
    headers = resp.headers.getlist("set-cookie")
    assert len(headers) == 1
    assert "tm_access_token=access-only" in headers[0]


def test_auth_google_branches(monkeypatch: pytest.MonkeyPatch):
    from backend import main as main_mod
    from fastapi.testclient import TestClient

    client = TestClient(main_mod.app)

    resp = client.post("/auth/google", json={})
    assert resp.status_code == 400

    monkeypatch.setenv("TM_GOOGLE_CLIENT_ID", "cid")
    monkeypatch.setattr(main_mod, "TM_GOOGLE_CLIENT_ID", "cid")
    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: (_ for _ in ()).throw(ValueError("bad token")))
    resp = client.post("/auth/google", json={"credential": "x"})
    assert resp.status_code == 401

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: {"aud": "cid", "iss": "https://evil.example.com", "email": "a", "email_verified": True})
    resp = client.post("/auth/google", json={"credential": "x"})
    assert resp.status_code == 401

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: {"aud": "cid", "iss": "https://accounts.google.com", "email": "a", "email_verified": False, "name": "n", "picture": "p"})
    resp = client.post("/auth/google", json={"credential": "x"})
    assert resp.status_code == 401

    monkeypatch.setattr(main_mod.google_id_token, "verify_oauth2_token", lambda *args, **kwargs: {"aud": "cid", "iss": "https://accounts.google.com", "email": "a", "email_verified": True, "name": "n", "picture": "p"})
    monkeypatch.setattr(main_mod, "determine_role", lambda email: None)
    resp = client.post("/auth/google", json={"credential": "x"})
    assert resp.status_code == 403

    monkeypatch.setattr(main_mod, "determine_role", lambda email: "admin")
    monkeypatch.setattr(main_mod, "_issue_auth_tokens", lambda **kwargs: ("tok", "rtok"))
    resp = client.post("/auth/google", json={"credential": "x"})
    assert resp.status_code == 200
    assert resp.json()["status"] == "success"


def test_auth_refresh_missing_cookie():
    from backend import main as main_mod
    from fastapi.testclient import TestClient

    client = TestClient(main_mod.app)
    resp = client.post("/auth/refresh")
    assert resp.status_code == 400


def test_auth_refresh_access_denied(monkeypatch: pytest.MonkeyPatch):
    from backend import main as main_mod
    from fastapi.testclient import TestClient

    monkeypatch.setattr(main_mod, "decode_refresh_token", lambda token: {"email": "x", "role": "viewer", "fid": "family", "jti": "token"})
    monkeypatch.setattr(main_mod, "determine_role", lambda email: None)

    client = TestClient(main_mod.app)
    resp = client.post("/auth/refresh", cookies={"tm_refresh_token": "r"})
    assert resp.status_code == 403


def test_auth_me(dummy_client):
    resp = dummy_client.get("/auth/me")
    assert resp.status_code == 200
    assert resp.json()["status"] == "success"


# ---- coverage: uncovered lines ----


def test_create_bookstore_factory():
    """Line 177: _create_bookstore factory"""
    from backend.main import _create_bookstore
    from backend.bookstore import Yes24Bookstore

    store = _create_bookstore()
    assert isinstance(store, Yes24Bookstore)
    assert store.verbose is True


def test_create_comics_manager_factory(monkeypatch):
    from backend import main as main_mod

    created = object()
    monkeypatch.setattr(main_mod, "ComicsManager", lambda: created)
    assert main_mod._create_comics_manager() is created


def test_create_category_mapping_factory(monkeypatch):
    from backend import main as main_mod

    created = object()
    monkeypatch.setattr(main_mod, "CategoryMapping", lambda: created)
    assert main_mod._create_category_mapping() is created


def test_update_book_error_path(dummy_client, monkeypatch):
    from backend import main as main_mod

    async def update_fail(*_args, **_kwargs):
        return None, "update failed"

    monkeypatch.setattr(main_mod.book_manager, "update_book", update_fail)
    payload = {"book_id": 1, "category": "A", "title": "T", "author": "U", "file_path": "a.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0}
    resp = dummy_client.put("/books/1", json=payload)
    assert resp.json() == {"status": "failure", "error": "update failed"}


def test_rename_category_error_path(dummy_client, monkeypatch):
    from backend import main as main_mod

    async def rename_fail(old_category, new_category):
        return None, f"cannot rename {old_category} -> {new_category}"

    monkeypatch.setattr(main_mod.book_manager._instance, "rename_category", rename_fail, raising=False)
    resp = dummy_client.put("/categories/rename", json={"old_category": "A", "new_category": "B"})
    assert resp.json() == {"status": "failure", "error": "cannot rename A -> B"}


def test_delete_category_error_path(dummy_client, monkeypatch):
    from backend import main as main_mod

    async def delete_fail(category):
        return None, f"cannot delete {category}"

    monkeypatch.setattr(main_mod.book_manager._instance, "delete_category", delete_fail, raising=False)
    resp = dummy_client.post("/categories/delete", json={"category": "A"})
    assert resp.json() == {"status": "failure", "error": "cannot delete A"}


def test_rename_category_mysql_mapping_failure(dummy_client, monkeypatch):
    """Line 309: rename_category MySQL mapping update failure"""
    from backend import main as main_mod

    async def rename_ok(old_category, new_category):
        return {"old_category": old_category, "new_category": new_category, "updated_count": 1, "fs_renamed": False}, None

    main_mod.book_manager._instance.rename_category = rename_ok

    class DummyCatMap:
        def rename_category(self, old, new, content_type="book"):
            return False

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.put("/categories/rename", json={"old_category": "A", "new_category": "B"})
    data = resp.json()
    assert data["status"] == "success"
    assert data["result"]["mapping_updated"] is False

    del main_mod.book_manager._instance.rename_category


def test_delete_category_hidden_subcategories(dummy_client, monkeypatch):
    """delete_category cleans hidden and latest-excluded subcategories."""
    from backend import main as main_mod

    async def delete_ok(category):
        return {"category": category, "deleted_count": 1}, None

    main_mod.book_manager._instance.delete_category = delete_ok

    hidden_removed = []
    latest_excluded_removed = []

    class DummyCatMap:
        def delete_category(self, cat, content_type="book", prefix=False):
            return True

        def set_hidden(self, cat, hidden, content_type="book"):
            if not hidden:
                hidden_removed.append(cat)
            return True

        def get_hidden_categories(self, content_type="book"):
            return ["A/sub1", "A/sub2", "B/other"]

        def set_latest_excluded(self, cat, excluded, content_type="book"):
            if not excluded:
                latest_excluded_removed.append(cat)
            return True

        def get_latest_excluded_categories(self, content_type="book"):
            return ["A/sub3", "A/sub4", "B/other"]

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.post("/categories/delete", json={"category": "A"})
    data = resp.json()
    assert data["status"] == "success"
    assert "A/sub1" in hidden_removed
    assert "A/sub2" in hidden_removed
    assert "B/other" not in hidden_removed
    assert "A/sub3" in latest_excluded_removed
    assert "A/sub4" in latest_excluded_removed
    assert "B/other" not in latest_excluded_removed

    del main_mod.book_manager._instance.delete_category


def test_set_category_keywords_unexpected_exception(dummy_client, monkeypatch):
    """Lines 673-675: set_category_keywords non-HTTPException error"""
    from backend import main as main_mod

    class DummyCatMap:
        def set_keywords(self, category, keywords, content_type="book"):
            raise TypeError("unexpected")

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.put("/category-mappings/A", json={"keywords": ["x"]})
    assert resp.status_code == 500
    assert resp.json()["detail"] == main_mod.GENERIC_MAPPING_ERROR_DETAIL


def test_update_all_category_mappings_failure(dummy_client, monkeypatch):
    """Line 739: update_all_category_mappings returns False"""
    from backend import main as main_mod

    class DummyCatMap:
        def update_all_mappings(self, mappings, content_type="book"):
            return False

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.put("/category-mappings", json={"mappings": {"A": ["x"]}})
    assert resp.status_code == 500
    assert "Failed to update mappings" in resp.json()["detail"]


def test_update_all_category_mappings_http_exception_reraise(dummy_client, monkeypatch):
    """Line 741: update_all_category_mappings HTTPException re-raise"""
    from backend import main as main_mod
    from fastapi import HTTPException

    class DummyCatMap:
        def update_all_mappings(self, mappings, content_type="book"):
            raise HTTPException(status_code=422, detail="validation error")

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.put("/category-mappings", json={"mappings": {"A": ["x"]}})
    assert resp.status_code == 422


def test_set_hidden_category_failure(dummy_client, monkeypatch):
    """Line 775: set_hidden_category returns False"""
    from backend import main as main_mod

    class DummyCatMap:
        def set_hidden(self, category, hidden, content_type="book"):
            return False

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.post("/hidden-categories/A", json={"hidden": True})
    assert resp.status_code == 500
    assert "Failed to update hidden category" in resp.json()["detail"]


def test_set_hidden_category_http_exception_reraise(dummy_client, monkeypatch):
    """Line 777: set_hidden_category HTTPException re-raise"""
    from backend import main as main_mod
    from fastapi import HTTPException

    class DummyCatMap:
        def set_hidden(self, category, hidden, content_type="book"):
            raise HTTPException(status_code=422, detail="test error")

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.post("/hidden-categories/A", json={"hidden": True})
    assert resp.status_code == 422


def test_delete_book_ok_path(dummy_client, monkeypatch):
    """Lines 238-239: delete_book 'Ok' 경로"""
    from backend import main as main_mod

    async def delete_ok(book_id: int):
        return "Ok", None

    monkeypatch.setattr(main_mod.book_manager, "delete_book", delete_ok)
    resp = dummy_client.delete("/books/1")
    data = resp.json()
    assert data["status"] == "success"
    assert data["result"] == "Ok"


def test_validate_book_not_found(dummy_client, monkeypatch):
    """Lines 270-271: validate_book 책 없음 경로"""
    from backend import main as main_mod

    async def get_book_none(book_id: int):
        return None, "not found"

    monkeypatch.setattr(main_mod.book_manager, "get_book", get_book_none)
    resp = dummy_client.get("/validate/999")
    data = resp.json()
    assert data["status"] == "failure"
    assert "Book not found" in data["error"]


def test_validate_book_unsupported_type(dummy_client, monkeypatch):
    """Lines 278-279: validate_book 지원하지 않는 파일 타입"""
    from backend import main as main_mod

    async def get_book_txt(book_id: int):
        data = {"book_id": book_id, "category": "A", "title": "T", "author": "U", "file_path": "a.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0}
        return DummyBook(data), None

    monkeypatch.setattr(main_mod.book_manager, "get_book", get_book_txt)
    resp = dummy_client.get("/validate/1")
    data = resp.json()
    assert data["status"] == "failure"
    assert "not supported" in data["error"]


def test_validate_book_error_result(dummy_client, monkeypatch):
    """Line 285: validate_book 에러 경로"""
    from backend import main as main_mod

    async def get_book_epub(book_id: int):
        data = {"book_id": book_id, "category": "A", "title": "T", "author": "U", "file_path": "a.epub", "file_type": "epub", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0}
        return DummyBook(data), None

    async def validate_epub_fail(book_id: int):
        return None, "validation failed"

    monkeypatch.setattr(main_mod.book_manager, "get_book", get_book_epub)
    monkeypatch.setattr(main_mod.book_manager, "validate_epub", validate_epub_fail)
    resp = dummy_client.get("/validate/1")
    data = resp.json()
    assert data["status"] == "failure"
    assert data["error"] == "validation failed"


def test_delete_category_mapping_not_deleted(dummy_client, monkeypatch):
    """Line 333: delete_category mapping_deleted=False 경로"""
    from backend import main as main_mod

    async def delete_ok(category):
        return {"category": category, "deleted_count": 1}, None

    main_mod.book_manager._instance.delete_category = delete_ok

    class DummyCatMap:
        def delete_category(self, cat, content_type="book", prefix=False):
            return False

        def set_hidden(self, cat, hidden, content_type="book"):
            return True

        def get_hidden_categories(self, content_type="book"):
            return []

        def set_latest_excluded(self, cat, excluded, content_type="book"):
            return True

        def get_latest_excluded_categories(self, content_type="book"):
            return []

    monkeypatch.setattr(main_mod.category_mapping, "_instance", DummyCatMap())

    resp = dummy_client.post("/categories/delete", json={"category": "A"})
    data = resp.json()
    assert data["status"] == "success"
    assert data["result"]["mapping_deleted"] is False

    del main_mod.book_manager._instance.delete_category


# ---- merged from test_main_env_guard.py ----


def test_main_requires_frontend_url():
    import importlib
    import os
    import sys

    prev = os.environ.pop("TM_FRONTEND_URL", None)
    try:
        if "backend.main" in sys.modules:
            del sys.modules["backend.main"]
        with pytest.raises(SystemExit):
            importlib.import_module("backend.main")
    finally:
        if prev is not None:
            os.environ["TM_FRONTEND_URL"] = prev
