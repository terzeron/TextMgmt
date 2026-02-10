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


def _create_epub_with_unbound_prefix(path: Path) -> None:
    """OPF에 선언되지 않은 네임스페이스 프리픽스(opf:role)가 있는 EPUB."""
    opf = """\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Unbound Prefix Test</dc:title>
    <dc:creator opf:role="aut" opf:file-as="Author, Test">Test Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
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


def _create_epub_without_container_xml(path: Path) -> None:
    """container.xml이 없지만 OPF는 존재하는 EPUB."""
    opf = _make_opf(2)
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        # container.xml 없음
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
        zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))


def _create_epub_without_spine(path: Path) -> None:
    """spine 요소가 없는 OPF를 가진 EPUB."""
    opf = """\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>No Spine Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
        zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))


def _create_epub_with_corrupted_container_xml(path: Path) -> None:
    """container.xml이 깨진 XML이지만 full-path 속성은 regex로 추출 가능한 EPUB."""
    bad_container = """\
<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"
              calibre:unknown-attr="value"/>
  </rootfiles>
</container>"""
    opf = _make_opf(2)
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', bad_container)
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
        zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))


def _create_epub_with_missing_opf(path: Path) -> None:
    """container.xml이 존재하지 않는 OPF 경로를 가리키는 EPUB."""
    container = """\
<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/nonexistent.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', container)
        # OPF 파일 없음


def _create_epub_with_no_opf_at_all(path: Path) -> None:
    """container.xml도 없고 .opf 파일도 없는 EPUB."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))


def _create_epub_with_binary_garbage_container(path: Path) -> None:
    """container.xml이 완전한 바이너리 쓰레기이지만 regex로 full-path를 추출할 수 있는 EPUB."""
    garbage = b'\x89PNG\r\n not XML at all full-path="OEBPS/content.opf" end of garbage'
    opf = _make_opf(2)
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', garbage)
        zf.writestr('OEBPS/content.opf', opf)
        zf.writestr('OEBPS/toc.ncx', '<ncx/>')
        zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
        zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))


