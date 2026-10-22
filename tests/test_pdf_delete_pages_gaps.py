#!/usr/bin/env python
"""BookManager.delete_pdf_pages 의 드문 분기(깨진 PDF 구조, 저장 실패, 재색인 실패)를 확인한다.

ES/Docker 없이 실행된다. 픽스처와 PDF 생성 도구는 tests/test_pdf_delete_pages.py 를 재사용한다.
"""

import io
import zlib
from unittest.mock import patch

import pikepdf
import pytest
from PIL import Image

import backend.book_manager as bm_mod
from tests.test_pdf_delete_pages import (  # noqa: F401  (픽스처)
    _crop,
    _first_image,
    _make_pixel_pdf,
    _upload,
    env,
    image_env,
    loader,
    pixel_env,
)


def _srgb_profile():
    from PIL import ImageCms

    return ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()


def _edit_pdf(path, edit):
    """PDF 를 열어 edit(pdf) 로 고친 뒤 같은 경로에 저장한다."""
    with pikepdf.open(path, allow_overwriting_input=True) as pdf:
        edit(pdf)
        pdf.save(path)


def _assert_untouched(bm, es, pdf, root, original):
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    assert not list(pdf.parent.glob(".*.part"))
    es.delete.assert_not_called()


# ── 크롭할 수 없는 PDF 구조 ──


def _no_contents(pdf):
    del pdf.pages[0].Contents


def _missing_image(pdf):
    pdf.pages[0].Resources.XObject = pikepdf.Dictionary()


def _bad_matrix(pdf):
    pdf.pages[0].Contents = pdf.make_stream(b"q abc 0 0 60 0 0 cm /Im0 Do Q")


def _partial_cover(pdf):
    pdf.pages[0].Contents = pdf.make_stream(b"q 50 0 0 30 0 0 cm /Im0 Do Q")


def _cmyk_jpeg(pdf):
    next(iter(pdf.pages[0].Resources.XObject.values())).ColorSpace = pikepdf.Name.DeviceCMYK


def _wrong_declared_size(pdf):
    next(iter(pdf.pages[0].Resources.XObject.values())).Width = 99


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "edit,expected",
    [
        (_no_contents, "리소스 없음"),
        (_missing_image, "이미지를 찾을 수 없음"),
        (_bad_matrix, "배치 행렬"),
        (_partial_cover, "쪽 전체를 덮지 않음"),
        (_cmyk_jpeg, "색상 구성"),
        (_wrong_declared_size, "크기 불일치"),
    ],
)
async def test_crop_rejects_broken_page_structure_and_changes_nothing(image_env, loader, edit, expected):
    bm, es, pdf, root = image_env
    _edit_pdf(pdf, edit)
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0.5, 0, 1, 1)])

    assert result is None and "1쪽" in error and expected in error
    _assert_untouched(bm, es, pdf, root, original)


# ── 크롭 결과의 구조 ──


@pytest.mark.asyncio
async def test_crop_accepts_single_item_filter_array(image_env, loader):
    bm, es, pdf, _ = image_env

    def wrap_filter(doc):
        image = next(iter(doc.pages[1].Resources.XObject.values()))
        image.Filter = pikepdf.Array([pikepdf.Name.DCTDecode])

    _edit_pdf(pdf, wrap_filter)

    result, error = await bm.delete_pdf_pages(1, [], [_crop(2, 0.5, 0, 1, 1)])

    assert error is None and result["cropped_pages"] == 1
    assert _first_image(pdf, 2)[:2] == (50, 60)


@pytest.mark.asyncio
async def test_crop_keeps_icc_color_space_when_channel_count_matches(pixel_env, loader):
    bm, _, pdf, _ = pixel_env

    def use_icc(doc):
        profile = doc.make_stream(_srgb_profile())
        profile.N = 3
        image = next(iter(doc.pages[0].Resources.XObject.values()))
        image.ColorSpace = pikepdf.Array([pikepdf.Name.ICCBased, profile])

    _edit_pdf(pdf, use_icc)

    result, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0.3, 0.2, 0.9, 0.8)])

    assert error is None and result["cropped_pages"] == 1
    with pikepdf.open(pdf) as doc:
        image = next(iter(doc.pages[0].Resources.XObject.values()))
        assert image.ColorSpace[0] == "/ICCBased" and int(image.ColorSpace[1].N) == 3
        assert str(image.Filter) == "/FlateDecode"


