import threading
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import AsyncMock

import pytest
from PIL import Image

from backend.cover_status import STANDARD_COVER_SIZES, CoverStatusCache, has_official_cover, pdf_first_image_size

LOCK = threading.Lock()


def make_pdf(path: Path, size: tuple[int, int] | None) -> Path:
    if size is None:
        from pypdf import PdfWriter

        writer = PdfWriter()
        writer.add_blank_page(width=100, height=100)
        with open(path, "wb") as fp:
            writer.write(fp)
        return path
    Image.new("RGB", size, (30, 30, 200)).save(path, format="PDF", resolution=72)
    return path


@pytest.mark.parametrize("size", sorted(STANDARD_COVER_SIZES))
def test_pdf_with_standard_cover_size_is_covered(tmp_path, size):
    pdf = make_pdf(tmp_path / "a.pdf", size)

    assert pdf_first_image_size(pdf, LOCK) == size
    assert has_official_cover(pdf, "pdf", LOCK) is True


def test_pdf_with_other_first_image_is_uncovered(tmp_path):
    pdf = make_pdf(tmp_path / "a.pdf", (690, 1600))

    assert has_official_cover(pdf, "pdf", LOCK) is False


def test_pdf_without_image_is_uncovered(tmp_path):
    pdf = make_pdf(tmp_path / "a.pdf", None)

    assert pdf_first_image_size(pdf, LOCK) is None
    assert has_official_cover(pdf, "pdf", LOCK) is False


def test_unsupported_type_is_not_reported_uncovered(tmp_path):
    assert has_official_cover(tmp_path / "a.txt", "txt", LOCK) is True


def test_cache_reuses_result_until_file_changes(tmp_path):
    pdf = make_pdf(tmp_path / "a.pdf", (690, 1600))
    cache = CoverStatusCache(tmp_path / ".cover_cache" / "status.sqlite3")
    calls = []

    def compute():
        calls.append(1)
        return has_official_cover(pdf, "pdf", LOCK)

    assert cache.get_or_compute(pdf, compute) is False
    assert cache.get_or_compute(pdf, compute) is False
    assert len(calls) == 1

    make_pdf(pdf, (480, 623))
    assert cache.get_or_compute(pdf, compute) is True
    assert len(calls) == 2


def make_manager(tmp_path):
    from backend.book_manager import BookManager

    manager = BookManager.__new__(BookManager)
    manager.path_prefix = tmp_path
    return manager


def book(name: str, tmp_path: Path, size: tuple[int, int]):
    return SimpleNamespace(file_path=make_pdf(tmp_path / name, size), file_type="pdf", name=name)


@pytest.mark.asyncio
async def test_uncovered_page_collects_across_scan_pages_and_keeps_scan_cursor(tmp_path):
    covered = book("covered.pdf", tmp_path, (480, 623))
    plain1 = book("plain1.pdf", tmp_path, (690, 1600))
    plain2 = book("plain2.pdf", tmp_path, (690, 1600))
    manager = make_manager(tmp_path)
    manager.get_cover_books_paged = AsyncMock(side_effect=[([(covered, ""), (plain1, "")], 3, "c1", None), ([(plain2, "")], 3, "c2", None)])

    items, total, next_cursor, error = await manager.get_uncovered_books_paged(size=2)

    assert error is None
    assert [entry[0].name for entry in items] == ["plain1.pdf", "plain2.pdf"]
    assert total is None
    assert next_cursor == "c2"
    assert manager.get_cover_books_paged.await_args_list[1].kwargs["cursor"] == "c1"


@pytest.mark.asyncio
async def test_uncovered_page_ends_when_list_ends(tmp_path):
    covered = book("covered.pdf", tmp_path, (480, 623))
    manager = make_manager(tmp_path)
    manager.get_cover_books_paged = AsyncMock(return_value=([(covered, "")], 1, None, None))

    items, total, next_cursor, error = await manager.get_uncovered_books_paged(size=5)

    assert (items, total, next_cursor, error) == ([], None, None, None)


