#!/usr/bin/env python
"""EPUB 미리보기(preview) 및 다운로드(download) 엔드포인트 통합 테스트.

테스트용 미니 EPUB을 직접 생성하여 실제 EPUB 파일이 없는 환경에서도 동작한다.
test_backend.py의 backend_test_setup / test_book fixture 패턴을 기반으로 한다.
"""

import io
import logging.config
import zipfile
import xml.etree.ElementTree as ET
from datetime import datetime
from pathlib import Path

import pytest

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)
logging.getLogger("elasticsearch").setLevel(logging.CRITICAL)

CATEGORY = "_epub"


# ── EPUB 생성 헬퍼 ────────────────────────────────────────

CONTAINER_XML = """\
<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"""


def _make_opf(chapter_count: int, include_css: bool = False, include_svg_cover: bool = False) -> str:
    """챕터 N개를 포함한 OPF XML을 생성."""
    items = []
    spine_refs = []
    for i in range(1, chapter_count + 1):
        items.append(f'    <item id="ch{i}" href="ch{i}.xhtml" media-type="application/xhtml+xml"/>')
        spine_refs.append(f'    <itemref idref="ch{i}"/>')

    if include_css:
        items.append('    <item id="style" href="style.css" media-type="text/css"/>')

    if include_svg_cover:
        items.append('    <item id="cover-image" href="../media/cover.jpg" media-type="image/jpeg"/>')

    return f"""\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test Book</dc:title>
    <dc:creator>Test Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
{chr(10).join(items)}
  </manifest>
  <spine toc="toc">
{chr(10).join(spine_refs)}
  </spine>
</package>"""


def _make_cover_xhtml_with_svg() -> str:
    """SVG <image> 태그로 커버 이미지를 참조하는 커버 페이지."""
    return """\
<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
<head><title>Cover</title></head>
<body>
<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     width="100%" height="100%" viewBox="0 0 600 800">
  <image xlink:href="../media/cover.jpg" width="600" height="800"/>
</svg>
</body>
</html>"""


def _make_chapter_xhtml(index: int, link_css: bool = False) -> str:
    css_link = '<link rel="stylesheet" href="style.css"/>' if link_css else ''
    return f"""\
<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter {index}</title>{css_link}</head>
<body><h1>Chapter {index}</h1><p>Content of chapter {index}.</p></body>
</html>"""


def _create_test_epub(path: Path, chapter_count: int = 3, include_css: bool = False,
                      include_svg_cover: bool = False) -> None:
    """테스트용 미니 EPUB을 생성."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', _make_opf(chapter_count, include_css, include_svg_cover))
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        for i in range(1, chapter_count + 1):
            if include_svg_cover and i == 1:
                zf.writestr(f'OEBPS/ch{i}.xhtml', _make_cover_xhtml_with_svg())
            else:
                zf.writestr(f'OEBPS/ch{i}.xhtml', _make_chapter_xhtml(i, link_css=include_css))
        if include_css:
            zf.writestr('OEBPS/style.css', 'body { margin: 1em; font-family: serif; }')
        if include_svg_cover:
            # 1x1 JPEG placeholder
            zf.writestr('media/cover.jpg', b'\xff\xd8\xff\xe0\x00\x10JFIF\xff\xd9')


def _create_epub_with_font(path: Path, font_size: int) -> None:
    """CSS url()로 폰트를 참조하는 EPUB. font_size로 폰트 크기를 제어."""
    opf = """\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Font Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="style" href="style.css" media-type="text/css"/>
    <item id="testfont" href="fonts/test.ttf" media-type="font/ttf"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
  </spine>
</package>"""

    chapter = """\
<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Ch1</title><link rel="stylesheet" href="style.css"/></head>
<body><p>Font test chapter</p></body>
</html>"""

    css = "@font-face { font-family: 'Test'; src: url('fonts/test.ttf'); }\nbody { font-family: 'Test', serif; }"

    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', chapter)
        zf.writestr('OEBPS/style.css', css)
        zf.writestr('OEBPS/fonts/test.ttf', b'\x00' * font_size)


def _create_epub_with_invalid_spine(path: Path) -> None:
    """manifest에 없는 idref(coverpage)를 spine에 포함하는 EPUB."""
    opf = """\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Invalid Spine Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="coverpage"/>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"""

    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
        zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))


def _create_epub_with_unreferenced_css(path: Path) -> None:
    """챕터에서 참조하지 않는 CSS(대용량 폰트 url() 포함)를 가진 EPUB."""
    opf = """\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Unreferenced CSS Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="used-style" href="used.css" media-type="text/css"/>
    <item id="unused-style" href="unused.css" media-type="text/css"/>
    <item id="bigfont" href="fonts/big.ttf" media-type="font/ttf"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
  </spine>
