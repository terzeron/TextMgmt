#!/usr/bin/env python
"""BookManager.delete_pdf_pages 단위 테스트 (ES/Docker 없이 mock 기반)."""

from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from pypdf import PdfReader, PdfWriter

import backend.book as book_mod
import backend.book_manager as bm_mod

_ENV = {
    "TM_BOOK_DIR": "",
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


def _make_pdf(path, num_pages):
    path.parent.mkdir(parents=True, exist_ok=True)
    writer = PdfWriter()
    for i in range(num_pages):
        # 페이지마다 크기를 달리해 어느 페이지가 남았는지 구분한다.
        writer.add_blank_page(width=100 + i, height=200)
    with open(path, "wb") as f:
        writer.write(f)


def _widths(path):
    return [int(p.mediabox.width) for p in PdfReader(str(path)).pages]


@pytest.fixture
def env(tmp_path, monkeypatch):
    pdf = tmp_path / "cat" / "book.pdf"
    _make_pdf(pdf, 5)
    doc = {"category": "cat", "title": "T", "author": "A", "file_path": "cat/book.pdf", "file_type": "pdf", "file_size": pdf.stat().st_size, "page_count": 5, "updated_time": "2024-01-01T00:00:00.000000"}
    # 모듈을 reload 하지 않는다: reload 하면 다른 테스트 파일이 들고 있는 옛 클래스와 어긋나 전체 실행에서 오염된다.
    for key, value in {**_ENV, "TM_BOOK_DIR": str(tmp_path), "TM_COMICS_DIR": str(tmp_path)}.items():
        monkeypatch.setenv(key, value)
    es = MagicMock()
    es.create_index.return_value = None
    es.search_by_id.return_value = doc
    es.delete.return_value = True
    monkeypatch.setattr(bm_mod, "ESManager", MagicMock(return_value=es))
    monkeypatch.setattr(book_mod.Book, "path_prefix", tmp_path)
    bm = bm_mod.BookManager()
    bm.add_book = AsyncMock(return_value=(1, None))
    return bm, es, pdf, tmp_path


@pytest.fixture
def loader():
    reindexed = {"category": "x", "title": "x", "author": "x", "file_path": "x", "file_type": "pdf", "file_size": 1, "page_count": 3}
    with patch("utils.loader.Loader.read_file", return_value={1: reindexed}) as m:
        yield m


@pytest.mark.asyncio
async def test_reindexed_document_gets_a_fresh_updated_time(env):
    """저장한 PDF 는 최신 책 목록(updated_time 순)에 오른다. 모킹 없이 실제 Loader.read_file 로 재색인 문서를 만든다."""
    from datetime import datetime

    bm, es, pdf, root = env

    result, error = await bm.delete_pdf_pages(1, [2])

    assert error is None
    indexed = bm.add_book.await_args.args[0][1]
    assert indexed["page_count"] == 4
    assert indexed["updated_time"] > "2024-01-01T00:00:00.000000"
    assert abs((datetime.now() - datetime.fromisoformat(indexed["updated_time"])).total_seconds()) < 60


@pytest.mark.asyncio
async def test_deletes_pages_and_keeps_original_in_trash(env, loader):
    bm, es, pdf, root = env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [2, 4])

    assert error is None
    assert result == {"remaining_pages": 3, "cropped_pages": 0, "split_pages": 0, "inserted_pages": 0, "backup": "0_trash/book.pdf", "backup_created": True}
    assert _widths(pdf) == [100, 102, 104]
    backup = root / "0_trash" / "book.pdf"
    assert backup.read_bytes() == original
    es.delete.assert_called_once_with(1)
    bm.add_book.assert_awaited_once()
    indexed = bm.add_book.await_args.args[0][1]
    assert indexed["category"] == "cat" and indexed["file_path"] == "cat/book.pdf"


@pytest.mark.asyncio
async def test_existing_backup_is_not_overwritten(env, loader):
    bm, _, pdf, root = env
    original = pdf.read_bytes()

    await bm.delete_pdf_pages(1, [1])
    result, error = await bm.delete_pdf_pages(1, [1])

    assert error is None
    assert result["backup_created"] is False
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    assert _widths(pdf) == [102, 103, 104]


@pytest.mark.asyncio
@pytest.mark.parametrize("pages", [[], [0], [6], [1, 2, 3, 4, 5]])
async def test_invalid_pages_leave_file_untouched(env, loader, pages):
    bm, es, pdf, root = env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, pages)

    assert result is None and error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    es.delete.assert_not_called()


@pytest.mark.asyncio
async def test_duplicate_pages_count_once(env, loader):
    bm, _, pdf, _ = env
    result, error = await bm.delete_pdf_pages(1, [2, 2, 2])
    assert error is None
    assert result["remaining_pages"] == 4


@pytest.mark.asyncio
async def test_non_pdf_is_rejected(env, loader):
    bm, es, _, _ = env
    es.search_by_id.return_value = {"category": "c", "title": "t", "author": "a", "file_path": "cat/book.epub", "file_type": "epub", "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000"}
    result, error = await bm.delete_pdf_pages(1, [1])
    assert result is None and "Not a PDF" in error


@pytest.mark.asyncio
async def test_unknown_book(env, loader):
    bm, es, _, _ = env
    es.search_by_id.return_value = None
    result, error = await bm.delete_pdf_pages(1, [1])
    assert result is None and "not found" in error.lower()


@pytest.mark.asyncio
async def test_reindex_failure_reports_backup_location(env, loader):
    bm, _, pdf, root = env
    bm.add_book = AsyncMock(return_value=(None, "boom"))

    result, error = await bm.delete_pdf_pages(1, [3])

    assert result is None
    assert "0_trash/book.pdf" in error
    assert (root / "0_trash" / "book.pdf").is_file()


# ── 쪽 순서 바꾸기 ──


@pytest.mark.asyncio
@pytest.mark.parametrize("order,expected", [([2, 1], [101, 100, 102, 103, 104]), ([1, 3, 2], [100, 102, 101, 103, 104]), ([5, 4, 3, 2, 1], [104, 103, 102, 101, 100]), ([2, 3, 1], [101, 102, 100, 103, 104])])
async def test_order_rearranges_pages_and_keeps_original_in_trash(env, loader, order, expected):
    bm, es, pdf, root = env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], order=order)

    assert error is None
    assert result["remaining_pages"] == 5 and result["backup"] == "0_trash/book.pdf"
    assert _widths(pdf) == expected
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    es.delete.assert_called_once_with(1)


@pytest.mark.asyncio
async def test_order_and_delete_use_original_page_numbers(env, loader):
    bm, _, pdf, _ = env

    result, error = await bm.delete_pdf_pages(1, [1], order=[2, 1, 3])

    assert error is None and result["remaining_pages"] == 4
    assert _widths(pdf) == [101, 102, 103, 104]


