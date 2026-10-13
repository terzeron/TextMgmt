#!/usr/bin/env python
"""BookManager의 드문 경로(오류 분기, 비동기 콜백, 경계 입력)를 mock 기반으로 확인한다.

ES/Docker 없이 실행된다. tests/test_book_manager.py 의 DummyES, make_manager 를 재사용한다.
"""

import errno
import io
import os
import zipfile
from pathlib import Path
from unittest.mock import MagicMock

import pytest
from PIL import Image

import backend.book_manager as bm_module
from backend.book_manager import BookManager
from utils.loader import Loader

from tests.test_book_manager import DummyES, asyncio_runner, make_doc, make_manager


@pytest.fixture(autouse=True)
def restore_path_prefixes():
    from backend.book import Book

    original = Book.path_prefix
    original_loader = Loader.path_prefix
    try:
        yield
    finally:
        Book.path_prefix = original
        Loader.path_prefix = original_loader


def _image_bytes(fmt: str, size: tuple[int, int] = (64, 64)) -> bytes:
    buf = io.BytesIO()
    Image.new("RGB", size, (10, 120, 200)).save(buf, format=fmt)
    return buf.getvalue()


# ---- 이미지/EPUB 미리보기 ----


def test_downscale_epub_image_keeps_non_jpeg_png_format():
    """GIF 같은 형식은 확장자와 실제 형식이 어긋나지 않게 그대로 돌려준다."""
    data = _image_bytes("GIF", (3000, 3000))
    assert BookManager._downscale_epub_image(data) == data


def test_preview_downscales_large_epub_images(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(BookManager, "EPUB_IMAGE_MIN_BYTES", 0)
    calls: list[int] = []
    original = BookManager._downscale_epub_image

    def spy(data: bytes) -> bytes:
        calls.append(len(data))
        return original(data)

    monkeypatch.setattr(BookManager, "_downscale_epub_image", staticmethod(spy))

    epub = tmp_path / "pic.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/ch1.xhtml", '<html><body><img src="images/a.png"/></body></html>')
        zf.writestr("OEBPS/images/a.png", _image_bytes("PNG"))
        zf.writestr(
            "OEBPS/content.opf",
            """<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="img" href="images/a.png" media-type="image/png"/>
              </manifest>
              <spine><itemref idref="c1"/></spine>
            </package>""",
        )
    manager = make_manager(tmp_path, make_doc("pic.epub", "epub"))

    resp = asyncio_runner(manager.get_book_preview(1, chapters=1))

    assert resp.status_code == 200
    assert calls, "이미지 멤버가 축소 경로를 지나야 한다"


# ---- 분류 후보 조립 ----


def test_model_decision_floor_ignores_unparsable_value():
    service = MagicMock()
    service.classifier.min_confidence = "높음"
    assert BookManager._model_decision_floor(service) == 0.0


def test_expected_accuracy_swallows_calibration_errors():
    service = MagicMock()
    service.classifier.expected_accuracy.side_effect = RuntimeError("보정 실패")
    assert BookManager._expected_accuracy(service, 0.9) is None


