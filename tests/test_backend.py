#!/usr/bin/env python

import logging.config
import shutil
from pathlib import Path

import pytest

logging.config.fileConfig(
    Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False
)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

CATEGORY = "_epub"


@pytest.fixture(scope="module")
def backend_test_setup(es_client, es_index, admin_auth_header):
    """Create BookManager and TestClient with test data loaded (공유된 ES 클라이언트 및 인덱스 사용)."""
    from fastapi.testclient import TestClient
    from backend.main import app
    from backend.book_manager import BookManager
    from utils.loader import Loader

    # Create BookManager and use shared ES client
    bm = BookManager()
    bm.es_manager.es = es_client

    # Load test data from actual files if available
    epub_path = bm.path_prefix / CATEGORY
    if epub_path.exists():
        data = Loader.read_files(epub_path, num_files=5)
        if data:
            bm.es_manager.insert(data, num_docs=20)
            LOGGER.info("Inserted %d epub documents", len(data))

    # Refresh index to make data searchable
    bm.es_manager.refresh()

    client = TestClient(app, headers=admin_auth_header)

    yield {"bm": bm, "client": client}


@pytest.fixture
def test_book(backend_test_setup):
    """Create a temporary test book for each test."""
    import asyncio
    from backend.book import Book
    from utils.loader import Loader

    bm = backend_test_setup["bm"]
    client = backend_test_setup["client"]

    epub_files = list(Book.path_prefix.glob(f"{CATEGORY}/*.epub"))
    if not epub_files:
        pytest.skip("No epub files available for testing")

    epub_file_path = epub_files[0]
    temp_file_path = (
        Book.path_prefix / epub_file_path.parent.name / ("temp_" + epub_file_path.name)
    )
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

        # Cleanup: delete the test book
        try:
            client.delete(f"/books/{book.book_id}")
        except Exception:
            pass
    finally:
        loop.close()


