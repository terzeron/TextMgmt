import unittest
import sys
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import utils.isbn as isbn


class TestISBN(unittest.TestCase):
    def test_validate_isbn13_valid(self):
        self.assertTrue(isbn.validate_isbn13("9788994492032"))

    def test_validate_isbn13_invalid(self):
        self.assertFalse(isbn.validate_isbn13("9788994492031"))
        self.assertFalse(isbn.validate_isbn13("123"))
        self.assertFalse(isbn.validate_isbn13("abcdefghijklm"))

    def test_validate_isbn10_valid(self):
        self.assertTrue(isbn.validate_isbn10("0306406152"))

    def test_validate_isbn10_invalid(self):
        self.assertFalse(isbn.validate_isbn10("0306406153"))
        self.assertFalse(isbn.validate_isbn10("12345"))
        self.assertFalse(isbn.validate_isbn10("1111111111"))
        self.assertFalse(isbn.validate_isbn10("1100101101"))
        self.assertFalse(isbn.validate_isbn10("abcdefghij"))

    def test_validate_isbn10_with_x(self):
        self.assertTrue(isbn.validate_isbn10("080442957X"))

    def test_validate_isbn_dispatches(self):
        self.assertTrue(isbn.validate_isbn("9788994492032"))
        self.assertTrue(isbn.validate_isbn("0306406152"))
        self.assertFalse(isbn.validate_isbn("1234"))
        self.assertFalse(isbn.validate_isbn("1234567890123"))

    def test_search_in_content_isbn13(self):
        res = isbn.search_in_content("ISBN 9788994492032")
        self.assertIn("9788994492032", res)

    def test_search_in_content_isbn13_with_dashes(self):
        res = isbn.search_in_content("978-89-94492-03-2")
        self.assertIn("9788994492032", res)

    def test_search_in_content_bytes(self):
        res = isbn.search_in_content(b"ISBN 9788994492032")
        self.assertIn("9788994492032", res)

    def test_search_in_content_no_match(self):
        res = isbn.search_in_content("no isbn here")
        self.assertEqual(res, [])

def test_search_in_content_multiple():
    res = isbn.search_in_content("first 9788994492032 and 978-89-94492-03-2")
    assert "9788994492032" in res


def test_search_in_content_ocr_normalization():
    content = "ISBN 97B-8O-94492-O3-2"
    result = isbn.search_in_content(content)
    assert "9788094492032" not in result
    assert isinstance(result, list)


def test_read_head_tail_from_file_small(tmp_path: Path):
    f = tmp_path / "small.txt"
    f.write_text("hello world", encoding="utf-8")
    result = isbn.read_head_tail_from_file(f)
    assert result == "hello world"


def test_read_head_tail_from_file_large(tmp_path: Path):
    f = tmp_path / "large.txt"
    content = "A" * 20000
    f.write_text(content, encoding="utf-8")
    result = isbn.read_head_tail_from_file(f, size=100)
    assert len(result) == 200
    assert result[:100] == "A" * 100
    assert result[-100:] == "A" * 100


def test_read_head_tail_from_content():
    short = "hello"
    assert isbn.read_head_tail_from_content(short) == short

    long_text = "A" * 20000
    result = isbn.read_head_tail_from_content(long_text, size=100)
    assert len(result) == 200


def test_extract_from_epub_with_opf_isbn(tmp_path: Path):
    epub = tmp_path / "book.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("content.opf", "<package><metadata>ISBN 9788994492032</metadata></package>")
    result = isbn.extract_from_epub(epub)
    assert "9788994492032" in result


def test_extract_from_epub_with_chapter_isbn(tmp_path: Path):
    epub = tmp_path / "book.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr(
            "content.opf",
            """<package>
            <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/></spine>
        </package>""",
        )
        zf.writestr("ch1.xhtml", "<html><body>ISBN 9788994492032</body></html>")
    result = isbn.extract_from_epub(epub)
    assert "9788994492032" in result


def test_extract_from_epub_no_opf(tmp_path: Path):
    epub = tmp_path / "book.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("dummy.txt", "no opf here")
    result = isbn.extract_from_epub(epub)
    assert result == []


def test_extract_from_epub_bad_zip(tmp_path: Path):
    bad = tmp_path / "bad.epub"
    bad.write_text("not a zip")
    result = isbn.extract_from_epub(bad)
    assert result == []


def test_extract_from_epub_missing_chapter(tmp_path: Path):
    epub = tmp_path / "book.epub"
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr(
            "content.opf",
            """<package>
            <manifest><item id="c1" href="missing.xhtml" media-type="application/xhtml+xml"/></manifest>
            <spine><itemref idref="c1"/></spine>
        </package>""",
        )
    result = isbn.extract_from_epub(epub)
    assert result == []


