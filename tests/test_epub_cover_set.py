"""EPUB 뷰어의 표지 이미지 추가: BookManager.set_epub_cover 와 POST /epub-cover/{id}.

운영 파일 대신 임시 EPUB 를 쓰고, 이미지 다운로드만 가짜로 바꾼다.
"""

from datetime import datetime
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest

from backend.book_manager import BookManager
from backend.epub_cover import CoverError, inspect_epub
from tests.test_cover_management import image_bytes, make_epub, rewrite_epub_members

URL = "https://img.example.com/cover.jpg"


def image_bytes_of(color):
    import io

    from PIL import Image

    output = io.BytesIO()
    Image.new("RGB", (120, 180), color).save(output, "JPEG")
    return output.getvalue()


@pytest.fixture
def env(tmp_path, monkeypatch):
    """(manager 대역, EPUB 경로, 책). get_book 은 이 EPUB 를 가리키는 책을 돌려준다."""
    monkeypatch.setattr("backend.cover_management.fetch_url_image", lambda url: image_bytes())
    path = make_epub(tmp_path / "분류" / "book.epub")
    book = SimpleNamespace(file_path=path, file_type="epub")
    es = MagicMock()
    es.update.return_value = True
    manager = SimpleNamespace(path_prefix=tmp_path, TRASH_DIRNAME="0_trash", es_manager=es, get_book=AsyncMock(return_value=(book, None)))
    manager._change_epub_cover = lambda *args, **kwargs: BookManager._change_epub_cover(manager, *args, **kwargs)
    return manager, path, book


async def call(manager, url=URL):
    return await BookManager.set_epub_cover(manager, 1, url)


async def remove(manager):
    return await BookManager.remove_epub_cover(manager, 1)


def members(path):
    import zipfile

    with zipfile.ZipFile(path) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


@pytest.mark.asyncio
async def test_adds_a_cover_to_an_epub_without_one(env):
    manager, path, _ = env
    assert not inspect_epub(path)["has_cover"]

    result, error = await call(manager)

    assert error is None
    assert inspect_epub(path)["has_cover"]
    assert result["file_size"] == path.stat().st_size
    manager.es_manager.update.assert_called_once()
    args, kwargs = manager.es_manager.update.call_args
    assert args == (1,) and kwargs["file_size"] == path.stat().st_size
    # 최신 책 목록(updated_time 순)에 오르도록 방금 시각을 색인한다. 만화 인덱스의 strptime 형식과도 맞아야 한다.
    stamp = datetime.strptime(kwargs["updated_time"], "%Y-%m-%dT%H:%M:%S.%f")
    assert abs((datetime.now() - stamp).total_seconds()) < 60
    assert "warning" not in result


@pytest.mark.asyncio
async def test_replaces_an_existing_cover_and_keeps_the_original_in_0_trash(env, tmp_path):
    manager, path, _ = env
    make_epub(path, cover=True)
    original = path.read_bytes()

    result, error = await call(manager)

    assert error is None
    assert path.read_bytes() != original
    # PDF 편집과 같이 0_trash 에 원래 이름으로 둔다. 숨김 폴더 .cover_backups 는 쓰지 않는다.
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original
    assert not (tmp_path / ".cover_backups").exists()


@pytest.mark.asyncio
async def test_second_edit_keeps_the_first_original(env, tmp_path, monkeypatch):
    manager, path, _ = env
    first_original = path.read_bytes()

    await call(manager)
    after_first = path.read_bytes()
    monkeypatch.setattr("backend.cover_management.fetch_url_image", lambda url: image_bytes_of("blue"))
    result, error = await call(manager)

    assert error is None
    assert path.read_bytes() != after_first
    # 처음 원본을 지킨다(덮어쓰지 않는다). 임시 롤백 파일도 남기지 않는다.
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == first_original
    assert [p.name for p in (tmp_path / "0_trash").iterdir()] == ["book.epub"]
    assert not list(path.parent.glob(".cover-*.tmp"))


