"""EPUB 쪽 이미지의 크롭·분할·삭제·삽입: epub_cover.edit_images, BookManager.edit_epub_images, POST /epub-images/{id}."""

import base64
import io
import zipfile
from types import SimpleNamespace
from unittest.mock import AsyncMock, MagicMock

import pytest
from lxml import etree
from PIL import Image

from backend.book_manager import BookManager
from backend.epub_cover import CoverError, OPF_NS, edit_images, revision

FULL = (0, 0, 1, 1, 0)


def jpeg(color, size=(200, 100)):
    output = io.BytesIO()
    Image.new("RGB", size, color).save(output, "JPEG")
    return output.getvalue()


def png(color, size=(200, 100)):
    output = io.BytesIO()
    Image.new("RGB", size, color).save(output, "PNG")
    return output.getvalue()


def gif():
    output = io.BytesIO()
    Image.new("P", (20, 20)).save(output, "GIF")
    return output.getvalue()


def page(*images, text=""):
    body = "".join(f'<div><img src="{src}"/></div>' for src in images)
    return f'<html xmlns="http://www.w3.org/1999/xhtml"><head><title>t</title></head><body>{text}{body}</body></html>'


def make_book(path, extra=None):
    """p1·p2·p3 는 이미지만 있는 쪽, text 는 글만 있는 쪽, two 는 이미지 둘, nav 는 p3 를 가리키는 목차."""
    members = {
        "mimetype": "application/epub+zip",
        "META-INF/container.xml": '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>',
        "OPS/book.opf": f'''<package xmlns="{OPF_NS}" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>책</dc:title></metadata><manifest>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/><item id="p2" href="p2.xhtml" media-type="application/xhtml+xml"/>
<item id="p3" href="p3.xhtml" media-type="application/xhtml+xml"/><item id="text" href="text.xhtml" media-type="application/xhtml+xml"/>
<item id="two" href="two.xhtml" media-type="application/xhtml+xml"/><item id="shared" href="shared.xhtml" media-type="application/xhtml+xml"/>
<item id="gif" href="gif.xhtml" media-type="application/xhtml+xml"/><item id="mixed" href="mixed.xhtml" media-type="application/xhtml+xml"/>
<item id="i1" href="img/1.jpg" media-type="image/jpeg"/><item id="i2" href="img/2.png" media-type="image/png"/>
<item id="i3" href="img/3.jpg" media-type="image/jpeg"/><item id="i4" href="img/4.jpg" media-type="image/jpeg"/>
<item id="ig" href="img/g.gif" media-type="image/gif"/></manifest>
<spine><itemref idref="p1"/><itemref idref="p2"/><itemref idref="p3"/><itemref idref="text"/><itemref idref="two"/><itemref idref="shared"/><itemref idref="gif"/><itemref idref="mixed"/></spine></package>''',
        "OPS/nav.xhtml": '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="p3.xhtml">3</a></li></ol></nav></body></html>',
        "OPS/p1.xhtml": page("img/1.jpg"),
        "OPS/p2.xhtml": page("img/2.png"),
        "OPS/p3.xhtml": page("img/3.jpg"),
        "OPS/text.xhtml": page(text="본문 글"),
        "OPS/two.xhtml": page("img/4.jpg", "img/5.jpg"),
        "OPS/shared.xhtml": page("img/4.jpg"),
        "OPS/gif.xhtml": page("img/g.gif"),
        "OPS/mixed.xhtml": page("img/5.jpg", text="설명 글"),
        "OPS/img/1.jpg": jpeg("red"),
        "OPS/img/2.png": png("green"),
        "OPS/img/3.jpg": jpeg("blue"),
        "OPS/img/4.jpg": jpeg("yellow"),
        "OPS/img/5.jpg": jpeg("pink"),
        "OPS/img/g.gif": gif(),
    }
    members.update(extra or {})
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", members.pop("mimetype"), compress_type=zipfile.ZIP_STORED)
        for name, data in members.items():
            archive.writestr(name, data)
    return path