@pytest.mark.asyncio
@pytest.mark.parametrize("order", [[1, 1], [2, 3], [0, 1], [1, 2, 3, 4, 5, 6], [1, 3]])
async def test_invalid_order_changes_nothing(env, loader, order):
    bm, es, pdf, root = env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], order=order)

    assert result is None and "순서" in error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    es.delete.assert_not_called()


# ── 이미지 크롭 ──


def _make_image_pdf(path, pages, size=(100, 60)):
    """쪽마다 왼쪽 절반은 빨강, 오른쪽 절반은 파랑인 JPEG 한 장짜리 PDF를 만든다.

    pages 원소: "image" | "text"(이미지 없는 쪽) | "rotated"(회전된 이미지 쪽)
    """
    import io

    import pikepdf
    from PIL import Image

    width, height = size
    source = Image.new("RGB", size, (255, 0, 0))
    source.paste((0, 0, 255), (width // 2, 0, width, height))
    jpeg = io.BytesIO()
    source.save(jpeg, "JPEG", quality=95, subsampling=0)

    pdf = pikepdf.new()
    for kind in pages:
        page = pdf.add_blank_page(page_size=(width, height))
        if kind == "text":
            page.Contents = pdf.make_stream(b"BT /F1 12 Tf 10 10 Td (hello) Tj ET")
            continue
        image = pdf.make_stream(jpeg.getvalue())
        image.Type = pikepdf.Name.XObject
        image.Subtype = pikepdf.Name.Image
        image.Width, image.Height = width, height
        image.ColorSpace = pikepdf.Name.DeviceRGB
        image.BitsPerComponent = 8
        image.Filter = pikepdf.Name.DCTDecode
        page.Resources = pikepdf.Dictionary(XObject=pikepdf.Dictionary(Im0=image))
        page.Contents = pdf.make_stream(f"q {width} 0 0 {height} 0 0 cm /Im0 Do Q".encode())
        if kind == "rotated":
            page.Rotate = 90
    path.parent.mkdir(parents=True, exist_ok=True)
    pdf.save(path)


def _first_image(path, page_no):
    """(너비, 높이, 중앙 픽셀 RGB, MediaBox 너비)를 돌려준다."""
    import io

    import pikepdf
    from PIL import Image

    with pikepdf.open(path) as pdf:
        page = pdf.pages[page_no - 1]
        image = next(iter(page.Resources.XObject.values()))
        decoded = Image.open(io.BytesIO(image.read_raw_bytes())).convert("RGB")
        box = [float(v) for v in page.mediabox]
        return int(image.Width), int(image.Height), decoded.getpixel((decoded.width // 2, decoded.height // 2)), box[2] - box[0]


@pytest.fixture
def image_env(env):
    bm, es, pdf, root = env
    _make_image_pdf(pdf, ["image", "image", "text", "image"])
    return bm, es, pdf, root


def _crop(page, x0, y0, x1, y1, angle=0.0):
    return {"page": page, "x0": x0, "y0": y0, "x1": x1, "y1": y1, "angle": angle}


def _decoded(path, page_no):
    import io

    import pikepdf
    from PIL import Image

    with pikepdf.open(path) as pdf:
        image = next(iter(pdf.pages[page_no - 1].Resources.XObject.values()))
        return Image.open(io.BytesIO(image.read_raw_bytes())).convert("RGB")


@pytest.mark.asyncio
async def test_crop_keeps_only_the_given_region(image_env, loader):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], [_crop(2, 0.5, 0, 1, 1)])

    assert error is None
    assert result["remaining_pages"] == 4 and result["cropped_pages"] == 1
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    width, height, center, box_w = _first_image(pdf, 2)
    assert (width, height) == (50, 60) and box_w == pytest.approx(50.0)
    assert center[2] > 200 and center[0] < 60  # 오른쪽(파랑)만 남았다
    assert _first_image(pdf, 1)[:2] == (100, 60)  # 다른 쪽은 그대로
    es.delete.assert_called_once_with(1)


@pytest.mark.asyncio
async def test_crop_arbitrary_rectangle_resizes_page_proportionally(image_env, loader):
    bm, _, pdf, _ = image_env

    _, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0.25, 0.25, 0.75, 0.75)])

    assert error is None
    import pikepdf

    width, height, _, box_w = _first_image(pdf, 1)
    assert (width, height) == (50, 30) and box_w == pytest.approx(50.0)
    with pikepdf.open(pdf) as doc:
        box = [float(v) for v in doc.pages[0].mediabox]
        assert box == pytest.approx([0, 0, 50, 30])


@pytest.mark.asyncio
async def test_crop_and_page_delete_use_original_page_numbers(image_env, loader):
    bm, _, pdf, _ = image_env

    result, error = await bm.delete_pdf_pages(1, [1], [_crop(4, 0.5, 0, 1, 1)])

    assert error is None
    assert result["remaining_pages"] == 3
    assert _first_image(pdf, 3)[:2] == (50, 60)  # 원본 4쪽이 3쪽이 되고 오른쪽 절반만 남음
    assert _first_image(pdf, 1)[:2] == (100, 60)  # 원본 2쪽


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "pages,crops",
    [
        ([], [_crop(3, 0.5, 0, 1, 1)]),  # 이미지 없는 쪽
        ([], [_crop(1, 0.5, 0, 1, 1), _crop(3, 0.5, 0, 1, 1)]),  # 하나라도 실패하면 전부 취소
        ([2], [_crop(2, 0.5, 0, 1, 1)]),  # 전체 삭제와 겹침
        ([], [_crop(9, 0.5, 0, 1, 1)]),  # 쪽 범위 밖
        ([], [_crop(1, 0.6, 0, 0.4, 1)]),  # x0 >= x1
        ([], [_crop(1, -0.1, 0, 1, 1)]),  # 쪽 밖
        ([], [_crop(1, 0, 0, 1.2, 1)]),  # 쪽 밖
        ([], [_crop(1, 0.5, 0.5, 0.55, 0.55)]),  # 너무 작음
        ([], [_crop(1, 0, 0, 0.5, 1), _crop(1, 0.5, 0, 1, 1)]),  # 같은 쪽에 둘 이상
    ],
)
async def test_unsupported_crop_changes_nothing(image_env, loader, pages, crops):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, pages, crops)

    assert result is None and error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    assert not list(pdf.parent.glob(".*.part"))
    es.delete.assert_not_called()