def test_failed_write_restores_the_current_file_not_the_older_backup(tmp_path, monkeypatch):
    """백업이 이미 있으면 그것은 처음 원본이다. 쓰다가 실패하면 지금 원본으로 되돌려야 한다."""
    import backend.epub_cover as module

    path = make_epub(tmp_path / "book.epub")
    trash = tmp_path / "0_trash"
    module.add_cover(path, image_bytes(), module.inspect_epub(path)["revision"], trash, backup_name="book.epub")
    current = path.read_bytes()
    older = (trash / "book.epub").read_bytes()
    assert current != older

    real_fsync = module.os.fsync
    calls = 0

    def fail_on_write_back(fd):
        nonlocal calls
        calls += 1
        if calls == 3:  # 롤백 사본(1), 편집본(2), 본 파일에 쓴 뒤(3)
            raise OSError("write failed")
        return real_fsync(fd)

    monkeypatch.setattr(module.os, "fsync", fail_on_write_back)
    with pytest.raises(OSError):
        module.add_cover(path, image_bytes_of("green"), module.inspect_epub(path)["revision"], trash, replace_existing=True, backup_name="book.epub")

    assert path.read_bytes() == current
    assert (trash / "book.epub").read_bytes() == older
    assert not list(tmp_path.glob(".cover-*.tmp"))


@pytest.mark.asyncio
async def test_rejects_a_book_that_is_not_an_epub(env):
    manager, path, book = env
    book.file_type = "pdf"
    before = path.read_bytes()

    result, error = await call(manager)

    assert result is None and "EPUB" in error
    assert path.read_bytes() == before


@pytest.mark.asyncio
async def test_reports_a_missing_book(env):
    manager, *_ = env
    manager.get_book.return_value = (None, "No book found by '1'")

    result, error = await call(manager)

    assert result is None and "No book found" in error


@pytest.mark.asyncio
async def test_blank_url_is_rejected_before_touching_the_file(env):
    manager, path, _ = env
    before = path.read_bytes()

    result, error = await call(manager, "   ")

    assert result is None and "URL" in error
    assert path.read_bytes() == before


@pytest.mark.asyncio
async def test_download_failure_returns_the_reason_and_keeps_the_file(env, monkeypatch):
    manager, path, _ = env

    def refuse(url):
        raise CoverError("내부 주소의 이미지는 사용할 수 없습니다")

    monkeypatch.setattr("backend.cover_management.fetch_url_image", refuse)
    before = path.read_bytes()

    result, error = await call(manager)

    assert result is None and "내부 주소" in error
    assert path.read_bytes() == before
    manager.es_manager.update.assert_not_called()


@pytest.mark.asyncio
async def test_unexpected_io_error_hides_internal_details(env, monkeypatch):
    manager, *_ = env

    def broken(url):
        raise OSError("/secret/disk/path failed")

    monkeypatch.setattr("backend.cover_management.fetch_url_image", broken)

    result, error = await call(manager)

    assert result is None
    assert "secret" not in error


@pytest.mark.asyncio
async def test_index_update_failure_keeps_the_saved_cover_and_warns(env):
    manager, path, _ = env
    manager.es_manager.update.return_value = False

    result, error = await call(manager)

    assert error is None
    assert inspect_epub(path)["has_cover"]
    assert "색인" in result["warning"]


@pytest.mark.asyncio
async def test_removes_a_cover_added_by_this_tool(env, tmp_path):
    manager, path, _ = env
    original = path.read_bytes()
    await call(manager)
    with_cover = path.read_bytes()
    manager.es_manager.update.reset_mock()

    result, error = await remove(manager)

    assert error is None
    assert not inspect_epub(path)["has_cover"]
    names = members(path)
    assert not [name for name in names if "tm-cover" in name]
    assert b"tm-cover" not in names["OPS/book.opf"]
    # 본문은 그대로다.
    assert members(path)["OPS/chapter.xhtml"] == "<html><body>본문을 보존한다.</body></html>".encode()
    assert result["cover_image"] is None and result["file_size"] == path.stat().st_size
    # 처음 원본은 0_trash 에 그대로 있다(표지를 넣을 때 보존한 것).
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original and with_cover != original
    _, kwargs = manager.es_manager.update.call_args
    assert kwargs["file_size"] == path.stat().st_size
    datetime.strptime(kwargs["updated_time"], "%Y-%m-%dT%H:%M:%S.%f")


