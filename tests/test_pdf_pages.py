#!/usr/bin/env python
"""get_pdf_pages 메서드 및 /pdf-pages/ 엔드포인트 단위 테스트.

Docker/ES 컨테이너 없이 mock 기반으로 실행 가능.
"""

import io
import os
import logging.config
import tempfile
from pathlib import Path
from unittest.mock import patch, MagicMock

import pytest
from fastapi.testclient import TestClient
from pypdf import PdfWriter

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


def create_test_pdf(num_pages: int, output_path: Path) -> None:
    """지정 페이지 수의 테스트 PDF 생성."""
    writer = PdfWriter()
    for i in range(num_pages):
        writer.add_blank_page(width=612, height=792)
    with open(output_path, "wb") as f:
        writer.write(f)


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
def mock_book_manager(temp_dir, test_pdf_path):
    """ESManager를 mock한 BookManager."""
    with patch.dict(os.environ, {
        "TM_WORK_DIR": str(temp_dir),
        "TM_ES_URL": "http://localhost:9200",
        "TM_ES_INDEX": "test",
        "TM_ES_USER": "",
        "TM_ES_PASSWORD": "",
        "TM_FRONTEND_URL": "http://localhost:3000",
    }):
        with patch("backend.book_manager.ESManager") as MockES:
            mock_es = MagicMock()
            MockES.return_value = mock_es
            mock_es.create_index.return_value = None

            from backend.book_manager import BookManager
            bm = BookManager()

            # search_by_id가 테스트 PDF를 가리키는 문서를 반환
            relative_path = str(test_pdf_path.relative_to(temp_dir))
            mock_es.search_by_id.return_value = {
                "category": "test_category",
                "title": "Test Book",
                "author": "Author",
                "file_path": relative_path,
                "file_type": "pdf",
                "file_size": test_pdf_path.stat().st_size,
                "line_count": 0,
                "page_count": 5,
                "isbn": "",
                "summary": "test",
                "updated_time": "2024-01-01T00:00:00.000000",
            }

            yield bm, mock_es


class TestGetPdfPages:
    """BookManager.get_pdf_pages 단위 테스트."""

    @pytest.mark.asyncio
    async def test_returns_single_page(self, mock_book_manager):
        bm, _ = mock_book_manager
        response = await bm.get_pdf_pages(book_id=1, start=1, end=1)

        assert response.status_code == 200
        assert response.headers.get("X-Total-Pages") == "5"
        assert response.media_type == "application/pdf"
        assert response.headers.get("Content-Encoding") == "identity"

        # 반환된 PDF가 1페이지인지 확인
        from pypdf import PdfReader
        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 1

    @pytest.mark.asyncio
    async def test_returns_page_range(self, mock_book_manager):
        bm, _ = mock_book_manager
        response = await bm.get_pdf_pages(book_id=1, start=2, end=4)

        assert response.status_code == 200
        assert response.headers.get("X-Total-Pages") == "5"

        from pypdf import PdfReader
        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 3  # 2, 3, 4

    @pytest.mark.asyncio
    async def test_returns_all_pages(self, mock_book_manager):
        bm, _ = mock_book_manager
        response = await bm.get_pdf_pages(book_id=1, start=1, end=5)

        assert response.status_code == 200
        from pypdf import PdfReader
        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 5

    @pytest.mark.asyncio
    async def test_end_clamped_to_total_pages(self, mock_book_manager):
        bm, _ = mock_book_manager
        response = await bm.get_pdf_pages(book_id=1, start=3, end=100)

        assert response.status_code == 200
        from pypdf import PdfReader
        reader = PdfReader(io.BytesIO(response.body))
        assert len(reader.pages) == 3  # 3, 4, 5

    @pytest.mark.asyncio
    async def test_start_exceeds_total_pages(self, mock_book_manager):
        bm, _ = mock_book_manager
        response = await bm.get_pdf_pages(book_id=1, start=100, end=200)

        assert response.status_code == 400

    @pytest.mark.asyncio
    async def test_book_not_found(self, mock_book_manager):
        bm, mock_es = mock_book_manager

        original = mock_es.search_by_id.return_value
        mock_es.search_by_id.return_value = None

        response = await bm.get_pdf_pages(book_id=999, start=1, end=1)
        assert response.status_code == 404

        mock_es.search_by_id.return_value = original

    @pytest.mark.asyncio
    async def test_cache_hit(self, mock_book_manager, temp_dir):
        bm, _ = mock_book_manager

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
    async def test_not_a_pdf_file(self, mock_book_manager, temp_dir):
        bm, mock_es = mock_book_manager

        # txt 파일 생성
        txt_path = temp_dir / "test_category" / "test.txt"
        txt_path.write_text("hello")

        original = mock_es.search_by_id.return_value
        mock_es.search_by_id.return_value = {
            **original,
            "file_path": "test_category/test.txt",
            "file_type": "txt",
        }

        response = await bm.get_pdf_pages(book_id=1, start=1, end=1)
        assert response.status_code == 400

        mock_es.search_by_id.return_value = original


class TestPdfPagesEndpoint:
    """FastAPI /pdf-pages/{book_id} 엔드포인트 테스트."""

    @pytest.fixture(autouse=True)
    def setup_client(self, mock_book_manager):
        bm, _ = mock_book_manager

        with patch.dict(os.environ, {
            "TM_FRONTEND_URL": "http://localhost:3000",
        }):
            # main.py의 book_manager를 mock된 것으로 교체
            from backend import main
            original_bm = main.book_manager
            main.book_manager = bm
            self.client = TestClient(main.app)
            yield
            main.book_manager = original_bm

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
        # X-Total-Pages 헤더가 CORS expose_headers에 포함되어야 함
        assert response.headers.get("X-Total-Pages") == "5"

    def test_get_pdf_pages_start_exceeds_total(self):
        response = self.client.get("/pdf-pages/1?start=100&end=200")
        assert response.status_code == 400