</package>"""

    chapter = """\
<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Ch1</title><link rel="stylesheet" href="used.css"/></head>
<body><p>Content</p></body>
</html>"""

    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', chapter)
        zf.writestr('OEBPS/used.css', 'body { margin: 1em; }')
        zf.writestr('OEBPS/unused.css',
                     "@font-face { font-family: 'Big'; src: url('fonts/big.ttf'); }")
        zf.writestr('OEBPS/fonts/big.ttf', b'\x00' * (600 * 1024))


def _create_corrupted_epub(path: Path, chapter_count: int = 3, missing_all: bool = False) -> None:
    """spine에 챕터가 등록되어 있지만 실제 ZIP에는 파일이 없는 손상 EPUB.

    missing_all=True이면 모든 챕터 파일을 제거한다.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', _make_opf(chapter_count))
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        if not missing_all:
            # ch1만 누락, 나머지는 포함
            for i in range(2, chapter_count + 1):
                zf.writestr(f'OEBPS/ch{i}.xhtml', _make_chapter_xhtml(i))


# ── fixtures ──────────────────────────────────────────────

@pytest.fixture(scope="module")
def backend_test_setup(es_client, es_index):
    """BookManager + TestClient (ES 공유)."""
    from fastapi.testclient import TestClient
    from backend.main import app
    from backend.book_manager import BookManager

    bm = BookManager()
    bm.es_manager.es = es_client
    bm.es_manager.refresh()

    client = TestClient(app)
    yield {"bm": bm, "client": client}


def _build_epub_data(bm, epub_path: Path) -> dict:
    """EPUB 파일의 ES 등록용 데이터를 생성."""
    inode = epub_path.stat().st_ino
    rel_path = epub_path.relative_to(bm.path_prefix)
    now = datetime.now().strftime("%Y-%m-%dT%H:%M:%S.%f")

    return {
        inode: {
            "category": CATEGORY,
            "title": epub_path.stem,
            "author": "Test Author",
            "file_path": str(rel_path),
            "file_type": "epub",
            "file_size": epub_path.stat().st_size,
            "line_count": 0,
            "page_count": 0,
            "isbn": "",
            "summary": "test epub",
            "updated_time": now,
        }
    }


def _register_epub_sync(bm, epub_path: Path) -> int:
    """EPUB 파일을 ES에 등록하고 book_id를 반환 (동기)."""
    import asyncio

    data = _build_epub_data(bm, epub_path)
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    try:
        book_id, error = loop.run_until_complete(bm.add_book(data))
        assert book_id and not error, f"Failed to register epub: {error}"
        return book_id
    finally:
        loop.close()


async def _register_epub_async(bm, epub_path: Path) -> int:
    """EPUB 파일을 ES에 등록하고 book_id를 반환 (비동기)."""
    data = _build_epub_data(bm, epub_path)
    book_id, error = await bm.add_book(data)
    assert book_id and not error, f"Failed to register epub: {error}"
    return book_id


def _cleanup_book(client, bm, book_id, epub_path):
    """테스트 후 정리."""
    try:
        client.delete(f"/books/{book_id}")
    except Exception:
        pass
    cache_dir = bm.path_prefix / ".preview_cache"
    for p in cache_dir.glob(f"{book_id}*"):
        p.unlink(missing_ok=True)
    epub_path.unlink(missing_ok=True)


@pytest.fixture
def test_book(backend_test_setup):
    """정상 EPUB을 생성·등록하고 테스트 후 정리."""
    bm = backend_test_setup["bm"]
    client = backend_test_setup["client"]

    epub_dir = bm.path_prefix / CATEGORY
    epub_path = epub_dir / "[Test Author] Test Preview Book.epub"
    _create_test_epub(epub_path, chapter_count=3)

    book_id = _register_epub_sync(bm, epub_path)
    cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"

    yield {"book_id": book_id, "bm": bm, "client": client, "epub_path": epub_path}

    # cleanup
    try:
        client.delete(f"/books/{book_id}")
    except Exception:
        pass
    cache_file.unlink(missing_ok=True)
    epub_path.unlink(missing_ok=True)


