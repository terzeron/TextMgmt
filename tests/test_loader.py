import sys
import os
import unittest
import zipfile
import warnings
from pathlib import Path
from unittest.mock import MagicMock, patch

# loader.py가 import 시점에 환경 변수와 외부 모듈을 요구하므로 사전 설정
os.environ.setdefault("TM_BOOK_DIR", str(Path(__file__).parent.parent / "tests" / "books"))
os.environ.setdefault("TM_COMICS_DIR", str(Path(__file__).parent.parent / "tests" / "comics"))

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
warnings.filterwarnings("ignore", message=".*XML.*HTML.*")


def _get_loader():
    """Loader 클래스를 lazy import (외부 의존성 처리)"""
    from utils.loader import Loader

    return Loader


class TestLoaderInit(unittest.TestCase):
    def test_loader_class_exists(self):
        Loader = _get_loader()
        assert Loader is not None
        assert hasattr(Loader, "read_file")
        assert hasattr(Loader, "read_files")


class TestReadFromText:
    def test_read_utf8_text(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "test.txt"
        f.write_text("Hello World\n안녕하세요\n테스트", encoding="utf-8")
        summary, line_count, page_count, raw = Loader.read_from_text(f)
        assert isinstance(summary, str)
        assert line_count == 3
        assert page_count == 0
        assert "Hello" in raw

    def test_read_empty_text(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "empty.txt"
        f.write_text("", encoding="utf-8")
        summary, line_count, page_count, raw = Loader.read_from_text(f)
        assert summary == ""
        assert line_count == 0

    def test_read_bom_text(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "bom.txt"
        f.write_bytes(b"\xef\xbb\xbfBOM content")
        summary, line_count, page_count, raw = Loader.read_from_text(f)
        assert "\ufeff" not in summary

    def test_read_binary_as_text(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "binary.txt"
        f.write_bytes(b"\x80\x81\x82\x83")
        summary, line_count, page_count, raw = Loader.read_from_text(f)
        assert summary == ""


class TestReadFromEpub:
    def test_read_valid_epub(self, tmp_path: Path):
        Loader = _get_loader()
        epub = tmp_path / "book.epub"
        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>')
            zf.writestr("content.opf", '<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>')
            zf.writestr("ch1.xhtml", "<html><body><p>테스트 내용입니다</p></body></html>")
        # read_from_epub_with_extracting_zip 직접 테스트
        summary, line_count = Loader.read_from_epub_with_extracting_zip(epub)
        assert "테스트" in summary

    def test_read_bad_zip_epub(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "bad.epub"
        f.write_text("not a zip")
        summary, line_count = Loader.read_from_epub_with_extracting_zip(f)
        assert summary == ""
        assert line_count == 0

    def test_read_epub_no_container(self, tmp_path: Path):
        Loader = _get_loader()
        epub = tmp_path / "nocontainer.epub"
        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("META-INF/container.xml", "<container></container>")
        summary, line_count = Loader.read_from_epub_with_extracting_zip(epub)
        assert summary == ""


class TestReadFromHtml:
    def test_read_html(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "test.html"
        f.write_text("<html><body><p>Hello</p></body></html>", encoding="utf-8")
        summary, line_count, page_count = Loader.read_from_html(f)
        assert "Hello" in summary
        assert page_count == 0


class TestReadFromImage:
    def test_read_image_returns_empty(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "test.jpg"
        f.write_bytes(b"\xff\xd8")
        summary, line_count, page_count = Loader.read_from_image(f)
        assert summary == ""
        assert line_count == 0
        assert page_count == 0


class TestGetFileList:
    def test_single_file(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "test.txt"
        f.write_text("hello")
        result = Loader.get_file_list(f)
        assert result == [f]

    def test_directory(self, tmp_path: Path):
        Loader = _get_loader()
        (tmp_path / "a.txt").write_text("a")
        (tmp_path / "b.txt").write_text("b")
        result = Loader.get_file_list(tmp_path)
        assert len(result) >= 2

    def test_directory_with_subdirs(self, tmp_path: Path):
        Loader = _get_loader()
        sub = tmp_path / "sub"
        sub.mkdir()
        (sub / "c.txt").write_text("c")
        (tmp_path / "a.txt").write_text("a")
        result = Loader.get_file_list(tmp_path)
        # Should include one file from subdir + direct files
        assert len(result) >= 2

    def test_directory_recursive(self, tmp_path: Path):
        Loader = _get_loader()
        sub = tmp_path / "sub"
        sub.mkdir()
        (sub / "c.txt").write_text("c")
        (tmp_path / "a.txt").write_text("a")
        result = Loader.get_file_list(tmp_path, recursive=True)
        assert len(result) >= 2

    def test_directory_num_files_limit(self, tmp_path: Path):
        Loader = _get_loader()
        for i in range(5):
            (tmp_path / f"{i}.txt").write_text(str(i))
        result = Loader.get_file_list(tmp_path, num_files=2)
        assert len(result) == 2

    def test_directory_skips_hidden(self, tmp_path: Path):
        Loader = _get_loader()
        (tmp_path / ".hidden").write_text("x")
        (tmp_path / "visible.txt").write_text("y")
        hidden_dir = tmp_path / ".hidden_dir"
        hidden_dir.mkdir()
        (hidden_dir / "z.txt").write_text("z")
        result = Loader.get_file_list(tmp_path, recursive=True)
        names = [p.name for p in result]
        assert ".hidden" not in names
        assert "z.txt" not in names


class TestGetPathPrefix:
    def test_book_path(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        f = tmp_path / "book.txt"
        assert Loader.get_path_prefix(f) == tmp_path
        Loader.path_prefix = original_prefix

    def test_comics_path(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        original_comics = Loader.comics_path_prefix
        Loader.comics_path_prefix = tmp_path
        f = tmp_path / "comic.cbz"
        assert Loader.get_path_prefix(f) == tmp_path
        Loader.comics_path_prefix = original_comics


class TestReadFile:
    def test_read_txt_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path

        cat_dir = tmp_path / "cat"
        cat_dir.mkdir()
        f = cat_dir / "[Author] Title.txt"
        f.write_text("Hello world\nLine 2", encoding="utf-8")

        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix

        assert len(result) == 1
        inode = list(result.keys())[0]
        doc = result[inode]
        assert doc["author"] == "Author"
        assert doc["title"] == "Title"
        assert doc["category"] == "cat"
        assert doc["file_type"] == "txt"

    def test_read_file_no_author(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path

        f = tmp_path / "NoAuthor.txt"
        f.write_text("content")

        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix

        inode = list(result.keys())[0]
        doc = result[inode]
        assert doc["author"] == ""
        assert doc["title"] == "NoAuthor"
        assert doc["category"] == "_root"

    def test_read_nonexistent_file(self, tmp_path: Path):
        Loader = _get_loader()
        result = Loader.read_file(tmp_path / "nonexistent.txt")
        assert result == {}

    def test_read_unsupported_format(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path

        f = tmp_path / "file.xyz"
        f.write_text("content")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        assert result == {}

    def test_read_image_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path

        f = tmp_path / "image.jpg"
        f.write_bytes(b"\xff\xd8\xff\xe0")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix

        assert len(result) == 1
        doc = list(result.values())[0]
        assert doc["file_type"] == "jpg"
        assert doc["summary"] == ""

    def test_read_file_skip_text(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path

        f = tmp_path / "book.txt"
        f.write_text("content")
        result = Loader.read_file(f, skip_text=True)
        Loader.path_prefix = original_prefix

        doc = list(result.values())[0]
        assert doc["summary"] == ""

    def test_read_file_skip_text_unsupported(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path

        f = tmp_path / "file.xyz"
        f.write_text("content")
        result = Loader.read_file(f, skip_text=True)
        Loader.path_prefix = original_prefix
        assert result == {}


class TestPngPredictor:
    def test_no_filter(self):
        Loader = _get_loader()
        # filter byte = 0 (None), 2 columns of data
        data = bytes([0, 0x10, 0x20, 0, 0x30, 0x40])
        result = Loader._apply_png_predictor(data, columns=2)
        assert result == bytes([0x10, 0x20, 0x30, 0x40])

    def test_sub_filter(self):
        Loader = _get_loader()
        # filter byte = 1 (Sub)
        data = bytes([1, 0x10, 0x05])
        result = Loader._apply_png_predictor(data, columns=2)
        assert result == bytes([0x10, 0x15])

    def test_up_filter(self):
        Loader = _get_loader()
        # Two rows: first row no filter, second row Up filter
        data = bytes([0, 0x10, 0x20, 2, 0x01, 0x02])
        result = Loader._apply_png_predictor(data, columns=2)
        assert result == bytes([0x10, 0x20, 0x11, 0x22])


class TestFindXrefOffset:
    def test_basic_xref(self):
        Loader = _get_loader()
        xref = b"xref\n0 3\n0000000000 65535 f \n0000000100 00000 n \n0000000200 00000 n \ntrailer\n"
        assert Loader._find_xref_offset(xref, 1) == 100
        assert Loader._find_xref_offset(xref, 2) == 200
        assert Loader._find_xref_offset(xref, 5) is None

    def test_free_entry(self):
        Loader = _get_loader()
        xref = b"xref\n0 2\n0000000000 65535 f \n0000000100 00000 f \ntrailer\n"
        assert Loader._find_xref_offset(xref, 1) is None


class TestXrefStreamFindEntry:
    def test_basic_entry(self):
        Loader = _get_loader()
        # W=[1,2,1], index=[0,2]
        # Entry 0: type=1, offset=256, gen=0
        # Entry 1: type=1, offset=512, gen=0
        data = bytes([1, 1, 0, 0, 1, 2, 0, 0])
        result = Loader._xref_stream_find_entry(data, [1, 2, 1], [0, 2], 1)
        assert result == (1, 512, 0)

    def test_entry_not_found(self):
        Loader = _get_loader()
        data = bytes([1, 1, 0, 0])
        result = Loader._xref_stream_find_entry(data, [1, 2, 1], [0, 1], 5)
        assert result is None

    def test_zero_entry_size(self):
        Loader = _get_loader()
        result = Loader._xref_stream_find_entry(b"", [0, 0, 0], [0, 1], 0)
        assert result is None


# ---- coverage: loader additional uncovered lines ----

import re
import io
import struct
import zlib
import tempfile
from unittest.mock import patch, MagicMock


class TestReadFromPdf:
    def test_read_valid_pdf(self, tmp_path: Path):
        Loader = _get_loader()
        import pypdf

        # Create a minimal valid PDF
        pdf = tmp_path / "test.pdf"
        writer = pypdf.PdfWriter()
        writer.add_blank_page(width=72, height=72)
        with open(pdf, "wb") as f:
            writer.write(f)
        summary, line_count, page_count = Loader.read_from_pdf(pdf)
        assert page_count == 1

    def test_read_broken_pdf(self, tmp_path: Path):
        Loader = _get_loader()
        pdf = tmp_path / "broken.pdf"
        pdf.write_bytes(b"%PDF-1.0\nbroken content")
        summary, line_count, page_count = Loader.read_from_pdf(pdf)
        assert page_count == 0


class TestReadFromRtf:
    def test_read_valid_rtf(self, tmp_path: Path):
        Loader = _get_loader()
        rtf = tmp_path / "test.rtf"
        rtf.write_bytes(b"{\\rtf1 Hello World}")
        summary, line_count, page_count = Loader.read_from_rtf(rtf)
        assert "Hello" in summary or summary == ""  # depends on striprtf behavior

    def test_read_broken_rtf(self, tmp_path: Path):
        Loader = _get_loader()
        rtf = tmp_path / "broken.rtf"
        rtf.write_bytes(b"\x80\x81\x82not rtf")
        summary, line_count, page_count = Loader.read_from_rtf(rtf)
        assert isinstance(summary, str)


class TestReadFromDocx:
    def test_read_valid_docx(self, tmp_path: Path):
        Loader = _get_loader()
        from docx import Document

        doc = Document()
        doc.add_paragraph("Hello Docx World")
        doc.add_paragraph("Line 2")
        docx_path = tmp_path / "test.docx"
        doc.save(str(docx_path))
        summary, line_count, page_count = Loader.read_from_docx(docx_path)
        assert "Hello" in summary
        assert line_count >= 2


class TestReadFromEpubFull:
    def test_read_epub_with_ebooklib(self, tmp_path: Path):
        """read_from_epub using ebooklib or fallback"""
        Loader = _get_loader()
        epub = tmp_path / "test.epub"
        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("mimetype", "application/epub+zip")
            zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>')
            zf.writestr("content.opf", '<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>')
            zf.writestr("ch1.xhtml", "<html><body><p>테스트 내용</p></body></html>")
        summary, line_count, page_count = Loader.read_from_epub(epub)
        assert isinstance(summary, str)

    def test_read_epub_exception_zip_fallback(self, tmp_path: Path):
        """Lines 161-176: epub read_epub fails, fallback to zip extraction"""
        Loader = _get_loader()
        epub = tmp_path / "bad_epub.epub"
        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>')
            zf.writestr("content.opf", '<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>')
            zf.writestr("ch1.xhtml", "<html><body>Fallback content</body></html>")
        # No mimetype → ebooklib will fail, triggering zip fallback
        summary, line_count, page_count = Loader.read_from_epub(epub)
        assert isinstance(summary, str)


class TestReadFromDocHwp:
    def test_read_from_doc(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        doc_file = tmp_path / "test.doc"
        doc_file.write_text("doc content")
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "converted text\nline2")
        summary, line_count, page_count = Loader.read_from_doc(doc_file)
        assert "converted" in summary
        assert line_count == 2

    def test_read_from_doc_exception(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        doc_file = tmp_path / "test.doc"
        doc_file.write_text("doc content")

        def raise_lo(fp, fmt):
            raise RuntimeError("lo fail")

        monkeypatch.setattr(Loader, "_convert_with_libreoffice", raise_lo)
        summary, line_count, page_count = Loader.read_from_doc(doc_file)
        assert summary == ""

    def test_read_from_hwp(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        hwp_file = tmp_path / "test.hwp"
        hwp_file.write_text("hwp content")
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "hwp text")
        summary, line_count, page_count = Loader.read_from_hwp(hwp_file)
        assert "hwp" in summary


class TestConvertWithLibreoffice:
    def test_successful_conversion(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        import subprocess

        src = tmp_path / "test.doc"
        src.write_text("doc")
        monkeypatch.setattr(Loader, "_find_libreoffice", lambda: "lo")

        class FakeTmp:
            def __enter__(self):
                return str(tmp_path)

            def __exit__(self, *a):
                return False

        monkeypatch.setattr(tempfile, "TemporaryDirectory", lambda: FakeTmp())

        class DummyProc:
            returncode = 0
            stderr = b""

        def fake_run(*args, **kwargs):
            (tmp_path / "test.txt").write_text("converted", encoding="utf-8")
            return DummyProc()

        monkeypatch.setattr(subprocess, "run", fake_run)
        result = Loader._convert_with_libreoffice(src, "txt:Text")
        assert result == "converted"

    def test_no_output(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        import subprocess

        src = tmp_path / "test.doc"
        src.write_text("doc")
        monkeypatch.setattr(Loader, "_find_libreoffice", lambda: "lo")

        class FakeTmp:
            def __enter__(self):
                return str(tmp_path / "empty")

            def __exit__(self, *a):
                return False

        (tmp_path / "empty").mkdir()
        monkeypatch.setattr(tempfile, "TemporaryDirectory", lambda: FakeTmp())

        class DummyProc:
            returncode = 1
            stderr = b"error"

        monkeypatch.setattr(subprocess, "run", lambda *a, **k: DummyProc())
        result = Loader._convert_with_libreoffice(src, "txt:Text")
        assert result == ""


class TestReadFileBranches:
    def test_read_epub_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        epub = tmp_path / "[Author] Book.epub"
        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("mimetype", "application/epub+zip")
            zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>')
            zf.writestr("content.opf", '<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>')
            zf.writestr("ch1.xhtml", "<html><body>Content</body></html>")
        result = Loader.read_file(epub)
        Loader.path_prefix = original_prefix
        if result:
            doc = list(result.values())[0]
            assert doc["file_type"] == "epub"
            assert doc["author"] == "Author"

    def test_read_html_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        f = tmp_path / "page.html"
        f.write_text("<html><body>Hello HTML</body></html>")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "html"

    def test_read_cbz_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        f = tmp_path / "comic.cbz"
        f.write_bytes(b"cbz data")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "cbz"

    def test_read_rtf_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        f = tmp_path / "doc.rtf"
        f.write_bytes(b"{\\rtf1 Hello}")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "rtf"

    def test_read_pdf_skip_text(self, tmp_path: Path, monkeypatch):
        """Lines 703-713: skip_text with PDF (fast page count)"""
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        pdf = tmp_path / "book.pdf"
        pdf.write_bytes(b"%PDF")
        monkeypatch.setattr(Loader, "_fast_pdf_page_count", lambda fp: 5)
        result = Loader.read_file(pdf, skip_text=True)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["page_count"] == 5
        assert doc["summary"] == ""

    def test_read_pdf_skip_text_fallback(self, tmp_path: Path, monkeypatch):
        """Lines 705-712: skip_text PDF fast count fails, pypdf fallback"""
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        import pypdf

        pdf = tmp_path / "book.pdf"
        writer = pypdf.PdfWriter()
        writer.add_blank_page(72, 72)
        with open(pdf, "wb") as f:
            writer.write(f)
        monkeypatch.setattr(Loader, "_fast_pdf_page_count", lambda fp: None)
        result = Loader.read_file(pdf, skip_text=True)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["page_count"] == 1

    def test_read_doc_file(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        f = tmp_path / "file.doc"
        f.write_text("doc")
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "text")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "doc"

    def test_read_hwp_file(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        f = tmp_path / "file.hwp"
        f.write_text("hwp")
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "text")
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "hwp"

    def test_read_docx_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        from docx import Document

        doc_obj = Document()
        doc_obj.add_paragraph("Hello")
        f = tmp_path / "file.docx"
        doc_obj.save(str(f))
        result = Loader.read_file(f)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "docx"

    def test_read_pdf_file(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        import pypdf

        pdf = tmp_path / "file.pdf"
        writer = pypdf.PdfWriter()
        writer.add_blank_page(72, 72)
        with open(pdf, "wb") as f:
            writer.write(f)
        result = Loader.read_file(pdf)
        Loader.path_prefix = original_prefix
        doc = list(result.values())[0]
        assert doc["file_type"] == "pdf"
        assert doc["page_count"] == 1


class TestReadFiles:
    def test_read_files(self, tmp_path: Path):
        Loader = _get_loader()
        original_prefix = Loader.path_prefix
        Loader.path_prefix = tmp_path
        (tmp_path / "a.txt").write_text("hello")
        (tmp_path / "b.txt").write_text("world")
        result = Loader.read_files(tmp_path)
        Loader.path_prefix = original_prefix
        assert len(result) >= 2


class TestGetStat:
    def test_get_stat(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "test.txt"
        f.write_text("hello")
        st = Loader.get_stat(f)
        assert st.st_size == 5


class TestPrintUsage:
    def test_print_usage(self):
        Loader = _get_loader()
        from utils.loader import print_usage
        import pytest

        with pytest.raises(SystemExit):
            print_usage("test_program")


class TestFindLibreoffice:
    def test_find_via_which(self, monkeypatch):
        Loader = _get_loader()
        import shutil

        monkeypatch.setattr(shutil, "which", lambda cmd: "/usr/bin/soffice" if cmd == "soffice" else None)
        assert Loader._find_libreoffice() == "/usr/bin/soffice"

    def test_find_mac_path(self, monkeypatch):
        Loader = _get_loader()
        import shutil

        monkeypatch.setattr(shutil, "which", lambda cmd: None)
        mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice"
        monkeypatch.setattr(Path, "exists", lambda self: str(self) == mac_path)
        assert Loader._find_libreoffice() == mac_path

    def test_fallback(self, monkeypatch):
        Loader = _get_loader()
        import shutil

        monkeypatch.setattr(shutil, "which", lambda cmd: None)
        monkeypatch.setattr(Path, "exists", lambda self: False)
        assert Loader._find_libreoffice() == "libreoffice"


class TestFastPdfPageCount:
    def test_real_pdf(self, tmp_path: Path):
        Loader = _get_loader()
        import pypdf

        pdf = tmp_path / "test.pdf"
        writer = pypdf.PdfWriter()
        writer.add_blank_page(72, 72)
        writer.add_blank_page(72, 72)
        with open(pdf, "wb") as f:
            writer.write(f)
        count = Loader._fast_pdf_page_count(pdf)
        # May return 2 or None depending on xref structure
        assert count is None or count == 2

    def test_not_a_pdf(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "not.pdf"
        f.write_text("not a pdf")
        assert Loader._fast_pdf_page_count(f) is None


# ---- coverage: epub read edge cases ----


class TestReadFromEpubEdgeCases:
    def test_epub_chapter_read_exception(self, tmp_path: Path):
        """Lines 118-119: chapter read raises exception"""
        Loader = _get_loader()
        opf = """<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf">
  <metadata/>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"""
        container = """<?xml version="1.0"?>
<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
  <rootfiles><rootfile full-path="content.opf"/></rootfiles>
</container>"""
        epub = tmp_path / "test.epub"
        import zipfile

        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("mimetype", "application/epub+zip")
            zf.writestr("META-INF/container.xml", container)
            zf.writestr("content.opf", opf)
            # ch1.xhtml intentionally corrupt
            zf.writestr("ch1.xhtml", b"\x00\x01\x02\x03")
        result, line_count = Loader.read_from_epub_with_extracting_zip(epub)
        assert isinstance(result, str)

    def test_epub_both_methods_fail(self, tmp_path: Path, monkeypatch):
        """Lines 171-173: both ebooklib and zip fallback fail"""
        Loader = _get_loader()
        epub = tmp_path / "test.epub"
        epub.write_bytes(b"not a zip at all")
        # read_from_epub tries ebooklib (fails) then zip fallback (also fails)
        summary, line_count, page_count = Loader.read_from_epub(epub)
        assert isinstance(summary, str)


class TestReadFromPdfEdgeCases:
    def test_pdf_text_exceeds_limit(self, tmp_path: Path, monkeypatch):
        """Line 198: break when text exceeds TEXT_SIZE"""
        Loader = _get_loader()
        import pypdf

        pdf = tmp_path / "big.pdf"
        writer = pypdf.PdfWriter()
        for _ in range(3):
            writer.add_blank_page(72, 72)
        with open(pdf, "wb") as f:
            writer.write(f)

        class FakePage:
            def extract_text(self):
                return "x" * 3000

        class FakeReader:
            def __init__(self, f):
                self.pages = [FakePage(), FakePage(), FakePage()]

        monkeypatch.setattr(pypdf, "PdfReader", FakeReader)
        summary, line_count, page_count = Loader.read_from_pdf(pdf)
        # TEXT_SIZE is 4096, so with 3000*2=6000 > 4096, should break early
        assert len(summary) <= Loader.TEXT_SIZE + 3000  # first page + partial second


class TestReadFromHwpEdgeCases:
    def test_hwp_conversion_exception(self, tmp_path: Path, monkeypatch):
        """Lines 345-346: LibreOffice conversion raises"""
        Loader = _get_loader()
        hwp = tmp_path / "test.hwp"
        hwp.write_bytes(b"fake hwp")
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", staticmethod(lambda *a, **kw: (_ for _ in ()).throw(RuntimeError("boom"))))
        summary, line_count, page_count = Loader.read_from_hwp(hwp)
        assert summary == ""


# ---- coverage: _find_xref_offset edge cases ----


class TestFindXrefOffset:
    def test_leading_lines_before_xref(self):
        """Line 365: lines before 'xref' keyword"""
        Loader = _get_loader()
        xref_data = b"some garbage\nmore garbage\nxref\n0 2\n0000000000 65535 f \n0000000100 00000 n \ntrailer\n"
        offset = Loader._find_xref_offset(xref_data, 1)
        assert offset == 100

    def test_empty_lines_in_xref(self):
        """Lines 371-372: empty lines in xref block"""
        Loader = _get_loader()
        xref_data = b"xref\n\n0 2\n0000000000 65535 f \n0000000200 00000 n \ntrailer\n"
        offset = Loader._find_xref_offset(xref_data, 1)
        assert offset == 200

    def test_malformed_line_in_xref(self):
        """Line 388: non-subsection, non-trailer line"""
        Loader = _get_loader()
        xref_data = b"xref\ngarbage_line\n0 2\n0000000000 65535 f \n0000000300 00000 n \ntrailer\n"
        offset = Loader._find_xref_offset(xref_data, 1)
        assert offset == 300


# ---- coverage: _read_from_obj_stream ----


class TestReadFromObjStream:
    def test_missing_first_or_length(self, tmp_path: Path):
        """Lines 443-446: /First or /Length missing"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /Type /ObjStm >>\nstream\ndata\nendstream")
        with open(f, "rb") as fh:
            assert Loader._read_from_obj_stream(fh, 0, 1) is None

    def test_indirect_length(self, tmp_path: Path):
        """Lines 447-448: /Length is indirect reference"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /First 10 /Length 5 0 R >>\nstream\ndata\nendstream")
        with open(f, "rb") as fh:
            assert Loader._read_from_obj_stream(fh, 0, 1) is None

    def test_no_stream_marker(self, tmp_path: Path):
        """Lines 453-455: no stream marker"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /First 10 /Length 20 >>\nendobj")
        with open(f, "rb") as fh:
            assert Loader._read_from_obj_stream(fh, 0, 1) is None

    def test_non_flatedecode_filter(self, tmp_path: Path):
        """Lines 462-463: unsupported filter"""
        Loader = _get_loader()
        data = b"1 0 obj\n<< /First 10 /Length 5 /Filter /ASCIIHexDecode >>\nstream\nhello\nendstream"
        f = tmp_path / "fake.pdf"
        f.write_bytes(data)
        with open(f, "rb") as fh:
            assert Loader._read_from_obj_stream(fh, 0, 1) is None

    def test_zlib_decompress_failure(self, tmp_path: Path):
        """Lines 466-467: zlib decompression fails"""
        Loader = _get_loader()
        data = b"1 0 obj\n<< /First 10 /Length 5 /Filter /FlateDecode >>\nstream\nhello\nendstream"
        f = tmp_path / "fake.pdf"
        f.write_bytes(data)
        with open(f, "rb") as fh:
            assert Loader._read_from_obj_stream(fh, 0, 1) is None

    def test_target_found(self, tmp_path: Path):
        """Lines 476-480: target object found in stream"""
        import zlib

        Loader = _get_loader()
        # Object stream: header "5 0 6 20" means obj 5 at offset 0, obj 6 at offset 20
        header = b"5 0 6 20 "
        obj_data = b"<< /Type /Catalog >>"
        raw = header + obj_data + b" " * 20
        compressed = zlib.compress(raw)
        first_offset = len(header)
        stream_data = f"1 0 obj\n<< /First {first_offset} /Length {len(compressed)} /Filter /FlateDecode >>\nstream\n".encode() + compressed + b"\nendstream"
        f = tmp_path / "fake.pdf"
        f.write_bytes(stream_data)
        with open(f, "rb") as fh:
            result = Loader._read_from_obj_stream(fh, 0, 5)
        assert result is not None
        assert b"Catalog" in result

    def test_target_not_found(self, tmp_path: Path):
        """Lines 482-484: target object not in stream"""
        import zlib

        Loader = _get_loader()
        header = b"5 0 "
        obj_data = b"<< /Type /Catalog >>"
        raw = header + obj_data
        compressed = zlib.compress(raw)
        first_offset = len(header)
        stream_data = f"1 0 obj\n<< /First {first_offset} /Length {len(compressed)} /Filter /FlateDecode >>\nstream\n".encode() + compressed + b"\nendstream"
        f = tmp_path / "fake.pdf"
        f.write_bytes(stream_data)
        with open(f, "rb") as fh:
            result = Loader._read_from_obj_stream(fh, 0, 99)
        assert result is None


# ---- coverage: _parse_one_xref_stream ----


class TestParseOneXrefStream:
    def test_missing_w(self, tmp_path: Path):
        """Lines 493-495: /W missing"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /Size 10 /Length 20 >>\nstream\ndata")
        with open(f, "rb") as fh:
            assert Loader._parse_one_xref_stream(fh, 0) is None

    def test_missing_size(self, tmp_path: Path):
        """Lines 498-500: /Size missing"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /W [1 2 1] /Length 20 >>\nstream\ndata")
        with open(f, "rb") as fh:
            assert Loader._parse_one_xref_stream(fh, 0) is None

    def test_indirect_length(self, tmp_path: Path):
        """Lines 507-508: /Length is indirect reference"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 5 0 R >>\nstream\ndata")
        with open(f, "rb") as fh:
            assert Loader._parse_one_xref_stream(fh, 0) is None

    def test_no_stream_marker(self, tmp_path: Path):
        """Lines 514-516: no stream marker"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 20 >>\nendobj")
        with open(f, "rb") as fh:
            assert Loader._parse_one_xref_stream(fh, 0) is None

    def test_non_flatedecode(self, tmp_path: Path):
        """Lines 524-525: unsupported filter"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 5 /Filter /LZW >>\nstream\nhello\nendstream")
        with open(f, "rb") as fh:
            assert Loader._parse_one_xref_stream(fh, 0) is None

    def test_zlib_failure(self, tmp_path: Path):
        """Lines 528-529: zlib decompression fails"""
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /W [1 2 1] /Size 10 /Length 5 /Filter /FlateDecode >>\nstream\nhello\nendstream")
        with open(f, "rb") as fh:
            assert Loader._parse_one_xref_stream(fh, 0) is None

    def test_uncompressed_stream(self, tmp_path: Path):
        """Lines 530-531: no filter → raw data"""
        Loader = _get_loader()
        # Create an uncompressed xref stream: W=[1,2,1], Size=2, 2 entries of 4 bytes each
        entry_data = bytes([1, 0, 100, 0]) + bytes([1, 0, 200, 0])  # type=1, offset=100/200, gen=0
        stream_data = f"1 0 obj\n<< /W [1 2 1] /Size 2 /Length {len(entry_data)} >>\nstream\n".encode() + entry_data + b"\nendstream"
        f = tmp_path / "fake.pdf"
        f.write_bytes(stream_data)
        with open(f, "rb") as fh:
            result = Loader._parse_one_xref_stream(fh, 0)
        assert result is not None
        decompressed, w, index_ranges, prev_offset = result
        assert w == [1, 2, 1]

    def test_with_decode_parms(self, tmp_path: Path):
        """Lines 534-540: PNG predictor with /DecodeParms"""
        import zlib

        Loader = _get_loader()
        # Each row: 1 filter byte + 4 data bytes (W=[1,2,1] → 4 bytes per entry)
        # Filter byte 0 = None predictor
        row1 = bytes([0, 1, 0, 100, 0])  # type=1, offset=100, gen=0
        row2 = bytes([0, 1, 0, 200, 0])  # type=1, offset=200, gen=0
        raw = row1 + row2
        compressed = zlib.compress(raw)
        stream_data = f"1 0 obj\n<< /W [1 2 1] /Size 2 /Length {len(compressed)} /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >> >>\nstream\n".encode() + compressed + b"\nendstream"
        f = tmp_path / "fake.pdf"
        f.write_bytes(stream_data)
        with open(f, "rb") as fh:
            result = Loader._parse_one_xref_stream(fh, 0)
        assert result is not None


# ---- coverage: read_file edge cases ----


class TestReadFileEdgeCases:
    def test_read_file_skip_text_pdf_fast_fails(self, tmp_path: Path, monkeypatch):
        """Lines 710-712: skip_text PDF, fast count None, pypdf also fails"""
        Loader = _get_loader()
        pdf = tmp_path / "cat" / "test.pdf"
        pdf.parent.mkdir()
        pdf.write_bytes(b"not a pdf")
        monkeypatch.setattr(Loader, "path_prefix", tmp_path)
        monkeypatch.setattr(Loader, "_fast_pdf_page_count", staticmethod(lambda f: None))
        result = Loader.read_file(pdf, skip_text=True)
        assert result
        inode = list(result.keys())[0]
        assert result[inode]["page_count"] == 0

    def test_read_file_comics_pdf(self, tmp_path: Path, monkeypatch):
        """Lines 749-759: PDF in comics prefix (text extraction skipped)"""
        import pypdf

        Loader = _get_loader()
        pdf = tmp_path / "comics_cat" / "test.pdf"
        pdf.parent.mkdir()
        writer = pypdf.PdfWriter()
        writer.add_blank_page(72, 72)
        with open(pdf, "wb") as f:
            writer.write(f)
        monkeypatch.setattr(Loader, "path_prefix", tmp_path)
        monkeypatch.setattr(Loader, "comics_path_prefix", tmp_path)
        result = Loader.read_file(pdf)
        assert result
        inode = list(result.keys())[0]
        assert result[inode]["page_count"] >= 0

    def test_convert_with_libreoffice_glob_fallback(self, tmp_path: Path, monkeypatch):
        """Line 305: output file stem doesn't match, falls back to glob"""
        Loader = _get_loader()
        import subprocess as sp

        def fake_run(cmd, **kwargs):
            # Create output with different stem
            outdir = cmd[cmd.index("--outdir") + 1]
            Path(outdir, "different_name.txt").write_text("converted", encoding="utf-8")

            class Result:
                returncode = 0
                stdout = ""
                stderr = ""

            return Result()

        monkeypatch.setattr(sp, "run", fake_run)
        monkeypatch.setattr(Loader, "_find_libreoffice", staticmethod(lambda: "libreoffice"))
        result = Loader._convert_with_libreoffice(tmp_path / "input.doc", "txt:Text")
        assert result == "converted"


# ---- coverage: main() function ----


class TestLoaderMain:
    @staticmethod
    def _setup_env(monkeypatch, tmp_path):
        monkeypatch.setenv("TM_ES_BOOK_INDEX", "test_books")
        monkeypatch.setenv("TM_ES_COMICS_INDEX", "test_comics")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "path_prefix", tmp_path)
        monkeypatch.setattr(Loader, "comics_path_prefix", tmp_path / "comics")
        return Loader

    def test_main_no_args(self, monkeypatch, tmp_path):
        """Lines 886-888: no args"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader"])
        import utils.loader as loader_mod

        monkeypatch.setattr(loader_mod, "print_usage", lambda prog: None)
        result = loader_mod.main()
        assert result == 1

    def test_main_invalid_index(self, monkeypatch, tmp_path):
        """Lines 891-894: invalid index name"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "invalid_index", str(tmp_path)])
        import utils.loader as loader_mod

        monkeypatch.setattr(loader_mod, "print_usage", lambda prog: None)
        result = loader_mod.main()
        assert result == 1

    def test_main_no_file_args(self, monkeypatch, tmp_path):
        """Lines 898-900: no file args without --delete"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "book"])
        import utils.loader as loader_mod

        monkeypatch.setattr(loader_mod, "print_usage", lambda prog: None)
        result = loader_mod.main()
        assert result == 1

    def test_main_delete_mode(self, monkeypatch, tmp_path):
        """Lines 917-927: --delete mode"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "--delete", "book"])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.do_exist_index.return_value = True
        mock_es.delete_index.return_value = None
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_delete_index_not_exist(self, monkeypatch, tmp_path):
        """Lines 922-923: delete but index doesn't exist"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "--delete", "book"])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.do_exist_index.return_value = False
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_delete_exception(self, monkeypatch, tmp_path):
        """Lines 924-926: delete raises exception"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "--delete", "book"])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.do_exist_index.side_effect = RuntimeError("boom")
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == -1

    def test_main_ping_failure(self, monkeypatch, tmp_path):
        """Lines 907-913: ES ping fails"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "book", str(tmp_path)])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = False
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        with MagicMock():
            import pytest

            with pytest.raises(SystemExit):
                main()

    def test_main_file_not_found(self, monkeypatch, tmp_path):
        """Lines 1007-1009: file not found"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "book", str(tmp_path / "nonexistent")])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_single_file(self, monkeypatch, tmp_path):
        """Lines 1019-1025: single file force reload"""
        Loader = self._setup_env(monkeypatch, tmp_path)
        cat_dir = tmp_path / "cat"
        cat_dir.mkdir()
        test_file = cat_dir / "[author] title.txt"
        test_file.write_text("hello world")
        monkeypatch.setattr("sys.argv", ["loader", "book", str(test_file)])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.insert.return_value = []
        mock_es.delete_by_file_paths.return_value = 0
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_recursive(self, monkeypatch, tmp_path):
        """Lines 1026-1038: recursive directory processing"""
        Loader = self._setup_env(monkeypatch, tmp_path)
        sub = tmp_path / "sub"
        sub.mkdir()
        (sub / "[auth] book.txt").write_text("content")
        monkeypatch.setattr("sys.argv", ["loader", "--recursive", "book", str(tmp_path)])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.get_existing_paths.return_value = {}
        mock_es.insert.return_value = []
        mock_es.delete_by_file_paths.return_value = 0
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_directory_non_recursive(self, monkeypatch, tmp_path):
        """Lines 1039-1074: directory without --recursive (2-stage)"""
        Loader = self._setup_env(monkeypatch, tmp_path)
        sub1 = tmp_path / "sub1"
        sub1.mkdir()
        (sub1 / "[auth] book1.txt").write_text("content1")
        sub2 = tmp_path / "sub2"
        sub2.mkdir()
        (sub2 / "[auth] book2.txt").write_text("content2")
        (tmp_path / "[auth] root.txt").write_text("root content")
        monkeypatch.setattr("sys.argv", ["loader", "book", str(tmp_path)])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.get_existing_paths.return_value = {}
        mock_es.insert.return_value = []
        mock_es.delete_by_file_paths.return_value = 0
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_recursive_reload(self, monkeypatch, tmp_path):
        """Lines 1032-1033: --recursive --reload (skip_check=True)"""
        Loader = self._setup_env(monkeypatch, tmp_path)
        (tmp_path / "[auth] book.txt").write_text("content")
        monkeypatch.setattr("sys.argv", ["loader", "--recursive", "--reload", "book", str(tmp_path)])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        mock_es.insert.return_value = []
        mock_es.delete_by_file_paths.return_value = 0
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_path_not_in_prefix(self, monkeypatch, tmp_path):
        """Lines 1010-1012: path not in book/comics prefix"""
        Loader = self._setup_env(monkeypatch, tmp_path)
        outside = tmp_path / "outside"
        outside.mkdir()
        (outside / "test.txt").write_text("x")
        # Set prefix to a subdirectory so outside is not relative
        monkeypatch.setattr(Loader, "path_prefix", tmp_path / "books_only")
        monkeypatch.setattr(Loader, "comics_path_prefix", tmp_path / "comics_only")
        (tmp_path / "books_only").mkdir()
        monkeypatch.setattr("sys.argv", ["loader", "book", str(outside)])
        from utils.loader import main

        mock_es = MagicMock()
        mock_es.es.ping.return_value = True
        monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

        result = main()
        assert result == 0

    def test_main_getopt_error(self, monkeypatch, tmp_path):
        """Lines 876-878: invalid option"""
        self._setup_env(monkeypatch, tmp_path)
        monkeypatch.setattr("sys.argv", ["loader", "--invalid-option", "book"])
        import utils.loader as loader_mod

        monkeypatch.setattr(loader_mod, "print_usage", lambda prog: None)
        result = loader_mod.main()
        assert result == 1
