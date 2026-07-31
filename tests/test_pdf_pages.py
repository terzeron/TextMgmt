#!/usr/bin/env python
"""get_pdf_pages 메서드 및 /pdf-pages/ 엔드포인트 단위 테스트.

Docker/ES 컨테이너 없이 mock 기반으로 실행 가능.
"""

import errno
import io
import os
import logging.config
import tempfile
from pathlib import Path
from unittest.mock import patch, MagicMock
import importlib

import pytest
from fastapi.testclient import TestClient
from pypdf import PdfWriter, PdfReader

logging.config.fileConfig(
    Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False
)
LOGGER = logging.getLogger(__name__)

_ENV = {
    "TM_BOOK_DIR": "",  # temp_dir로 덮어씌움
    "TM_COMICS_DIR": "",
    "TM_ES_COMICS_INDEX": "test_comics",
    "TM_ES_URL": "http://localhost:9200",
    "TM_ES_BOOK_INDEX": "test",
    "TM_ES_USER": "",
    "TM_ES_PASSWORD": "",
    "TM_FRONTEND_URL": "http://localhost:3000",
    "TM_JWT_SECRET": "test_jwt_secret_for_testing_minimum_32bytes",
    "TM_ADMIN_EMAIL": "admin@test.com",
    "TM_ALLOWED_EMAILS": "viewer@test.com",
}


def create_test_pdf(num_pages: int, output_path: Path) -> None:
    """지정 페이지 수의 테스트 PDF 생성."""
    writer = PdfWriter()
    for _ in range(num_pages):
        writer.add_blank_page(width=612, height=792)
    with open(output_path, "wb") as f:
        writer.write(f)


def _make_doc(
    relative_path: str, file_size: int, page_count: int = 5, file_type: str = "pdf"
) -> dict:
    """ES 문서 dict 생성 헬퍼."""
    return {
        "category": "test_category",
        "title": "Test Book",
        "author": "Author",
        "file_path": relative_path,
        "file_type": file_type,
        "file_size": file_size,
        "line_count": 0,
        "page_count": page_count,
        "isbn": "",
        "summary": "test",
        "updated_time": "2024-01-01T00:00:00.000000",
    }


@pytest.fixture(scope="module")
def temp_dir():
    """임시 디렉토리 제공."""
    with tempfile.TemporaryDirectory() as d:
        yield Path(d)


@pytest.fixture(scope="module")
def test_pdf_path(temp_dir):
    """5페이지 테스트 PDF 생성."""
    pdf_path = temp_dir / "test_category" / "test_book.pdf"
    pdf_path.parent.mkdir(parents=True, exist_ok=True)
    create_test_pdf(5, pdf_path)
    return pdf_path


@pytest.fixture(scope="module")
def _default_doc(temp_dir, test_pdf_path):
    """기본 ES 문서 (모듈 전체에서 재사용)."""
    relative_path = str(test_pdf_path.relative_to(temp_dir))
    return _make_doc(relative_path, test_pdf_path.stat().st_size)


@pytest.fixture(scope="module")
def book_manager_module(temp_dir, _default_doc):
    """ESManager를 mock한 BookManager (모듈 스코프)."""
    env = {**_ENV, "TM_BOOK_DIR": str(temp_dir), "TM_COMICS_DIR": str(temp_dir)}
    with patch.dict(os.environ, env):
        import backend.book as book_mod
        import backend.book_manager as bm_mod
        importlib.reload(book_mod)
        importlib.reload(bm_mod)

        with patch.object(bm_mod, "ESManager") as MockES:
            mock_es = MagicMock()
            MockES.return_value = mock_es
            mock_es.create_index.return_value = None
            mock_es.search_by_id.return_value = _default_doc

            BookManager = bm_mod.BookManager
            Book = book_mod.Book

            bm = BookManager()
            # Book.path_prefix는 import 시점에 고정되므로 temp_dir로 패치
            original_prefix = Book.path_prefix
            Book.path_prefix = temp_dir
            yield bm, mock_es
            Book.path_prefix = original_prefix


@pytest.fixture(autouse=True)
def _reset_mock_es(book_manager_module, _default_doc):
    """각 테스트 전에 mock_es.search_by_id를 기본 상태로 리셋."""
    _, mock_es = book_manager_module
    mock_es.search_by_id.return_value = _default_doc
    yield