# ── 응답 파싱 헬퍼 ────────────────────────────────────────

def _parse_epub_zip(content: bytes) -> zipfile.ZipFile:
    return zipfile.ZipFile(io.BytesIO(content), 'r')


def _get_spine_idrefs(zf: zipfile.ZipFile) -> list:
    cnt_ns = 'urn:oasis:names:tc:opendocument:xmlns:container'
    container = ET.fromstring(zf.read('META-INF/container.xml'))
    rootfile = container.find(f'.//{{{cnt_ns}}}rootfile')
    opf_path = rootfile.get('full-path', '')

    opf_ns = 'http://www.idpf.org/2007/opf'
    opf = ET.fromstring(zf.read(opf_path))
    spine_el = opf.find(f'.//{{{opf_ns}}}spine')
    return [ref.get('idref') for ref in spine_el.findall(f'{{{opf_ns}}}itemref')]


def _get_manifest_items(zf: zipfile.ZipFile) -> dict:
    """EPUB ZIP에서 manifest의 {id: {href, media-type}} 딕셔너리를 추출."""
    cnt_ns = 'urn:oasis:names:tc:opendocument:xmlns:container'
    container = ET.fromstring(zf.read('META-INF/container.xml'))
    rootfile = container.find(f'.//{{{cnt_ns}}}rootfile')
    opf_path = rootfile.get('full-path', '')

    opf_ns = 'http://www.idpf.org/2007/opf'
    opf = ET.fromstring(zf.read(opf_path))
    result = {}
    for item in opf.findall(f'.//{{{opf_ns}}}item'):
        result[item.get('id', '')] = {
            'href': item.get('href', ''),
            'media-type': item.get('media-type', ''),
        }
    return result


# ── tests: preview 엔드포인트 ─────────────────────────────