def _create_epub_with_ncx_extra_refs(path: Path, chapter_count: int = 3, total_ncx_points: int = 10) -> None:
    """NCX에 실제 챕터보다 많은 navPoint가 포함된 EPUB 생성.

    chapter_count개의 챕터 파일만 포함하고, NCX에는 total_ncx_points개의 navPoint를 생성.
    """
    path.parent.mkdir(parents=True, exist_ok=True)
    ncx_ns = 'http://www.daisy.org/z3986/2005/ncx/'
    nav_points = []
    for i in range(1, total_ncx_points + 1):
        nav_points.append(f'''  <navPoint id="np-{i}" playOrder="{i}">
    <navLabel><text>Chapter {i}</text></navLabel>
    <content src="ch{i}.xhtml#frag{i}"/>
  </navPoint>''')
    ncx = f'''<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}" version="2005-1">
  <navMap>
{chr(10).join(nav_points)}
  </navMap>
</ncx>'''
    with zipfile.ZipFile(str(path), 'w', zipfile.ZIP_DEFLATED) as zf:
        zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
        zf.writestr('META-INF/container.xml', CONTAINER_XML)
        zf.writestr('OEBPS/content.opf', _make_opf(chapter_count))
        zf.writestr('OEBPS/toc.ncx', ncx)
        for i in range(1, chapter_count + 1):
            zf.writestr(f'OEBPS/ch{i}.xhtml', _make_chapter_xhtml(i))


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
        """모든 챕터 파일이 누락된 EPUB → 유효성 검증에서 422 반환."""
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
            assert response.status_code == 422, \
                f"All chapters missing should return 422, got {response.status_code}"
            assert "no valid spine chapters" in response.text
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

    @pytest.mark.asyncio
    async def test_preview_strips_font_face_for_excluded_font(self, backend_test_setup):
        """제외된 대용량 폰트의 @font-face 선언이 CSS에서 제거된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] FontFace Strip Book.epub"
        _create_epub_with_font(epub_path, font_size=600 * 1024)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=1")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            for name in zf.namelist():
                if name.endswith('.css'):
                    css = zf.read(name).decode('utf-8', errors='replace')
                    assert '@font-face' not in css, \
                        f"@font-face for excluded font should be stripped from {name}"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)


# ── tests: EPUB 구조 오류 복원 ────────────────────────────

class TestEpubErrorRecovery:
    """다양한 EPUB 구조 오류에서 500 대신 정상 미리보기 또는 적절한 응답을 반환하는지 검증."""

    # ── unbound namespace prefix ─────────────────────────

    @pytest.mark.asyncio
    async def test_preview_unbound_namespace_prefix(self, backend_test_setup):
        """OPF에 선언되지 않은 네임스페이스 프리픽스(opf:role)가 있어도 미리보기가 생성된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Unbound Prefix Book.epub"
        _create_epub_with_unbound_prefix(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200, \
                f"Unbound prefix should not cause 500, got {response.status_code}: {response.text}"

            zf = _parse_epub_zip(response.content)
            assert 'mimetype' in zf.namelist()
            names = zf.namelist()
            assert any('ch1' in n for n in names), "ch1 should be in preview"
            assert any('ch2' in n for n in names), "ch2 should be in preview"
            assert response.headers.get("x-total-chapters") == "2"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_total_chapters_unbound_prefix(self, backend_test_setup):
        """OPF에 선언되지 않은 프리픽스가 있어도 총 챕터 수가 올바르게 반환된다."""
        from backend.book_manager import BookManager

        bm = backend_test_setup["bm"]
        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Unbound Prefix Chapters.epub"
        _create_epub_with_unbound_prefix(epub_path)

        try:
            count = BookManager._get_epub_total_chapters(epub_path)
            assert count == 2, f"Expected 2 chapters, got {count}"
        finally:
            epub_path.unlink(missing_ok=True)

    # ── missing container.xml ────────────────────────────

    @pytest.mark.asyncio
    async def test_preview_missing_container_xml(self, backend_test_setup):
        """container.xml이 없어도 OPF를 직접 탐색하여 미리보기가 생성된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] No Container Book.epub"
        _create_epub_without_container_xml(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200, \
                f"Missing container.xml should not cause 500, got {response.status_code}: {response.text}"

            zf = _parse_epub_zip(response.content)
            names = zf.namelist()
            assert 'mimetype' in names
            assert any('ch1' in n for n in names), "ch1 should be in preview"
            assert any('ch2' in n for n in names), "ch2 should be in preview"
            # container.xml이 원본에 없으므로 미리보기에도 없어야 함
            assert 'META-INF/container.xml' not in names
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_total_chapters_missing_container_xml(self, backend_test_setup):
        """container.xml이 없어도 총 챕터 수가 올바르게 반환된다."""
        from backend.book_manager import BookManager

        bm = backend_test_setup["bm"]
        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] No Container Chapters.epub"
        _create_epub_without_container_xml(epub_path)

        try:
            count = BookManager._get_epub_total_chapters(epub_path)
            assert count == 2, f"Expected 2 chapters, got {count}"
        finally:
            epub_path.unlink(missing_ok=True)

    # ── corrupted container.xml (lxml recover) ───────────

    @pytest.mark.asyncio
    async def test_preview_corrupted_container_xml(self, backend_test_setup):
        """container.xml에 선언되지 않은 네임스페이스 프리픽스가 있어도 lxml recover로 미리보기가 생성된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Bad Container Book.epub"
        _create_epub_with_corrupted_container_xml(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200, \
                f"Corrupted container.xml should not cause 500, got {response.status_code}: {response.text}"

            zf = _parse_epub_zip(response.content)
            names = zf.namelist()
            assert any('ch1' in n for n in names), "ch1 should be in preview"
            assert any('ch2' in n for n in names), "ch2 should be in preview"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    # ── missing spine ────────────────────────────────────

    @pytest.mark.asyncio
    async def test_preview_missing_spine(self, backend_test_setup):
        """spine이 없어도 manifest의 XHTML 문서 순서로 미리보기가 생성된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] No Spine Book.epub"
        _create_epub_without_spine(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200, \
                f"Missing spine should not cause 500, got {response.status_code}: {response.text}"

            zf = _parse_epub_zip(response.content)
            names = zf.namelist()
            assert 'mimetype' in names
            assert any('ch1' in n for n in names), "ch1 should be in preview"
            assert any('ch2' in n for n in names), "ch2 should be in preview"
            # toc.ncx는 비문서 항목이므로 챕터로 선택되면 안 됨
            assert not any('toc.ncx' in n for n in names), \
                "toc.ncx (non-XHTML) should not be selected as chapter"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_total_chapters_missing_spine(self, backend_test_setup):
        """spine이 없으면 총 챕터 수가 0으로 반환된다."""
        from backend.book_manager import BookManager

        bm = backend_test_setup["bm"]
        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] No Spine Chapters.epub"
        _create_epub_without_spine(epub_path)

        try:
            count = BookManager._get_epub_total_chapters(epub_path)
            assert count == 0, f"Expected 0 (no spine), got {count}"
        finally:
            epub_path.unlink(missing_ok=True)

    # ── OPF missing in archive ───────────────────────────

    @pytest.mark.asyncio
    async def test_preview_opf_missing_in_archive(self, backend_test_setup):
        """container.xml이 존재하지 않는 OPF를 가리키면 422를 반환한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Missing OPF Book.epub"
        _create_epub_with_missing_opf(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 422, \
                f"Missing OPF in archive should return 422, got {response.status_code}"
            assert "OPF" in response.text
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    # ── no OPF at all ────────────────────────────────────

    @pytest.mark.asyncio
    async def test_preview_no_opf_at_all(self, backend_test_setup):
        """container.xml도 없고 .opf 파일도 없으면 422를 반환한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] No OPF Book.epub"
        _create_epub_with_no_opf_at_all(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 422, \
                f"No OPF at all should return 422, got {response.status_code}"
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    # ── binary garbage container.xml (regex fallback) ───

    @pytest.mark.asyncio
    async def test_preview_binary_garbage_container_xml(self, backend_test_setup):
        """container.xml이 완전한 바이너리 쓰레기여도 regex 폴백으로 미리보기가 생성된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Binary Garbage Container Book.epub"
        _create_epub_with_binary_garbage_container(epub_path)

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200, \
                f"Binary garbage container.xml should not cause 500, got {response.status_code}: {response.text}"

            zf = _parse_epub_zip(response.content)
            names = zf.namelist()
            assert any('ch1' in n for n in names), "ch1 should be in preview"
            assert any('ch2' in n for n in names), "ch2 should be in preview"
            zf.close()
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_total_chapters_corrupted_container_xml(self, backend_test_setup):
        """container.xml이 깨진 XML이어도 총 챕터 수가 올바르게 반환된다."""
        from backend.book_manager import BookManager

        bm = backend_test_setup["bm"]
        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Corrupted Container Chapters.epub"
        _create_epub_with_corrupted_container_xml(epub_path)

        try:
            count = BookManager._get_epub_total_chapters(epub_path)
            assert count == 2, f"Expected 2 chapters, got {count}"
        finally:
            epub_path.unlink(missing_ok=True)

    @pytest.mark.asyncio
    async def test_total_chapters_binary_garbage_container(self, backend_test_setup):
        """container.xml이 바이너리 쓰레기여도 regex 폴백으로 총 챕터 수가 반환된다."""
        from backend.book_manager import BookManager

        bm = backend_test_setup["bm"]
        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Garbage Container Chapters.epub"
        _create_epub_with_binary_garbage_container(epub_path)

        try:
            count = BookManager._get_epub_total_chapters(epub_path)
            assert count == 2, f"Expected 2 chapters, got {count}"
        finally:
            epub_path.unlink(missing_ok=True)

    # ── bad ZIP ──────────────────────────────────────────

    @pytest.mark.asyncio
    async def test_preview_bad_zip_file(self, backend_test_setup):
        """손상된 ZIP 파일은 422를 반환한다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Bad Zip Book.epub"
        epub_path.parent.mkdir(parents=True, exist_ok=True)
        epub_path.write_bytes(b"this is not a zip file at all")

        book_id = await _register_epub_async(bm, epub_path)

        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 422, \
                f"Bad ZIP should return 422, got {response.status_code}"
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_total_chapters_bad_zip(self, backend_test_setup):
        """손상된 ZIP 파일에 대해 총 챕터 수가 0으로 반환된다."""
        from backend.book_manager import BookManager

        bm = backend_test_setup["bm"]
        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Bad Zip Chapters.epub"
        epub_path.parent.mkdir(parents=True, exist_ok=True)
        epub_path.write_bytes(b"not a zip")

        try:
            count = BookManager._get_epub_total_chapters(epub_path)
            assert count == 0
        finally:
            epub_path.unlink(missing_ok=True)


# ── tests: _find_opf_path 단위 테스트 ────────────────────

class TestFindOpfPath:
    """BookManager._find_opf_path 헬퍼 단위 테스트."""

    def test_normal_container_xml(self, tmp_path):
        """정상 container.xml에서 OPF 경로를 추출한다."""
        from backend.book_manager import BookManager
        epub_path = tmp_path / "test.epub"
        with zipfile.ZipFile(str(epub_path), 'w') as zf:
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', '<package/>')
        with zipfile.ZipFile(str(epub_path), 'r') as zin:
            assert BookManager._find_opf_path(zin) == 'OEBPS/content.opf'

    def test_missing_container_xml_finds_opf(self, tmp_path):
        """container.xml이 없으면 ZIP 내 .opf 파일을 직접 찾는다."""
        from backend.book_manager import BookManager
        epub_path = tmp_path / "test.epub"
        with zipfile.ZipFile(str(epub_path), 'w') as zf:
            zf.writestr('content.opf', '<package/>')
        with zipfile.ZipFile(str(epub_path), 'r') as zin:
            assert BookManager._find_opf_path(zin) == 'content.opf'

    def test_corrupted_container_xml_regex_fallback(self, tmp_path):
        """container.xml이 깨진 XML이면 regex로 full-path를 추출한다."""
        from backend.book_manager import BookManager
        bad_container = b'<container><rootfiles><rootfile full-path="OEBPS/pkg.opf" broken:attr="x"/></rootfiles></container>'
        epub_path = tmp_path / "test.epub"
        with zipfile.ZipFile(str(epub_path), 'w') as zf:
            zf.writestr('META-INF/container.xml', bad_container)
            zf.writestr('OEBPS/pkg.opf', '<package/>')
        with zipfile.ZipFile(str(epub_path), 'r') as zin:
            result = BookManager._find_opf_path(zin)
            assert result == 'OEBPS/pkg.opf', f"Regex fallback should find OPF, got '{result}'"

    def test_no_opf_returns_empty(self, tmp_path):
        """container.xml도 .opf 파일도 없으면 빈 문자열을 반환한다."""
        from backend.book_manager import BookManager
        epub_path = tmp_path / "test.epub"
        with zipfile.ZipFile(str(epub_path), 'w') as zf:
            zf.writestr('mimetype', 'application/epub+zip')
        with zipfile.ZipFile(str(epub_path), 'r') as zin:
            assert BookManager._find_opf_path(zin) == ''

    def test_container_xml_empty_rootfile(self, tmp_path):
        """container.xml에 rootfile이 있지만 full-path가 비어있으면 .opf 직접 탐색으로 폴백."""
        from backend.book_manager import BookManager
        container = """\
<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="" media-type="application/oebps-package+xml"/></rootfiles>
</container>"""
        epub_path = tmp_path / "test.epub"
        with zipfile.ZipFile(str(epub_path), 'w') as zf:
            zf.writestr('META-INF/container.xml', container)
            zf.writestr('OPS/book.opf', '<package/>')
        with zipfile.ZipFile(str(epub_path), 'r') as zin:
            result = BookManager._find_opf_path(zin)
            assert result == 'OPS/book.opf', f"Should fallback to direct scan, got '{result}'"

    def test_binary_garbage_container_regex_fallback(self, tmp_path):
        """container.xml이 완전한 바이너리 쓰레기여도 regex로 full-path를 추출한다."""
        from backend.book_manager import BookManager
        garbage = b'\x89PNG\r\n not XML at all full-path="OPS/pkg.opf" end'
        epub_path = tmp_path / "test.epub"
        with zipfile.ZipFile(str(epub_path), 'w') as zf:
            zf.writestr('META-INF/container.xml', garbage)
            zf.writestr('OPS/pkg.opf', '<package/>')
        with zipfile.ZipFile(str(epub_path), 'r') as zin:
            result = BookManager._find_opf_path(zin)
            assert result == 'OPS/pkg.opf', f"Binary garbage regex fallback should find OPF, got '{result}'"


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


class TestValidatePreviewEpub:
    """BookManager._validate_preview_epub 단위 테스트."""

    def test_valid_epub_passes(self, tmp_path):
        """정상 EPUB은 (True, None)을 반환한다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "valid.epub"
        _create_test_epub(epub, chapter_count=3)

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is True
        assert err is None

    def test_missing_mimetype_rejected(self, tmp_path):
        """mimetype 파일이 없으면 거부된다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "no_mimetype.epub"
        epub.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(str(epub), 'w') as zf:
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', _make_opf(1))
            zf.writestr('OEBPS/toc.ncx', '<ncx/>')
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is False
        assert "mimetype" in err

    def test_invalid_mimetype_rejected(self, tmp_path):
        """mimetype 내용이 잘못되면 거부된다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "bad_mimetype.epub"
        epub.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(str(epub), 'w') as zf:
            zf.writestr('mimetype', 'text/plain', compress_type=zipfile.ZIP_STORED)
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', _make_opf(1))
            zf.writestr('OEBPS/toc.ncx', '<ncx/>')
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is False
        assert "invalid mimetype" in err

    def test_missing_opf_rejected(self, tmp_path):
        """OPF 파일이 없으면 거부된다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "no_opf.epub"
        epub.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(str(epub), 'w') as zf:
            zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is False
        assert "OPF" in err

    def test_no_valid_spine_chapters_rejected(self, tmp_path):
        """유효한 spine 챕터가 0개이면 거부된다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "empty_spine.epub"
        _create_corrupted_epub(epub, chapter_count=2, missing_all=True)

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is False
        assert "no valid spine chapters" in err

    def test_missing_ncx_toc_attribute_removed(self, tmp_path):
        """NCX 파일이 ZIP에 없으면 toc 속성이 제거되고 통과한다."""
        from backend.book_manager import BookManager
        from lxml import etree

        opf_ns = 'http://www.idpf.org/2007/opf'
        # toc="toc"이 있지만 toc.ncx 파일 없는 EPUB
        epub = tmp_path / "no_ncx.epub"
        epub.parent.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(str(epub), 'w') as zf:
            zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', _make_opf(2))
            # toc.ncx 파일 생략
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
            zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is True, f"Expected valid, got: {err}"

        # 수정된 EPUB에서 toc 속성이 제거되었는지 확인
        with zipfile.ZipFile(str(epub), 'r') as zf:
            opf = etree.fromstring(zf.read('OEBPS/content.opf'))
            spine_el = opf.find(f'.//{{{opf_ns}}}spine')
            assert spine_el.get('toc') is None, "toc attribute should be removed"

    def test_invalid_spine_idref_removed(self, tmp_path):
        """manifest에 없는 spine idref가 자동 제거되고 통과한다."""
        from backend.book_manager import BookManager
        from lxml import etree

        opf_ns = 'http://www.idpf.org/2007/opf'
        epub = tmp_path / "bad_idref.epub"
        _create_epub_with_invalid_spine(epub)

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is True, f"Expected valid, got: {err}"

        # 수정된 EPUB에서 coverpage idref가 제거되었는지 확인
        with zipfile.ZipFile(str(epub), 'r') as zf:
            opf = etree.fromstring(zf.read('OEBPS/content.opf'))
            spine_el = opf.find(f'.//{{{opf_ns}}}spine')
            idrefs = [ref.get('idref') for ref in spine_el.findall(f'{{{opf_ns}}}itemref')]
            assert 'coverpage' not in idrefs, "invalid idref should be removed"
            assert 'ch1' in idrefs
            assert 'ch2' in idrefs

    def test_spine_item_missing_from_zip_removed(self, tmp_path):
        """manifest에는 있지만 ZIP에 파일이 없는 spine 항목이 자동 제거된다."""
        from backend.book_manager import BookManager
        from lxml import etree

        opf_ns = 'http://www.idpf.org/2007/opf'
        # ch1이 누락된 EPUB (ch2, ch3은 존재)
        epub = tmp_path / "missing_ch1.epub"
        _create_corrupted_epub(epub, chapter_count=3, missing_all=False)

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is True, f"Expected valid, got: {err}"

        # ch1이 spine에서 제거되었는지 확인
        with zipfile.ZipFile(str(epub), 'r') as zf:
            opf = etree.fromstring(zf.read('OEBPS/content.opf'))
            spine_el = opf.find(f'.//{{{opf_ns}}}spine')
            idrefs = [ref.get('idref') for ref in spine_el.findall(f'{{{opf_ns}}}itemref')]
            assert 'ch1' not in idrefs, "missing ch1 should be removed from spine"
            assert 'ch2' in idrefs
            assert 'ch3' in idrefs

    def test_corrupted_zip_rejected(self, tmp_path):
        """손상된 ZIP 파일은 거부된다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "corrupted.epub"
        epub.write_bytes(b'this is not a zip file')

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is False
        assert "corrupted ZIP" in err

    def test_missing_spine_element_rejected(self, tmp_path):
        """spine 요소가 없는 OPF는 거부된다."""
        from backend.book_manager import BookManager

        epub = tmp_path / "no_spine.epub"
        _create_epub_without_spine(epub)

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is False
        assert "spine" in err

    def test_toc_id_not_in_manifest_removed(self, tmp_path):
        """toc 속성이 manifest에 없는 ID를 참조하면 제거된다."""
        from backend.book_manager import BookManager
        from lxml import etree

        opf_ns = 'http://www.idpf.org/2007/opf'
        opf_content = """\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Test</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="nonexistent_toc">
    <itemref idref="ch1"/>
  </spine>
</package>"""

        epub = tmp_path / "bad_toc_id.epub"
        with zipfile.ZipFile(str(epub), 'w') as zf:
            zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', opf_content)
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is True, f"Expected valid, got: {err}"

        with zipfile.ZipFile(str(epub), 'r') as zf:
            opf = etree.fromstring(zf.read('OEBPS/content.opf'))
            spine_el = opf.find(f'.//{{{opf_ns}}}spine')
            assert spine_el.get('toc') is None

    def test_no_rewrite_when_clean(self, tmp_path):
        """문제가 없는 EPUB은 재작성하지 않는다 (mtime 불변)."""
        import os
        from backend.book_manager import BookManager

        epub = tmp_path / "clean.epub"
        _create_test_epub(epub, chapter_count=2)
        mtime_before = os.path.getmtime(epub)

        valid, err = BookManager._validate_preview_epub(epub)
        assert valid is True
        mtime_after = os.path.getmtime(epub)
        assert mtime_before == mtime_after, "clean EPUB should not be rewritten"


class TestNcxFiltering:
    """미리보기 EPUB 생성 시 NCX navPoint 필터링 테스트."""

    @pytest.mark.asyncio
    async def test_preview_strips_ncx_extra_navpoints(self, backend_test_setup):
        """NCX에서 미리보기에 포함되지 않은 파일 참조 navPoint가 제거된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] NCX Extra Refs.epub"
        _create_epub_with_ncx_extra_refs(epub_path, chapter_count=5, total_ncx_points=10)

        book_id = await _register_epub_async(bm, epub_path)
        try:
            cache_file = bm.path_prefix / ".preview_cache" / f"{book_id}.epub"
            cache_file.unlink(missing_ok=True)

            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            ncx_data = zf.read('OEBPS/toc.ncx').decode('utf-8')
            zf.close()

            ncx_tree = ET.fromstring(ncx_data)
            ncx_ns = 'http://www.daisy.org/z3986/2005/ncx/'
            nav_points = ncx_tree.findall(f'.//{{{ncx_ns}}}navPoint')

            # 2챕터만 요청했으므로 ch1, ch2만 포함
            assert len(nav_points) <= 2, \
                f"NCX should have at most 2 navPoints, got {len(nav_points)}"

            # 남아있는 navPoint의 src가 모두 ch1 또는 ch2를 참조
            for np in nav_points:
                content = np.find(f'{{{ncx_ns}}}content')
                src = content.get('src', '')
                src_file = src.split('#')[0]
                assert src_file in ('ch1.xhtml', 'ch2.xhtml'), \
                    f"NCX navPoint references unexpected file: {src_file}"
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_preview_keeps_fragment_only_navpoints(self, backend_test_setup):
        """NCX에서 fragment-only src를 가진 navPoint는 유지된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] NCX Fragment Only.epub"
        epub_path.parent.mkdir(parents=True, exist_ok=True)

        ncx_ns = 'http://www.daisy.org/z3986/2005/ncx/'
        ncx = f'''<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}" version="2005-1">
  <navMap>
    <navPoint id="np-1" playOrder="1">
      <navLabel><text>Chapter 1</text></navLabel>
      <content src="ch1.xhtml"/>
    </navPoint>
    <navPoint id="np-frag" playOrder="2">
      <navLabel><text>Fragment Ref</text></navLabel>
      <content src="#bookmark"/>
    </navPoint>
    <navPoint id="np-missing" playOrder="3">
      <navLabel><text>Missing</text></navLabel>
      <content src="ch99.xhtml"/>
    </navPoint>
  </navMap>
</ncx>'''

        with zipfile.ZipFile(str(epub_path), 'w', zipfile.ZIP_DEFLATED) as zf:
            zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', _make_opf(2))
            zf.writestr('OEBPS/toc.ncx', ncx)
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
            zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))

        book_id = await _register_epub_async(bm, epub_path)
        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            ncx_data = zf.read('OEBPS/toc.ncx').decode('utf-8')
            zf.close()

            ncx_tree = ET.fromstring(ncx_data)
            nav_points = ncx_tree.findall(f'.//{{{ncx_ns}}}navPoint')
            nav_ids = [np.get('id') for np in nav_points]

            assert 'np-1' in nav_ids, "ch1 navPoint should be kept"
            assert 'np-frag' in nav_ids, "fragment-only navPoint should be kept"
            assert 'np-missing' not in nav_ids, "missing file navPoint should be removed"
        finally:
            _cleanup_book(client, bm, book_id, epub_path)

    @pytest.mark.asyncio
    async def test_preview_guide_href_with_fragment(self, backend_test_setup):
        """guide reference의 href에 fragment가 포함되어도 올바르게 처리된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] Guide Fragment.epub"
        epub_path.parent.mkdir(parents=True, exist_ok=True)

        opf = '''\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Guide Fragment Test</dc:title>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
  <guide>
    <reference type="text" title="Start" href="ch1.xhtml#start"/>
    <reference type="toc" title="TOC" href="missing.xhtml#toc"/>
  </guide>
</package>'''

        with zipfile.ZipFile(str(epub_path), 'w', zipfile.ZIP_DEFLATED) as zf:
            zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', opf)
            zf.writestr('OEBPS/toc.ncx', '<ncx/>')
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
            zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))

        book_id = await _register_epub_async(bm, epub_path)
        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            opf_data = zf.read('OEBPS/content.opf').decode('utf-8')
            zf.close()

            opf_tree = ET.fromstring(opf_data)
            opf_ns = 'http://www.idpf.org/2007/opf'
            guide_refs = opf_tree.findall(f'.//{{{opf_ns}}}reference')

            # ch1.xhtml#start → ch1.xhtml은 포함되어 있으므로 유지
            # missing.xhtml#toc → missing.xhtml은 없으므로 제거
            hrefs = [r.get('href', '') for r in guide_refs]
            assert any('ch1.xhtml' in h for h in hrefs), \
                "guide reference to ch1.xhtml#start should be kept"
            assert not any('missing.xhtml' in h for h in hrefs), \
                "guide reference to missing.xhtml should be removed"
        finally:
            _cleanup_book(client, bm, book_id, epub_path)