@pytest.mark.asyncio
async def test_rotated_crop_straightens_the_region(image_env, loader):
    bm, _, pdf, root = image_env
    original = pdf.read_bytes()

    # 시계 방향 90°: 박스의 위쪽이 화면의 오른쪽을 향한다 → 출력 윗줄은 원본 오른쪽(파랑), 아랫줄은 왼쪽(빨강)
    result, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0.35, 0.1, 0.65, 0.9, 90)])

    assert error is None and result["cropped_pages"] == 1
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    width, height, _, box_w = _first_image(pdf, 1)
    assert (width, height) == (30, 48) and box_w == pytest.approx(30.0)
    image = _decoded(pdf, 1)
    top, bottom = image.getpixel((15, 3)), image.getpixel((15, 44))
    assert top[2] > 200 and top[0] < 60
    assert bottom[0] > 200 and bottom[2] < 60


@pytest.mark.asyncio
async def test_rotated_crop_output_size_follows_the_box(image_env, loader):
    bm, _, pdf, _ = image_env

    _, error = await bm.delete_pdf_pages(1, [], [_crop(2, 0.4, 0.3, 0.6, 0.7, 30)])

    assert error is None
    width, height, _, box_w = _first_image(pdf, 2)
    assert (width, height) == (20, 24) and box_w == pytest.approx(20.0)
    assert _first_image(pdf, 1)[:2] == (100, 60)  # 다른 쪽은 그대로


@pytest.mark.asyncio
async def test_rotated_split_pages(image_env, loader):
    bm, _, pdf, _ = image_env

    split = {"page": 1, "first": {"x0": 0.35, "y0": 0.1, "x1": 0.65, "y1": 0.9, "angle": 90.0}, "second": {"x0": 0.1, "y0": 0.1, "x1": 0.4, "y1": 0.9}}
    result, error = await bm.delete_pdf_pages(1, [], None, [split])

    assert error is None and result["remaining_pages"] == 5
    assert _first_image(pdf, 1)[:2] == (30, 48)
    assert _first_image(pdf, 2)[:2] == (30, 48)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "crop",
    [
        _crop(1, 0, 0, 1, 1, 10),  # 기울이면 꼭짓점이 쪽 밖으로 나간다
        _crop(1, 0.05, 0.05, 0.95, 0.95, 45),
        _crop(1, 0.2, 0.2, 0.8, 0.8, 400),  # 각도 범위 밖
        _crop(1, 0.2, 0.2, 0.8, 0.8, float("nan")),
        _crop(1, 0.5, 0.5, 0.52, 0.52, 20),  # 너무 작음
    ],
)
async def test_invalid_rotated_crop_changes_nothing(image_env, loader, crop):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], [crop])

    assert result is None and error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    es.delete.assert_not_called()


def _split(page, first, second):
    box = lambda r: dict(zip(("x0", "y0", "x1", "y1"), r))  # noqa: E731
    return {"page": page, "first": box(first), "second": box(second)}


LEFT, RIGHT = (0, 0, 0.5, 1), (0.5, 0, 1, 1)  # image_env 의 왼쪽은 빨강, 오른쪽은 파랑


def _page_count(path):
    import pikepdf

    with pikepdf.open(path) as pdf:
        return len(pdf.pages)


@pytest.mark.asyncio
async def test_split_replaces_the_page_with_first_then_second(image_env, loader):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    # 앞쪽은 오른쪽(파랑), 뒤쪽은 왼쪽(빨강)으로 지정한다: 순서는 영역이 아니라 first/second 가 정한다.
    result, error = await bm.delete_pdf_pages(1, [], None, [_split(2, RIGHT, LEFT)])

    assert error is None
    assert result["remaining_pages"] == 5 and result["split_pages"] == 1
    assert _page_count(pdf) == 5
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    first, second = _first_image(pdf, 2), _first_image(pdf, 3)
    assert first[:2] == (50, 60) and first[3] == pytest.approx(50.0)
    assert second[:2] == (50, 60) and second[3] == pytest.approx(50.0)
    assert first[2][2] > 200 and first[2][0] < 60  # 2쪽: 파랑
    assert second[2][0] > 200 and second[2][2] < 60  # 3쪽: 빨강
    assert _first_image(pdf, 1)[:2] == (100, 60)  # 앞의 쪽은 그대로
    assert _first_image(pdf, 5)[:2] == (100, 60)  # 뒤의 쪽은 한 칸 밀렸다
    es.delete.assert_called_once_with(1)


@pytest.mark.asyncio
async def test_split_pages_move_together_with_their_page_when_reordered(image_env, loader):
    bm, _, pdf, _ = image_env

    result, error = await bm.delete_pdf_pages(1, [], None, [_split(2, LEFT, RIGHT)], None, [2, 1])

    assert error is None and result["remaining_pages"] == 5
    assert [_first_image(pdf, n)[:2] for n in (1, 2, 3)] == [(50, 60), (50, 60), (100, 60)]


@pytest.mark.asyncio
async def test_split_boxes_may_overlap(image_env, loader):
    bm, _, pdf, _ = image_env

    _, error = await bm.delete_pdf_pages(1, [], None, [_split(1, (0, 0, 0.6, 1), (0.4, 0, 1, 1))])

    assert error is None
    assert _first_image(pdf, 1)[:2] == (60, 60)
    assert _first_image(pdf, 2)[:2] == (60, 60)


@pytest.mark.asyncio
async def test_split_and_delete_use_original_page_numbers(image_env, loader):
    bm, _, pdf, _ = image_env

    # 원본 1쪽 삭제 + 원본 2쪽 분할 + 원본 4쪽 크롭: 편집본은 [2a, 2b, 3(text), 4(crop)]
    result, error = await bm.delete_pdf_pages(1, [1], [_crop(4, 0.5, 0, 1, 1)], [_split(2, LEFT, RIGHT)])

    assert error is None
    assert result["remaining_pages"] == 4
    assert [_first_image(pdf, n)[:2] for n in (1, 2)] == [(50, 60), (50, 60)]
    assert _first_image(pdf, 4)[:2] == (50, 60)
    import pikepdf

    with pikepdf.open(pdf) as doc:
        assert "/XObject" not in doc.pages[2].Resources  # 3쪽은 이미지 없는 원본 3쪽