def read(path):
    with zipfile.ZipFile(path) as archive:
        return {name: archive.read(name) for name in archive.namelist()}


def spine(path):
    package = etree.fromstring(read(path)["OPS/book.opf"])
    return [ref.get("idref") for ref in package.iter(f"{{{OPF_NS}}}itemref")]


def manifest_ids(path):
    package = etree.fromstring(read(path)["OPS/book.opf"])
    return {item.get("id") for item in package.iter(f"{{{OPF_NS}}}item")}


def size(data):
    with Image.open(io.BytesIO(data)) as image:
        return image.size


@pytest.fixture
def book(tmp_path):
    path = make_book(tmp_path / "책" / "book.epub")
    return path, tmp_path / "0_trash"


def edit(book, **kwargs):
    path, trash = book
    args = {"crops": [], "splits": [], "deletes": [], "inserts": []}
    args.update(kwargs)
    return edit_images(path, revision(path), trash, backup_name="book.epub", **args)


def test_crop_replaces_the_image_in_place_keeping_the_format(book):
    path, trash = book
    before = read(path)
    result = edit(book, crops=[{"idref": "p1", "rect": (0.25, 0, 0.75, 1, 0)}, {"idref": "p2", "rect": (0, 0, 0.5, 0.5, 0)}])
    after = read(path)
    assert size(after["OPS/img/1.jpg"]) == (100, 100)
    assert size(after["OPS/img/2.png"]) == (100, 50)
    assert after["OPS/img/2.png"][:8] == b"\x89PNG\r\n\x1a\n"
    assert after["OPS/p1.xhtml"] == before["OPS/p1.xhtml"]
    assert (trash / "book.epub").exists()
    assert result["cover_image"] is None and result["idrefs"] == {"inserted": [], "split": {}}


def test_tilted_crop_straightens_the_image(book):
    path, _ = book
    edit(book, crops=[{"idref": "p3", "rect": (0.3, 0.3, 0.7, 0.7, 3)}])
    assert size(read(path)["OPS/img/3.jpg"]) == (80, 40)


@pytest.mark.parametrize("idref,reason", [
    ("text", "이미지가 0개"),
    ("two", "공유"),
    ("p1:5", "이미지를 찾을 수 없음"),
    ("p1:-1", "위치"),
    ("gif", "지원하지 않는 이미지 형식"),
    ("nope", "찾을 수 없습니다"),
])
def test_crop_refusals_leave_the_book_untouched(book, idref, reason):
    path, _ = book
    original = path.read_bytes()
    idref, _, index = idref.partition(":")
    with pytest.raises(CoverError, match=reason):
        edit(book, crops=[{"idref": idref, "index": int(index or 0), "rect": FULL}])
    assert path.read_bytes() == original


def test_crop_refuses_an_image_shared_with_another_page(book):
    path, _ = book
    with pytest.raises(CoverError, match="공유"):
        edit(book, crops=[{"idref": "shared", "rect": FULL}])


def test_crop_region_errors_are_reported_in_korean(book):
    with pytest.raises(CoverError, match="크롭할 수 없습니다 \\(크롭 영역이 너무 작음\\)"):
        edit(book, crops=[{"idref": "p1", "rect": (0, 0, 0.01, 1, 0)}])


def test_split_replaces_the_original_and_adds_a_page_right_after(book):
    path, _ = book
    result = edit(book, splits=[{"idref": "p1", "first": (0, 0, 0.5, 1, 0), "second": (0.5, 0, 1, 1, 0)}])
    new_id = result["idrefs"]["split"]["p1"]
    assert new_id.startswith("tm-split-page-")
    order = spine(path)
    assert order[:3] == ["p1", new_id, "p2"]
    after = read(path)
    assert size(after["OPS/img/1.jpg"]) == (100, 100)
    page_member = f"OPS/{new_id}.xhtml"
    image_member = "OPS/" + new_id.replace("-page", "") + ".jpg"
    assert page_member in after and size(after[image_member]) == (100, 100)
    assert new_id in manifest_ids(path)