@pytest.mark.asyncio
async def test_remove_clears_the_stale_thumbnail_and_preview_caches(env, tmp_path):
    manager, path, _ = env
    await call(manager)
    book_id = path.stat().st_ino
    stale = []
    for directory, name in ((".cover_cache", f"{book_id}.v1.jpg"), (".preview_cache", f"{book_id}_10.epub")):
        (tmp_path / directory).mkdir(exist_ok=True)
        (tmp_path / directory / name).write_bytes(b"stale")
        stale.append(tmp_path / directory / name)

    await remove(manager)

    assert not any(item.exists() for item in stale)


@pytest.mark.asyncio
@pytest.mark.parametrize("action", ["remove", "replace"])
async def test_cover_edit_clears_caches_for_index_id_and_current_inode(env, tmp_path, action):
    """ES의 책 ID와 파일 inode가 달라도 편집 화면의 캐시를 지워야 한다."""
    manager, path, _ = env
    await call(manager)
    indexed_id = 1  # get_book(1)이 현재 파일을 가리킨다.
    inode = path.stat().st_ino
    assert inode != indexed_id
    stale = []
    keep = []
    for directory, suffix in ((".cover_cache", ".v1.jpg"), (".preview_cache", "_ch10_v3.epub")):
        cache_dir = tmp_path / directory
        cache_dir.mkdir(exist_ok=True)
        for book_id in (indexed_id, inode):
            cached = cache_dir / f"{book_id}{suffix}"
            cached.write_bytes(b"stale")
            stale.append(cached)
        unrelated = cache_dir / f"{indexed_id}0{suffix}"
        unrelated.write_bytes(b"keep")
        keep.append(unrelated)

    _, error = await (remove(manager) if action == "remove" else call(manager))

    assert error is None
    assert not any(cached.exists() for cached in stale)
    assert all(cached.read_bytes() == b"keep" for cached in keep)


@pytest.mark.asyncio
async def test_replaced_publisher_cover_stays_in_the_backup(env, tmp_path):
    """기존 표지는 백업에 보존하고, 새 표지를 지운 뒤 다시 나타나지 않아야 한다."""
    manager, path, _ = env
    make_epub(path, cover=True)
    original = path.read_bytes()
    await call(manager)
    assert "OPS/cover.jpg" not in members(path)
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original

    result, error = await remove(manager)

    assert error is None
    assert not inspect_epub(path)["has_cover"]
    assert not [name for name in members(path) if "tm-cover" in name]


@pytest.mark.asyncio
async def test_removes_a_publisher_cover_and_preserves_the_original_backup(env, tmp_path):
    manager, path, _ = env
    make_epub(path, cover=True)
    before = path.read_bytes()

    result, error = await remove(manager)

    assert error is None
    assert result["cover_image"] is None
    assert not inspect_epub(path)["has_cover"]
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == before
    assert members(path)["OPS/chapter.xhtml"] == "<html><body>본문을 보존한다.</body></html>".encode()
    assert not list(path.parent.glob(".cover-*.tmp"))
    manager.es_manager.update.assert_called_once()


@pytest.mark.asyncio
async def test_remove_refuses_a_book_without_a_cover(env, tmp_path):
    manager, path, _ = env
    before = path.read_bytes()

    result, error = await remove(manager)

    assert result is None and error
    assert path.read_bytes() == before
    assert not (tmp_path / "0_trash").exists()


