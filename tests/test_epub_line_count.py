"""EPUB 행수는 소스 포맷과 무관하게 본문의 논리적 행을 센다."""

from pathlib import Path
from zipfile import ZipFile

import pytest
from ebooklib import epub

from utils.loader import Loader


def write_epub(path: Path, bodies: list[str]) -> None:
    manifest = "".join(f'<item id="c{i}" href="c{i}.xhtml" media-type="application/xhtml+xml"/>' for i in range(len(bodies)))
    spine = "".join(f'<itemref idref="c{i}"/>' for i in range(len(bodies)))
    with ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip")
        archive.writestr("META-INF/container.xml", '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
        archive.writestr("content.opf", f'<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">lines-test</dc:identifier><dc:title>행수 테스트</dc:title><dc:language>ko</dc:language></metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>')
        for i, body in enumerate(bodies):
            archive.writestr(f"c{i}.xhtml", f'<html xmlns="http://www.w3.org/1999/xhtml"><head><title>본문 제목</title><style>p {{ color: black; }}</style></head><body>{body}</body></html>')


@pytest.fixture(name="reader", params=["read_from_epub", "read_from_epub_with_extracting_zip"])
def epub_reader(request):
    return getattr(Loader, request.param)


def test_line_count_ignores_source_formatting(tmp_path, reader):
    bodies = [
        "<p>첫 문단의 <span>강조</span> 내용</p><p>둘째 문단</p>",
        "\n  <p>\n    첫 문단의\n    <span>강조</span>\n    내용\n  </p>\n\n  <p>둘째 문단</p>\n",
    ]
    counts = []
    for i, body in enumerate(bodies):
        path = tmp_path / f"format-{i}.epub"
        write_epub(path, [body])
        epub.read_epub(path)  # 일반 경로도 fallback 없이 읽을 수 있는 fixture다.
        counts.append(reader(path)[1])
    assert counts == [2, 2]


@pytest.mark.parametrize(("body", "expected"), [
    ("<p>첫 줄<br/>둘째 줄<br/>셋째 줄</p>", 3),
    ("<p>첫 줄<br/><br/>둘째 줄</p><p> </p><p>&#160;</p>", 2),
    ("<div><h1>제목</h1><p>본문 <em>강조</em></p></div><ul><li>하나</li><li>둘</li></ul>", 4),
    ("<div>앞부분<p>문단</p>뒷부분</div>", 3),
    ("<div><div><p>중첩 문단</p></div></div>", 1),
    ("<table><tr><td>칸 하나</td><td>칸 둘</td></tr></table>", 2),
    ("<pre>첫 줄\n둘째 줄\n\n셋째 줄</pre>", 3),
    ("태그 없는\n본문", 1),
    ("<p>본문</p><!-- 주석 --><script>ignored();</script><style>p {color:red}</style>", 1),
    ("\n<p> </p><p>&#160;</p><br/><img src='cover.jpg'/><!-- 주석 -->", 0),
])
def test_logical_line_count(tmp_path, reader, body, expected):
    path = tmp_path / "lines.epub"
    write_epub(path, [body])
    assert reader(path)[1] == expected


def test_counts_all_chapters_beyond_summary_limit(tmp_path, reader):
    path = tmp_path / "chapters.epub"
    write_epub(path, [f"<p>{'가' * (Loader.TEXT_SIZE + 1)}</p>", "<p>둘째 장</p>", "<p>셋째 장</p>"])
    assert reader(path)[1] == 3


def test_forced_zip_fallback_uses_same_line_count(tmp_path, monkeypatch):
    path = tmp_path / "fallback.epub"
    write_epub(path, ["<h1>제목</h1><p>첫 줄<br/>둘째 줄</p>"])
    expected = Loader.read_from_epub(path)[1]

    def fail_read(_path):
        raise ValueError("force ZIP fallback")

    monkeypatch.setattr("utils.loader.epub.read_epub", fail_read)
    assert Loader.read_from_epub(path)[1] == expected == 3