class TestBookPreview:

    @pytest.mark.asyncio
    async def test_preview_returns_valid_epub(self, test_book):
        """정상 EPUB → 200 + 유효한 ZIP (mimetype, container.xml 포함)."""
        client = test_book["client"]
        book_id = test_book["book_id"]

        response = client.get(f"/preview/{book_id}?chapters=3")
        assert response.status_code == 200
        assert "epub" in response.headers.get("content-type", "")

        zf = _parse_epub_zip(response.content)
        assert 'mimetype' in zf.namelist()
        assert zf.read('mimetype') == b'application/epub+zip'
        assert 'META-INF/container.xml' in zf.namelist()
        zf.close()

    @pytest.mark.asyncio
    async def test_preview_response_headers(self, test_book):
        """preview 응답에 Content-Encoding: identity와 Cache-Control: no-transform이 포함된다."""
        client = test_book["client"]
        book_id = test_book["book_id"]

        response = client.get(f"/preview/{book_id}?chapters=3")
        assert response.status_code == 200
        assert response.headers.get("content-encoding") == "identity"
        assert response.headers.get("cache-control") == "no-transform"

    @pytest.mark.asyncio
    async def test_preview_limits_chapters(self, test_book):
        """chapters=1 → spine의 itemref가 1개 이하."""
        client = test_book["client"]
        book_id = test_book["book_id"]
        bm = test_book["bm"]

        # 캐시 무효화
        cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
        cache_file.unlink(missing_ok=True)

        response = client.get(f"/preview/{book_id}?chapters=1")
        assert response.status_code == 200

        zf = _parse_epub_zip(response.content)
        idrefs = _get_spine_idrefs(zf)
        assert len(idrefs) <= 1
        zf.close()

    @pytest.mark.asyncio
    async def test_preview_strips_unreferenced_manifest_items(self, test_book):
        """preview에서 사용되지 않는 manifest item이 제거된다."""
        client = test_book["client"]
        book_id = test_book["book_id"]
        bm = test_book["bm"]

        cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
        cache_file.unlink(missing_ok=True)

        response = client.get(f"/preview/{book_id}?chapters=1")
        assert response.status_code == 200

        zf = _parse_epub_zip(response.content)
        manifest = _get_manifest_items(zf)
        spine = _get_spine_idrefs(zf)

        # spine에 없고 toc도 아닌 manifest item이 없어야 함 (CSS 제외)
        for item_id, info in manifest.items():
            if item_id not in spine and item_id != 'toc' and 'css' not in info['media-type']:
                pytest.fail(f"Unreferenced manifest item found: {item_id} ({info['href']})")
        zf.close()

    @pytest.mark.asyncio
    async def test_preview_caches_result(self, test_book):
        """두 번째 요청 시 동일 응답 (캐시 히트)."""
        client = test_book["client"]
        book_id = test_book["book_id"]
        bm = test_book["bm"]

        cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}_ch3.epub"
        cache_file.unlink(missing_ok=True)

        resp1 = client.get(f"/preview/{book_id}?chapters=3")
        assert resp1.status_code == 200
        assert cache_file.exists()

        resp2 = client.get(f"/preview/{book_id}?chapters=3")
        assert resp2.status_code == 200
        assert resp1.content == resp2.content

    @pytest.mark.asyncio
    async def test_preview_nonexistent_book_returns_404(self, backend_test_setup):
        """없는 book_id → 404."""
        client = backend_test_setup["client"]
        response = client.get("/preview/999999999?chapters=3")
        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_preview_missing_chapter_graceful(self, backend_test_setup):
        """버그 #2 회귀 테스트 — 챕터 파일 1개 누락된 EPUB → 500이 아닌 200.

        spine에 등록된 ch1이 실제 ZIP에 없을 때,
        KeyError로 인한 500 대신 graceful하게 처리되어야 한다.
        """
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        corrupted_path = epub_dir / "[Test Author] Corrupted Preview Book.epub"
        _create_corrupted_epub(corrupted_path, chapter_count=3)

        book_id = await _register_epub_async(bm, corrupted_path)

        try:
            cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
            cache_file.unlink(missing_ok=True)

            response = client.get(f"/preview/{book_id}?chapters=3")
            assert response.status_code == 200, \
                f"Missing chapter should not cause 500, got {response.status_code}"
        finally:
            _cleanup_book(client, bm, book_id, corrupted_path)

    @pytest.mark.asyncio
    async def test_preview_all_chapters_missing_graceful(self, backend_test_setup):
        """모든 챕터 파일이 누락된 EPUB → 500이 아닌 200."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        corrupted_path = epub_dir / "[Test Author] All Missing Preview Book.epub"
        _create_corrupted_epub(corrupted_path, chapter_count=3, missing_all=True)

        book_id = await _register_epub_async(bm, corrupted_path)

        try:
            cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
            cache_file.unlink(missing_ok=True)

            response = client.get(f"/preview/{book_id}?chapters=3")
            assert response.status_code == 200, \
                f"All chapters missing should not cause 500, got {response.status_code}"
        finally:
            _cleanup_book(client, bm, book_id, corrupted_path)

    @pytest.mark.asyncio
    async def test_preview_includes_css_resources(self, backend_test_setup):
        """CSS를 참조하는 EPUB → preview에 CSS 파일이 포함된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        css_path = epub_dir / "[Test Author] CSS Preview Book.epub"
        _create_test_epub(css_path, chapter_count=2, include_css=True)

        book_id = await _register_epub_async(bm, css_path)

        try:
            cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
            cache_file.unlink(missing_ok=True)

            response = client.get(f"/preview/{book_id}?chapters=1")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            assert 'OEBPS/style.css' in zf.namelist(), \
                "CSS file should be included in preview"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, css_path)

    @pytest.mark.asyncio
    async def test_preview_includes_svg_image_cover(self, backend_test_setup):
        """SVG <image>로 커버를 참조하는 EPUB → preview에 커버 이미지가 포함된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        svg_cover_path = epub_dir / "[Test Author] SVG Cover Book.epub"
        _create_test_epub(svg_cover_path, chapter_count=2, include_svg_cover=True)

        book_id = await _register_epub_async(bm, svg_cover_path)

        try:
            cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
            cache_file.unlink(missing_ok=True)

            response = client.get(f"/preview/{book_id}?chapters=1")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            assert 'media/cover.jpg' in zf.namelist(), \
                "SVG <image> referenced cover.jpg should be included in preview"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, svg_cover_path)

    @pytest.mark.asyncio
    async def test_preview_excludes_large_font(self, backend_test_setup):
        """500KB 초과 폰트 파일은 미리보기에서 제외된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Large Font Book.epub"
        _create_epub_with_font(epub_path, font_size=600 * 1024)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=1")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            assert 'OEBPS/fonts/test.ttf' not in zf.namelist(), \
                "Large font (>500KB) should be excluded from preview"
            assert 'OEBPS/style.css' in zf.namelist(), \
                "CSS should still be included even when its referenced font is excluded"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_preview_includes_small_font(self, backend_test_setup):
        """500KB 이하 폰트 파일은 미리보기에 포함된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Small Font Book.epub"
        _create_epub_with_font(epub_path, font_size=100 * 1024)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=1")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            assert 'OEBPS/fonts/test.ttf' in zf.namelist(), \
                "Small font (<=500KB) should be included in preview"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_preview_skips_invalid_spine_items(self, backend_test_setup):
        """manifest에 없는 spine idref는 필터링되고 정상 동작한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Invalid Spine Book.epub"
        _create_epub_with_invalid_spine(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=3")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            spine = _get_spine_idrefs(zf)
            assert 'coverpage' not in spine, \
                "Invalid spine idref (not in manifest) should be filtered out"
            assert 'ch1' in spine
            assert 'ch2' in spine
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_preview_excludes_unreferenced_css(self, backend_test_setup):
        """챕터에서 참조하지 않는 CSS와 그 CSS가 참조하는 폰트가 미리보기에서 제외된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Unreferenced CSS Book.epub"
        _create_epub_with_unreferenced_css(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=1")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            names = zf.namelist()
            assert 'OEBPS/used.css' in names, \
                "Referenced CSS should be included"
            assert 'OEBPS/unused.css' not in names, \
                "Unreferenced CSS should NOT be included"
            assert 'OEBPS/fonts/big.ttf' not in names, \
                "Font referenced only by unreferenced CSS should NOT be included"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)


# ── tests: download 엔드포인트 ────────────────────────────

class TestBookDownload:

    @pytest.mark.asyncio
    async def test_download_by_book_id_only(self, test_book):
        """book_id만으로 download 요청 → 200 + EPUB 바이너리."""
        client = test_book["client"]
        book_id = test_book["book_id"]

        response = client.get(f"/download/{book_id}")
        assert response.status_code == 200
        assert len(response.content) > 0
        content_type = response.headers.get("content-type", "")
        assert "epub" in content_type or "octet-stream" in content_type

    @pytest.mark.asyncio
    async def test_download_response_headers(self, test_book):
        """download 응답에 Content-Encoding: identity와 Cache-Control: no-transform이 포함된다."""
        client = test_book["client"]
        book_id = test_book["book_id"]

        response = client.get(f"/download/{book_id}")
        assert response.status_code == 200
        assert response.headers.get("content-encoding") == "identity"
        assert response.headers.get("cache-control") == "no-transform"

    @pytest.mark.asyncio
    async def test_download_with_path_returns_404(self, test_book):
        """라우트 제거 후 /download/{id}/{path} → 404 (회귀 테스트)."""
        client = test_book["client"]
        book_id = test_book["book_id"]

        response = client.get(f"/download/{book_id}/some/path.epub")
        # {path:path} 라우트 제거 후, 이 URL은 매칭되는 라우트가 없으므로 404
        assert response.status_code == 404

    @pytest.mark.asyncio
    async def test_download_nonexistent_book(self, backend_test_setup):
        """없는 book_id → 200 + 빈 문자열 응답."""
        client = backend_test_setup["client"]
        response = client.get("/download/999999999")
        # get_book_content는 doc 못 찾으면 빈 문자열 반환 (FastAPI가 JSON "\"\"" 또는 "" 직렬화)
        assert response.status_code == 200
        assert response.text.strip('"') == ""

    @pytest.mark.asyncio
    async def test_download_returns_valid_epub_content(self, test_book):
        """download한 EPUB이 유효한 ZIP 파일이다."""
        client = test_book["client"]
        book_id = test_book["book_id"]

        response = client.get(f"/download/{book_id}")
        assert response.status_code == 200

        # 유효한 ZIP/EPUB인지 확인
        zf = _parse_epub_zip(response.content)
        assert 'mimetype' in zf.namelist()
        assert 'META-INF/container.xml' in zf.namelist()
        zf.close()


# ── tests: 캐시 정리 (eviction) ────────────────────────────

class TestCacheEviction:
    """BookManager._evict_old_cache 단위 테스트."""

    def test_old_files_are_deleted(self, tmp_path):
        """1일 이상 된 파일이 삭제된다."""
        import time
        from backend.book_manager import BookManager

        old_file = tmp_path / "old.epub"
        old_file.write_text("old")
        # mtime을 2일 전으로 설정
        old_mtime = time.time() - 2 * 86400
        import os
        os.utime(old_file, (old_mtime, old_mtime))

        new_file = tmp_path / "new.epub"
        new_file.write_text("new")

        BookManager._evict_old_cache(tmp_path)

        assert not old_file.exists()
        assert new_file.exists()

    def test_recent_files_are_kept(self, tmp_path):
        """1일 미만 파일은 보존된다."""
        from backend.book_manager import BookManager

        recent = tmp_path / "recent.pdf"
        recent.write_text("recent")

        BookManager._evict_old_cache(tmp_path)

        assert recent.exists()

    def test_exactly_one_day_old_is_deleted(self, tmp_path):
        """정확히 1일 된 파일도 삭제된다."""
        import time, os
        from backend.book_manager import BookManager

        edge_file = tmp_path / "edge.html"
        edge_file.write_text("edge")
        edge_mtime = time.time() - 86400 - 1  # 1일 + 1초
        os.utime(edge_file, (edge_mtime, edge_mtime))

        BookManager._evict_old_cache(tmp_path)

        assert not edge_file.exists()

    def test_subdirectories_are_not_deleted(self, tmp_path):
        """하위 디렉토리는 삭제하지 않는다."""
        import time, os
        from backend.book_manager import BookManager

        sub_dir = tmp_path / "subdir"
        sub_dir.mkdir()
        old_mtime = time.time() - 2 * 86400
        os.utime(sub_dir, (old_mtime, old_mtime))

        BookManager._evict_old_cache(tmp_path)

        assert sub_dir.exists()

    def test_empty_directory_no_error(self, tmp_path):
        """빈 디렉토리에서 에러 없이 동작한다."""
        from backend.book_manager import BookManager

        BookManager._evict_old_cache(tmp_path)  # 에러 없이 완료

    def test_nonexistent_directory_no_error(self, tmp_path):
        """존재하지 않는 디렉토리에서 에러 없이 동작한다."""
        from backend.book_manager import BookManager

        fake_dir = tmp_path / "nonexistent"
        BookManager._evict_old_cache(fake_dir)  # 에러 없이 완료

    def test_multiple_old_files_all_deleted(self, tmp_path):
        """오래된 파일이 여러 개일 때 모두 삭제된다."""
        import time, os
        from backend.book_manager import BookManager

        old_mtime = time.time() - 2 * 86400
        old_files = []
        for i in range(5):
            f = tmp_path / f"old_{i}.epub"
            f.write_text(f"old_{i}")
            os.utime(f, (old_mtime, old_mtime))
            old_files.append(f)

        new_file = tmp_path / "new.pdf"
        new_file.write_text("new")

        BookManager._evict_old_cache(tmp_path)

        for f in old_files:
            assert not f.exists()
        assert new_file.exists()

    def test_deletion_failure_continues_processing(self, tmp_path):
        """개별 파일 삭제 실패 시 나머지 파일 처리가 계속된다."""
        import time, os
        from unittest.mock import patch
        from backend.book_manager import BookManager

        old_mtime = time.time() - 2 * 86400
        files = []
        for i in range(3):
            f = tmp_path / f"old_{i}.epub"
            f.write_text(f"old_{i}")
            os.utime(f, (old_mtime, old_mtime))
            files.append(f)

        original_unlink = Path.unlink
        first_call = [True]

        def mock_unlink(self, *args, **kwargs):
            if first_call[0]:
                first_call[0] = False
                raise PermissionError("mocked deletion failure")
            original_unlink(self, *args, **kwargs)

        with patch.object(Path, 'unlink', mock_unlink):
            BookManager._evict_old_cache(tmp_path)

        # 1개는 실패로 남고, 나머지 2개는 삭제됨
        remaining = [f for f in files if f.exists()]
        assert len(remaining) == 1

    def test_just_written_cache_file_is_not_evicted(self, tmp_path):
        """방금 쓴 캐시 파일과 오래된 파일이 공존할 때, 새 파일만 보존된다."""
        import time, os
        from backend.book_manager import BookManager

        old_mtime = time.time() - 3 * 86400
        old_files = []
        for name in ["1.pdf", "2_ch5.epub", "3.html", "4_p1-3.pdf"]:
            f = tmp_path / name
            f.write_text("old")
            os.utime(f, (old_mtime, old_mtime))
            old_files.append(f)

        # 방금 생성된 파일
        just_written = tmp_path / "5_ch10.epub"
        just_written.write_text("fresh")

        BookManager._evict_old_cache(tmp_path)

        for f in old_files:
            assert not f.exists(), f"{f.name} should have been evicted"
        assert just_written.exists()


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