def attach_publisher_cover_page(path, *, shared=False):
    """형님 EPUB과 같은 SVG 표지, guide, NCX 구조를 만든다."""
    from lxml import etree
    from backend.epub_cover import OPF_NS

    def attach(files):
        package = etree.fromstring(files["OPS/book.opf"])
        manifest = package.find(f"{{{OPF_NS}}}manifest")
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id="titlepage", href="Text/front.xhtml", **{"media-type": "application/xhtml+xml"})
        etree.SubElement(manifest, f"{{{OPF_NS}}}item", id="ncx", href="toc.ncx", **{"media-type": "application/x-dtbncx+xml"})
        spine = package.find(f"{{{OPF_NS}}}spine")
        spine.insert(0, etree.Element(f"{{{OPF_NS}}}itemref", idref="titlepage"))
        spine.set("toc", "ncx")
        guide = etree.SubElement(package, f"{{{OPF_NS}}}guide")
        etree.SubElement(guide, f"{{{OPF_NS}}}reference", type="cover", href="Text/front.xhtml#cover")
        files["OPS/book.opf"] = etree.tostring(package)
        files["OPS/Text/front.xhtml"] = b'<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Cover</title></head><body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../cover.jpg"/></svg></body></html>'
        files["OPS/toc.ncx"] = b'<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="cover" playOrder="1"><navLabel><text>Cover</text></navLabel><content src="Text/front.xhtml#cover"/></navPoint><navPoint id="chapter" playOrder="2"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml"/></navPoint></navMap></ncx>'
        if package.get("version") == "3.0":
            etree.SubElement(manifest, f"{{{OPF_NS}}}item", id="nav", href="nav.xhtml", properties="nav", **{"media-type": "application/xhtml+xml"})
            files["OPS/nav.xhtml"] = b'<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="Text/front.xhtml#cover">Cover</a></li><li><a href="chapter.xhtml">Chapter</a></li></ol></nav></body></html>'
            files["OPS/book.opf"] = etree.tostring(package)
        if shared:
            files["OPS/chapter.xhtml"] = b'<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Keep this chapter.</p><img src="cover.jpg"/></body></html>'

    rewrite_epub_members(path, attach)


@pytest.mark.asyncio
@pytest.mark.parametrize("action", ["remove", "replace"])
@pytest.mark.parametrize("version", ["2.0", "3.0"])
async def test_publisher_svg_cover_is_detached_from_spine_guide_and_toc(env, tmp_path, action, version):
    from lxml import etree

    manager, path, _ = env
    make_epub(path, version=version, cover=True)
    attach_publisher_cover_page(path)
    original = path.read_bytes()
    before = members(path)
    stat = path.stat()

    result, error = await (remove(manager) if action == "remove" else call(manager))

    assert error is None
    after = members(path)
    package = etree.fromstring(after["OPS/book.opf"])
    assert not package.xpath('//*[local-name()="itemref" and @idref="titlepage"]')
    assert not package.xpath('//*[local-name()="reference" and @type="cover" and starts-with(@href,"Text/front.xhtml")]')
    toc = etree.fromstring(after["OPS/toc.ncx"])
    assert not toc.xpath('//*[local-name()="content" and starts-with(@src,"Text/front.xhtml")]')
    assert toc.xpath('//*[local-name()="content" and @src="chapter.xhtml"]')
    assert toc.xpath('string(//*[local-name()="navPoint"]/@playOrder)') == "1"
    if version == "3.0":
        nav = etree.fromstring(after["OPS/nav.xhtml"])
        assert nav.xpath('//*[local-name()="a"]/@href') == ["chapter.xhtml"]
    assert after["OPS/chapter.xhtml"] == before["OPS/chapter.xhtml"]
    assert after["OPS/style.css"] == before["OPS/style.css"]
    assert path.stat().st_ino == stat.st_ino
    assert path.stat().st_mtime_ns == stat.st_mtime_ns
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original
    assert inspect_epub(path)["has_cover"] is (action == "replace")
    if action == "replace":
        _, error = await remove(manager)
        assert error is None
        assert not inspect_epub(path)["has_cover"]