class TestOpfNamespaceFix:
    """OPF에 선언되지 않은 네임스페이스 프리픽스(opf:role 등)가 있을 때 수정 검증."""

    @pytest.mark.asyncio
    async def test_preview_fixes_undeclared_opf_prefix(self, backend_test_setup):
        """opf:role이 xmlns:opf 없이 사용된 OPF가 유효한 XML로 출력된다."""
        bm = backend_test_setup["bm"]
        client = backend_test_setup["client"]

        epub_dir = bm.path_prefix / CATEGORY
        epub_path = epub_dir / "[Test Author] OPF Prefix.epub"
        epub_path.parent.mkdir(parents=True, exist_ok=True)

        # xmlns:opf 선언 없이 opf:role 사용 (실제 EPUB에서 흔한 패턴)
        opf = '''\
<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="BookId">urn:uuid:test</dc:identifier>
    <dc:title>OPF Prefix Test</dc:title>
    <dc:creator opf:role="aut">Author</dc:creator>
  </metadata>
  <manifest>
    <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="toc">
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>'''

        with zipfile.ZipFile(str(epub_path), 'w', zipfile.ZIP_DEFLATED) as zf:
            zf.writestr('mimetype', 'application/epub+zip', compress_type=zipfile.ZIP_STORED)
            zf.writestr('META-INF/container.xml', CONTAINER_XML)
            zf.writestr('OEBPS/content.opf', opf)
            zf.writestr('OEBPS/toc.ncx', '<ncx/>')
            zf.writestr('OEBPS/ch1.xhtml', _make_chapter_xhtml(1))
            zf.writestr('OEBPS/ch2.xhtml', _make_chapter_xhtml(2))

        book_id = await _register_epub_async(bm, epub_path)
        try:
            response = client.get(f"/preview/{book_id}?chapters=2")
            assert response.status_code == 200

            zf = _parse_epub_zip(response.content)
            opf_data = zf.read('OEBPS/content.opf').decode('utf-8')
            zf.close()

            # 출력 OPF가 유효한 XML인지 확인 (strict 파싱)
            ET.fromstring(opf_data)  # 파싱 실패 시 예외 발생

            # xmlns:opf 선언이 추가되었는지 확인
            assert 'xmlns:opf=' in opf_data, "xmlns:opf declaration should be added"
            assert 'opf:role="aut"' in opf_data, "opf:role attribute should be preserved"
        finally:
            _cleanup_book(client, bm, book_id, epub_path)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