def test_split_in_a_page_with_text_happens_inside_the_page(tmp_path):
    path = make_book(tmp_path / "t" / "book.epub", {"OPS/p1.xhtml": page("img/1.jpg", text="<p>설명 글</p>")})
    result = edit((path, tmp_path / "0_trash"), splits=[{"idref": "p1", "first": (0, 0, 0.5, 1, 0), "second": (0.5, 0, 1, 1, 0)}])
    assert result["idrefs"]["split"] == {} and spine(path)[:2] == ["p1", "p2"]
    text = read(path)["OPS/p1.xhtml"].decode()
    assert text.count("<img") == 2 and "설명 글" in text


def test_split_refuses_an_image_shared_with_another_page(book):
    with pytest.raises(CoverError, match="공유"):
        edit(book, splits=[{"idref": "mixed", "first": FULL, "second": FULL}])


def test_delete_removes_the_page_and_its_unused_image(book):
    path, _ = book
    edit(book, deletes=[{"idref": "p2"}])
    after = read(path)
    assert "p2" not in spine(path) and "p2" not in manifest_ids(path) and "i2" not in manifest_ids(path)
    assert "OPS/p2.xhtml" not in after and "OPS/img/2.png" not in after


def test_delete_keeps_an_image_other_pages_still_use(book):
    path, _ = book
    edit(book, deletes=[{"idref": "shared", "index": 0}])
    assert "OPS/img/4.jpg" in read(path)


def test_delete_refuses_pages_the_toc_points_to_and_edits_text_pages_in_place(book):
    with pytest.raises(CoverError, match="목차가 가리키는"):
        edit(book, deletes=["p3"])
    # 글이 함께 있는 쪽은 쪽 안에서 이미지만 지운다(5.jpg 는 two 쪽도 쓰므로 파일은 남는다).
    edit(book, deletes=[{"idref": "mixed"}])
    after = read(book[0])
    assert "<img" not in after["OPS/mixed.xhtml"].decode() and "설명 글" in after["OPS/mixed.xhtml"].decode()
    assert "OPS/img/5.jpg" in after


def test_delete_refuses_the_tool_cover_page(tmp_path):
    path = make_book(tmp_path / "b.epub")
    members = read(path)
    members["OPS/book.opf"] = members["OPS/book.opf"].replace(b'<spine>', b'<spine><itemref idref="tm-cover-page-x"/>').replace(b'</manifest>', b'<item id="tm-cover-page-x" href="p1.xhtml" media-type="application/xhtml+xml"/></manifest>')
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", members.pop("mimetype"), compress_type=zipfile.ZIP_STORED)
        for name, data in members.items():
            archive.writestr(name, data)
    with pytest.raises(CoverError, match="표지"):
        edit((path, tmp_path / "0_trash"), deletes=["tm-cover-page-x"])


def test_delete_refuses_to_empty_the_book(tmp_path):
    path = make_book(tmp_path / "b.epub")
    members = read(path)
    members["OPS/book.opf"] = members["OPS/book.opf"].replace(b'<spine>', b'<spine><itemref idref="x"/>', 0)
    only = b'<spine><itemref idref="p1"/></spine>'
    start = members["OPS/book.opf"].index(b"<spine>")
    members["OPS/book.opf"] = members["OPS/book.opf"][:start] + only + b"</package>"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", members.pop("mimetype"), compress_type=zipfile.ZIP_STORED)
        for name, data in members.items():
            archive.writestr(name, data)
    with pytest.raises(CoverError, match="모든 쪽"):
        edit((path, tmp_path / "0_trash"), deletes=["p1"])


def test_multiple_inserts_keep_their_order_before_the_same_page(book):
    path, _ = book
    result = edit(book, inserts=[("p2", jpeg("red")), ("p2", jpeg("blue")), ("p1", jpeg("green"))])
    first, second, third = result["idrefs"]["inserted"]
    order = spine(path)
    assert order[:5] == [third, "p1", first, second, "p2"]