def test_fill_second_with_model_keeps_two_candidates_untouched(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    candidates = [{"category": "A"}, {"category": "B"}]
    assert manager._fill_second_with_model(candidates, [("C", 0.9)]) == [{"category": "A"}, {"category": "B"}]


def test_build_candidates_without_model_uses_top_two_bookstore_votes(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    result = manager._build_candidates(None, "not_found", None, [("A", 3), ("B", 2), ("C", 1)])
    assert [c["category"] for c in result] == ["A", "B"]


def test_fill_second_with_bookstore_keeps_two_candidates_untouched(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    candidates = [{"category": "A"}, {"category": "B"}]
    assert manager._fill_second_with_bookstore(candidates, [("C", 3)]) == [{"category": "A"}, {"category": "B"}]


def test_vote_count_returns_recorded_votes_for_known_category():
    assert BookManager._vote_count_for("A", [("B", 5), ("A", 3)]) == 3


def test_normalize_stored_file_path_strips_leading_dot_slash(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    assert manager._normalize_stored_file_path("./A/x.txt") == "A/x.txt"


def test_vote_count_defaults_to_one_when_category_has_no_votes():
    assert BookManager._vote_count_for("Z", [("A", 3)]) == 1


# ---- 분류 제안 ----


def _category_with_files(tmp_path: Path, category: str, names: list[str]) -> None:
    (tmp_path / category).mkdir(parents=True, exist_ok=True)
    for name in names:
        (tmp_path / category / name).write_text("본문", encoding="utf-8")


def test_propose_rejects_unsafe_category_name(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    result, err = asyncio_runner(manager.propose_category_changes("../밖"))
    assert result == {}
    assert err == "잘못된 카테고리 경로입니다"


def test_propose_awaits_async_progress_callback_and_records_failures(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    _category_with_files(tmp_path, "A", ["ok.txt", "bad.txt"])
    manager = make_manager(tmp_path, DummyES())

    def fake_propose(file_path, *args, **kwargs):
        if file_path.name == "bad.txt":
            raise RuntimeError("분류기 오류")
        return {"target_category": "B", "grade": "high"}

    monkeypatch.setattr(manager, "_propose_category_for_file", fake_propose)
    reports: list[dict] = []

    async def on_progress(payload: dict) -> None:
        reports.append(payload)

    result, err = asyncio_runner(manager.propose_category_changes("A", use_bookstore=False, use_content_meta=False, on_progress=on_progress))

    assert err is None
    assert [item["title"] for item in result["items"]] == ["ok"]
    assert result["failures"] == [{"file_path": "A/bad.txt", "error": "분류 제안에 실패했습니다"}]
    assert any("updated_items" in r for r in reports)


# ---- 분류 적용 ----


def _apply(manager: BookManager, items: list[dict], allowed: set[str], **kwargs):
    return asyncio_runner(manager.apply_category_changes(items, allowed, **kwargs))


def test_apply_awaits_async_callbacks(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    _category_with_files(tmp_path, "1_A", ["a.txt"])
    (tmp_path / "2_B").mkdir()
    manager = make_manager(tmp_path, DummyES())
    done: list[str] = []
    progress: list[dict] = []

    async def fake_move(*args, **kwargs):
        return {"status": "moved"}, None

    async def keep_going() -> bool:
        return True

    async def on_item_done(payload: dict) -> None:
        done.append(payload["apply_status"])

    async def on_progress(payload: dict) -> None:
        progress.append(payload)

    monkeypatch.setattr(manager, "_move_classified_file", fake_move)

    result, err = _apply(manager, [{"file_path": "1_A/a.txt", "target_category": "2_B"}], {"1_A/a.txt"}, should_continue=keep_going, on_item_done=on_item_done, on_progress=on_progress)

    assert err is None
    assert result["applied_count"] == 1
    assert done == ["moving", "moved"]
    assert progress == [{"apply_total_count": 1, "applied_count": 1, "failed_count": 0}]


def test_apply_records_move_error_as_failed(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    _category_with_files(tmp_path, "1_A", ["a.txt"])
    (tmp_path / "2_B").mkdir()
    manager = make_manager(tmp_path, DummyES())

    async def failing_move(*args, **kwargs):
        return None, "이동 실패"

    monkeypatch.setattr(manager, "_move_classified_file", failing_move)

    result, _ = _apply(manager, [{"file_path": "1_A/a.txt", "target_category": "2_B"}], {"1_A/a.txt"})

    assert result["failed_count"] == 1
    assert result["results"][0]["apply_error"] == "이동 실패"


def test_apply_rejects_file_when_resolve_raises_oserror(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    _category_with_files(tmp_path, "1_A", ["a.txt"])
    (tmp_path / "2_B").mkdir()
    manager = make_manager(tmp_path, DummyES())
    original_resolve = Path.resolve

    def flaky_resolve(self, *args, **kwargs):
        if self.name == "a.txt":
            raise OSError(errno.EIO, "입출력 오류")
        return original_resolve(self, *args, **kwargs)

    monkeypatch.setattr(Path, "resolve", flaky_resolve)

    result, _ = _apply(manager, [{"file_path": "1_A/a.txt", "target_category": "2_B"}], {"1_A/a.txt"})

    assert result["results"][0]["apply_error"] == "파일 경로가 corpus 밖입니다"
    assert (tmp_path / "1_A" / "a.txt").exists()


# ---- 카테고리 파일 열거 ----


def test_iter_category_files_skips_subdirectories_and_outside_symlinked_category(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    _category_with_files(tmp_path, "A", ["keep.txt"])
    (tmp_path / "A" / "sub").mkdir()

    assert [p.name for p in manager._iter_category_indexable_files("A")] == ["keep.txt"]

    outside = tmp_path.parent / f"{tmp_path.name}_outside"
    outside.mkdir()
    (outside / "leak.txt").write_text("밖", encoding="utf-8")
    (tmp_path / "L").symlink_to(outside, target_is_directory=True)
    try:
        assert manager._iter_category_indexable_files("L") == []
    finally:
        (outside / "leak.txt").unlink()
        outside.rmdir()


# ---- 중복 판정/번호 붙이기/이동 ----


def test_is_duplicate_content_treats_unreadable_file_as_not_duplicate(tmp_path: Path):
    source = tmp_path / "a.txt"
    source.write_text("x")
    assert BookManager._is_duplicate_content(source, tmp_path / "없음.txt") is False


def test_next_numbered_path_gives_up_when_all_slots_taken(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(bm_module, "DUPLICATE_NAME_MAX_INDEX", 1)
    target = tmp_path / "book.txt"
    (tmp_path / "book (1).txt").write_text("x")
    assert BookManager._next_numbered_path(target) is None


def test_move_reports_when_numbered_names_are_exhausted(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(bm_module, "DUPLICATE_NAME_MAX_INDEX", 1)
    manager = make_manager(tmp_path, DummyES())
    _category_with_files(tmp_path, "A", ["book.txt"])
    _category_with_files(tmp_path, "B", ["book.txt", "book (1).txt"])
    (tmp_path / "A" / "book.txt").write_text("다른 내용입니다", encoding="utf-8")

    result, err = asyncio_runner(manager._move_classified_file(tmp_path / "A" / "book.txt", "B", "book"))

    assert result is None
    assert "너무 많습니다" in err


def test_move_removes_identical_duplicate_when_clean_existing(tmp_path: Path):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    _category_with_files(tmp_path, "A", ["book.txt"])
    _category_with_files(tmp_path, "B", ["book.txt"])

    result, err = asyncio_runner(manager._move_classified_file(tmp_path / "A" / "book.txt", "B", "book", clean_existing=True, source_category="A"))

    assert err is None
    assert result["status"] == "duplicate_cleaned"
    assert not (tmp_path / "A" / "book.txt").exists()
    assert es.deleted_file_paths == [(["A/book.txt"], [])]


def test_move_reports_duplicate_cleanup_failure(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    manager = make_manager(tmp_path, DummyES())
    _category_with_files(tmp_path, "A", ["book.txt"])
    _category_with_files(tmp_path, "B", ["book.txt"])

    def boom(self):
        raise PermissionError("삭제 거부")

    monkeypatch.setattr(Path, "unlink", boom)

    result, err = asyncio_runner(manager._move_classified_file(tmp_path / "A" / "book.txt", "B", "book", clean_existing=True))

    assert result is None
    assert err.startswith("중복 파일 정리 실패")


def test_move_dry_run_reports_plain_move_without_touching_files(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    _category_with_files(tmp_path, "A", ["book.txt"])
    (tmp_path / "B").mkdir()

    result, err = asyncio_runner(manager._move_classified_file(tmp_path / "A" / "book.txt", "B", "book", dry_run=True))

    assert err is None
    assert result["action"] == "move"
    assert (tmp_path / "A" / "book.txt").exists()


def test_move_rolls_back_when_loader_returns_no_single_document(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    manager = make_manager(tmp_path, DummyES())
    _category_with_files(tmp_path, "A", ["book.txt"])
    (tmp_path / "B").mkdir()
    monkeypatch.setattr(Loader, "read_file", staticmethod(lambda *a, **k: {}))

    result, err = asyncio_runner(manager._move_classified_file(tmp_path / "A" / "book.txt", "B", "book"))

    assert result is None
    assert "지원하지 않는 파일 형식" in err
    assert (tmp_path / "A" / "book.txt").exists()


# ---- 불일치 집계 ----


def test_category_mismatches_omit_categories_without_anomalies(tmp_path: Path):
    es = DummyES()
    es.all_file_paths = [("A", "A/ok.txt")]
    manager = make_manager(tmp_path, es)
    _category_with_files(tmp_path, "A", ["ok.txt"])

    result = manager.get_category_mismatches(force_refresh=True)

    assert all(item["category"] != "A" for item in result["mismatches"])


# ---- _root 재적재 ----


def test_reload_root_files_reports_unreadable_root(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    manager = make_manager(tmp_path, DummyES())

    def deny(path):
        raise PermissionError("읽기 거부")

    monkeypatch.setattr(os, "scandir", deny)

    result, err = asyncio_runner(manager._reload_root_files())

    assert result == {}
    assert "최상위 디렉토리를 읽을 수 없습니다" in err


def test_reload_root_files_reports_refresh_and_batch_failures(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    es = DummyES()
    manager = make_manager(tmp_path, es)
    (tmp_path / "top.txt").write_text("x")

    def broken_refresh():
        raise RuntimeError("refresh 불가")

    es.refresh = broken_refresh

    async def fake_bulk(file_names, clean_existing, **kwargs):
        return {}, [{"error": "색인 실패"}]

    monkeypatch.setattr(manager, "_bulk_index_files", fake_bulk)

    result, err = asyncio_runner(manager._reload_root_files())

    assert result == {}
    assert err == "최상위 파일 재적재 실패 2건: 색인 실패"


# ---- 커버 추출 ----


def _epub_with(tmp_path: Path, members: dict[str, bytes | str]) -> zipfile.ZipFile:
    path = tmp_path / "t.epub"
    with zipfile.ZipFile(path, "w") as zf:
        for name, content in members.items():
            zf.writestr(name, content)
    return zipfile.ZipFile(path)


_CONTAINER = '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>'


def test_find_epub_cover_member_returns_none_without_opf(tmp_path: Path):
    with _epub_with(tmp_path, {"readme.txt": "x"}) as zin:
        assert BookManager._find_epub_cover_member(zin) is None


def test_find_epub_cover_member_returns_none_when_opf_member_is_missing(tmp_path: Path):
    with _epub_with(tmp_path, {"META-INF/container.xml": _CONTAINER}) as zin:
        assert BookManager._find_epub_cover_member(zin) is None


def test_find_epub_cover_member_returns_none_when_opf_is_not_xml(tmp_path: Path):
    with _epub_with(tmp_path, {"META-INF/container.xml": _CONTAINER, "OEBPS/content.opf": "xml 이 아닌 문자열"}) as zin:
        assert BookManager._find_epub_cover_member(zin) is None


def test_find_epub_cover_member_ignores_comment_nodes(tmp_path: Path):
    opf = """<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <!-- 주석 노드는 태그가 문자열이 아니다 -->
      <manifest><item id="cover" href="img/c.png" media-type="image/png"/></manifest>
    </package>"""
    with _epub_with(tmp_path, {"META-INF/container.xml": _CONTAINER, "OEBPS/content.opf": opf}) as zin:
        assert BookManager._find_epub_cover_member(zin) == "OEBPS/img/c.png"


def test_blank_cover_check_returns_false_when_image_has_no_colors():
    sample = MagicMock()
    sample.quantize.return_value.getcolors.return_value = None
    image = MagicMock()
    image.convert.return_value.resize.return_value = sample

    assert BookManager._is_mostly_blank_cover(image) is False


def test_extract_cover_thumbnail_ignores_unsupported_type(tmp_path: Path):
    assert BookManager._extract_cover_thumbnail(tmp_path / "a.txt", "txt") is None


def test_get_cover_returns_404_when_file_is_missing(tmp_path: Path):
    manager = make_manager(tmp_path, make_doc("gone.epub", "epub"))
    resp = asyncio_runner(manager.get_cover(1))
    assert resp.status_code == 404


def test_get_cover_returns_503_when_storage_access_fails(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    (tmp_path / "book.epub").write_bytes(b"PK")
    manager = make_manager(tmp_path, make_doc("book.epub", "epub"))
    original_is_file = Path.is_file

    def broken_is_file(self, *args, **kwargs):
        if self.name == "book.epub":
            raise OSError(errno.EIO, "입출력 오류")
        return original_is_file(self, *args, **kwargs)

    monkeypatch.setattr(Path, "is_file", broken_is_file)

    resp = asyncio_runner(manager.get_cover(1))

    assert resp.status_code == 503
