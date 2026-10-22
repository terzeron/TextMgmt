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


@pytest.mark.parametrize("cover_decl", [pytest.param('<meta name="cover" content="cover-img"/>', id="meta-cover"), pytest.param("", id="cover-image-property")])
def test_preview_keeps_cover_image_not_referenced_by_chapters(tmp_path: Path, cover_decl: str):
    properties = ' properties="cover-image"' if not cover_decl else ""
    epub = tmp_path / "cov.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip")
        zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        zf.writestr("OEBPS/ch1.xhtml", "<html><body><p>본문</p></body></html>")
        zf.writestr("OEBPS/images/front.jpg", _image_bytes("JPEG"))
        zf.writestr(
            "OEBPS/content.opf",
            f"""<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <metadata>{cover_decl}</metadata>
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="cover-img" href="images/front.jpg" media-type="image/jpeg"{properties}/>
              </manifest>
              <spine><itemref idref="c1"/></spine>
            </package>""",
        )
    manager = make_manager(tmp_path, make_doc("cov.epub", "epub"))

    resp = asyncio_runner(manager.get_book_preview(1, chapters=1))

    assert resp.status_code == 200
    with zipfile.ZipFile(resp.path) as zf:
        assert "OEBPS/images/front.jpg" in zf.namelist()
        assert 'id="cover-img"' in zf.read("OEBPS/content.opf").decode()


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


def test_find_epub_cover_member_falls_back_to_first_spine_page_image(tmp_path: Path):
    """표지 선언이 없고 meta cover 가 존재하지 않는 id 를 가리켜도 첫 spine 페이지의 이미지를 쓴다."""
    opf = """<?xml version="1.0"?>
    <package xmlns="http://www.idpf.org/2007/opf">
      <metadata><meta name="cover" content="cover"/></metadata>
      <manifest>
        <item id="id0" href="0.html" media-type="application/xhtml+xml"/>
        <item id="id1" href="1.html" media-type="application/xhtml+xml"/>
        <item id="img1" href="images/img1.webp" media-type="image/webp"/>
        <item id="img2" href="images/img2.webp" media-type="image/webp"/>
      </manifest>
      <spine><itemref idref="id0"/><itemref idref="id1"/></spine>
    </package>"""
    members = {
        "META-INF/container.xml": _CONTAINER,
        "OEBPS/content.opf": opf,
        "OEBPS/0.html": '<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/img1.webp"/></body></html>',
        "OEBPS/1.html": '<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="images/img2.webp"/></body></html>',
        "OEBPS/images/img1.webp": "x",
        "OEBPS/images/img2.webp": "x",
    }
    with _epub_with(tmp_path, members) as zin:
        assert BookManager._find_epub_cover_member(zin) == "OEBPS/images/img1.webp"


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


# ---- 커버 목록(작품 묶음 + 커서) ----


def _cover_doc(path: str, title: str, category: str, file_type: str = "epub") -> dict:
    doc = make_doc(path, file_type)
    doc.update({"title": title, "category": category, "file_path": path})
    return doc


def _fake_cover_es(manager: BookManager, stats: dict, files: list[tuple[dict, int]], reps: dict[str, list[dict]]) -> dict:
    """ES 없이 커서 조회가 하는 일(최신순 정렬, search_after, 전체 건수)을 흉내 낸다. 받은 인자는 calls에 모은다.

    files는 (문서, 수정 시각) 쌍이다. 정렬값은 ES와 같은 [수정 시각, 생성 시각, 파일 경로]다.
    """
    calls: dict = {"files": [], "reps": []}
    entries = [(book_id, doc, [updated, 0, doc["file_path"]]) for book_id, (doc, updated) in enumerate(files, start=1)]
    ordered = sorted(entries, key=lambda item: (-item[2][0], -item[2][1], item[2][2]))

    def search_cover_files_paged(categories, size, search_after, exclude):
        calls["files"].append({"categories": categories, "size": size, "search_after": search_after, "exclude": exclude})
        eligible = [item for item in ordered if item[1]["category"] in categories]
        key = lambda item: (-item[2][0], -item[2][1], item[2][2])  # noqa: E731
        window = [item for item in eligible if not search_after or key(item) > (-search_after[0], -search_after[1], search_after[2])]
        return window[:size], len(eligible)

    def search_group_representatives(keys, exclude):
        calls["reps"].append({"keys": keys, "exclude": exclude})
        return {key: [(900 + index, doc) for index, doc in enumerate(reps[key])] for key in keys if key in reps}

    manager.es_manager.aggregate_cover_categories = lambda: stats
    manager.es_manager.search_cover_files_paged = search_cover_files_paged
    manager.es_manager.search_group_representatives = search_group_representatives
    return calls