class TestGetPdfPages:
    """BookManager.get_pdf_pages 단위 테스트."""

    @pytest.mark.asyncio
    async def test_returns_single_page(self, book_manager_module):
        bm, _ = book_manager_module
        response = await bm.get_pdf_pages(book_id=1, start=1, end=1)

        assert response.status_code == 200
        assert response.headers.get("X-Total-Pages") == "5"
        assert response.media_type == "application/pdf"
        assert response.headers.get("Content-Encoding") == "identity"

        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 1

    @pytest.mark.asyncio
    async def test_returns_page_range(self, book_manager_module):
        bm, _ = book_manager_module
        response = await bm.get_pdf_pages(book_id=1, start=2, end=4)

        assert response.status_code == 200
        assert response.headers.get("X-Total-Pages") == "5"

        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 3  # 2, 3, 4

    @pytest.mark.asyncio
    async def test_returns_all_pages(self, book_manager_module):
        bm, _ = book_manager_module
        response = await bm.get_pdf_pages(book_id=1, start=1, end=5)

        assert response.status_code == 200
        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 5

    @pytest.mark.asyncio
    async def test_end_clamped_to_total_pages(self, book_manager_module):
        bm, _ = book_manager_module
        response = await bm.get_pdf_pages(book_id=1, start=3, end=100)

        assert response.status_code == 200
        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 3  # 3, 4, 5

    @pytest.mark.asyncio
    async def test_start_exceeds_total_pages(self, book_manager_module):
        bm, _ = book_manager_module
        response = await bm.get_pdf_pages(book_id=1, start=100, end=200)

        assert response.status_code == 400

    @pytest.mark.asyncio
    async def test_book_not_found(self, book_manager_module):
        bm, mock_es = book_manager_module
        mock_es.search_by_id.return_value = None

        response = await bm.get_pdf_pages(book_id=999, start=1, end=1)
        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_cache_hit(self, book_manager_module, temp_dir):
        bm, _ = book_manager_module

        # 첫 번째 호출: 캐시 생성
        response1 = await bm.get_pdf_pages(book_id=1, start=1, end=2)
        assert response1.status_code == 200

        # 캐시 파일 존재 확인
        cache_file = temp_dir / ".preview_cache" / "1_p1-2.pdf"
        assert cache_file.exists()

        # 두 번째 호출: 캐시 히트 (같은 결과)
        response2 = await bm.get_pdf_pages(book_id=1, start=1, end=2)
        assert response2.status_code == 200
        assert response2.headers.get("X-Total-Pages") == "5"

    @pytest.mark.asyncio
    async def test_not_a_pdf_file(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module

        txt_path = temp_dir / "test_category" / "test.txt"
        txt_path.write_text("hello")

        mock_es.search_by_id.return_value = _make_doc(
            "test_category/test.txt",
            txt_path.stat().st_size,
            file_type="txt",
        )

        response = await bm.get_pdf_pages(book_id=1, start=1, end=1)
        assert response.status_code == 400

    @pytest.mark.asyncio
    async def test_missing_file_returns_404(self, book_manager_module):
        bm, mock_es = book_manager_module
        mock_es.search_by_id.return_value = _make_doc("test_category/gone.pdf", 1234)

        response = await bm.get_pdf_pages(book_id=1, start=1, end=1)
        assert response.status_code == 404
        assert b"File not found" in response.body

    @pytest.mark.asyncio
    async def test_storage_io_error_returns_503_with_reason(self, book_manager_module):
        """스토리지 마운트가 끊겨 stat()이 EIO를 내면 사유를 담은 503을 반환해야 한다.

        예전에는 이 OSError가 그대로 escape해 CORSMiddleware를 우회한 500이 나갔고,
        브라우저에는 "Failed to fetch"로만 보여 원인을 알 수 없었다.
        """
        bm, _ = book_manager_module
        eio = OSError(errno.EIO, os.strerror(errno.EIO))

        with patch.object(Path, "is_file", side_effect=eio):
            response = await bm.get_pdf_pages(book_id=1, start=1, end=1)

        assert response.status_code == 503
        assert b"Storage access error" in response.body
        assert os.strerror(errno.EIO).encode() in response.body


class TestPdfPagesEndpoint:
    """FastAPI /pdf-pages/{book_id} 엔드포인트 테스트."""

    @pytest.fixture(autouse=True)
    def setup_client(self, book_manager_module, temp_dir, _default_doc):
        _, mock_es = book_manager_module

        with (
            patch("backend.comics_manager.ESManager") as MockComicsES,
            patch("backend.category_mapping.CategoryMapping._init_db"),
        ):
            mock_comics_es = MagicMock()
            MockComicsES.return_value = mock_comics_es
            mock_comics_es.create_index.return_value = None

            from backend import main
            from backend.book import Book

            # 라우터 클로저가 캡처한 manager의 내부 속성을 패치
            orig_es = main.book_manager.es_manager
            orig_prefix = main.book_manager.path_prefix
            orig_book_prefix = Book.path_prefix

            main.book_manager.es_manager = mock_es
            main.book_manager.path_prefix = temp_dir
            Book.path_prefix = temp_dir
            try:
                main.book_manager.item_class.path_prefix = temp_dir
            except Exception:
                pass
            mock_es.search_by_id.return_value = _default_doc

            from backend.auth import create_jwt_token

            token = create_jwt_token(
                email="admin@test.com", role="admin", name="Test Admin"
            )
            self.client = TestClient(
                main.app, cookies={"tm_access_token": token}
            )
            yield

            main.book_manager.es_manager = orig_es
            main.book_manager.path_prefix = orig_prefix
            Book.path_prefix = orig_book_prefix

    def test_get_pdf_pages_default_params(self):
        response = self.client.get("/pdf-pages/1")
        assert response.status_code == 200
        assert "X-Total-Pages" in response.headers

    def test_get_pdf_pages_with_range(self):
        response = self.client.get("/pdf-pages/1?start=2&end=3")
        assert response.status_code == 200
        assert response.headers["X-Total-Pages"] == "5"
        assert response.headers["content-type"] == "application/pdf"

    def test_get_pdf_pages_x_total_pages_exposed(self):
        response = self.client.get("/pdf-pages/1?start=1&end=1")
        assert response.status_code == 200
        assert response.headers.get("X-Total-Pages") == "5"

    def test_get_pdf_pages_start_exceeds_total(self):
        response = self.client.get("/pdf-pages/1?start=100&end=200")
        assert response.status_code == 400