def test_insert_may_target_a_page_deleted_in_the_same_request(book):
    path, _ = book
    result = edit(book, inserts=[("p2", jpeg("red"))], deletes=["p2"])
    inserted = result["idrefs"]["inserted"][0]
    assert spine(path)[:2] == ["p1", inserted] and "p2" not in spine(path)


def test_insert_refuses_unknown_positions(book):
    with pytest.raises(CoverError, match="위치"):
        edit(book, inserts=[("nope", jpeg("red"))])


def test_combined_request_is_applied_atomically(book):
    path, _ = book
    original = path.read_bytes()
    with pytest.raises(CoverError):
        edit(book, crops=[{"idref": "p1", "rect": (0, 0, 0.5, 1, 0)}], deletes=["p3"])
    assert path.read_bytes() == original
    result = edit(book, crops=[{"idref": "p1", "rect": (0, 0, 0.5, 1, 0)}], splits=[{"idref": "p2", "first": (0, 0, 0.5, 1, 0), "second": (0.5, 0, 1, 1, 0)}], deletes=["shared"], inserts=[("text", jpeg("red"))])
    assert size(read(path)["OPS/img/1.jpg"]) == (100, 100)
    assert len(result["idrefs"]["inserted"]) == 1 and "p2" in result["idrefs"]["split"]
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None


def test_empty_and_duplicate_requests_are_rejected(book):
    with pytest.raises(CoverError, match="편집이 없습니다"):
        edit(book)
    with pytest.raises(CoverError, match="두 번"):
        edit(book, crops=[{"idref": "p1", "rect": FULL}], deletes=["p1"])


def test_stale_revision_is_rejected_and_backup_is_kept_once(book):
    path, trash = book
    with pytest.raises(CoverError, match="변경"):
        edit_images(path, "stale", trash, crops=[{"idref": "p1", "rect": FULL}], splits=[], deletes=[], inserts=[])
    original = path.read_bytes()
    edit(book, crops=[{"idref": "p1", "rect": (0, 0, 0.5, 1, 0)}])
    edit(book, crops=[{"idref": "p1", "rect": (0, 0, 0.5, 1, 0)}])
    assert (trash / "book.epub").read_bytes() == original


@pytest.fixture
def env(tmp_path, monkeypatch):
    monkeypatch.setattr("backend.cover_management.fetch_url_image", lambda url: jpeg("red"))
    path = make_book(tmp_path / "분류" / "book.epub")
    es = MagicMock()
    es.update.return_value = True
    manager = SimpleNamespace(path_prefix=tmp_path, TRASH_DIRNAME="0_trash", es_manager=es, get_book=AsyncMock(return_value=(SimpleNamespace(file_path=path, file_type="epub"), None)))
    manager._change_epub_cover = lambda *args, **kwargs: BookManager._change_epub_cover(manager, *args, **kwargs)
    return manager, path, tmp_path


def rect(x0, x1):
    return {"x0": x0, "y0": 0, "x1": x1, "y1": 1, "angle": 0}


@pytest.mark.asyncio
async def test_manager_applies_a_combined_edit_and_keeps_the_original_in_trash(env):
    manager, path, tmp_path = env
    original = path.read_bytes()
    data = base64.b64encode(jpeg("blue")).decode()
    result, error = await BookManager.edit_epub_images(
        manager, 1,
        [{"idref": "p1", **rect(0, 0.5)}],
        [{"idref": "p2", "first": rect(0, 0.5), "second": rect(0.5, 1)}],
        [{"idref": "shared"}],
        [{"before_idref": "text", "url": "https://img.example.com/a.jpg"}, {"before_idref": "text", "image_data": data}],
    )
    assert error is None
    assert len(result["idrefs"]["inserted"]) == 2 and "p2" in result["idrefs"]["split"]
    assert (tmp_path / "0_trash" / "book.epub").read_bytes() == original
    manager.es_manager.update.assert_called_once()