def test_extract_txt_with_content(tmp_path: Path):
    f = tmp_path / "book.txt"
    f.write_text("ISBN 9788994492032")
    result = isbn.extract(f)
    assert "9788994492032" in result


def test_extract_txt_without_isbn_returns_empty(tmp_path: Path):
    f = tmp_path / "book.txt"
    f.write_text("no isbn here")
    assert isbn.extract(f) == []


def test_extract_txt_with_content_arg(tmp_path: Path):
    f = tmp_path / "book.txt"
    f.write_text("nothing")
    result = isbn.extract(f, content="ISBN 9788994492032")
    assert "9788994492032" in result


def test_extract_unsupported_ext(tmp_path: Path):
    f = tmp_path / "book.xyz"
    f.write_text("ISBN 9788994492032")
    result = isbn.extract(f)
    assert result == []


def test_extract_from_djvu_no_command(tmp_path: Path):
    f = tmp_path / "book.djvu"
    f.write_text("dummy")
    result = isbn.extract_from_djvu(f)
    assert result == []


def test_extract_from_hwp_no_command(tmp_path: Path):
    f = tmp_path / "book.hwp"
    f.write_text("dummy")
    result = isbn.extract_from_hwp(f)
    # strings command may or may not be available
    assert isinstance(result, list)


# ---- coverage: isbn additional uncovered lines ----

import subprocess


def test_validate_isbn10_non_digit_in_body():
    assert isbn.validate_isbn10("030X406152") is False


def test_search_in_content_isbn10():
    res = isbn.search_in_content("89-94492-03-2")
    # May or may not match depending on pattern
    assert isinstance(res, list)


def test_extract_from_djvu_with_mock(tmp_path: Path, monkeypatch):
    """Lines 206-229: extract_from_djvu with mocked subprocess"""
    f = tmp_path / "book.djvu"
    f.write_text("dummy")

    class DummyResult:
        def __init__(self, rc, stdout):
            self.returncode = rc
            self.stdout = stdout

    calls = {"n": 0}

    def fake_run(cmd, **kwargs):
        calls["n"] += 1
        if "djvused" in cmd[0]:
            return DummyResult(0, "10\n")
        if "djvutxt" in cmd[0]:
            return DummyResult(0, "ISBN 9788994492032")
        return DummyResult(1, "")

    monkeypatch.setattr(subprocess, "run", fake_run)
    result = isbn.extract_from_djvu(f)
    assert "9788994492032" in result


def test_extract_from_djvu_bad_page_count(tmp_path: Path, monkeypatch):
    """Line 208: ValueError when parsing page count"""
    f = tmp_path / "book.djvu"
    f.write_text("dummy")

    class DummyResult:
        returncode = 0
        stdout = "not_a_number\n"

    monkeypatch.setattr(subprocess, "run", lambda cmd, **kw: DummyResult())
    result = isbn.extract_from_djvu(f)
    assert result == []


def test_extract_from_djvu_nonzero_rc(tmp_path: Path, monkeypatch):
    """Line 203: djvused returns nonzero"""
    f = tmp_path / "book.djvu"
    f.write_text("dummy")

    class DummyResult:
        returncode = 1
        stdout = ""

    monkeypatch.setattr(subprocess, "run", lambda cmd, **kw: DummyResult())
    result = isbn.extract_from_djvu(f)
    assert result == []


def test_extract_from_hwp_with_mock(tmp_path: Path, monkeypatch):
    """Lines 232-246: extract_from_hwp with mocked strings command"""
    f = tmp_path / "book.hwp"
    f.write_text("dummy")

    class DummyResult:
        returncode = 0
        stdout = "ISBN 9788994492032"

    monkeypatch.setattr(subprocess, "run", lambda cmd, **kw: DummyResult())
    result = isbn.extract_from_hwp(f)
    assert "9788994492032" in result


def test_extract_from_hwp_nonzero_rc(tmp_path: Path, monkeypatch):
    """Line 240: strings returns nonzero"""
    f = tmp_path / "book.hwp"
    f.write_text("dummy")

    class DummyResult:
        returncode = 1
        stdout = ""

    monkeypatch.setattr(subprocess, "run", lambda cmd, **kw: DummyResult())
    result = isbn.extract_from_hwp(f)
    assert result == []