@pytest.mark.asyncio
async def test_delete_after_split_removes_the_right_page(image_env, loader):
    bm, _, pdf, _ = image_env

    result, error = await bm.delete_pdf_pages(1, [4], None, [_split(2, LEFT, RIGHT)])

    assert error is None and result["remaining_pages"] == 4  # 4 + 1 - 1
    assert _page_count(pdf) == 4


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "pages,crops,splits",
    [
        ([], None, [_split(3, LEFT, RIGHT)]),  # 이미지 없는 쪽
        ([], None, [_split(1, LEFT, RIGHT), _split(3, LEFT, RIGHT)]),  # 하나라도 실패하면 전부 취소
        ([2], None, [_split(2, LEFT, RIGHT)]),  # 삭제와 겹침
        ([], [_crop(2, 0, 0, 1, 1)], [_split(2, LEFT, RIGHT)]),  # 크롭과 겹침
        ([], None, [_split(9, LEFT, RIGHT)]),  # 쪽 범위 밖
        ([], None, [_split(1, (0.6, 0, 0.4, 1), RIGHT)]),  # 앞 영역이 비어 있음
        ([], None, [_split(1, LEFT, (0.5, 0.5, 0.55, 0.55))]),  # 뒤 영역이 너무 작음
        ([], None, [_split(1, LEFT, RIGHT), _split(1, LEFT, RIGHT)]),  # 같은 쪽에 둘 이상
    ],
)
async def test_unsupported_split_changes_nothing(image_env, loader, pages, crops, splits):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, pages, crops, splits)

    assert result is None and error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    assert not list(pdf.parent.glob(".*.part"))
    es.delete.assert_not_called()


# ── 이미지 URL 로 쪽 끼워넣기 ──


def _png(width=200, height=300):
    """위쪽 절반은 초록, 아래쪽 절반은 노랑인 PNG 바이트."""
    import io

    from PIL import Image

    image = Image.new("RGB", (width, height), (0, 200, 0))
    image.paste((230, 230, 0), (0, height // 2, width, height))
    out = io.BytesIO()
    image.save(out, "PNG")
    return out.getvalue()


@pytest.fixture
def fetch():
    with patch("backend.direct_cover_url.fetch_url_image", return_value=_png()) as m:
        yield m


def _insert(page, url="https://img.example.com/a.png"):
    return {"page": page, "url": url}


def _mediabox_size(path, page_no):
    import pikepdf

    with pikepdf.open(path) as pdf:
        box = [float(v) for v in pdf.pages[page_no - 1].mediabox]
        return box[2] - box[0], box[3] - box[1]


@pytest.mark.asyncio
async def test_insert_adds_an_image_page_before_the_given_page(image_env, loader, fetch):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(2)])

    assert error is None
    assert result["remaining_pages"] == 5 and result["inserted_pages"] == 1
    fetch.assert_called_once_with("https://img.example.com/a.png")
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    assert _page_count(pdf) == 5
    # 새 2쪽: 폭은 기준 쪽(원본 2쪽, 100pt)에 맞추고 높이는 이미지 비율(200x300 → 150pt)
    width, height = _mediabox_size(pdf, 2)
    assert width == pytest.approx(100.0) and height == pytest.approx(150.0)
    image = _decoded(pdf, 2)
    assert image.size == (200, 300)
    assert image.getpixel((100, 40))[1] > 150 and image.getpixel((100, 40))[0] < 80  # 위쪽: 초록
    assert image.getpixel((100, 260))[0] > 180 and image.getpixel((100, 260))[2] < 80  # 아래쪽: 노랑
    assert _first_image(pdf, 3)[:2] == (100, 60)  # 원본 2쪽은 한 칸 밀렸다
    assert _first_image(pdf, 1)[:2] == (100, 60)
    es.delete.assert_called_once_with(1)


@pytest.mark.asyncio
async def test_inserted_page_moves_together_with_its_reference_page_when_reordered(image_env, loader, fetch):
    bm, _, pdf, _ = image_env

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(2)], [2, 1])

    assert error is None and result["remaining_pages"] == 5
    assert _mediabox_size(pdf, 1)[1] == pytest.approx(150.0)  # 삽입한 쪽이 기준 쪽(원본 2쪽)과 함께 맨 앞으로 갔다
    assert _mediabox_size(pdf, 2)[1] == pytest.approx(60.0)
    assert _mediabox_size(pdf, 3)[1] == pytest.approx(60.0)


@pytest.mark.asyncio
async def test_inserted_page_can_be_cropped_later(image_env, loader, fetch):
    bm, _, pdf, _ = image_env

    _, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(1)])
    assert error is None
    result, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0, 0, 0.5, 0.5)])

    assert error is None and result["cropped_pages"] == 1
    assert _first_image(pdf, 1)[:2] == (100, 150)


@pytest.mark.asyncio
async def test_insert_with_delete_split_and_crop_uses_original_numbers(image_env, loader, fetch):
    bm, _, pdf, _ = image_env
    import pikepdf

    # 원본 [1, 2, 3(text), 4] → 1 삭제, 2 앞에 삽입, 2 분할, 4 앞에 삽입
    # 기대: [삽입, 2a, 2b, 3(text), 삽입, 4]
    result, error = await bm.delete_pdf_pages(1, [1], None, [_split(2, LEFT, RIGHT)], [_insert(2), _insert(4)])

    assert error is None
    assert result["remaining_pages"] == 6 and result["inserted_pages"] == 2
    # 삽입 쪽의 폭은 기준 쪽의 편집 전 폭(100pt)이다: 분할로 기준 쪽이 반쪽(50pt)이 되어도 같다.
    assert _mediabox_size(pdf, 1) == pytest.approx((100.0, 150.0))
    assert _first_image(pdf, 2)[:2] == (50, 60)
    assert _first_image(pdf, 3)[:2] == (50, 60)
    with pikepdf.open(pdf) as doc:
        assert "/XObject" not in doc.pages[3].Resources  # 원본 3쪽(text)
    assert _mediabox_size(pdf, 5)[1] == pytest.approx(150.0)
    assert _first_image(pdf, 6)[:2] == (100, 60)


@pytest.mark.asyncio
async def test_inserted_page_survives_when_the_reference_page_is_deleted(image_env, loader, fetch):
    bm, _, pdf, _ = image_env

    result, error = await bm.delete_pdf_pages(1, [2], None, None, [_insert(2)])

    assert error is None and result["remaining_pages"] == 4
    assert _mediabox_size(pdf, 2)[1] == pytest.approx(150.0)  # 삽입한 쪽이 원본 2쪽 자리에 남는다
    assert _page_count(pdf) == 4
    import pikepdf

    with pikepdf.open(pdf) as doc:
        assert "/XObject" not in doc.pages[2].Resources  # 그 다음은 원본 3쪽(text)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "inserts",
    [
        [_insert(9)],  # 쪽 범위 밖
        [_insert(0)],
        [_insert(1), _insert(1, "https://img.example.com/b.png")],  # 같은 쪽에 둘 이상
        [_insert(1, "   ")],  # 빈 URL
    ],
)
async def test_invalid_insert_changes_nothing(image_env, loader, fetch, inserts):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], None, None, inserts)

    assert result is None and error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    assert not list(pdf.parent.glob(".*.part"))
    es.delete.assert_not_called()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "fetch_effect,expected",
    [
        (ValueError("공개 HTTP/HTTPS 이미지 URL만 사용할 수 있습니다"), "1쪽 앞에 넣을 이미지를 가져올 수 없습니다"),
        (OSError("Name or service not known"), "이미지 서버 주소를 확인하지 못했습니다"),
    ],
)
async def test_fetch_failure_changes_nothing(image_env, loader, fetch_effect, expected):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    with patch("backend.direct_cover_url.fetch_url_image", side_effect=fetch_effect):
        result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(1)])

    assert result is None and expected in error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    es.delete.assert_not_called()