def _titles(items) -> list[tuple[str, str]]:
    return [(book.title, display) for book, display in items]


def _cover_fixture(tmp_path: Path):
    """디렉토리 A: 직속 파일 가(수정 400), 다(수정 200)와 작품 디렉토리 나작품(최근 300), 라작품(최근 100)."""
    manager = make_manager(tmp_path, DummyES())
    files = [(_cover_doc("A/가.epub", "가", "A"), 400), (_cover_doc("A/다.pdf", "다", "A", "pdf"), 200)]
    reps = {
        "A/나작품": [_cover_doc("A/나작품/나작품 02권.pdf", "나작품 02권", "A/나작품", "pdf"), _cover_doc("A/나작품/나작품 01권.pdf", "나작품 01권", "A/나작품", "pdf")],
        "A/라작품": [_cover_doc("A/라작품/라작품 1권.pdf", "라작품 1권", "A/라작품", "pdf")],
    }
    stats = {"A": (2, 400.0, 0.0), "A/나작품": (12, 300.0, 0.0), "A/라작품": (1, 100.0, 0.0)}
    return manager, files, reps, stats


def test_get_cover_books_paged_merges_files_and_work_groups_newest_first(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    calls = _fake_cover_es(manager, stats, files, reps)

    items, total, next_cursor, error = asyncio_runner(manager.get_cover_books_paged(category="A", size=10))

    assert error is None
    assert next_cursor is None
    assert total == 4
    # 묶음은 1권을 대표로 하고 작품 디렉토리 이름을 display_title로 갖는다. 순서는 최신순이다.
    assert _titles(items) == [("가", ""), ("나작품 01권", "나작품"), ("다", ""), ("라작품 1권", "라작품")]
    assert calls["files"][0]["categories"] == ["A"]
    assert calls["reps"][0]["keys"] == ["A/나작품", "A/라작품"]


def test_get_cover_books_paged_pages_continue_without_gaps_or_repeats(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    _fake_cover_es(manager, stats, files, reps)

    seen: list[tuple[str, str]] = []
    cursor = ""
    for _ in range(10):
        items, total, cursor_next, error = asyncio_runner(manager.get_cover_books_paged(category="A", size=1, cursor=cursor))
        assert error is None
        assert total == 4
        seen += _titles(items)
        if cursor_next is None:
            break
        cursor = cursor_next
    else:
        pytest.fail("커서가 끝나지 않는다")

    assert seen == [("가", ""), ("나작품 01권", "나작품"), ("다", ""), ("라작품 1권", "라작품")]


def test_get_cover_books_paged_root_shows_root_files_and_one_card_per_top_level_directory(tmp_path: Path):
    manager = make_manager(tmp_path, DummyES())
    files = [(_cover_doc("루트책.epub", "루트책", "_root"), 50), (_cover_doc("3_판타지/가.epub", "가", "3_판타지"), 10)]
    reps = {"3_판타지": [files[1][0]], "0_웹툰": [_cover_doc("0_웹툰/작품 01권.pdf", "작품 01권", "0_웹툰/작품", "pdf")]}
    stats = {"_root": (1, 50.0, 0.0), "3_판타지": (12, 70.0, 0.0), "0_웹툰/작품": (3, 90.0, 0.0)}
    calls = _fake_cover_es(manager, stats, files, reps)

    items, total, next_cursor, error = asyncio_runner(manager.get_cover_books_paged(size=10))

    assert error is None
    assert next_cursor is None
    # 3_판타지 직속 책은 펼치지 않고 디렉토리마다 대표 한 권만 나온다. 최신 시각이 큰 묶음부터다.
    assert calls["files"][0]["categories"] == ["_root"]
    assert calls["reps"][0]["keys"] == ["0_웹툰", "3_판타지"]
    assert total == 3
    assert _titles(items) == [("작품 01권", "0_웹툰"), ("가", "3_판타지"), ("루트책", "")]


def test_get_cover_books_paged_scopes_to_selected_directory(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    calls = _fake_cover_es(manager, stats, files, reps)

    items, total, _cursor, error = asyncio_runner(manager.get_cover_books_paged(category="A", size=10))

    assert error is None
    assert total == 4
    assert calls["files"][0]["categories"] == ["A"]
    assert [display for _book, display in items] == ["", "나작품", "", "라작품"]


def test_get_cover_books_paged_excludes_hidden_categories_everywhere(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    calls = _fake_cover_es(manager, stats, files, reps)

    items, total, _cursor, _error = asyncio_runner(manager.get_cover_books_paged(category="A", size=10, exclude_categories=["A/나작품"]))

    assert total == 3
    assert calls["reps"][0]["keys"] == ["A/라작품"]
    assert calls["files"][0]["exclude"] == ["A/나작품"]
    assert calls["reps"][0]["exclude"] == ["A/나작품"]
    assert [display for _book, display in items] == ["", "", "라작품"]


def test_get_cover_books_paged_skips_group_without_representative(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    reps.pop("A/나작품")
    _fake_cover_es(manager, stats, files, reps)

    items, _total, _cursor, _error = asyncio_runner(manager.get_cover_books_paged(category="A", size=10))

    assert [display for _book, display in items] == ["", "", "라작품"]


def test_get_cover_books_paged_keeps_paging_when_only_files_remain(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    _fake_cover_es(manager, stats, files, reps)

    items, _total, next_cursor, _error = asyncio_runner(manager.get_cover_books_paged(category="A", size=2))

    assert [display for _book, display in items] == ["", "나작품"]
    assert next_cursor is not None


@pytest.mark.parametrize(
    "bad_cursor",
    [
        "bad_cursor!!!",
        # 디코딩은 되지만 [그룹, 파일] 두 칸 형식이 아니다.
        "WzFd",
        "WyJ4IiwgbnVsbF0=",
    ],
)
def test_get_cover_books_paged_rejects_malformed_cursor(tmp_path: Path, bad_cursor: str):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    _fake_cover_es(manager, stats, files, reps)

    assert asyncio_runner(manager.get_cover_books_paged(cursor=bad_cursor)) == ([], 0, None, "invalid cursor")


def test_get_cover_books_paged_rejects_wrong_cursor_member_shapes(tmp_path: Path):
    from backend.book_manager import encode_category_cursor

    manager, files, reps, stats = _cover_fixture(tmp_path)
    _fake_cover_es(manager, stats, files, reps)

    # 묶음·파일 커서는 [수정 시각, 생성 시각, 경로] 세 칸이어야 한다.
    for member in ([["a"], None], [None, "x"], [None, [1, 2]], [[1, 2, "k", "extra"], None]):
        assert asyncio_runner(manager.get_cover_books_paged(cursor=encode_category_cursor(member)))[3] == "invalid cursor"


def test_get_cover_books_paged_clamps_page_size(tmp_path: Path):
    manager, files, reps, stats = _cover_fixture(tmp_path)
    calls = _fake_cover_es(manager, stats, files, reps)

    asyncio_runner(manager.get_cover_books_paged(size=0))
    asyncio_runner(manager.get_cover_books_paged(size=10**9))

    assert calls["files"][0]["size"] == 1
    assert calls["files"][1]["size"] == bm_module.MAX_CATEGORY_PAGE_SIZE


def test_extract_cover_thumbnail_scales_tall_first_page_to_exact_cover_size(tmp_path: Path):
    """쪽 폭 66pt 의 세로로 긴 쪽은 pdfium 렌더 결과가 한 픽셀 어긋나(300x451) 도 썸네일 크기가 고정된다."""
    import pikepdf

    pdf_path = tmp_path / "tall.pdf"
    pdf = pikepdf.new()
    pdf.add_blank_page(page_size=(66, 165))
    pdf.save(pdf_path)

    data = BookManager._extract_cover_thumbnail(pdf_path, "pdf", 300)

    assert data is not None
    with Image.open(io.BytesIO(data)) as thumb:
        assert thumb.format == "JPEG"
        assert thumb.size == (300, round(300 / BookManager.COVER_ASPECT_RATIO))
