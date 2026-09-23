#!/usr/bin/env python
"""get_cover 메서드 및 /cover/ 엔드포인트 단위 테스트.

Docker/ES 컨테이너 없이 mock 기반으로 실행 가능.
"""

import io
import os
import logging.config
import tempfile
import zipfile
from pathlib import Path
from unittest.mock import patch, MagicMock

import pytest
from fastapi import HTTPException
from fastapi.testclient import TestClient
from PIL import Image
from pypdf import PdfWriter

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
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

_CONTAINER_XML = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"""


def _png_bytes(width: int = 600, height: int = 900) -> bytes:
    buf = io.BytesIO()
    Image.new("RGB", (width, height), (200, 30, 30)).save(buf, format="PNG")
    return buf.getvalue()


def create_test_epub(output_path: Path, metadata: str = "", manifest_items: str = "", images: dict[str, bytes] | None = None) -> None:
    """OPF metadata/manifest 조각과 이미지 멤버를 받아 최소 EPUB을 만든다."""
    opf = f"""<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>T</dc:title>{metadata}</metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    {manifest_items}
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"""
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output_path, "w") as z:
        z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", _CONTAINER_XML)
        z.writestr("OEBPS/content.opf", opf)
        z.writestr("OEBPS/ch1.xhtml", "<html><body><p>x</p></body></html>")
        for name, data in (images or {}).items():
            z.writestr(f"OEBPS/{name}", data)


def create_test_pdf(output_path: Path) -> None:
    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "wb") as f:
        writer.write(f)


def _make_doc(relative_path: str, file_type: str) -> dict:
    return {"category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type, "file_size": 1, "line_count": 0, "page_count": 1, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"}


def _assert_jpeg_thumbnail(body: bytes) -> None:
    img = Image.open(io.BytesIO(body))
    assert img.format == "JPEG"
    assert img.width <= 300


@pytest.fixture(scope="module")
def temp_dir():
    with tempfile.TemporaryDirectory() as d:
        yield Path(d)


@pytest.fixture(scope="module")
def book_manager_module(temp_dir):
    """ESManager를 mock한 BookManager (모듈 스코프).

    backend.book 을 reload 하지 않는다. env 를 패치한 채 reload 하면 Book.path_prefix 가
    temp_dir 로 굳어 뒤에 도는 다른 테스트 모듈이 깨진다. path_prefix 는 patch.object 로 되돌린다.
    """
    env = {**_ENV, "TM_BOOK_DIR": str(temp_dir), "TM_COMICS_DIR": str(temp_dir)}
    import backend.book_manager as bm_mod

    with patch.dict(os.environ, env), patch.object(bm_mod, "ESManager") as MockES, patch.object(bm_mod.BookManager.item_class, "path_prefix", temp_dir):
        mock_es = MagicMock()
        MockES.return_value = mock_es
        mock_es.create_index.return_value = None
        yield bm_mod.BookManager(), mock_es


def _use_doc(mock_es, relative_path: str, file_type: str) -> None:
    mock_es.search_by_id.return_value = _make_doc(relative_path, file_type)


class TestGetCover:
    """BookManager.get_cover 단위 테스트."""

    @pytest.mark.asyncio
    async def test_epub_cover_image_property(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        create_test_epub(temp_dir / "test_category" / "prop.epub", manifest_items='<item id="img" href="images/front.png" media-type="image/png" properties="cover-image"/>', images={"images/front.png": _png_bytes()})
        _use_doc(mock_es, "test_category/prop.epub", "epub")

        response = await bm.get_cover(book_id=101)

        assert response.status_code == 200
        assert response.media_type == "image/jpeg"
        _assert_jpeg_thumbnail(response.body)

    @pytest.mark.asyncio
    async def test_epub_meta_cover(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        create_test_epub(temp_dir / "test_category" / "meta.epub", metadata='<meta name="cover" content="img1"/>', manifest_items='<item id="img1" href="a.jpg" media-type="image/png"/>', images={"a.jpg": _png_bytes()})
        _use_doc(mock_es, "test_category/meta.epub", "epub")

        response = await bm.get_cover(book_id=102)

        assert response.status_code == 200
        _assert_jpeg_thumbnail(response.body)

    @pytest.mark.asyncio
    async def test_epub_cover_named_item(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        create_test_epub(temp_dir / "test_category" / "named.epub", manifest_items='<item id="i1" href="img/other.png" media-type="image/png"/><item id="i2" href="img/Cover.png" media-type="image/png"/>', images={"img/other.png": _png_bytes(10, 10), "img/Cover.png": _png_bytes()})
        _use_doc(mock_es, "test_category/named.epub", "epub")

        response = await bm.get_cover(book_id=103)

        assert response.status_code == 200
        # other.png(10x10)가 아니라 Cover.png(600x900)를 골라야 세로 비율이 나온다
        img = Image.open(io.BytesIO(response.body))
        assert img.height > img.width

    @pytest.mark.asyncio
    async def test_epub_without_cover_returns_404(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        create_test_epub(temp_dir / "test_category" / "nocover.epub", manifest_items='<item id="i1" href="fig.png" media-type="image/png"/>', images={"fig.png": _png_bytes()})
        _use_doc(mock_es, "test_category/nocover.epub", "epub")

        response = await bm.get_cover(book_id=104)

        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_broken_epub_returns_404(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        broken = temp_dir / "test_category" / "broken.epub"
        broken.write_bytes(b"not a zip")
        _use_doc(mock_es, "test_category/broken.epub", "epub")

        response = await bm.get_cover(book_id=105)

        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_pdf_first_page(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        create_test_pdf(temp_dir / "test_category" / "doc.pdf")
        _use_doc(mock_es, "test_category/doc.pdf", "pdf")

        response = await bm.get_cover(book_id=106)

        assert response.status_code == 200
        _assert_jpeg_thumbnail(response.body)

    @pytest.mark.asyncio
    async def test_text_format_returns_404(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        txt = temp_dir / "test_category" / "a.txt"
        txt.write_text("hello")
        _use_doc(mock_es, "test_category/a.txt", "txt")

        response = await bm.get_cover(book_id=107)

        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_book_not_found(self, book_manager_module):
        bm, mock_es = book_manager_module
        mock_es.search_by_id.return_value = None

        response = await bm.get_cover(book_id=999)

        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_cache_hit_skips_extraction(self, book_manager_module, temp_dir):
        bm, mock_es = book_manager_module
        create_test_pdf(temp_dir / "test_category" / "cached.pdf")
        _use_doc(mock_es, "test_category/cached.pdf", "pdf")

        first = await bm.get_cover(book_id=108)
        assert first.status_code == 200
        assert (temp_dir / ".cover_cache" / "108.jpg").exists()

        with patch.object(type(bm), "_extract_cover_thumbnail", side_effect=AssertionError("must use cache")):
            second = await bm.get_cover(book_id=108)

        assert second.status_code == 200


class TestCoverEndpoint:
    """FastAPI /cover/{book_id} 엔드포인트 테스트."""

    @pytest.fixture(autouse=True)
    def setup_client(self, book_manager_module, temp_dir):
        _, mock_es = book_manager_module

        with patch("backend.comics_manager.ESManager") as MockComicsES, patch("backend.category_mapping.CategoryMapping._init_db"):
            MockComicsES.return_value = MagicMock()

            from backend import main
            from backend.book import Book

            orig_es = main.book_manager.es_manager
            orig_prefix = main.book_manager.path_prefix
            orig_book_prefix = Book.path_prefix

            main.book_manager.es_manager = mock_es
            main.book_manager.path_prefix = temp_dir
            Book.path_prefix = temp_dir
            # 다른 테스트가 backend.book 을 reload 했으면 manager 는 이전 Book 클래스를 쥐고 있다.
            orig_item_prefix = main.book_manager.item_class.path_prefix
            main.book_manager.item_class.path_prefix = temp_dir
            create_test_pdf(temp_dir / "test_category" / "endpoint.pdf")
            _use_doc(mock_es, "test_category/endpoint.pdf", "pdf")

            from backend.auth import create_jwt_token

            self.main = main
            token = create_jwt_token(email="admin@test.com", role="admin", name="Test Admin")
            self.client = TestClient(main.app, cookies={"tm_access_token": token})
            yield

            main.book_manager.es_manager = orig_es
            main.book_manager.path_prefix = orig_prefix
            Book.path_prefix = orig_book_prefix
            main.book_manager.item_class.path_prefix = orig_item_prefix

    def test_returns_jpeg(self):
        response = self.client.get("/cover/201")
        assert response.status_code == 200
        assert response.headers["content-type"] == "image/jpeg"

    def test_requires_auth(self):
        response = TestClient(self.main.app).get("/cover/201")
        assert response.status_code == 401

    def test_checks_viewer_access(self):
        with patch.object(self.main, "_ensure_viewer_book_allowed", side_effect=HTTPException(status_code=403, detail="forbidden")):
            response = self.client.get("/cover/201")
        assert response.status_code == 403