@pytest.mark.asyncio
async def test_crop_removes_stale_page_boxes(image_env, loader):
    bm, _, pdf, _ = image_env

    def add_boxes(doc):
        page = doc.pages[1]
        for key in ("/CropBox", "/TrimBox", "/BleedBox", "/ArtBox"):
            page[key] = pikepdf.Array([0, 0, 100, 60])

    _edit_pdf(pdf, add_boxes)

    _, error = await bm.delete_pdf_pages(1, [], [_crop(2, 0.5, 0, 1, 1)])

    assert error is None
    with pikepdf.open(pdf) as doc:
        page = doc.pages[1]
        assert not any(key in page for key in ("/CropBox", "/TrimBox", "/BleedBox", "/ArtBox"))
        assert float(page.mediabox[2]) == pytest.approx(50.0)


# ── 올린 이미지 ──


@pytest.mark.asyncio
async def test_upload_preview_rejects_too_many_pixels(image_env):
    bm, *_ = image_env

    with patch("backend.epub_cover.MAX_IMAGE_PIXELS", 100):
        jpeg, error = await bm.preview_insert_upload(_png_bytes(20, 20))

    assert jpeg is None and "해상도가 너무 큽니다" in error


@pytest.mark.asyncio
async def test_upload_insert_with_too_many_pixels_changes_nothing(image_env, loader):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    with patch("backend.epub_cover.MAX_IMAGE_PIXELS", 100):
        result, error = await bm.delete_pdf_pages(1, [], None, None, [_upload(2, _png_bytes(20, 20))])

    assert result is None and "해상도가 너무 큽니다" in error
    _assert_untouched(bm, es, pdf, root, original)


def _png_bytes(width, height):
    out = io.BytesIO()
    Image.new("RGB", (width, height), (10, 120, 200)).save(out, "PNG")
    return out.getvalue()


@pytest.mark.asyncio
async def test_upload_preview_hides_unexpected_errors(image_env):
    bm, *_ = image_env

    with patch.object(bm_mod.BookManager, "_normalize_uploaded_image", side_effect=RuntimeError("secret detail")):
        jpeg, error = await bm.preview_insert_upload(b"data")

    assert jpeg is None and error == "이미지를 읽지 못했습니다"


# ── 저장 단계 실패 ──


@pytest.mark.asyncio
async def test_missing_file_is_reported(env, loader):
    bm, es, pdf, _ = env
    pdf.unlink()

    result, error = await bm.delete_pdf_pages(1, [1])

    assert result is None and error == "File not found: book.pdf"
    es.delete.assert_not_called()


@pytest.mark.asyncio
async def test_os_error_while_saving_reports_the_system_reason(env, loader):
    bm, es, pdf, root = env
    original = pdf.read_bytes()

    with patch.object(bm_mod.BookManager, "_remove_pdf_pages_with_backup", side_effect=OSError(28, "No space left on device")):
        result, error = await bm.delete_pdf_pages(1, [1])

    assert result is None and error == "파일 처리 실패: No space left on device"
    _assert_untouched(bm, es, pdf, root, original)


@pytest.mark.asyncio
async def test_unexpected_error_while_saving_names_only_the_exception_type(env, loader):
    bm, es, pdf, root = env
    original = pdf.read_bytes()

    with patch.object(bm_mod.BookManager, "_remove_pdf_pages_with_backup", side_effect=RuntimeError("internal path /x/y")):
        result, error = await bm.delete_pdf_pages(1, [1])

    assert result is None and error == "PDF 페이지 삭제 실패 (RuntimeError)"
    _assert_untouched(bm, es, pdf, root, original)


# ── 재색인 실패 ──


@pytest.mark.asyncio
async def test_unreadable_saved_file_reports_backup_location(env):
    bm, es, pdf, root = env

    with patch("utils.loader.Loader.read_file", return_value={}):
        result, error = await bm.delete_pdf_pages(1, [3])

    assert result is None
    assert "재색인할 문서를 읽지 못했습니다" in error and "0_trash/book.pdf" in error
    bm.add_book.assert_not_awaited()
    assert (root / "0_trash" / "book.pdf").is_file()


@pytest.mark.asyncio
async def test_es_delete_failure_stops_before_reindexing(env, loader):
    bm, es, pdf, root = env
    es.delete.return_value = False

    result, error = await bm.delete_pdf_pages(1, [3])

    assert result is None
    assert "ES 문서 삭제 실패" in error and "0_trash/book.pdf" in error
    bm.add_book.assert_not_awaited()