@pytest.mark.asyncio
async def test_uncovered_page_stops_at_budget_even_without_items(tmp_path):
    covered = book("covered.pdf", tmp_path, (480, 623))
    manager = make_manager(tmp_path)
    manager.UNCOVERED_SCAN_BUDGET_SECONDS = -1
    manager.get_cover_books_paged = AsyncMock(side_effect=[([(covered, "")], 9, "c1", None), ([(covered, "")], 9, "c2", None)])

    items, _total, next_cursor, error = await manager.get_uncovered_books_paged(size=5)

    assert (items, next_cursor, error) == ([], "c1", None)
    assert manager.get_cover_books_paged.await_count == 1


@pytest.mark.asyncio
async def test_uncovered_page_stops_at_page_cap_and_returns_resume_cursor(tmp_path):
    covered = book("covered.pdf", tmp_path, (480, 623))
    manager = make_manager(tmp_path)
    manager.UNCOVERED_MAX_SCAN_PAGES = 2
    manager.get_cover_books_paged = AsyncMock(side_effect=[([(covered, "")], 9, "c1", None), ([(covered, "")], 9, "c2", None), ([(covered, "")], 9, "c3", None)])

    items, _total, next_cursor, _error = await manager.get_uncovered_books_paged(size=5)

    assert items == []
    assert next_cursor == "c2"
    assert manager.get_cover_books_paged.await_count == 2


@pytest.mark.asyncio
async def test_uncovered_scans_run_at_most_two_at_once(tmp_path):
    import asyncio

    manager = make_manager(tmp_path)
    running = 0
    peak = 0

    async def slow_page(**_kwargs):
        nonlocal running, peak
        running += 1
        peak = max(peak, running)
        await asyncio.sleep(0.02)
        running -= 1
        return [], 0, None, None

    manager.get_cover_books_paged = slow_page

    await asyncio.gather(*(manager.get_uncovered_books_paged() for _ in range(6)))

    assert peak == manager.UNCOVERED_MAX_CONCURRENT_SCANS


@pytest.mark.asyncio
async def test_uncovered_page_passes_through_errors(tmp_path):
    manager = make_manager(tmp_path)
    manager.get_cover_books_paged = AsyncMock(return_value=([], 0, None, "invalid cursor"))

    assert await manager.get_uncovered_books_paged(cursor="x") == ([], None, None, "invalid cursor")


@pytest.mark.asyncio
async def test_unreadable_file_is_not_reported_uncovered(tmp_path):
    broken = SimpleNamespace(file_path=tmp_path / "missing.pdf", file_type="pdf", name="missing.pdf")
    manager = make_manager(tmp_path)

    assert manager._uncovered_flags([broken]) == [False]


def test_pdf_without_pages_has_no_first_image(tmp_path, monkeypatch):
    import pypdfium2

    closed = []

    class EmptyDocument:
        def __init__(self, fp):
            pass

        def __len__(self):
            return 0

        def close(self):
            closed.append(True)

    monkeypatch.setattr(pypdfium2, "PdfDocument", EmptyDocument)
    pdf = tmp_path / "empty.pdf"
    pdf.write_bytes(b"%PDF-1.4")

    assert pdf_first_image_size(pdf, LOCK) is None
    assert has_official_cover(pdf, "pdf", LOCK) is False
    assert closed == [True, True]


@pytest.mark.parametrize("cover", [False, True])
def test_epub_is_judged_by_cover_metadata(tmp_path, cover):
    import io
    import zipfile

    path = tmp_path / "book.epub"
    item = '<item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>' if cover else ""
    image = io.BytesIO()
    Image.new("RGB", (120, 180), "red").save(image, "JPEG")
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        archive.writestr("META-INF/container.xml", '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>')
        archive.writestr("OPS/book.opf", f'<package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>t</dc:title></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>{item}</manifest><spine><itemref idref="chapter"/></spine></package>')
        archive.writestr("OPS/chapter.xhtml", "<html><body>x</body></html>")
        if cover:
            archive.writestr("OPS/cover.jpg", image.getvalue())

    assert has_official_cover(path, "epub", LOCK) is cover