@pytest.mark.asyncio
async def test_non_image_download_is_rejected(image_env, loader):
    bm, _, pdf, root = image_env
    original = pdf.read_bytes()

    with patch("backend.direct_cover_url.fetch_url_image", return_value=b"<html>not an image</html>"):
        result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(1)])

    assert result is None and "이미지" in error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()


@pytest.mark.asyncio
@pytest.mark.parametrize("url", ["ftp://example.com/a.png", "file:///etc/passwd", "http://127.0.0.1/a.png", "http://[::1]/a.png", "http://user:pw@example.com/a.png"])
async def test_unsafe_urls_are_rejected_by_the_real_fetcher(image_env, loader, url):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(1, url)])

    assert result is None and error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    es.delete.assert_not_called()


# ── 저장 전 미리보기 ──


@pytest.mark.asyncio
async def test_preview_returns_the_normalized_jpeg(image_env, fetch):
    bm, *_ = image_env

    jpeg, error = await bm.preview_insert_image("  https://img.example.com/a.png  ")

    assert error is None
    fetch.assert_called_once_with("https://img.example.com/a.png")
    assert jpeg[:3] == b"\xff\xd8\xff"  # PNG 를 내려받아도 저장될 형식인 JPEG 로 돌려준다
    import io

    from PIL import Image

    assert Image.open(io.BytesIO(jpeg)).size == (200, 300)


@pytest.mark.asyncio
async def test_preview_matches_what_the_save_inserts(image_env, loader, fetch):
    bm, _, pdf, _ = image_env

    preview, _ = await bm.preview_insert_image("https://img.example.com/a.png")
    _, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(1)])

    import pikepdf

    assert error is None
    with pikepdf.open(pdf) as doc:
        saved = next(iter(doc.pages[0].Resources.XObject.values())).read_raw_bytes()
    assert saved == preview  # 화면에서 본 이미지가 그대로 삽입된다


@pytest.mark.asyncio
@pytest.mark.parametrize("url", ["", "   ", "ftp://example.com/a.png", "http://127.0.0.1/a.png", "http://[::1]/a.png"])
async def test_preview_rejects_empty_and_unsafe_urls(image_env, url):
    bm, *_ = image_env

    jpeg, error = await bm.preview_insert_image(url)

    assert jpeg is None and error


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "effect,expected",
    [
        (ValueError("내부 주소의 이미지는 사용할 수 없습니다"), "내부 주소의 이미지는 사용할 수 없습니다"),
        (OSError("Name or service not known"), "이미지 서버 주소를 확인하지 못했습니다"),
        (RuntimeError("boom"), "이미지를 가져오지 못했습니다"),
    ],
)
async def test_preview_reports_fetch_failures(image_env, effect, expected):
    bm, *_ = image_env

    with patch("backend.direct_cover_url.fetch_url_image", side_effect=effect):
        jpeg, error = await bm.preview_insert_image("https://img.example.com/a.png")

    assert jpeg is None and error == expected


@pytest.mark.asyncio
async def test_preview_rejects_non_image_download(image_env):
    bm, *_ = image_env

    with patch("backend.direct_cover_url.fetch_url_image", return_value=b"<html>nope</html>"):
        jpeg, error = await bm.preview_insert_image("https://img.example.com/a.png")

    assert jpeg is None and "이미지" in error