@pytest.mark.asyncio
async def test_removed_cover_does_not_promote_the_next_image_only_page(env):
    manager, path, _ = env
    make_epub(path, cover=True)
    attach_publisher_cover_page(path)
    chapter = b'<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="illustration.jpg"/></body></html>'

    def illustrate(files):
        files["OPS/chapter.xhtml"] = chapter
        files["OPS/illustration.jpg"] = image_bytes_of("green")
        files["OPS/book.opf"] = files["OPS/book.opf"].replace(b'</manifest>', b'<item id="illustration" href="illustration.jpg" media-type="image/jpeg"/></manifest>')

    rewrite_epub_members(path, illustrate)

    _, error = await remove(manager)

    assert error is None
    assert not inspect_epub(path)["has_cover"]
    assert members(path)["OPS/chapter.xhtml"] == chapter
    assert "OPS/illustration.jpg" in members(path)


@pytest.mark.asyncio
async def test_removal_preserves_a_cover_image_used_in_a_chapter(env, tmp_path):
    manager, path, _ = env
    make_epub(path, cover=True)
    attach_publisher_cover_page(path, shared=True)
    original = path.read_bytes()

    result, error = await remove(manager)

    assert result is None and "본문" in error
    assert path.read_bytes() == original
    assert not (tmp_path / "0_trash").exists()


@pytest.mark.asyncio
async def test_remove_rejects_a_non_epub_and_a_missing_book(env):
    manager, _, book = env
    book.file_type = "pdf"
    result, error = await remove(manager)
    assert result is None and "EPUB" in error

    manager.get_book.return_value = (None, "No book found by '1'")
    result, error = await remove(manager)
    assert result is None and "No book found" in error


@pytest.mark.asyncio
async def test_remove_second_time_is_refused_because_nothing_is_left(env):
    manager, path, _ = env
    await call(manager)
    await remove(manager)
    after = path.read_bytes()

    result, error = await remove(manager)

    assert result is None and error
    assert path.read_bytes() == after


@pytest.mark.asyncio
async def test_remove_index_failure_keeps_the_change_and_warns(env):
    manager, path, _ = env
    await call(manager)
    manager.es_manager.update.return_value = False

    result, error = await remove(manager)

    assert error is None
    assert "색인" in result["warning"]