class TestBackend:
    @pytest.mark.asyncio
    async def test_update_book(self, test_book):
        book = test_book["book"]
        client = test_book["client"]

        doc = {
            "book_id": book.book_id,
            "category": book.category,
            "title": "renamed_" + book.title,
            "author": "anonymous_" + book.author,
            "file_path": book.category
            + "/renamed_"
            + book.title
            + "."
            + book.file_type,
            "file_type": book.file_type,
            "file_size": 100,
            "summary": "summary1",
            "updated_time": "2021-01-01T00:00:00.000000",
        }

        response = client.put(f"/books/{book.book_id}", json=doc)
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": "Ok"}

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
        temp_file_path = (
            Book.path_prefix
            / epub_file_path.parent.name
            / ("to_be_deleted_" + epub_file_path.name)
        )
        shutil.copy(epub_file_path, temp_file_path)
        data = Loader.read_file(temp_file_path)

        book_id, error = await bm.add_book(data)
        assert book_id and not error

        book, error = await bm.get_book(book_id)
        assert book and not error

        response = client.delete(f"/books/{book.book_id}")
        assert response
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": "Ok"}

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
        media_type = BookManager.MEDIA_TYPES.get(
            book.file_path.suffix, "application/octet-stream"
        )
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
            excluded_ids = [
                b["book_id"]
                for b in response_data["result"]
                if b["category"] == book.category
            ]
            assert len(excluded_ids) == 0, (
                f"제외된 카테고리 '{book.category}'의 책이 포함됨"
            )

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
            response = client.put(
                "/categories/rename",
                json={
                    "old_category": old_cat,
                    "new_category": new_cat,
                },
            )
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
            response = client.put(
                "/categories/rename",
                json={
                    "old_category": old_cat,
                    "new_category": new_cat,
                },
            )
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

        response = client.put(
            "/categories/rename",
            json={
                "old_category": "_nonexistent_cat_xyz",
                "new_category": "_new_cat_xyz",
            },
        )
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

        response = client.post(
            "/categories/delete",
            json={
                "category": "_nonexistent_del_xyz",
            },
        )
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
        data = {
            file_path.stat().st_ino: {
                "category": category,
                "title": file_path.stem,
                "author": "Test Author",
                "file_path": str(rel),
                "file_type": file_path.suffix.lstrip("."),
                "file_size": file_path.stat().st_size,
                "line_count": 0,
                "page_count": 0,
                "isbn": "",
                "summary": "conflict test",
                "updated_time": now,
            }
        }
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
            doc = {
                "book_id": book_id,
                "category": CATEGORY,
                "title": "conflict_dst",
                "author": "Test Author",
                "file_path": f"{CATEGORY}/conflict_dst.epub",
                "file_type": "epub",
                "file_size": 100,
                "updated_time": "2021-01-01T00:00:00.000000",
            }

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "failure"
            assert "CONFLICT:" in data["error"]
            assert src_path.exists(), "Source file should not have been moved"
            assert dst_path.read_bytes() == b"existing", (
                "Destination should be unchanged"
            )
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
            doc = {
                "book_id": book_id,
                "category": CATEGORY,
                "title": "conflict_relpath_dst",
                "author": "Test Author",
                "file_path": f"{CATEGORY}/conflict_relpath_dst.epub",
                "file_type": "epub",
                "file_size": 100,
                "updated_time": "2021-01-01T00:00:00.000000",
            }

            response = client.put(f"/books/{book_id}", json=doc)
            data = response.json()
            error_msg = data.get("error", "")
            assert f"{CATEGORY}/conflict_relpath_dst.epub" in error_msg, (
                f"CONFLICT error should contain relative path, got: {error_msg}"
            )
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
            doc = {
                "book_id": book_id,
                "category": CATEGORY,
                "title": "force_dst",
                "author": "Test Author",
                "file_path": f"{CATEGORY}/force_dst.epub",
                "file_type": "epub",
                "file_size": 100,
                "updated_time": "2021-01-01T00:00:00.000000",
            }

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
            doc = {
                "book_id": book_id,
                "category": CATEGORY,
                "title": "samefile_test",
                "author": "Changed Author Name",
                "file_path": f"{CATEGORY}/samefile_test.epub",
                "file_type": "epub",
                "file_size": 100,
                "updated_time": "2021-01-01T00:00:00.000000",
            }

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "success", (
                f"Same file path should not cause conflict, got: {data}"
            )
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
            doc = {
                "book_id": book_id,
                "category": new_dir,
                "title": "movedir_test",
                "author": "Test Author",
                "file_path": f"{new_dir}/movedir_test.epub",
                "file_type": "epub",
                "file_size": 100,
                "updated_time": "2021-01-01T00:00:00.000000",
            }

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
            doc = {
                "book_id": book_id,
                "category": CATEGORY,
                "title": "missing_renamed",
                "author": "Test Author",
                "file_path": f"{CATEGORY}/missing_renamed.epub",
                "file_type": "epub",
                "file_size": 100,
                "updated_time": "2021-01-01T00:00:00.000000",
            }

            response = client.put(f"/books/{book_id}", json=doc)
            assert response.status_code == 200
            data = response.json()
            assert data["status"] == "failure"
            assert (
                "can't move" in data.get("error", "").lower()
                or "error" in data.get("error", "").lower()
            )
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
        from backend.auth import create_refresh_token

        refresh_token = create_refresh_token(
            email=self.auth_mod.TM_ADMIN_EMAIL, role="admin"
        )
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", json={"refresh_token": refresh_token})
        assert response.status_code == 200
        data = response.json()
        assert "token" in data

    def test_refresh_with_expired_token_returns_401(self, backend_test_setup):
        import time
        import jwt

        expired_payload = {
            "type": "refresh",
            "email": self.auth_mod.TM_ADMIN_EMAIL,
            "role": "admin",
            "exp": int(time.time()) - 100,
            "iat": int(time.time()) - 200,
        }
        from backend.auth import JWT_SECRET, JWT_ALGORITHM

        token = jwt.encode(expired_payload, JWT_SECRET, algorithm=JWT_ALGORITHM)
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", json={"refresh_token": token})
        assert response.status_code == 401

    def test_refresh_with_access_token_returns_401(self, backend_test_setup):
        from backend.auth import create_jwt_token

        access_token = create_jwt_token(
            email=self.auth_mod.TM_ADMIN_EMAIL, role="admin"
        )
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", json={"refresh_token": access_token})
        assert response.status_code == 401

    def test_refresh_without_token_returns_400(self, backend_test_setup):
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", json={})
        assert response.status_code == 400

    def test_refresh_with_unauthorized_email_returns_403(self, backend_test_setup):
        from backend.auth import create_refresh_token

        refresh_token = create_refresh_token(email="hacker@evil.com", role="admin")
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", json={"refresh_token": refresh_token})
        assert response.status_code == 403

    def test_refreshed_token_works_for_api_calls(self, backend_test_setup):
        from backend.auth import create_refresh_token

        refresh_token = create_refresh_token(
            email=self.auth_mod.TM_ADMIN_EMAIL, role="admin"
        )
        client = self._get_unauthenticated_client(backend_test_setup)
        response = client.post("/auth/refresh", json={"refresh_token": refresh_token})
        new_token = response.json()["token"]
        # 새 토큰으로 인증 필요한 API 호출
        from fastapi.testclient import TestClient
        from backend.main import app

        auth_client = TestClient(app, headers={"Authorization": f"Bearer {new_token}"})
        cat_response = auth_client.get("/categories")
        assert cat_response.status_code == 200


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