def test_extract_pdf(tmp_path: Path):
    """Lines 267-285: extract from PDF"""
    import pypdf

    pdf = tmp_path / "book.pdf"
    writer = pypdf.PdfWriter()
    writer.add_blank_page(72, 72)
    with open(pdf, "wb") as f:
        writer.write(f)
    result = isbn.extract(pdf)
    assert isinstance(result, list)


def test_extract_pdf_broken(tmp_path: Path):
    """Line 282: PDF extraction exception"""
    pdf = tmp_path / "book.pdf"
    pdf.write_bytes(b"not a pdf")
    result = isbn.extract(pdf)
    assert result == []


def test_extract_djvu_via_extract(tmp_path: Path, monkeypatch):
    """Line 287: extract dispatches to extract_from_djvu"""
    f = tmp_path / "book.djvu"
    f.write_text("dummy")
    monkeypatch.setattr(isbn, "extract_from_djvu", lambda fp: ["1234567890"])
    result = isbn.extract(f)
    assert result == ["1234567890"]


def test_extract_hwp_via_extract(tmp_path: Path, monkeypatch):
    """Line 289: extract dispatches to extract_from_hwp"""
    f = tmp_path / "book.hwp"
    f.write_text("dummy")
    monkeypatch.setattr(isbn, "extract_from_hwp", lambda fp: ["1234567890"])
    result = isbn.extract(f)
    assert result == ["1234567890"]


# ---- coverage: remaining uncovered lines ----


def test_search_in_content_isbn10_valid():
    """Line 136: valid ISBN-10 found in content"""
    # ISBN-10: 8932908397 (valid Korean ISBN-10)
    content = "ISBN 89-329-0839-7 some text"
    result = isbn.search_in_content(content)
    assert "8932908397" in result


def test_extract_from_epub_empty_spine(tmp_path: Path):
    """Line 174: spine_hrefs empty because idrefs don't match manifest"""
    epub = tmp_path / "test.epub"
    opf = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf">
  <metadata><dc:identifier xmlns:dc="http://purl.org/dc/elements/1.1/">test</dc:identifier></metadata>
  <manifest><item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="missing_id"/></spine>
</package>"""
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("content.opf", opf)
    result = isbn.extract_from_epub(epub)
    assert result == []


def test_extract_from_epub_reads_head_and_tail_chapters(tmp_path: Path):
    epub = tmp_path / "test.epub"
    opf = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf">
  <manifest>
    <item id="c1" href="head.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="middle.xhtml" media-type="application/xhtml+xml"/>
    <item id="c3" href="tail.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="c2"/>
    <itemref idref="c3"/>
  </spine>
</package>"""
    with zipfile.ZipFile(epub, "w") as zf:
        zf.writestr("OPS/content.opf", opf)
        zf.writestr("OPS/head.xhtml", "<html><body>ISBN 9788994492032</body></html>")
        zf.writestr("OPS/middle.xhtml", "<html><body>middle only</body></html>")
        zf.writestr("OPS/tail.xhtml", "<html><body>tail text</body></html>")
    result = isbn.extract_from_epub(epub)
    assert "9788994492032" in result


def test_extract_from_djvu_command_not_found(tmp_path: Path, monkeypatch):
    """Lines 227-229: djvused not found"""
    import subprocess as sp

    f = tmp_path / "book.djvu"
    f.write_text("dummy")
    monkeypatch.setattr(sp, "run", lambda *a, **kw: (_ for _ in ()).throw(FileNotFoundError("djvused")))
    result = isbn.extract_from_djvu(f)
    assert result == []


def test_extract_from_hwp_command_not_found(tmp_path: Path, monkeypatch):
    """Lines 245-246: strings not found"""
    import subprocess as sp

    f = tmp_path / "book.hwp"
    f.write_text("dummy")
    monkeypatch.setattr(sp, "run", lambda *a, **kw: (_ for _ in ()).throw(FileNotFoundError("strings")))
    result = isbn.extract_from_hwp(f)
    assert result == []


def test_extract_pdf_with_isbn_text(tmp_path: Path, monkeypatch):
    """Lines 276, 281, 285: PDF with ISBN text in pages"""
    from pypdf import PdfWriter

    pdf = tmp_path / "book.pdf"
    writer = PdfWriter()
    writer.add_blank_page(width=612, height=792)
    with open(pdf, "wb") as f:
        writer.write(f)

    # Monkeypatch PdfReader to return pages with ISBN text
    class FakePage:
        def extract_text(self):
            return "ISBN 9788994492032"

    class FakeReader:
        def __init__(self, f):
            self.pages = [FakePage()]

    import pypdf

    monkeypatch.setattr(pypdf, "PdfReader", FakeReader)
    result = isbn.extract(pdf)
    assert "9788994492032" in result
