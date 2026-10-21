"""손상된 기존 표지를 교체하되 새 이미지와 EPUB 구조는 검증한다."""

import base64
import zipfile
from pathlib import Path

import pytest
from PIL import Image

from backend import cover_management, epub_cover
from tests.test_cover_management import image_bytes, make_epub, rewrite_epub_members


def broken_cover(tmp_path):
    path = make_epub(tmp_path / "book.epub", cover=True)
    rewrite_epub_members(path, lambda members: members.update({"OPS/cover.jpg": b"not a JPEG"}))
    return path


def test_inspection_reads_headers_without_decoding_pixels(tmp_path, monkeypatch):
    path = make_epub(tmp_path / "book.epub", cover=True)
    original = path.read_bytes()
    byte_size = len(image_bytes())

    def no_decode(*args, **kwargs):
        pytest.fail("목록 검사에서 이미지 전체를 디코딩했다")

    monkeypatch.setattr(Image.Image, "load", no_decode)
    result = epub_cover.inspect_epub(path)
    assert result["cover_image"] == {"width": 120, "height": 180, "byte_size": byte_size}
    assert path.read_bytes() == original


@pytest.mark.parametrize("covered", [False, True])
def test_broken_cover_is_repairable_in_both_listing_modes(tmp_path, covered):
    path = broken_cover(tmp_path)
    original = path.read_bytes()
    item = cover_management.CoverManagement(tmp_path).missing("", "", 60, covered=covered)["items"][0]
    assert item["status"] == "covered"
    assert item["has_cover"] is True
    assert item["cover_error"]
    assert item["cover_image"] is None
    assert path.read_bytes() == original


@pytest.mark.parametrize("method", ["image", "url"])
def test_broken_cover_search_and_replacement_preserve_body_and_backup(tmp_path, monkeypatch, method):
    path = broken_cover(tmp_path)
    original = path.read_bytes()
    stat = path.stat()
    service = cover_management.CoverManagement(tmp_path)
    item = service.missing("", "", 60)["items"][0]
    monkeypatch.setattr(cover_management, "search_candidates", lambda store, metadata: {"status": "not_found", "candidates": []})
    assert service.candidates("yes24", "book.epub", include_covered=True)["status"] == "not_found"
    # 기존 표지 존재 여부와 덮어쓰기 동의는 손상 여부와 별개다.
    with pytest.raises(epub_cover.CoverError, match="이미 표지"):
        epub_cover.add_cover(path, image_bytes(), item["revision"], tmp_path / ".cover_backups")
    with pytest.raises(epub_cover.CoverError):
        service.apply_image("book.epub", base64.b64encode(b"bad replacement").decode(), item["revision"])
    assert path.read_bytes() == original
    assert not (tmp_path / ".cover_backups").exists()
    if method == "image":
        result = service.apply_image("book.epub", base64.b64encode(image_bytes()).decode(), item["revision"])
    else:
        monkeypatch.setattr(cover_management, "fetch_url_image", lambda url: image_bytes())
        result = service.apply_url("book.epub", "https://example.org/cover.jpg", item["revision"], replace_existing=True)
    assert not epub_cover.inspect_epub(path).get("cover_error")
    assert Path(tmp_path / result["backup_path"]).read_bytes() == original
    assert (path.stat().st_ino, path.stat().st_mtime_ns) == (stat.st_ino, stat.st_mtime_ns)
    with zipfile.ZipFile(path) as archive:
        assert archive.read("OPS/chapter.xhtml") == "<html><body>본문을 보존한다.</body></html>".encode()


def test_broken_package_remains_a_fatal_error(tmp_path):
    path = broken_cover(tmp_path)
    rewrite_epub_members(path, lambda members: members.update({"OPS/book.opf": b"not XML"}))
    item = cover_management.CoverManagement(tmp_path).missing("", "", 60)["items"][0]
    assert item["status"] == "error"
    assert "cover_error" not in item


@pytest.mark.parametrize("kind", ["svg", "pixels"])
def test_unreadable_existing_cover_metadata_is_a_warning(tmp_path, monkeypatch, kind):
    path = make_epub(tmp_path / "book.epub", cover=True)
    if kind == "svg":
        def svg(members):
            members["OPS/book.opf"] = members["OPS/book.opf"].replace(b"cover.jpg", b"cover.svg").replace(b"image/jpeg", b"image/svg+xml")
            members["OPS/cover.svg"] = b"not SVG"
        rewrite_epub_members(path, svg)
    else:
        monkeypatch.setattr(epub_cover, "MAX_IMAGE_PIXELS", 1)
    result = epub_cover.inspect_epub(path)
    assert result["has_cover"]
    assert result["cover_error"]
    assert result["cover_image"] is None