class TestEpubCoverEndpoint:
    @pytest.fixture
    def client(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        # test_auth 가 backend.auth/main 을 reload 하므로, 앱 라우트가 실제로 잡고 있는 객체를 오버라이드해야 한다.
        require_admin = main.require_admin

        fake = MagicMock()
        fake.set_epub_cover = AsyncMock(return_value=({"file_size": 10}, None))
        fake.remove_epub_cover = AsyncMock(return_value=({"file_size": 9}, None))
        proxy = main.book_manager
        previous = proxy._instance
        proxy._instance = fake
        main.app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
        try:
            yield TestClient(main.app), fake
        finally:
            main.app.dependency_overrides.pop(require_admin, None)
            proxy._instance = previous

    def test_success(self, client):
        http, fake = client
        response = http.post("/epub-cover/1", json={"url": URL})
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": {"file_size": 10}}
        fake.set_epub_cover.assert_awaited_once_with(1, URL, None)

    def test_failure_is_reported_in_the_body(self, client):
        http, fake = client
        fake.set_epub_cover.return_value = (None, "EPUB이 변경되었습니다. 목록을 새로 불러오세요")
        body = http.post("/epub-cover/1", json={"url": URL}).json()
        assert body == {"status": "failure", "error": "EPUB이 변경되었습니다. 목록을 새로 불러오세요"}

    def test_empty_or_oversized_url_is_a_validation_error(self, client):
        http, _ = client
        assert http.post("/epub-cover/1", json={"url": ""}).status_code == 422
        assert http.post("/epub-cover/1", json={"url": "x" * 5000}).status_code == 422

    def test_requires_admin(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        response = TestClient(main.app).post("/epub-cover/1", json={"url": URL})
        assert response.status_code in (401, 403)

    def test_comics_router_exposes_the_same_endpoint(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        response = TestClient(main.app).post("/comics/epub-cover/1", json={"url": URL})
        assert response.status_code in (401, 403)

    def test_delete_success(self, client):
        http, fake = client
        response = http.delete("/epub-cover/1")
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": {"file_size": 9}}
        fake.remove_epub_cover.assert_awaited_once_with(1)

    def test_delete_failure_is_reported_in_the_body(self, client):
        http, fake = client
        fake.remove_epub_cover.return_value = (None, "이 EPUB의 표지는 이 도구가 넣은 것이 아니라 삭제할 수 없습니다")
        body = http.delete("/epub-cover/1").json()
        assert body == {"status": "failure", "error": "이 EPUB의 표지는 이 도구가 넣은 것이 아니라 삭제할 수 없습니다"}

    def test_delete_requires_admin_on_books_and_comics(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        http = TestClient(main.app)
        assert http.delete("/epub-cover/1").status_code in (401, 403)
        assert http.delete("/comics/epub-cover/1").status_code in (401, 403)


# ── 올린 이미지(image_data)로 표지 넣기 ──


def b64_of(color="red"):
    import base64

    return base64.b64encode(image_bytes_of(color)).decode()


@pytest.mark.asyncio
async def test_adds_a_cover_from_an_uploaded_image_without_fetching(env, monkeypatch):
    manager, path, _ = env

    def fail(url):
        raise AssertionError("올린 이미지는 URL 을 내려받지 않는다")

    monkeypatch.setattr("backend.cover_management.fetch_url_image", fail)

    result, error = await BookManager.set_epub_cover(manager, 1, image_data=b64_of())

    assert error is None
    assert inspect_epub(path)["has_cover"]
    assert result["file_size"] == path.stat().st_size
    manager.es_manager.update.assert_called_once()


@pytest.mark.asyncio
async def test_uploaded_cover_replaces_an_existing_one_and_keeps_the_original_in_0_trash(env, tmp_path):
    manager, path, _ = env
    make_epub(path, cover=True)
    original = path.read_bytes()

    _, error = await BookManager.set_epub_cover(manager, 1, image_data=b64_of("blue"))

    assert error is None
    assert path.read_bytes() != original
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original
    assert not (tmp_path / ".cover_backups").exists()


@pytest.mark.asyncio
@pytest.mark.parametrize("data", ["***not-base64***", "bm90IGFuIGltYWdl"])  # 두 번째는 base64 이지만 이미지가 아니다
async def test_unreadable_uploaded_cover_is_rejected_without_touching_the_epub(env, tmp_path, data):
    manager, path, _ = env
    original = path.read_bytes()

    result, error = await BookManager.set_epub_cover(manager, 1, image_data=data)

    assert result is None and error
    assert path.read_bytes() == original
    assert not (tmp_path / "0_trash").exists()
    manager.es_manager.update.assert_not_called()


@pytest.mark.asyncio
async def test_without_url_or_image_data_nothing_is_changed(env):
    manager, path, _ = env
    original = path.read_bytes()

    result, error = await BookManager.set_epub_cover(manager, 1)

    assert result is None and "비어" in error
    assert path.read_bytes() == original


class TestEpubCoverUploadEndpoint:
    @pytest.fixture
    def client(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        fake = MagicMock()
        fake.set_epub_cover = AsyncMock(return_value=({"file_size": 10}, None))
        proxy = main.book_manager
        previous = proxy._instance
        proxy._instance = fake
        main.app.dependency_overrides[main.require_admin] = lambda: {"role": "admin"}
        try:
            yield TestClient(main.app), fake
        finally:
            main.app.dependency_overrides.pop(main.require_admin, None)
            proxy._instance = previous

    def test_image_data_is_passed_to_the_manager(self, client):
        http, fake = client
        response = http.post("/epub-cover/1", json={"image_data": "QUJD"})
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": {"file_size": 10}}
        fake.set_epub_cover.assert_awaited_once_with(1, None, "QUJD")

    @pytest.mark.parametrize("body", [{}, {"url": URL, "image_data": "QUJD"}, {"image_data": ""}])
    def test_requires_exactly_one_source(self, client, body):
        http, fake = client
        assert http.post("/epub-cover/1", json=body).status_code == 422
        fake.set_epub_cover.assert_not_awaited()