class TestInsertPreviewEndpoint:
    """POST /pdf-pages/{id}/insert-preview: 관리자만 호출할 수 있고, 성공하면 JPEG, 실패하면 400 과 사유를 돌려준다."""

    @pytest.fixture
    def client(self):
        from unittest.mock import AsyncMock, MagicMock

        from fastapi.testclient import TestClient

        import backend.main as main
        from backend.auth import require_admin

        fake = MagicMock()
        fake.preview_insert_image = AsyncMock(return_value=(b"\xff\xd8\xffJPEG", None))
        proxy = main.book_manager
        previous = proxy._instance
        proxy._instance = fake
        main.app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
        try:
            yield TestClient(main.app), fake
        finally:
            main.app.dependency_overrides.pop(require_admin, None)
            proxy._instance = previous

    def test_success_returns_jpeg_without_caching(self, client):
        http, fake = client
        response = http.post("/pdf-pages/1/insert-preview", json={"url": "https://img.example.com/a.png"})
        assert response.status_code == 200
        assert response.headers["content-type"] == "image/jpeg"
        assert response.headers["cache-control"] == "private, no-store"
        assert response.content == b"\xff\xd8\xffJPEG"
        fake.preview_insert_image.assert_awaited_once_with("https://img.example.com/a.png")

    def test_failure_returns_400_with_the_reason(self, client):
        http, fake = client
        fake.preview_insert_image.return_value = (None, "내부 주소의 이미지는 사용할 수 없습니다")
        response = http.post("/pdf-pages/1/insert-preview", json={"url": "http://127.0.0.1/a.png"})
        assert response.status_code == 400
        assert "내부 주소" in response.text

    def test_empty_or_oversized_url_is_a_validation_error(self, client):
        http, _ = client
        assert http.post("/pdf-pages/1/insert-preview", json={"url": ""}).status_code == 422
        assert http.post("/pdf-pages/1/insert-preview", json={"url": "x" * 5000}).status_code == 422

    def test_requires_admin(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        response = TestClient(main.app).post("/pdf-pages/1/insert-preview", json={"url": "https://img.example.com/a.png"})
        assert response.status_code in (401, 403)


# ── Flate 등 JPEG 이 아닌 이미지 쪽 (무손실로 다시 저장) ──

FLATE_W, FLATE_H = 100, 60


def _flate_source(kind):
    """kind 별 원본 이미지. 왼쪽 절반과 오른쪽 절반이 구분된다."""
    from PIL import Image

    if kind in ("rgb", "rgb_png"):
        image = Image.new("RGB", (FLATE_W, FLATE_H), (255, 0, 0))
        image.paste((0, 0, 255), (FLATE_W // 2, 0, FLATE_W, FLATE_H))
    elif kind == "gray":
        image = Image.new("L", (FLATE_W, FLATE_H), 60)
        image.paste(200, (FLATE_W // 2, 0, FLATE_W, FLATE_H))
    elif kind == "bw":
        image = Image.new("1", (FLATE_W, FLATE_H), 0)
        image.paste(1, (FLATE_W // 2, 0, FLATE_W, FLATE_H))
    elif kind == "palette":
        image = Image.new("P", (FLATE_W, FLATE_H), 0)
        image.paste(1, (FLATE_W // 2, 0, FLATE_W, FLATE_H))
        image.putpalette([255, 0, 0, 0, 0, 255] + [0] * 756)
    else:
        raise ValueError(kind)
    return image


def _png_up_filter(raw, row_bytes):
    """PDF /Predictor 15(PNG) 용으로 각 행을 Up 필터로 바꾼다."""
    rows = [raw[i : i + row_bytes] for i in range(0, len(raw), row_bytes)]
    out = bytearray()
    previous = bytes(row_bytes)
    for row in rows:
        out.append(2)
        out.extend((value - prior) & 255 for value, prior in zip(row, previous))
        previous = row
    return bytes(out)


def _make_pixel_pdf(path, pages):
    """pages: kind 문자열 목록. 이미지 한 장이 쪽 전체를 덮는 PDF 를 만든다.

    kind: rgb, rgb_png(PNG 예측기), gray, bw(1비트), palette(Indexed), jpeg, cmyk, smask, jbig2, corrupt
    """
    import io
    import zlib

    import pikepdf
    from PIL import Image

    pdf = pikepdf.new()
    for kind in pages:
        page = pdf.add_blank_page(page_size=(FLATE_W, FLATE_H))
        if kind == "jpeg":
            source = _flate_source("rgb")
            jpeg = io.BytesIO()
            source.save(jpeg, "JPEG", quality=95, subsampling=0)
            image = pdf.make_stream(jpeg.getvalue())
            image.Filter = pikepdf.Name.DCTDecode
            image.ColorSpace = pikepdf.Name.DeviceRGB
            image.BitsPerComponent = 8
        elif kind == "cmyk":
            raw = Image.new("CMYK", (FLATE_W, FLATE_H), (0, 0, 0, 0)).tobytes()
            image = pdf.make_stream(zlib.compress(raw))
            image.Filter = pikepdf.Name.FlateDecode
            image.ColorSpace = pikepdf.Name.DeviceCMYK
            image.BitsPerComponent = 8
        elif kind == "jbig2":
            image = pdf.make_stream(b"\x00" * 16)
            image.Filter = pikepdf.Name.JBIG2Decode
            image.ColorSpace = pikepdf.Name.DeviceGray
            image.BitsPerComponent = 1
        elif kind == "corrupt":
            image = pdf.make_stream(b"this is not zlib data at all")
            image.Filter = pikepdf.Name.FlateDecode
            image.ColorSpace = pikepdf.Name.DeviceRGB
            image.BitsPerComponent = 8
        else:
            base = "rgb" if kind == "smask" else kind
            source = _flate_source(base)
            if kind == "rgb_png":
                raw = _png_up_filter(source.tobytes(), FLATE_W * 3)
                image = pdf.make_stream(zlib.compress(raw))
                image.DecodeParms = pikepdf.Dictionary(Predictor=15, Colors=3, BitsPerComponent=8, Columns=FLATE_W)
            else:
                image = pdf.make_stream(zlib.compress(source.tobytes()))
            image.Filter = pikepdf.Name.FlateDecode
            if source.mode == "P":
                palette = bytes([255, 0, 0, 0, 0, 255])
                image.ColorSpace = pikepdf.Array([pikepdf.Name.Indexed, pikepdf.Name.DeviceRGB, 1, pikepdf.String(palette)])
                image.BitsPerComponent = 8
            elif source.mode == "RGB":
                image.ColorSpace = pikepdf.Name.DeviceRGB
                image.BitsPerComponent = 8
            elif source.mode == "L":
                image.ColorSpace = pikepdf.Name.DeviceGray
                image.BitsPerComponent = 8
            else:  # "1"
                image.ColorSpace = pikepdf.Name.DeviceGray
                image.BitsPerComponent = 1
            if kind == "smask":
                mask = pdf.make_stream(zlib.compress(Image.new("L", (FLATE_W, FLATE_H), 255).tobytes()))
                mask.Filter = pikepdf.Name.FlateDecode
                mask.Type = pikepdf.Name.XObject
                mask.Subtype = pikepdf.Name.Image
                mask.Width, mask.Height = FLATE_W, FLATE_H
                mask.ColorSpace = pikepdf.Name.DeviceGray
                mask.BitsPerComponent = 8
                image.SMask = mask
        image.Type = pikepdf.Name.XObject
        image.Subtype = pikepdf.Name.Image
        image.Width, image.Height = FLATE_W, FLATE_H
        page.Resources = pikepdf.Dictionary(XObject=pikepdf.Dictionary(Im0=image))
        page.Contents = pdf.make_stream(f"q {FLATE_W} 0 0 {FLATE_H} 0 0 cm /Im0 Do Q".encode())
    path.parent.mkdir(parents=True, exist_ok=True)
    pdf.save(path)


def _pixel_image(path, page_no):
    """(PIL 이미지, 이미지 XObject)를 돌려준다. 어떤 필터든 디코딩한다."""
    import pikepdf

    with pikepdf.open(path) as pdf:
        xobject = next(iter(pdf.pages[page_no - 1].Resources.XObject.values()))
        info = {"filter": str(xobject.get("/Filter")), "bpc": int(xobject.BitsPerComponent), "cs": str(xobject.ColorSpace)}
        return pikepdf.PdfImage(xobject).as_pil_image().copy(), info


def _same_pixels(a, b):
    from PIL import ImageChops

    return a.mode == b.mode and a.size == b.size and ImageChops.difference(a.convert("RGB"), b.convert("RGB")).getbbox() is None


@pytest.fixture
def pixel_env(env):
    bm, es, pdf, root = env
    _make_pixel_pdf(pdf, ["rgb", "rgb_png", "gray", "bw", "palette", "jpeg"])
    return bm, es, pdf, root


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "page,kind,mode,cs",
    [(1, "rgb", "RGB", "/DeviceRGB"), (2, "rgb_png", "RGB", "/DeviceRGB"), (3, "gray", "L", "/DeviceGray")],
)
async def test_flate_crop_is_lossless(pixel_env, loader, page, kind, mode, cs):
    bm, _, pdf, root = pixel_env
    rect = (0.3, 0.2, 0.9, 0.8)
    expected = _flate_source(kind).crop((30, 12, 90, 48))

    result, error = await bm.delete_pdf_pages(1, [], [_crop(page, *rect)])

    assert error is None and result["cropped_pages"] == 1
    image, info = _pixel_image(pdf, page)
    assert info == {"filter": "/FlateDecode", "bpc": 8, "cs": cs}  # JPEG 로 바꾸지 않는다
    assert image.mode == mode and _same_pixels(image, expected)  # 픽셀이 완전히 같다
    assert _mediabox_size(pdf, page) == pytest.approx((60.0, 36.0))
    assert (root / "0_trash" / "book.pdf").is_file()
    assert _pixel_image(pdf, 6)[1]["filter"] == "/DCTDecode"  # JPEG 쪽은 그대로


@pytest.mark.asyncio
async def test_one_bit_crop_stays_one_bit_with_row_padding(pixel_env, loader):
    bm, _, pdf, _ = pixel_env
    # 너비 50(8의 배수가 아님)이라 행 끝에 패딩 비트가 생긴다
    _, error = await bm.delete_pdf_pages(1, [], [_crop(4, 0.25, 0, 0.75, 1)])

    assert error is None
    image, info = _pixel_image(pdf, 4)
    assert info["bpc"] == 1 and info["cs"] == "/DeviceGray" and image.mode == "1"
    assert image.size == (50, 60)
    assert _same_pixels(image, _flate_source("bw").crop((25, 0, 75, 60)))


@pytest.mark.asyncio
async def test_palette_image_becomes_rgb(pixel_env, loader):
    bm, _, pdf, _ = pixel_env

    _, error = await bm.delete_pdf_pages(1, [], [_crop(5, 0.5, 0, 1, 1)])

    assert error is None
    image, info = _pixel_image(pdf, 5)
    assert info["cs"] == "/DeviceRGB" and image.mode == "RGB"
    assert image.getpixel((10, 10)) == (0, 0, 255)  # 오른쪽 절반(팔레트 1번 = 파랑)


@pytest.mark.asyncio
@pytest.mark.parametrize("page", [1, 2, 3])
async def test_rotated_flate_crop_straightens_the_region(pixel_env, loader, page):
    bm, _, pdf, _ = pixel_env

    result, error = await bm.delete_pdf_pages(1, [], [_crop(page, 0.35, 0.1, 0.65, 0.9, 90)])

    assert error is None and result["cropped_pages"] == 1
    image, info = _pixel_image(pdf, page)
    assert info["filter"] == "/FlateDecode" and image.size == (30, 48)
    top, bottom = image.convert("RGB").getpixel((15, 3)), image.convert("RGB").getpixel((15, 44))
    # rgb/rgb_png: 위 파랑, 아래 빨강. gray: 위 200(밝음), 아래 60(어두움)
    assert sum(top) > sum(bottom) if page == 3 else (top[2] > 200 and bottom[0] > 200)


@pytest.mark.asyncio
async def test_rotated_one_bit_crop_becomes_eight_bit_gray(pixel_env, loader):
    bm, _, pdf, _ = pixel_env

    _, error = await bm.delete_pdf_pages(1, [], [_crop(4, 0.35, 0.1, 0.65, 0.9, 90)])

    assert error is None
    image, info = _pixel_image(pdf, 4)
    assert image.mode == "L" and info["bpc"] == 8 and info["cs"] == "/DeviceGray"
    assert image.getpixel((15, 3)) > 200 and image.getpixel((15, 44)) < 60  # 위 흰색, 아래 검정


@pytest.mark.asyncio
async def test_split_works_on_flate_pages(pixel_env, loader):
    bm, _, pdf, _ = pixel_env

    result, error = await bm.delete_pdf_pages(1, [], None, [_split(2, RIGHT, LEFT)])

    assert error is None and result["remaining_pages"] == 7
    first, _ = _pixel_image(pdf, 2)
    second, _ = _pixel_image(pdf, 3)
    assert first.getpixel((10, 10)) == (0, 0, 255) and second.getpixel((10, 10)) == (255, 0, 0)


@pytest.mark.asyncio
async def test_mixed_jpeg_and_flate_pages_in_one_save(pixel_env, loader):
    bm, _, pdf, _ = pixel_env

    result, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0.5, 0, 1, 1), _crop(6, 0.5, 0, 1, 1)])

    assert error is None and result["cropped_pages"] == 2
    assert _pixel_image(pdf, 1)[1]["filter"] == "/FlateDecode"
    assert _pixel_image(pdf, 6)[1]["filter"] == "/DCTDecode"


@pytest.mark.asyncio
@pytest.mark.parametrize("kind,expected", [("cmyk", "색상 모드"), ("smask", "색상 구성"), ("jbig2", "지원하지 않는 이미지 형식"), ("corrupt", "해독")])
async def test_unsupported_pixel_images_change_nothing(env, loader, kind, expected):
    bm, es, pdf, root = env
    _make_pixel_pdf(pdf, [kind])
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], [_crop(1, 0.5, 0, 1, 1)])

    assert result is None and expected in error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    assert not list(pdf.parent.glob(".*.part"))
    es.delete.assert_not_called()


@pytest.mark.asyncio
async def test_rotated_page_is_rejected(env, loader):
    bm, _, pdf, _ = env
    _make_image_pdf(pdf, ["image", "rotated"])
    result, error = await bm.delete_pdf_pages(1, [], [_crop(2, 0.5, 0, 1, 1)])
    assert result is None and "회전" in error


@pytest.mark.asyncio
async def test_save_clears_cover_cache(image_env, loader):
    bm, _, _, root = image_env
    cache = root / ".cover_cache"
    cache.mkdir()
    (cache / "1.v4.jpg").write_bytes(b"old")
    (cache / "2.v4.jpg").write_bytes(b"other book")

    _, error = await bm.delete_pdf_pages(1, [1])

    assert error is None
    assert not (cache / "1.v4.jpg").exists()
    assert (cache / "2.v4.jpg").exists()


# ── 올린 이미지(image_data)로 쪽 끼워넣기 ──


def _b64(data):
    import base64

    return base64.b64encode(data).decode()


def _upload(page, data=None):
    return {"page": page, "image_data": _b64(data if data is not None else _png())}


def _jpeg_with_orientation(orientation, width=200, height=100):
    import io

    from PIL import Image

    exif = Image.Exif()
    exif[0x0112] = orientation
    out = io.BytesIO()
    Image.new("RGB", (width, height), (10, 120, 200)).save(out, "JPEG", exif=exif)
    return out.getvalue()


@pytest.mark.asyncio
async def test_insert_uploaded_image_adds_a_page_without_fetching(image_env, loader, fetch):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_upload(2)])

    assert error is None
    assert result["inserted_pages"] == 1
    fetch.assert_not_called()
    assert (root / "0_trash" / "book.pdf").read_bytes() == original
    assert _page_count(pdf) == 5
    width, height = _mediabox_size(pdf, 2)
    assert width == pytest.approx(100.0) and height == pytest.approx(150.0)


@pytest.mark.asyncio
async def test_uploaded_and_url_inserts_can_be_mixed(image_env, loader, fetch):
    bm, _, pdf, _ = image_env

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(1), _upload(3)])

    assert error is None and result["inserted_pages"] == 2
    assert _page_count(pdf) == 6


