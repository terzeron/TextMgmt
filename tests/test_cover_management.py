"""표지 관리: 운영 파일 대신 임시 EPUB로 저장과 원본 보호를 검증한다."""

import io
import json
import os
import zipfile
from pathlib import Path

import pytest
from PIL import Image


def make_epub(path, version="3.0", cover=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        archive.writestr("META-INF/container.xml", '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>')
        archive.writestr("OPS/book.opf", f'''<package xmlns="http://www.idpf.org/2007/opf" version="{version}"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>테스트 책</dc:title><dc:creator>저자</dc:creator></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/>{'<item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>' if cover else ''}</manifest><spine><itemref idref="chapter"/></spine></package>''')
        archive.writestr("OPS/chapter.xhtml", "<html><body>본문을 보존한다.</body></html>")
        archive.writestr("OPS/style.css", "body { color: red; }")
        if cover:
            archive.writestr("OPS/cover.jpg", image_bytes())
    return path


def image_bytes():
    output = io.BytesIO()
    Image.new("RGB", (120, 180), "red").save(output, "JPEG")
    return output.getvalue()


def test_original_cover_preview_uses_saved_600px_image(cover_api, tmp_path):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import add_cover, inspect_epub
    client, _, _ = cover_api
    path = make_epub(tmp_path / "book.epub")
    output = io.BytesIO()
    Image.new("RGB", (1200, 1800), "blue").save(output, "JPEG")
    add_cover(path, output.getvalue(), inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    original = path.read_bytes()
    service = CoverManagement(tmp_path)
    with Image.open(io.BytesIO(service.preview("book.epub"))) as image:
        assert image.width == 300
    with Image.open(io.BytesIO(service.preview("book.epub", original=True))) as image:
        assert image.size == (600, 900)
    response = client.get("/admin/book-covers/preview?file_path=book.epub&original=true")
    assert response.status_code == 200
    assert response.headers["content-type"] == "image/jpeg"
    with Image.open(io.BytesIO(response.content)) as image:
        assert image.size == (600, 900)
    assert path.read_bytes() == original


@pytest.mark.parametrize("change", ["missing_image", "bad_zip", "writer_lock", "size_limit"])
def test_original_preview_handles_changes_between_inspection_and_read(tmp_path, monkeypatch, change):
    import fcntl
    from backend import cover_management as module
    path = make_epub(tmp_path / "book.epub", cover=True)
    real_inspect = module.inspect_epub
    writers = []
    def inspect_then_change(target):
        result = real_inspect(target)
        if change == "missing_image":
            rewrite_epub_members(target, lambda members: members.pop("OPS/cover.jpg"))
        elif change == "bad_zip":
            target.write_bytes(b"external write")
        elif change == "writer_lock":
            writer = target.open("r+b")
            fcntl.flock(writer, fcntl.LOCK_EX | fcntl.LOCK_NB)
            writers.append(writer)
        return result
    monkeypatch.setattr(module, "inspect_epub", inspect_then_change)
    if change == "size_limit":
        monkeypatch.setattr(module, "MAX_IMAGE_BYTES", 1)
    try:
        with pytest.raises(module.CoverError):
            module.CoverManagement(tmp_path).preview("book.epub", original=True)
    finally:
        for writer in writers:
            writer.close()
    assert not (tmp_path / ".cover_backups").exists()


def test_replacement_preserves_unrelated_assets_with_generated_cover_names(tmp_path):
    from lxml import etree
    from backend.epub_cover import OPF_NS, add_cover, inspect_epub

    path = make_epub(tmp_path / "book.epub")
    add_cover(path, image_bytes(), inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    assets = {"OPS/tm-cover-0000000000000000.jpg": image_bytes(),
              "OPS/tm-cover-page-0000000000000000.xhtml": b'<html xmlns="http://www.w3.org/1999/xhtml"><body>appendix</body></html>',
              "OPS/fonts/font.otf": b"\x00\x01font"}
    def attach(members):
        members.update(assets)
        package = etree.fromstring(members["OPS/book.opf"])
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id="illustration", href="tm-cover-0000000000000000.jpg", **{"media-type": "image/jpeg"})
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id="appendix", href="tm-cover-page-0000000000000000.xhtml", **{"media-type": "application/xhtml+xml"})
        etree.SubElement(package.find(f"{{{OPF_NS}}}spine"), f"{{{OPF_NS}}}itemref", idref="appendix")
        members["OPS/book.opf"] = etree.tostring(package)
    rewrite_epub_members(path, attach)
    result = add_cover(path, image_bytes(), inspect_epub(path)["revision"], tmp_path / ".cover_backups", replace_existing=True)
    with zipfile.ZipFile(path) as archive:
        for name, content in assets.items():
            assert archive.read(name) == content
        package = etree.fromstring(archive.read("OPS/book.opf"))
        assert package.xpath('count(//*[local-name()="itemref" and @idref="appendix"])') == 1
        assert archive.testzip() is None
    assert Path(result["backup_path"]).is_file()


def test_replacement_rejects_external_change_before_creating_another_backup(tmp_path):
    from backend.epub_cover import CoverError, add_cover, inspect_epub
    path = make_epub(tmp_path / "book.epub")
    backup_dir = tmp_path / ".cover_backups"
    saved = add_cover(path, image_bytes(), inspect_epub(path)["revision"], backup_dir)
    backups = list(backup_dir.iterdir())
    path.write_bytes(path.read_bytes() + b"external update")
    changed = path.read_bytes()
    with pytest.raises(CoverError, match="변경"):
        add_cover(path, image_bytes(), saved["revision"], backup_dir, replace_existing=True)
    assert path.read_bytes() == changed
    assert list(backup_dir.iterdir()) == backups
    assert not list(tmp_path.glob(".cover-*.tmp"))


def test_cover_writer_respects_actual_reader_lock(tmp_path):
    import fcntl
    from backend.epub_cover import CoverError, add_cover, inspect_epub
    path = make_epub(tmp_path / "book.epub")
    original = path.read_bytes()
    expected = inspect_epub(path)["revision"]
    with path.open("rb") as reader:
        fcntl.flock(reader, fcntl.LOCK_SH | fcntl.LOCK_NB)
        with pytest.raises(CoverError, match="저장 중"):
            add_cover(path, image_bytes(), expected, tmp_path / ".cover_backups")
    assert path.read_bytes() == original
    assert not (tmp_path / ".cover_backups").exists()


@pytest.mark.parametrize("image_data", ["한글", "YQ=", "YQ==extra", "\nYQ=="])
def test_apply_image_rejects_malformed_base64_without_changing_book(tmp_path, image_data):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError, inspect_epub
    path = make_epub(tmp_path / "book.epub", cover=True)
    original = path.read_bytes()
    with pytest.raises(CoverError):
        CoverManagement(tmp_path).apply_image("book.epub", image_data, inspect_epub(path)["revision"])
    assert path.read_bytes() == original
    assert not (tmp_path / ".cover_backups").exists()


def test_apply_image_payload_limit_is_validated_before_decoding():
    from pydantic import ValidationError
    from backend.cover_management import ApplyImageModel
    from backend.epub_cover import MAX_IMAGE_BYTES
    encoded_limit = ((MAX_IMAGE_BYTES + 2) // 3) * 4
    with pytest.raises(ValidationError, match="string_too_long"):
        ApplyImageModel(file_path="book.epub", revision="r1", image_data="A" * (encoded_limit + 1))


def test_cover_replacement_api_requires_admin_role(cover_api, tmp_path):
    from fastapi import HTTPException
    from backend.cover_management import require_admin
    from backend.epub_cover import inspect_epub
    client, app, _ = cover_api
    path = make_epub(tmp_path / "book.epub", cover=True)
    original = path.read_bytes()
    def deny():
        raise HTTPException(status_code=403, detail="관리자만 사용할 수 있습니다")
    app.dependency_overrides[require_admin] = deny
    assert client.post("/admin/book-covers/apply-image", json={
        "file_path": "book.epub", "revision": inspect_epub(path)["revision"], "image_data": "invalid",
    }).status_code == 403
    assert path.read_bytes() == original


@pytest.mark.parametrize("relative", ["../book.epub", ".cover_backups/book.epub", "/etc/passwd"])
def test_cover_replacement_api_rejects_paths_outside_book_scope(cover_api, tmp_path, relative):
    import base64
    client, _, _ = cover_api
    path = make_epub(tmp_path / "book.epub", cover=True)
    original = path.read_bytes()
    response = client.post("/admin/book-covers/apply-image", json={
        "file_path": relative, "revision": "r1", "image_data": base64.b64encode(image_bytes()).decode(),
    })
    assert response.status_code == 409
    assert path.read_bytes() == original


@pytest.mark.parametrize("version", ["2.0", "3.0"])
def test_cover_replacement_preserves_content_and_has_one_generated_cover(tmp_path, version):
    from backend.book_manager import BookManager
    from backend.epub_cover import add_cover, inspect_epub, normalize_image
    from lxml import etree

    path = make_epub(tmp_path / "book.epub", version)
    stat = path.stat()
    first = add_cover(path, image_bytes(), inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    before = path.read_bytes()
    output = io.BytesIO()
    Image.new("RGB", (1800, 2700), "blue").save(output, "PNG")
    for data in (output.getvalue(), output.getvalue(), image_bytes()):
        result = add_cover(path, data, inspect_epub(path)["revision"], tmp_path / ".cover_backups", replace_existing=True)
        assert Path(result["backup_path"]).read_bytes() == before
        assert path.stat().st_ino == stat.st_ino
        assert path.stat().st_mtime_ns == stat.st_mtime_ns
        with zipfile.ZipFile(path) as archive:
            member = BookManager._find_epub_cover_member(archive)
            assert archive.read(member) == normalize_image(data, max_width=600)
            assert len([name for name in archive.namelist() if "tm-cover-" in name]) == 2
            package = etree.fromstring(archive.read("OPS/book.opf"))
            assert package.xpath('count(//*[local-name()="itemref"])') == 2
            assert archive.read("OPS/chapter.xhtml") == "<html><body>본문을 보존한다.</body></html>".encode()
            assert archive.read("OPS/style.css") == b"body { color: red; }"
            assert len(archive.namelist()) == len(set(archive.namelist()))
        before = path.read_bytes()
    assert first["revision"] != result["revision"]


def test_browser_image_replaces_cover_after_server_candidates_are_deleted(tmp_path, monkeypatch):
    import base64
    from backend.cover_management import CoverManagement
    from backend.epub_cover import inspect_epub, CoverError

    path = make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    saved = service.apply("book.epub", candidates[0]["candidate_id"], inspect_epub(path)["revision"])
    assert not service._book_cache("book.epub").exists()
    before = path.read_bytes()
    with pytest.raises(CoverError):
        service.apply_image("book.epub", "not valid base64!", saved["revision"])
    with pytest.raises(CoverError):
        service.apply_image("book.epub", base64.b64encode(b"invalid image").decode(), saved["revision"])
    with pytest.raises(CoverError, match="변경"):
        service.apply_image("book.epub", base64.b64encode(image_bytes()).decode(), "stale")
    assert path.read_bytes() == before
    replaced = service.apply_image("book.epub", base64.b64encode(image_bytes()).decode(), saved["revision"])
    assert replaced["revision"] != saved["revision"]
    assert (tmp_path / replaced["backup_path"]).read_bytes() == before


def test_research_and_direct_url_replacement_allow_saved_book_explicitly(tmp_path, monkeypatch):
    from backend import cover_management as module
    from backend.epub_cover import inspect_epub, CoverError
    path = make_epub(tmp_path / "book.epub", cover=True)
    service = module.CoverManagement(tmp_path)
    monkeypatch.setattr(module, "search_candidates", lambda *args: {"status": "not_found", "candidates": []})
    monkeypatch.setattr(module, "fetch_url_image", lambda *args: image_bytes())
    with pytest.raises(CoverError, match="이미 표지"):
        service.candidates("yes24", "book.epub")
    original = path.read_bytes()
    assert service.candidates("yes24", "book.epub", include_covered=True)["status"] == "not_found"
    assert path.read_bytes() == original
    saved = service.apply_url("book.epub", "https://example.org/a", inspect_epub(path)["revision"], replace_existing=True)
    assert (tmp_path / saved["backup_path"]).read_bytes() == original


def test_retry_bypasses_error_cache_and_aladin_old_results(tmp_path, monkeypatch):
    from backend import cover_management as module
    from backend.epub_cover import inspect_epub
    import hashlib
    path = make_epub(tmp_path / "book.epub")
    service = module.CoverManagement(tmp_path)
    calls = []
    def search(store, metadata):
        calls.append(store)
        raise ValueError("offline")
    monkeypatch.setattr(module, "search_candidates", search)
    assert service.candidates("yes24", "book.epub")["status"] == "error"
    assert service.candidates("yes24", "book.epub")["status"] == "error"
    assert calls == ["yes24"]
    assert service.candidates("yes24", "book.epub", refresh=True)["status"] == "error"
    assert calls == ["yes24", "yes24"]
    key = hashlib.sha256(f"aladin:book.epub:{inspect_epub(path)['revision']}".encode()).hexdigest()
    service._write(service._book_cache("book.epub") / f"search-{key}.json", b'{"status":"not_found","candidates":[]}')
    assert service.candidates("aladin", "book.epub")["status"] == "error"
    assert calls[-1] == "aladin"


def test_apply_image_api_auth_revision_and_index_update(cover_api, tmp_path):
    import base64
    from backend.cover_management import require_admin
    from backend.epub_cover import inspect_epub
    client, app, manager = cover_api
    path = make_epub(tmp_path / "book.epub", cover=True)
    body = {"file_path": "book.epub", "revision": inspect_epub(path)["revision"], "image_data": base64.b64encode(image_bytes()).decode()}
    app.dependency_overrides.clear()
    assert client.post("/admin/book-covers/apply-image", json=body).status_code == 401
    app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
    original = path.read_bytes()
    assert client.post("/admin/book-covers/apply-image", json={**body, "image_data": ""}).status_code == 422
    assert client.post("/admin/book-covers/apply-image", json={**body, "image_data": "invalid!"}).status_code == 409
    assert client.post("/admin/book-covers/apply-image", json={**body, "revision": "stale"}).status_code == 409
    assert path.read_bytes() == original
    response = client.post("/admin/book-covers/apply-image", json=body)
    assert response.status_code == 200
    assert manager.es_manager.update.called
    assert (tmp_path / response.json()["result"]["backup_path"]).read_bytes() == original


def test_failed_replacement_restores_previous_cover(tmp_path, monkeypatch):
    from backend import epub_cover as module
    path = make_epub(tmp_path / "book.epub", cover=True)
    original = path.read_bytes()
    stat = path.stat()
    real_fsync = os.fsync
    calls = 0
    def fail_once(fd):
        nonlocal calls
        calls += 1
        if calls == 3:
            raise OSError("write failed")
        return real_fsync(fd)
    monkeypatch.setattr(module.os, "fsync", fail_once)
    with pytest.raises(OSError):
        module.add_cover(path, image_bytes(), module.inspect_epub(path)["revision"], tmp_path / ".cover_backups", replace_existing=True)
    assert path.read_bytes() == original
    assert (path.stat().st_ino,path.stat().st_mtime_ns)==(stat.st_ino,stat.st_mtime_ns)


@pytest.mark.parametrize("version", ["2.0", "3.0"])
def test_missing_opf_namespace_recognizes_existing_cover_without_modifying_book(tmp_path, version):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError, add_cover, inspect_epub

    path = make_epub(tmp_path / "book.epub", version, cover=True)
    rewrite_epub_members(path, lambda members: members.__setitem__(
        "OPS/book.opf", members["OPS/book.opf"].replace(b"<dc:creator>", b'<dc:creator opf:role="aut">')))
    original = path.read_bytes()
    stat = path.stat()

    metadata = inspect_epub(path)
    assert metadata["has_cover"]
    assert metadata["author"] == "저자"
    assert CoverManagement(tmp_path).missing("", "", 60)["items"] == []
    with pytest.raises(CoverError, match="이미 표지"):
        add_cover(path, image_bytes(), metadata["revision"], tmp_path / ".cover_backups")
    assert path.read_bytes() == original
    assert (path.stat().st_ino, path.stat().st_mtime_ns) == (stat.st_ino, stat.st_mtime_ns)
    assert not (tmp_path / ".cover_backups").exists()


def test_missing_opf_namespace_is_valid_xml_after_cover_save(tmp_path):
    from lxml import etree
    from backend.epub_cover import OPF_NS, add_cover, inspect_epub

    path = make_epub(tmp_path / "book.epub")
    rewrite_epub_members(path, lambda members: members.__setitem__(
        "OPS/book.opf", members["OPS/book.opf"].replace(b"<dc:creator>", b'<dc:creator opf:role="aut">')))
    original = path.read_bytes()

    result = add_cover(path, image_bytes(), inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    assert inspect_epub(path)["has_cover"]
    assert Path(result["backup_path"]).read_bytes() == original
    with zipfile.ZipFile(path) as archive:
        package = etree.fromstring(archive.read("OPS/book.opf"))
        creator = package.xpath('//*[local-name()="creator"]')[0]
        assert creator.get(f"{{{OPF_NS}}}role") == "aut"
        assert archive.read("OPS/chapter.xhtml") == "<html><body>본문을 보존한다.</body></html>".encode()


@pytest.mark.parametrize("content", [
    b'',
    b'<package xmlns="http://www.idpf.org/2007/opf"><metadata></package>',
    b'<package xmlns="http://www.idpf.org/2007/opf"><metadata><creator unknown:role="aut"/></metadata></package>',
    b'<package xmlns="http://www.idpf.org/2007/opf"><metadata><creator opf:role="aut"></metadata></package>',
])
def test_namespace_compatibility_does_not_accept_other_xml_errors(tmp_path, content):
    from backend.epub_cover import CoverError, inspect_epub

    path = make_epub(tmp_path / "book.epub")
    rewrite_epub_members(path, lambda members: members.__setitem__("OPS/book.opf", content))
    original = path.read_bytes()
    with pytest.raises(CoverError, match="검사할 수 없습니다"):
        inspect_epub(path)
    assert path.read_bytes() == original


def test_namespace_compatibility_does_not_resolve_external_entities(tmp_path):
    from backend.epub_cover import CoverError, inspect_epub

    secret = tmp_path / "private.txt"
    secret.write_text("private-value")
    path = make_epub(tmp_path / "book.epub")
    content = (f'<!DOCTYPE package [<!ENTITY external SYSTEM "{secret.as_uri()}">]>'
               '<package xmlns="http://www.idpf.org/2007/opf"><metadata>'
               '<creator opf:role="aut">&external;</creator></metadata></package>').encode()
    rewrite_epub_members(path, lambda members: members.__setitem__("OPS/book.opf", content))
    with pytest.raises(CoverError):
        inspect_epub(path)


def test_direct_url_saves_cover_and_clear_cache_preserves_epub(tmp_path, monkeypatch):
    from backend import cover_management
    from backend.epub_cover import inspect_epub
    path = make_epub(tmp_path / "book.epub")
    service = cover_management.CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    metadata = inspect_epub(path)
    downloaded = []
    monkeypatch.setattr(cover_management, "fetch_url_image", lambda url: downloaded.append(url) or image_bytes())
    result = service.apply_url("book.epub", "https://example.org/cover.jpg", metadata["revision"])
    assert downloaded == ["https://example.org/cover.jpg"]
    assert inspect_epub(path)["has_cover"]
    assert not service._candidate_path(candidates[0]["candidate_id"]).exists()
    saved = path.read_bytes()
    service.clear_cache("book.epub", result["revision"])
    assert path.read_bytes() == saved
    assert (tmp_path / result["backup_path"]).exists()


def test_clear_cache_is_book_scoped_and_revision_checked(tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import inspect_epub, CoverError
    path = make_epub(tmp_path / "book.epub")
    make_epub(tmp_path / "other.epub")
    service = CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    other, _ = cached_candidates(service, monkeypatch, "other.epub")
    original = path.read_bytes()
    with pytest.raises(CoverError, match="변경"):
        service.clear_cache("book.epub", "stale")
    assert service._candidate_path(candidates[0]["candidate_id"]).exists()
    service.clear_cache("book.epub", inspect_epub(path)["revision"])
    assert not service._candidate_path(candidates[0]["candidate_id"]).exists()
    assert service._candidate_path(other[0]["candidate_id"]).exists()
    assert path.read_bytes() == original


@pytest.mark.parametrize("version", ["2.0", "3.0"])
def test_cover_save_preserves_book_id_and_content(tmp_path, version):
    from backend.epub_cover import inspect_epub, add_cover

    path = make_epub(tmp_path / "한글 책.epub", version)
    original = path.read_bytes()
    inode = path.stat().st_ino
    metadata = inspect_epub(path)
    assert metadata["title"] == "테스트 책"
    assert not metadata["has_cover"]
    result = add_cover(path, image_bytes(), metadata["revision"], tmp_path / ".cover_backups")
    assert path.stat().st_ino == inode
    assert inspect_epub(path)["has_cover"]
    assert result["revision"] != metadata["revision"]
    assert Path(result["backup_path"]).read_bytes() == original
    with zipfile.ZipFile(path) as archive:
        assert archive.infolist()[0].filename == "mimetype"
        assert archive.infolist()[0].compress_type == zipfile.ZIP_STORED
        assert archive.read("OPS/chapter.xhtml") == "<html><body>본문을 보존한다.</body></html>".encode()
        assert archive.read("OPS/style.css") == b"body { color: red; }"


def test_stale_revision_and_existing_cover_do_not_modify_file(tmp_path):
    from backend.epub_cover import inspect_epub, add_cover, CoverError

    path = make_epub(tmp_path / "test.epub")
    original = path.read_bytes()
    with pytest.raises(CoverError, match="변경"):
        add_cover(path, image_bytes(), "old", tmp_path / ".cover_backups")
    assert path.read_bytes() == original
    revision = inspect_epub(path)["revision"]
    add_cover(path, image_bytes(), revision, tmp_path / ".cover_backups")
    saved = path.read_bytes()
    with pytest.raises(CoverError, match="표지"):
        add_cover(path, image_bytes(), inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    assert path.read_bytes() == saved


def test_invalid_image_does_not_touch_original(tmp_path):
    from backend.epub_cover import inspect_epub, add_cover, CoverError

    path = make_epub(tmp_path / "test.epub")
    original = path.read_bytes()
    with pytest.raises(CoverError):
        add_cover(path, b"not an image", inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    assert path.read_bytes() == original


def test_failed_write_restores_original(tmp_path, monkeypatch):
    from backend import epub_cover

    path = make_epub(tmp_path / "test.epub")
    original = path.read_bytes()
    revision = epub_cover.inspect_epub(path)["revision"]
    real_fsync = os.fsync
    calls = 0

    def fail_once(fd):
        nonlocal calls
        calls += 1
        if calls == 3:
            raise OSError("write failed")
        return real_fsync(fd)

    monkeypatch.setattr(epub_cover.os, "fsync", fail_once)
    with pytest.raises(OSError):
        epub_cover.add_cover(path, image_bytes(), revision, tmp_path / ".cover_backups")
    assert path.read_bytes() == original


def test_filesystem_listing_includes_unindexed_epub_and_reports_errors(tmp_path):
    from backend.cover_management import CoverManagement

    make_epub(tmp_path / "目录" / "missing.EPUB")
    make_epub(tmp_path / "目录" / "covered.epub", cover=True)
    (tmp_path / "目录" / "broken.epub").write_bytes(b"bad zip")
    (tmp_path / ".cover_backups").mkdir()
    manager = CoverManagement(tmp_path)
    assert [entry["label"] for entry in manager.directories("")] == ["目录"]
    page = manager.missing("目录", "", 60)
    assert {item["status"] for item in page["items"]} == {"missing", "error"}
    assert len(page["items"]) == 2
    assert page["next_cursor"] is None
    for item in page["items"]:
        assert item["book_id"] == (tmp_path / item["file_path"]).stat().st_ino
        assert item["category"] == "目录"
        assert item["revision"]


@pytest.mark.parametrize("page_href,metadata,guide,image_markup,has_cover", [
    ("Text/cover.xhtml", "", "", '<!-- 표지 설명 --><img src="../Images/120%28550%29.jpg" />', True),
    ("Text/front.xhtml", "", '<guide><reference type="cover" href="Text/front.xhtml#cover" /></guide>', '<img src="../Images/120%28550%29.jpg" />', True),
    ("Text/front.xhtml", '<meta name="cover" content="page" />', "", '<img src="../Images/120%28550%29.jpg" />', True),
    ("Text/cover.xhtml", "", "", '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../Images/120%28550%29.jpg" /></svg>', True),
    ("Text/chapter.xhtml", "", "", '<img src="../Images/120%28550%29.jpg" />', False),
    ("Text/cover.xhtml", "", "", '<img src="missing.jpg" />', False),
    ("Text/cover.xhtml", "", "", '<img src="https://example.com/cover.jpg" />', False),
])
def test_cover_page_image_detection_and_existing_cover_protection(tmp_path, page_href, metadata, guide, image_markup, has_cover):
    from backend.book_manager import BookManager
    from backend.cover_management import CoverManagement
    from backend.epub_cover import inspect_epub, add_cover, CoverError

    path = tmp_path / "html-cover.epub"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        archive.writestr("META-INF/container.xml", '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>')
        archive.writestr("OEBPS/content.opf", f'''<package xmlns="http://www.idpf.org/2007/opf" version="2.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>요재지이 형식</dc:title>{metadata}</metadata><manifest><item id="page" href="{page_href}" media-type="application/xhtml+xml"/><item id="x120_550_.jpg" href="Images/120%28550%29.jpg" media-type="image/jpeg"/></manifest><spine><itemref idref="page"/></spine>{guide}</package>''')
        archive.writestr(f"OEBPS/{page_href}", f'<html xmlns="http://www.w3.org/1999/xhtml"><body>{image_markup}</body></html>')
        archive.writestr("OEBPS/Images/120(550).jpg", image_bytes())
    original = path.read_bytes()
    inspected = inspect_epub(path)
    assert inspected["has_cover"] is has_cover
    with zipfile.ZipFile(path) as archive:
        assert BookManager._find_epub_cover_member(archive) == ("OEBPS/Images/120(550).jpg" if has_cover else None)
    assert bool(CoverManagement(tmp_path).missing("", "", 60)["items"]) is not has_cover
    if has_cover:
        assert BookManager._extract_cover_thumbnail(path, "epub") is not None
        with pytest.raises(CoverError, match="이미 표지"):
            add_cover(path, image_bytes(), inspected["revision"], tmp_path / ".cover_backups")
        assert path.read_bytes() == original


@pytest.mark.parametrize("relative", ["../outside.epub", "/etc/passwd", ".cover_backups/a.epub"])
def test_root_guard_rejects_unsafe_paths(tmp_path, relative):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError

    with pytest.raises(CoverError):
        CoverManagement(tmp_path).resolve(relative)


def test_symlink_is_excluded(tmp_path):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError

    path = make_epub(tmp_path / "real.epub")
    (tmp_path / "link.epub").symlink_to(path)
    manager = CoverManagement(tmp_path)
    with pytest.raises(CoverError):
        manager.resolve("link.epub")
    assert len(manager.missing("", "", 60)["items"]) == 1


def test_admin_api_save_and_preview(tmp_path, monkeypatch):
    from fastapi import FastAPI
    from fastapi.testclient import TestClient
    from unittest.mock import Mock
    from backend.cover_management import CoverManagement, create_cover_router, Book, require_admin

    monkeypatch.setattr(Book, "path_prefix", tmp_path)
    path = make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    metadata = __import__("backend.epub_cover", fromlist=["inspect_epub"]).inspect_epub(path)
    candidate_id = "a" * 64
    record = {"store": "yes24", "file_path": "book.epub", "revision": metadata["revision"], "image_url": "https://image.yes24.com/book.jpg"}
    service._write(service.cache / f"candidate-{candidate_id}.json", __import__("json").dumps(record).encode())
    service._write(service.cache / f"image-{candidate_id}.jpg", image_bytes())
    manager = Mock()
    manager.es_manager.search_by_id.return_value = None
    app = FastAPI()
    app.include_router(create_cover_router(lambda: manager))
    client = TestClient(app)
    for endpoint in ("directories", "missing", f"candidates/yes24?file_path=book.epub", f"candidate-image/{candidate_id}", "preview?file_path=book.epub"):
        assert client.get("/admin/book-covers/" + endpoint).status_code == 401
    assert client.post("/admin/book-covers/apply", json={"file_path": "book.epub", "candidate_id": candidate_id, "revision": metadata["revision"]}).status_code == 401
    app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
    listed = client.get("/admin/book-covers/missing").json()["result"]["items"][0]
    assert listed["book_id"] == path.stat().st_ino
    assert listed["category"] == "_root"
    response = client.post("/admin/book-covers/apply", json={"file_path": "book.epub", "candidate_id": candidate_id, "revision": metadata["revision"]})
    assert response.status_code == 200
    assert response.json()["result"]["book_id"] == path.stat().st_ino
    assert client.get("/admin/book-covers/preview?file_path=book.epub").headers["content-type"] == "image/jpeg"
    assert client.get("/admin/book-covers/missing").json()["result"]["items"] == []
    assert not (service.cache / f"candidate-{candidate_id}.json").exists()
    assert not (service.cache / f"image-{candidate_id}.jpg").exists()


def cached_candidates(service, monkeypatch, relative="book.epub", stores=("yes24",)):
    from backend import cover_management

    def search(store, metadata):
        return {"status": "success", "candidates": [
            {"title": metadata["title"], "author": "저자", "detail_url": "https://www.yes24.com/",
             "image_url": f"https://image.yes24.com/{store}-{index}.jpg"}
            for index in range(2)
        ]}

    monkeypatch.setattr(cover_management, "search_candidates", search)
    fetches = []

    def fetch(*args):
        fetches.append(args[1])
        return image_bytes()

    monkeypatch.setattr(cover_management, "fetch_bytes", fetch)
    candidates = [candidate for store in stores for candidate in service.candidates(store, relative)["candidates"]]
    for candidate in candidates:
        service.image(candidate["candidate_id"])
    return candidates, fetches


def test_pending_cache_survives_age_and_service_restart(tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement

    make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    candidates, fetches = cached_candidates(service, monkeypatch)
    import time
    old_time = time.time() - 30 * 86400
    for path in service.cache.rglob("*"):
        if path.is_file():
            os.utime(path, (old_time, old_time))
    restarted = CoverManagement(tmp_path)
    for candidate in candidates:
        assert restarted.image(candidate["candidate_id"])
    assert len(fetches) == 2
    monkeypatch.setattr("backend.cover_management.search_candidates", lambda *args: pytest.fail("캐시한 검색을 다시 요청했다"))
    restored = restarted.candidates("yes24", "book.epub")["candidates"]
    assert [item["candidate_id"] for item in restored] == [item["candidate_id"] for item in candidates]
    assert all(item["width"] == 120 and item["height"] == 180 and item["byte_size"] > 0 for item in restored)


def test_success_removes_all_book_candidates_but_keeps_other_book_and_backup(tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement
    from backend.cover_sources import STORES
    from backend.epub_cover import inspect_epub, CoverError

    path = make_epub(tmp_path / "book.epub")
    make_epub(tmp_path / "other.epub")
    service = CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch, stores=tuple(STORES))
    other, _ = cached_candidates(service, monkeypatch, relative="other.epub")
    result = service.apply("book.epub", candidates[0]["candidate_id"], inspect_epub(path)["revision"])
    assert inspect_epub(path)["has_cover"]
    assert (tmp_path / result["backup_path"]).is_file()
    assert len(list(service.cache.rglob("*.jpg"))) == 2
    assert all(json.loads(record.read_text()).get("file_path") != "book.epub" for record in service.cache.rglob("candidate-*.json"))
    assert service.image(other[0]["candidate_id"])
    with pytest.raises(CoverError):
        service.image(candidates[1]["candidate_id"])


def test_failed_apply_keeps_candidates_for_retry(tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError

    path = make_epub(tmp_path / "book.epub")
    original = path.read_bytes()
    service = CoverManagement(tmp_path)
    candidates, fetches = cached_candidates(service, monkeypatch)
    before = {str(file): file.read_bytes() for file in service.cache.rglob("*") if file.suffix in (".json", ".jpg")}
    with pytest.raises(CoverError):
        service.apply("book.epub", candidates[0]["candidate_id"], "stale revision")
    assert path.read_bytes() == original
    assert before == {str(file): file.read_bytes() for file in service.cache.rglob("*") if file.suffix in (".json", ".jpg")}
    assert service.image(candidates[0]["candidate_id"])
    assert len(fetches) == 2


def test_apply_waits_for_download_and_removes_its_cache(tmp_path, monkeypatch):
    import threading
    from concurrent.futures import ThreadPoolExecutor
    from backend import cover_management
    from backend.epub_cover import inspect_epub, CoverError

    path = make_epub(tmp_path / "book.epub")
    service = cover_management.CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    pending_id = candidates[1]["candidate_id"]
    for image in service.cache.rglob(f"image-{pending_id}.jpg"):
        image.unlink()
    started = threading.Event()
    release = threading.Event()
    def download(*args):
        started.set()
        assert release.wait(3)
        return image_bytes()
    monkeypatch.setattr(cover_management, "fetch_bytes", download)
    old_revision = inspect_epub(path)["revision"]
    with ThreadPoolExecutor(max_workers=2) as pool:
        downloading = pool.submit(service.image, pending_id)
        assert started.wait(3)
        applying = pool.submit(service.apply, "book.epub", candidates[0]["candidate_id"], old_revision)
        try:
            assert not applying.done()
        finally:
            release.set()
        assert downloading.result(timeout=3)
        assert applying.result(timeout=3)
    assert not list(service.cache.rglob("*.jpg"))
    with pytest.raises(CoverError):
        service.image(pending_id)


def test_legacy_cache_provides_image_size_without_external_fetch(tmp_path, monkeypatch):
    import hashlib
    from backend import cover_management
    from backend.epub_cover import inspect_epub

    path = make_epub(tmp_path / "book.epub")
    service = cover_management.CoverManagement(tmp_path)
    old_revision = inspect_epub(path)["revision"]
    candidate_id = "b" * 64
    record = {"store": "yes24", "file_path": "book.epub", "revision": old_revision, "image_url": "https://image.yes24.com/book.jpg"}
    key = hashlib.sha256(f"yes24:book.epub:{old_revision}".encode()).hexdigest()
    service._write(service.cache / f"candidate-{candidate_id}.json", json.dumps(record).encode())
    service._write(service.cache / f"image-{candidate_id}.jpg", image_bytes())
    service._write(service.cache / f"search-{key}.json", json.dumps({"status": "success", "candidates": [{"candidate_id": candidate_id}]}).encode())
    monkeypatch.setattr(cover_management, "fetch_bytes", lambda *args: pytest.fail("외부 이미지 요청"))
    monkeypatch.setattr(cover_management, "search_candidates", lambda *args: pytest.fail("외부 검색 요청"))
    result = service.candidates("yes24", "book.epub")["candidates"][0]
    assert (result["width"], result["height"]) == (120, 180)
    assert result["byte_size"] == len(image_bytes())


@pytest.mark.parametrize("operation", ["wrong_store", "covered", "stale_image", "missing_image", "bad_id", "unknown_id", "no_cover_preview", "unrenderable", "stale_url", "covered_url"])
def test_service_rejects_invalid_and_stale_operations(tmp_path, monkeypatch, operation):
    from backend import cover_management as module
    from backend.epub_cover import CoverError, inspect_epub
    path = make_epub(tmp_path / "book.epub")
    service = module.CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    candidate_id = candidates[0]["candidate_id"]
    rev = inspect_epub(path)["revision"]
    original = path.read_bytes()
    if operation in ("covered", "covered_url", "unrenderable"):
        service.apply("book.epub", candidate_id, rev)
        rev = inspect_epub(path)["revision"]
        original = path.read_bytes()
    if operation == "stale_image":
        record = service._candidate(candidate_id)
        record["revision"] = "stale"
        service._write(service._candidate_path(candidate_id), json.dumps(record).encode())
    if operation == "missing_image":
        (service._candidate_path(candidate_id).parent / f"image-{candidate_id}.jpg").unlink()
    if operation == "unrenderable":
        monkeypatch.setattr("backend.book_manager.BookManager._extract_cover_thumbnail", lambda *args: None)
    actions = {
        "wrong_store": lambda: service.candidates("unknown", "book.epub"),
        "covered": lambda: service.candidates("yes24", "book.epub"),
        "stale_image": lambda: service.image(candidate_id),
        "missing_image": lambda: service.apply("book.epub", candidate_id, rev),
        "bad_id": lambda: service.image("bad"),
        "unknown_id": lambda: service.image("e" * 64),
        "no_cover_preview": lambda: service.preview("book.epub"),
        "unrenderable": lambda: service.preview("book.epub"),
        "stale_url": lambda: service.apply_url("book.epub", "https://example.org/a", "stale"),
        "covered_url": lambda: service.apply_url("book.epub", "https://example.org/a", rev),
    }
    with pytest.raises(CoverError):
        actions[operation]()
    assert path.read_bytes() == original


def test_search_error_cache_expiry_cancel_and_cache_double_check(tmp_path, monkeypatch):
    from backend import cover_management as module
    from backend.cover_sources import FetchCancelled
    make_epub(tmp_path / "book.epub")
    service = module.CoverManagement(tmp_path)
    monkeypatch.setattr(module, "search_candidates", lambda *args: (_ for _ in ()).throw(ValueError("blocked")))
    assert service.candidates("yes24", "book.epub")["status"] == "error"
    assert service.candidates("yes24", "book.epub")["status"] == "error"
    for cached in service._book_cache("book.epub").glob("search-*.json"):
        os.utime(cached, (0, 0))
    monkeypatch.setattr(module, "search_candidates", lambda *args: (_ for _ in ()).throw(FetchCancelled()))
    with pytest.raises(FetchCancelled):
        service.candidates("yes24", "book.epub")
    values = iter([None, None, {"status": "not_found", "candidates": []}])
    monkeypatch.setattr(service, "_cached_search", lambda *args: next(values))
    assert service.candidates("yes24", "book.epub")["status"] == "not_found"


def test_invalid_cached_records_refresh_and_image_lock_double_check(tmp_path, monkeypatch):
    from contextlib import contextmanager
    from backend.cover_management import CoverManagement
    make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    candidate_id = candidates[0]["candidate_id"]
    record_path = service._candidate_path(candidate_id)
    record = service._candidate(candidate_id)
    image_path = record_path.parent / f"image-{candidate_id}.jpg"
    image_path.unlink()
    @contextmanager
    def cache_appears(*args):
        service._write(image_path, image_bytes())
        yield
    monkeypatch.setattr("backend.cover_management.store_lock", cache_appears)
    assert service.image(candidate_id) == image_bytes()
    record_path.unlink()
    assert service.candidates("yes24", "book.epub")["status"] == "success"
    bad = service.cache / "bad.json"
    service._write(bad, b"[]")
    assert service._read_json(bad) is None
    service._write(bad, b"not json")
    assert service._read_json(bad) is None
    assert record["file_path"] == "book.epub"


def test_success_warns_on_candidate_cleanup_failure_and_invalidates_only_own_previews(tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import inspect_epub
    path = make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    book_id = path.stat().st_ino
    for name in (".cover_cache", ".preview_cache"):
        directory = tmp_path / name
        directory.mkdir()
        (directory / f"{book_id}.jpg").write_bytes(b"stale")
        (directory / f"{book_id}_ch10.epub").write_bytes(b"stale")
        (directory / f"{book_id}0.jpg").write_bytes(b"other book")
        (directory / f"{book_id}_directory").mkdir()
    monkeypatch.setattr(service, "_delete_book_cache", lambda *args: (_ for _ in ()).throw(OSError("disk")))
    result = service.apply("book.epub", candidates[0]["candidate_id"], inspect_epub(path)["revision"])
    assert "warning" in result
    assert inspect_epub(path)["has_cover"]
    for name in (".cover_cache", ".preview_cache"):
        assert not (tmp_path / name / f"{book_id}.jpg").exists()
        assert not (tmp_path / name / f"{book_id}_ch10.epub").exists()
        assert (tmp_path / name / f"{book_id}0.jpg").exists()
        assert (tmp_path / name / f"{book_id}_directory").is_dir()


@pytest.mark.parametrize("relative,directory", [("missing", True), ("book.txt", False), (".hidden/book.epub", False)])
def test_missing_and_internal_paths_are_rejected(tmp_path, relative, directory):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError
    with pytest.raises(CoverError):
        CoverManagement(tmp_path).resolve(relative, directory=directory)


@pytest.mark.parametrize("cursor", ["abc", "1" * 11])
def test_invalid_listing_cursor_is_rejected(tmp_path, cursor):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError
    with pytest.raises(CoverError):
        CoverManagement(tmp_path).missing("", cursor, 60)


@pytest.fixture
def cover_api(tmp_path, monkeypatch):
    from fastapi import FastAPI
    from fastapi.testclient import TestClient
    from unittest.mock import Mock
    from backend.cover_management import Book, create_cover_router, require_admin
    monkeypatch.setattr(Book, "path_prefix", tmp_path)
    manager = Mock()
    manager.es_manager.search_by_id.return_value = {"title": "책"}
    manager.es_manager.update.return_value = True
    app = FastAPI()
    app.include_router(create_cover_router(lambda: manager))
    client = TestClient(app)
    app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
    return client, app, manager


def test_all_api_gets_and_clear_cache_response(cover_api, tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import inspect_epub
    client, _, _ = cover_api
    path = make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    candidates, _ = cached_candidates(service, monkeypatch)
    assert client.get("/admin/book-covers/directories").json()["result"] == []
    assert client.get("/admin/book-covers/candidates/yes24?file_path=book.epub").json()["result"]["status"] == "success"
    image = client.get("/admin/book-covers/candidate-image/" + candidates[0]["candidate_id"])
    assert image.status_code == 200 and image.headers["content-type"] == "image/jpeg"
    assert client.post("/admin/book-covers/cache/clear", json={"file_path": "book.epub", "revision": inspect_epub(path)["revision"]}).json()["result"]["cleared"]
    assert client.get("/admin/book-covers/candidate-image/bad").status_code == 409
    assert client.get("/admin/book-covers/missing?limit=0").status_code == 422
    monkeypatch.setattr(CoverManagement, "directories", lambda *args: (_ for _ in ()).throw(OSError("secret disk details")))
    response = client.get("/admin/book-covers/directories")
    assert response.status_code == 503
    assert "secret" not in response.text


@pytest.mark.parametrize("index", ["ok", "missing", "failed", "exception"])
def test_direct_url_api_updates_index_or_reports_post_save_warning(cover_api, tmp_path, monkeypatch, index):
    from backend import cover_management as module
    from backend.epub_cover import inspect_epub
    client, _, manager = cover_api
    path = make_epub(tmp_path / "book.epub")
    monkeypatch.setattr(module, "fetch_url_image", lambda url: image_bytes())
    if index == "missing":
        manager.es_manager.search_by_id.return_value = None
    elif index == "failed":
        manager.es_manager.update.return_value = False
    elif index == "exception":
        manager.es_manager.search_by_id.side_effect = RuntimeError("ES unavailable")
    response = client.post("/admin/book-covers/apply-url", json={"file_path": "book.epub", "revision": inspect_epub(path)["revision"], "url": "https://example.org/a"})
    assert response.status_code == 200
    result = response.json()["result"]
    assert inspect_epub(path)["has_cover"]
    assert ("warning" in result) == (index in ("failed", "exception"))
    if index == "ok":
        manager.es_manager.update.assert_called_with(path.stat().st_ino, file_size=path.stat().st_size)


def test_new_api_auth_and_input_validation(cover_api, monkeypatch):
    from backend import auth
    from backend.cover_management import require_admin
    client, app, _ = cover_api
    body = {"file_path": "book.epub", "revision": "r1", "url": "https://example.org/a"}
    app.dependency_overrides.clear()
    for endpoint in ("apply-url", "cache/clear"):
        assert client.post("/admin/book-covers/" + endpoint, json=body).status_code == 401
    monkeypatch.setattr(auth, "_extract_payload", lambda request: {"email": "viewer@test.com"})
    for endpoint in ("apply-url", "cache/clear"):
        assert client.post("/admin/book-covers/" + endpoint, json=body).status_code == 403
    app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
    assert client.post("/admin/book-covers/apply-url", json={**body, "url": ""}).status_code == 422
    assert client.post("/admin/book-covers/apply", json={**body, "candidate_id": "bad"}).status_code == 422


def test_disconnected_search_cancels_worker(cover_api, monkeypatch):
    import threading
    import time
    from starlette.requests import Request
    from backend import cover_management as module
    client, _, _ = cover_api
    finished = threading.Event()
    def search(*args, **kwargs):
        try:
            for _ in range(100):
                module.check_cancelled()
                time.sleep(0.01)
        finally:
            finished.set()
    async def disconnected(self):
        return True
    monkeypatch.setattr(module.CoverManagement, "candidates", search)
    monkeypatch.setattr(Request, "is_disconnected", disconnected)
    response = client.get("/admin/book-covers/candidates/yes24?file_path=book.epub")
    assert response.status_code == 499
    assert finished.wait(2)


def rewrite_epub_members(path, transform):
    """작은 fixture의 멤버를 중복 entry 없이 바꾼다."""
    with zipfile.ZipFile(path) as archive:
        members = {name: archive.read(name) for name in archive.namelist()}
    transform(members)
    with zipfile.ZipFile(path, "w") as archive:
        for name, data in members.items():
            archive.writestr(name, data, compress_type=zipfile.ZIP_STORED if name == "mimetype" else zipfile.ZIP_DEFLATED)


@pytest.mark.parametrize("mode", ["size", "container", "opf", "package", "opf_path", "count", "pixels", "empty_image", "large_image", "svg", "large_existing"])
def test_epub_validation_limits_and_svg_cover(tmp_path, monkeypatch, mode):
    from unittest.mock import Mock
    from backend import epub_cover as module
    path = make_epub(tmp_path / "book.epub", cover=mode in ("svg", "large_existing"))
    if mode == "size":
        monkeypatch.setattr(module, "MAX_EPUB_BYTES", 1)
    elif mode == "container":
        monkeypatch.setattr(module, "MAX_XML_BYTES", 1)
    elif mode == "opf":
        monkeypatch.setattr(module, "MAX_XML_BYTES", 300)
    elif mode == "package":
        rewrite_epub_members(path, lambda members: members.__setitem__("OPS/book.opf", b"<wrong/>"))
    elif mode == "opf_path":
        monkeypatch.setattr(module.BookManager, "_find_opf_path", lambda archive: "../outside.opf")
    elif mode == "count":
        archive = Mock()
        archive.infolist.return_value = [None] * 20001
        with pytest.raises(module.CoverError):
            module._read_package(archive)
        return
    elif mode in ("pixels", "empty_image", "large_image"):
        if mode == "pixels":
            monkeypatch.setattr(module, "MAX_IMAGE_PIXELS", 1)
        elif mode == "large_image":
            monkeypatch.setattr(module, "MAX_IMAGE_BYTES", 1)
        with pytest.raises(module.CoverError):
            module.normalize_image(b"" if mode == "empty_image" else image_bytes())
        return
    elif mode == "svg":
        def svg(members):
            members["OPS/book.opf"] = members["OPS/book.opf"].replace(b'cover.jpg', b'cover.svg').replace(b'image/jpeg', b'image/svg+xml')
            members["OPS/cover.svg"] = b'<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>'
            del members["OPS/cover.jpg"]
        rewrite_epub_members(path, svg)
        assert module.inspect_epub(path)["has_cover"]
        return
    elif mode == "large_existing":
        monkeypatch.setattr(module, "MAX_IMAGE_BYTES", 1)
    with pytest.raises(module.CoverError):
        module.inspect_epub(path)


@pytest.mark.parametrize("isbn,expected", [("urn:isbn:978-1234567890", "9781234567890"), ("1234567890", "1234567890"), ("invalid", "")])
def test_isbn_normalization_and_title_fallback(tmp_path, isbn, expected):
    from backend.epub_cover import inspect_epub
    path = make_epub(tmp_path / "fallback.epub")
    def metadata(members):
        members["OPS/book.opf"] = members["OPS/book.opf"].replace('<dc:title>테스트 책</dc:title>'.encode(), f'<dc:identifier>{isbn}</dc:identifier>'.encode())
    rewrite_epub_members(path, metadata)
    result = inspect_epub(path)
    assert result["title"] == "fallback"
    assert result["isbn"] == expected


@pytest.mark.parametrize("mode", ["manifest", "metadata", "spine", "id", "path", "old_declarations", "revision", "invalid_stage", "no_staged_cover", "locked"])
def test_cover_rebuild_guards_and_old_declaration_cleanup(tmp_path, monkeypatch, mode):
    import hashlib
    from lxml import etree
    from backend import epub_cover as module
    path = make_epub(tmp_path / "book.epub")
    image = module.normalize_image(image_bytes())
    cover_id = "tm-cover-" + hashlib.sha256(image).hexdigest()[:16]
    def modify(members):
        package = etree.fromstring(members["OPS/book.opf"])
        if mode in ("manifest", "metadata", "spine"):
            package.remove(package.find(f"{{{module.OPF_NS}}}{mode}"))
        elif mode == "id":
            manifest = package.find(f"{{{module.OPF_NS}}}manifest")
            etree.SubElement(manifest, f"{{{module.OPF_NS}}}item", id=cover_id, href="chapter.xhtml", **{"media-type": "application/xhtml+xml"})
        elif mode == "path":
            members[f"OPS/{cover_id}.jpg"] = b"unused member"
        elif mode == "old_declarations":
            manifest = package.find(f"{{{module.OPF_NS}}}manifest")
            for index, properties in enumerate(("cover-image", "cover-image other")):
                etree.SubElement(manifest, f"{{{module.OPF_NS}}}item", id=f"stale{index}", href=f"missing{index}.jpg", properties=properties, **{"media-type": "image/jpeg"})
            etree.SubElement(package.find(f"{{{module.OPF_NS}}}metadata"), f"{{{module.OPF_NS}}}meta", name="cover", content="stale0")
            etree.SubElement(package.find(f"{{{module.OPF_NS}}}metadata"), f"{{{module.OPF_NS}}}meta", name="other", content="preserve")
        members["OPS/book.opf"] = etree.tostring(package)
    rewrite_epub_members(path, modify)
    original = path.read_bytes()
    rev = module.revision(path)
    if mode == "locked":
        monkeypatch.setattr(module.fcntl, "flock", lambda *args: (_ for _ in ()).throw(BlockingIOError()))
        with pytest.raises(module.CoverError, match="저장 중"):
            module.inspect_epub(path)
    elif mode == "revision":
        values = iter([rev, "changed"])
        monkeypatch.setattr(module, "revision", lambda *args: next(values))
    elif mode == "invalid_stage":
        monkeypatch.setattr(zipfile.ZipFile, "testzip", lambda self: "corrupt")
    elif mode == "no_staged_cover":
        inspect = module._inspect_archive
        monkeypatch.setattr(module, "_inspect_archive", lambda archive: {**inspect(archive), "has_cover": False})
    if mode == "old_declarations":
        module.add_cover(path, image_bytes(), rev, tmp_path / ".cover_backups")
        with zipfile.ZipFile(path) as archive:
            package = etree.fromstring(archive.read("OPS/book.opf"))
            assert package.xpath('count(//*[local-name()="meta" and @name="cover"])') == 1
            assert package.xpath('string(//*[local-name()="item" and @id="stale1"]/@properties)') == "other"
        assert module.inspect_epub(path)["has_cover"]
    else:
        with pytest.raises(module.CoverError):
            module.add_cover(path, image_bytes(), rev, tmp_path / ".cover_backups")
        assert path.read_bytes() == original
        assert not list(tmp_path.glob(".cover-*.tmp"))


def test_resolved_escape_and_incomplete_legacy_cache_are_safe(tmp_path, monkeypatch):
    from backend.cover_management import CoverManagement
    from backend.epub_cover import CoverError, inspect_epub
    path = make_epub(tmp_path / "book.epub")
    service = CoverManagement(tmp_path)
    original_resolve = Path.resolve
    with monkeypatch.context() as patch:
        patch.setattr(Path, "resolve", lambda value, *args, **kwargs: tmp_path.parent / "outside.epub" if value == path else original_resolve(value, *args, **kwargs))
        with pytest.raises(CoverError, match="디렉토리 밖"):
            service.resolve("book.epub")
    candidates, _ = cached_candidates(service, monkeypatch)
    record_path = service._candidate_path(candidates[0]["candidate_id"])
    record = service._candidate(candidates[0]["candidate_id"])
    del record["width"]
    service._write(record_path, json.dumps(record).encode())
    (record_path.parent / f"image-{candidates[0]['candidate_id']}.jpg").unlink()
    result = service.candidates("yes24", "book.epub")["candidates"][0]
    assert "width" not in result
    service._write(service.cache / "candidate-invalid.json", b"[]")
    service.clear_cache("book.epub", inspect_epub(path)["revision"])
    assert (service.cache / "candidate-invalid.json").exists()


@pytest.mark.parametrize("markup,oversized", [(b"", False), (b"\xff", False), (b"<html/>", True)])
def test_unreadable_or_oversized_cover_page_is_not_a_cover(tmp_path, monkeypatch, markup, oversized):
    from backend.book_manager import BookManager
    path = make_epub(tmp_path / "book.epub")
    def modify(members):
        members["OPS/book.opf"] = members["OPS/book.opf"].replace(b"chapter.xhtml", b"cover.xhtml")
        members["OPS/cover.xhtml"] = markup
    rewrite_epub_members(path, modify)
    with zipfile.ZipFile(path) as archive:
        if oversized:
            archive.getinfo("OPS/cover.xhtml").file_size = 5 * 1024 * 1024
        assert BookManager._find_epub_cover_member(archive) is None


def test_guide_missing_cover_page_is_ignored(tmp_path):
    from backend.book_manager import BookManager
    path = make_epub(tmp_path / "book.epub")
    rewrite_epub_members(path, lambda members: members.__setitem__("OPS/book.opf", members["OPS/book.opf"].replace(b"</package>", b'<guide><reference type="cover" href="missing.xhtml"/></guide></package>')))
    with zipfile.ZipFile(path) as archive:
        assert BookManager._find_epub_cover_member(archive) is None


@pytest.mark.parametrize("width,height,format", [(1800, 2700, "JPEG"), (1200, 600, "PNG"), (300, 450, "PNG"), (6000, 8000, "JPEG")])
def test_saved_cover_is_at_most_600_pixels_wide_and_keeps_aspect_ratio(tmp_path, width, height, format):
    from backend.epub_cover import add_cover, inspect_epub, normalize_image
    from backend.book_manager import BookManager
    buffer = io.BytesIO()
    with Image.new("RGB", (width, height), "red") as image:
        image.save(buffer, format)
    raw = buffer.getvalue()
    candidate = normalize_image(raw)
    with Image.open(io.BytesIO(candidate)) as image:
        assert image.width <= 2000
        if width > 600:
            assert image.width > 600
    path = make_epub(tmp_path / "book.epub")
    add_cover(path, raw, inspect_epub(path)["revision"], tmp_path / ".cover_backups")
    with zipfile.ZipFile(path) as archive:
        member = BookManager._find_epub_cover_member(archive)
        with Image.open(io.BytesIO(archive.read(member))) as image:
            assert image.width == min(width, 600)
            assert image.height == round(height * min(width, 600) / width)


@pytest.mark.parametrize("host", ["image.yes24.com", "yes24.com"])
def test_direct_store_url_uses_same_store_lock(tmp_path, monkeypatch, host):
    from contextlib import contextmanager
    from backend import cover_management as module
    from backend.epub_cover import inspect_epub
    path = make_epub(tmp_path / "book.epub")
    stores = []
    @contextmanager
    def locked(cache, store):
        stores.append(store)
        yield
    monkeypatch.setattr(module, "store_lock", locked)
    monkeypatch.setattr(module, "fetch_url_image", lambda url: image_bytes())
    module.CoverManagement(tmp_path).apply_url("book.epub", f"https://{host}/cover.jpg", inspect_epub(path)["revision"])
    assert stores == ["yes24"]