@pytest.mark.asyncio
async def test_manager_reports_errors_as_messages(env):
    manager, path, _ = env
    original = path.read_bytes()
    _, error = await BookManager.edit_epub_images(manager, 1, [{"idref": "text", **rect(0, 1)}], [], [], [])
    assert "크롭할 수 없습니다" in error
    _, error = await BookManager.edit_epub_images(manager, 1, [], [], [], [{"before_idref": "p1", "image_data": "!!!"}])
    assert "읽을 수 없습니다" in error
    assert path.read_bytes() == original


class TestRoute:
    @pytest.fixture
    def client(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        require_admin = main.require_admin
        fake = MagicMock()
        fake.edit_epub_images = AsyncMock(return_value=({"idrefs": {"inserted": [], "split": {}}}, None))
        proxy = main.book_manager
        previous = proxy._instance
        proxy._instance = fake
        main.app.dependency_overrides[require_admin] = lambda: {"role": "admin"}
        try:
            yield TestClient(main.app), fake
        finally:
            main.app.dependency_overrides.pop(require_admin, None)
            proxy._instance = previous

    def test_success_passes_the_agreed_body_to_the_manager(self, client):
        http, fake = client
        body = {
            "crops": [{"idref": "p1", "index": 1, "x0": 0.1, "y0": 0.1, "x1": 0.9, "y1": 0.9, "angle": 2}],
            "splits": [{"idref": "p2", "first": rect(0, 0.5), "second": rect(0.5, 1)}],
            "deletes": [{"idref": "p3"}],
            "inserts": [{"before_idref": "p1", "url": "https://img.example.com/a.jpg"}],
        }
        response = http.post("/epub-images/1", json=body)
        assert response.status_code == 200
        assert response.json() == {"status": "success", "result": {"idrefs": {"inserted": [], "split": {}}}}
        args = fake.edit_epub_images.await_args.args
        assert args[0] == 1 and args[1][0]["idref"] == "p1" and args[1][0]["index"] == 1 and args[1][0]["angle"] == 2
        assert args[2][0]["first"]["x1"] == 0.5 and args[3] == [{"idref": "p3", "index": 0}] and args[4][0]["before_idref"] == "p1"

    def test_failure_is_reported_in_the_body(self, client):
        http, fake = client
        fake.edit_epub_images.return_value = (None, "크롭할 수 없습니다 (이미지가 0개인 쪽)")
        assert http.post("/epub-images/1", json={"deletes": [{"idref": "x"}]}).json() == {"status": "failure", "error": "크롭할 수 없습니다 (이미지가 0개인 쪽)"}

    def test_invalid_bodies_are_validation_errors(self, client):
        http, _ = client
        assert http.post("/epub-images/1", json={"deletes": [{"idref": ""}]}).status_code == 422
        assert http.post("/epub-images/1", json={"deletes": [{"idref": "p1", "index": -1}]}).status_code == 422
        assert http.post("/epub-images/1", json={"inserts": [{"before_idref": "p1"}]}).status_code == 422

    def test_requires_admin(self):
        from fastapi.testclient import TestClient

        import backend.main as main

        assert TestClient(main.app).post("/epub-images/1", json={}).status_code in (401, 403)


# ── <svg><image> 로 이미지를 넣은 쪽 ──────────────────────────────────────────
def _svg_epub(path, size=(300, 400), title_text="표지", cover=False):
    import io
    import zipfile

    from PIL import Image

    buf = io.BytesIO()
    Image.new("RGB", size, (200, 30, 30)).save(buf, "JPEG")
    page = (
        '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:xlink="http://www.w3.org/1999/xlink">'
        f'<head><title>c</title></head><body><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size[0]} {size[1]}" width="100%" height="100%">'
        f'<title>{title_text}</title><image width="{size[0]}" height="{size[1]}" xlink:href="c.jpg"/></svg></body></html>'
    )
    opf = (
        '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">'
        '<dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title><dc:language>ko</dc:language></metadata><manifest>'
        '<item id="c" href="c.xhtml" media-type="application/xhtml+xml"/><item id="ci" href="c.jpg" media-type="image/jpeg" __COVER__/>'
        '<item id="d" href="d.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c"/><itemref idref="d"/></spine></package>'
    )
    opf = opf.replace("__COVER__", 'properties="cover-image"' if cover else "")
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml", '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
        z.writestr("OEBPS/content.opf", opf)
        z.writestr("OEBPS/c.xhtml", page)
        z.writestr("OEBPS/c.jpg", buf.getvalue())
        z.writestr("OEBPS/d.xhtml", '<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>d</title></head><body><p>본문</p></body></html>')


def test_svg_image_page_can_be_cropped_and_viewbox_follows(tmp_path):
    import io
    import zipfile

    from PIL import Image

    from backend import epub_cover

    path = tmp_path / "svg.epub"
    _svg_epub(path)
    rect = (0.0, 0.0, 0.5, 0.5, 0.0)
    epub_cover.edit_images(path, epub_cover.revision(path), tmp_path / "trash", crops=[{"idref": "c", "rect": rect}], splits=[], deletes=[], inserts=[])
    with zipfile.ZipFile(path) as z:
        with Image.open(io.BytesIO(z.read("OEBPS/c.jpg"))) as image:
            assert image.size == (150, 200)
        page = z.read("OEBPS/c.xhtml").decode()
    assert 'viewBox="0 0 150 200"' in page
    assert 'width="150"' in page and 'height="200"' in page


def test_svg_image_page_counts_as_image_only_despite_svg_title(tmp_path):
    from backend import epub_cover

    path = tmp_path / "svg.epub"
    _svg_epub(path)
    result = epub_cover.edit_images(path, epub_cover.revision(path), tmp_path / "trash", crops=[], splits=[], deletes=[{"idref": "c"}] and ["c"], inserts=[])
    assert result["revision"]


def test_cover_target_crops_the_cover_image_and_fits_its_svg_page(tmp_path):
    import io
    import zipfile

    from PIL import Image

    from backend import epub_cover

    path = tmp_path / "cover.epub"
    _svg_epub(path, cover=True)
    epub_cover.edit_images(path, epub_cover.revision(path), tmp_path / "trash", crops=[{"idref": "@cover", "rect": (0.0, 0.0, 0.5, 1.0, 0.0)}], splits=[], deletes=[], inserts=[])
    with zipfile.ZipFile(path) as z:
        with Image.open(io.BytesIO(z.read("OEBPS/c.jpg"))) as image:
            assert image.size == (150, 400)
        assert 'viewBox="0 0 150 400"' in z.read("OEBPS/c.xhtml").decode()


def test_cover_target_without_a_cover_is_refused(tmp_path):
    import pytest

    from backend import epub_cover

    path = tmp_path / "nocover.epub"
    _svg_epub(path)
    with pytest.raises(epub_cover.CoverError, match="표지 이미지를 찾을 수 없음"):
        epub_cover.edit_images(path, epub_cover.revision(path), tmp_path / "trash", crops=[{"idref": "@cover", "rect": (0.0, 0.0, 0.5, 1.0, 0.0)}], splits=[], deletes=[], inserts=[])


# ── 한 쪽에 이미지가 여러 장인 책(실제 책처럼 xmlns 없는 비정형 HTML) ─────────────────────
MULTI_PAGE = (
    '<html>\n  <head>\n    <title>엄마의 말뚝</title>\n  </head>\n    <body><li class="chapter">\n'
    '<p><img alt="image" height="597" src="../Images/m1.jpg" width="412"/></p>\n'
    '<p><img alt="image" height="597" src="../Images/m2.jpg" width="412"/></p>\n'
    '</li></body></html>'
)


def make_multi(path, page=MULTI_PAGE):
    opf = (
        f'<package xmlns="{OPF_NS}" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>t</dc:title></metadata>'
        '<manifest><item id="multi" href="Text/multi.xhtml" media-type="application/xhtml+xml"/><item id="t" href="Text/t.xhtml" media-type="application/xhtml+xml"/>'
        '<item id="m1" href="Images/m1.jpg" media-type="image/jpeg"/><item id="m2" href="Images/m2.jpg" media-type="image/jpeg"/></manifest>'
        '<spine><itemref idref="multi"/><itemref idref="t"/></spine></package>'
    )
    members = {
        "META-INF/container.xml": '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>',
        "OEBPS/content.opf": opf,
        "OEBPS/Text/multi.xhtml": page,
        "OEBPS/Text/t.xhtml": "<html><body><p>다른 쪽</p></body></html>",
        "OEBPS/Images/m1.jpg": jpeg("red"),
        "OEBPS/Images/m2.jpg": jpeg("green"),
    }
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        for name, data in members.items():
            archive.writestr(name, data)
    return path


@pytest.fixture
def multi(tmp_path):
    return make_multi(tmp_path / "multi.epub"), tmp_path / "0_trash"


def verify(path):
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None
        from backend.epub_cover import _read_package

        _read_package(archive)


@pytest.mark.parametrize("index,member", [(0, "OEBPS/Images/m1.jpg"), (1, "OEBPS/Images/m2.jpg")])
def test_multi_image_page_crops_each_image_separately(multi, index, member):
    path, _ = multi
    before = read(path)
    edit(multi, crops=[{"idref": "multi", "index": index, "rect": (0, 0, 0.5, 1, 0)}])
    after = read(path)
    assert size(after[member]) == (100, 100)
    other = "OEBPS/Images/m2.jpg" if index == 0 else "OEBPS/Images/m1.jpg"
    assert after[other] == before[other] and after["OEBPS/Text/multi.xhtml"] == before["OEBPS/Text/multi.xhtml"]
    verify(path)


def test_multi_image_page_split_adds_an_image_inside_the_page(multi):
    path, _ = multi
    before = read(path)
    result = edit(multi, splits=[{"idref": "multi", "index": 1, "first": (0, 0, 0.5, 1, 0), "second": (0.5, 0, 1, 1, 0)}])
    assert result["idrefs"]["split"] == {}
    after = read(path)
    page_text = after["OEBPS/Text/multi.xhtml"].decode()
    assert page_text.count("<img") == 3
    new_names = [name for name in after if "tm-split-" in name]
    assert len(new_names) == 1 and size(after[new_names[0]]) == (100, 100) and size(after["OEBPS/Images/m2.jpg"]) == (100, 100)
    # 새 이미지는 opf 와 같은 디렉터리(OEBPS)에 두고, 쪽(OEBPS/Text)에서는 상대경로로 가리킨다.
    assert new_names[0].startswith("OEBPS/tm-split-") and f'src="../{new_names[0].rsplit("/", 1)[1]}"' in page_text
    assert any(item.startswith("tm-split-") for item in manifest_ids_of(path, "OEBPS/content.opf"))
    # 이미지 태그 하나가 늘었을 뿐 나머지 문서·파일은 그대로다.
    assert page_text.replace(page_text[page_text.index("<br/>"):page_text.index("<br/>") + 5], "").count("<p>") == 2
    assert after["OEBPS/Images/m1.jpg"] == before["OEBPS/Images/m1.jpg"] and after["OEBPS/Text/t.xhtml"] == before["OEBPS/Text/t.xhtml"]
    assert page_text.startswith("<html>\n  <head>") and "<li class=\"chapter\">" in page_text
    verify(path)


def manifest_ids_of(path, opf):
    package = etree.fromstring(read(path)[opf])
    return {item.get("id") for item in package.iter(f"{{{OPF_NS}}}item")}


def test_multi_image_page_delete_removes_the_wrapper_and_the_unused_image(multi):
    path, _ = multi
    edit(multi, deletes=[{"idref": "multi", "index": 0}])
    after = read(path)
    page_text = after["OEBPS/Text/multi.xhtml"].decode()
    assert page_text.count("<img") == 1 and "m1.jpg" not in page_text and page_text.count("<p>") == 1
    assert "OEBPS/Images/m1.jpg" not in after and "m1" not in manifest_ids_of(path, "OEBPS/content.opf")
    assert "OEBPS/Images/m2.jpg" in after
    verify(path)


def test_multi_image_page_mixed_edits_apply_without_shifting_indexes(multi):
    path, _ = multi
    edit(multi, crops=[{"idref": "multi", "index": 0, "rect": (0, 0, 0.5, 1, 0)}], splits=[{"idref": "multi", "index": 1, "first": (0, 0, 0.5, 1, 0), "second": (0.5, 0, 1, 1, 0)}])
    after = read(path)
    assert size(after["OEBPS/Images/m1.jpg"]) == (100, 100) and size(after["OEBPS/Images/m2.jpg"]) == (100, 100)
    assert after["OEBPS/Text/multi.xhtml"].decode().count("<img") == 3
    verify(path)


def test_split_then_delete_on_the_same_page_use_original_indexes(multi):
    path, _ = multi
    edit(multi, splits=[{"idref": "multi", "index": 0, "first": (0, 0, 0.5, 1, 0), "second": (0.5, 0, 1, 1, 0)}], deletes=[{"idref": "multi", "index": 1}])
    page_text = read(path)["OEBPS/Text/multi.xhtml"].decode()
    assert page_text.count("<img") == 2 and "m2.jpg" not in page_text and "m1.jpg" in page_text
    verify(path)


def test_same_image_twice_is_refused(multi):
    path, _ = multi
    original = path.read_bytes()
    with pytest.raises(CoverError, match="두 번"):
        edit(multi, crops=[{"idref": "multi", "index": 1, "rect": FULL}], deletes=[{"idref": "multi", "index": 1}])
    with pytest.raises(CoverError, match="두 번"):
        edit(multi, crops=[{"idref": "multi", "rect": FULL}, {"idref": "multi", "index": 0, "rect": FULL}])
    assert path.read_bytes() == original


def test_index_out_of_range_is_refused(multi):
    path, _ = multi
    original = path.read_bytes()
    for kind in ("crops", "splits", "deletes"):
        entry = {"idref": "multi", "index": 2}
        if kind == "crops":
            entry["rect"] = FULL
        elif kind == "splits":
            entry.update(first=FULL, second=FULL)
        with pytest.raises(CoverError, match="이미지를 찾을 수 없음"):
            edit(multi, **{kind: [entry]})
    assert path.read_bytes() == original


def test_crop_refuses_when_two_tags_in_the_page_use_the_same_image(tmp_path):
    page = MULTI_PAGE.replace("m2.jpg", "m1.jpg")
    book = (make_multi(tmp_path / "dup.epub", page), tmp_path / "0_trash")
    with pytest.raises(CoverError, match="공유"):
        edit(book, crops=[{"idref": "multi", "index": 0, "rect": FULL}])


def test_mismatched_tag_count_is_refused():
    from backend.epub_cover import _edit_page_images

    with pytest.raises(CoverError, match="쪽 구조를 해석할 수 없습니다"):
        _edit_page_images(b"<html><body><img src='a.jpg'/></body></html>", 2, [(0, "delete", None)])


def test_comment_images_are_not_counted(tmp_path):
    page = MULTI_PAGE.replace("<li", '<!-- <img src="../Images/m1.jpg"/> --><li')
    book = (make_multi(tmp_path / "comment.epub", page), tmp_path / "0_trash")
    edit(book, deletes=[{"idref": "multi", "index": 1}])
    text = read(book[0])["OEBPS/Text/multi.xhtml"].decode()
    assert text.count("<img") == 2 and "m2.jpg" not in text.replace("<!-- <img", "")


def test_undecodable_page_is_refused(tmp_path):
    page = MULTI_PAGE.encode() + b"\xff\xfe"
    book = (make_multi(tmp_path / "bad.epub", page), tmp_path / "0_trash")
    with pytest.raises(CoverError):
        edit(book, deletes=[{"idref": "multi", "index": 0}])