@pytest.mark.asyncio
async def test_saved_upload_is_the_exact_preview_image(image_env, loader):
    """미리보기로 받은 JPEG 를 그대로 보내 저장하면 다시 인코딩하지 않아 화면에서 본 이미지가 삽입된다."""
    bm, _, pdf, _ = image_env

    preview, error = await bm.preview_insert_upload(_png())
    assert error is None
    _, error = await bm.delete_pdf_pages(1, [], None, None, [_upload(1, preview)])

    import pikepdf

    assert error is None
    with pikepdf.open(pdf) as doc:
        saved = next(iter(doc.pages[0].Resources.XObject.values())).read_raw_bytes()
    assert saved == preview


@pytest.mark.asyncio
@pytest.mark.parametrize("orientation, expected", [(1, (200, 100)), (6, (100, 200)), (8, (100, 200)), (3, (200, 100))])
async def test_upload_preview_applies_exif_orientation(image_env, orientation, expected):
    """폰 카메라 사진은 회전을 EXIF 로 기록한다. 반영하지 않으면 세로 사진이 옆으로 눕는다."""
    bm, *_ = image_env

    jpeg, error = await bm.preview_insert_upload(_jpeg_with_orientation(orientation))

    import io

    from PIL import Image

    assert error is None
    assert Image.open(io.BytesIO(jpeg)).size == expected


@pytest.mark.asyncio
async def test_upload_preview_shrinks_a_large_image(image_env):
    bm, *_ = image_env

    jpeg, error = await bm.preview_insert_upload(_png(3000, 1500))

    import io

    from PIL import Image

    assert error is None
    assert Image.open(io.BytesIO(jpeg)).size == (2000, 1000)


@pytest.mark.asyncio
@pytest.mark.parametrize("data", [b"", b"not an image", b"\xff\xd8\xff garbage"])
async def test_upload_preview_rejects_empty_and_unreadable_data(image_env, data):
    bm, *_ = image_env

    jpeg, error = await bm.preview_insert_upload(data)

    assert jpeg is None and error


@pytest.mark.asyncio
async def test_upload_preview_rejects_oversized_data(image_env):
    from backend.epub_cover import MAX_IMAGE_BYTES

    bm, *_ = image_env

    jpeg, error = await bm.preview_insert_upload(b"\x00" * (MAX_IMAGE_BYTES + 1))

    assert jpeg is None and "큽니다" in error


@pytest.mark.asyncio
async def test_unreadable_upload_fails_without_touching_the_files(image_env, loader):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_upload(2, b"not an image")])

    assert result is None and "2쪽 앞에 넣을 이미지를 가져올 수 없습니다" in error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()
    es.delete.assert_not_called()


@pytest.mark.asyncio
async def test_invalid_base64_is_rejected(image_env, loader):
    bm, es, pdf, root = image_env
    original = pdf.read_bytes()

    result, error = await bm.delete_pdf_pages(1, [], None, None, [{"page": 2, "image_data": "***not-base64***"}])

    assert result is None and "2쪽 앞에 넣을 이미지 데이터를 읽을 수 없습니다" in error
    assert pdf.read_bytes() == original
    assert not (root / "0_trash").exists()


@pytest.mark.asyncio
async def test_two_sources_for_the_same_page_are_rejected(image_env, loader, fetch):
    bm, *_ = image_env

    result, error = await bm.delete_pdf_pages(1, [], None, None, [_insert(2), _upload(2)])

    assert result is None and "둘 이상" in error


class TestInsertUploadEndpoint:
    """POST /pdf-pages/{id}/insert-preview-upload 와 image_data 를 받는 저장 엔드포인트."""

    @pytest.fixture
    def client(self):
        from unittest.mock import AsyncMock, MagicMock

        from fastapi.testclient import TestClient

        import backend.main as main

        fake = MagicMock()
        fake.preview_insert_upload = AsyncMock(return_value=(b"\xff\xd8\xffJPEG", None))
        fake.delete_pdf_pages = AsyncMock(return_value=({"remaining_pages": 5}, None))
        proxy = main.book_manager
        previous = proxy._instance
        proxy._instance = fake
        main.app.dependency_overrides[main.require_admin] = lambda: {"role": "admin"}
        try:
            yield TestClient(main.app), fake
        finally:
            main.app.dependency_overrides.pop(main.require_admin, None)
            proxy._instance = previous

    def test_upload_returns_jpeg_without_caching(self, client):
        http, fake = client
        response = http.post("/pdf-pages/1/insert-preview-upload", files={"file": ("a.png", b"PNGDATA", "image/png")})
        assert response.status_code == 200
        assert response.headers["content-type"] == "image/jpeg"
        assert response.headers["cache-control"] == "private, no-store"
        assert response.content == b"\xff\xd8\xffJPEG"
        fake.preview_insert_upload.assert_awaited_once_with(b"PNGDATA")

    def test_upload_failure_returns_400_with_the_reason(self, client):
        http, fake = client
        fake.preview_insert_upload.return_value = (None, "이미지 형식을 읽을 수 없습니다")
        response = http.post("/pdf-pages/1/insert-preview-upload", files={"file": ("a.txt", b"x", "text/plain")})
        assert response.status_code == 400
        assert "형식" in response.text

    def test_delete_passes_the_new_page_order(self, client):
        http, fake = client
        assert http.post("/pdf-pages/1/delete", json={"order": [2, 1]}).json()["status"] == "success"
        assert fake.delete_pdf_pages.await_args.args[5] == [2, 1]

    def test_upload_without_a_file_is_a_validation_error(self, client):
        http, _ = client
        assert http.post("/pdf-pages/1/insert-preview-upload").status_code == 422

    def test_upload_requires_admin(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        response = TestClient(main.app).post("/pdf-pages/1/insert-preview-upload", files={"file": ("a.png", b"x", "image/png")})
        assert response.status_code in (401, 403)

    def test_delete_accepts_image_data(self, client):
        http, fake = client
        response = http.post("/pdf-pages/1/delete", json={"inserts": [{"page": 2, "image_data": "QUJD"}]})
        assert response.status_code == 200 and response.json()["status"] == "success"
        inserts = fake.delete_pdf_pages.await_args.args[4]
        assert inserts == [{"page": 2, "url": None, "image_data": "QUJD"}]

    @pytest.mark.parametrize("insert", [{"page": 2}, {"page": 2, "url": "https://a.example/a.png", "image_data": "QUJD"}, {"page": 2, "image_data": ""}])
    def test_delete_requires_exactly_one_source(self, client, insert):
        http, fake = client
        assert http.post("/pdf-pages/1/delete", json={"inserts": [insert]}).status_code == 422
        fake.delete_pdf_pages.assert_not_awaited()
