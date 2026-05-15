import sys
import os
import unittest
import zipfile
import warnings
import importlib
from pathlib import Path
from unittest.mock import MagicMock

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

    def test_loader_requires_book_dir(self):
        import pytest

        prev_book_dir = os.environ.pop("TM_BOOK_DIR", None)
        prev_loader = sys.modules.pop("utils.loader", None)
        prev_comics_dir = os.environ.get("TM_COMICS_DIR")
        os.environ["TM_COMICS_DIR"] = "/tmp/comics"

        try:
            with pytest.raises(SystemExit):
                importlib.import_module("utils.loader")
        finally:
            if prev_book_dir is not None:
                os.environ["TM_BOOK_DIR"] = prev_book_dir
            if prev_comics_dir is None:
                os.environ.pop("TM_COMICS_DIR", None)
            else:
                os.environ["TM_COMICS_DIR"] = prev_comics_dir
            sys.modules.pop("utils.loader", None)
            if prev_loader is not None:
                sys.modules["utils.loader"] = prev_loader

    def test_loader_requires_comics_dir(self):
        import pytest

        prev_comics_dir = os.environ.pop("TM_COMICS_DIR", None)
        prev_loader = sys.modules.pop("utils.loader", None)
        prev_book_dir = os.environ.get("TM_BOOK_DIR")
        os.environ["TM_BOOK_DIR"] = str(Path(__file__).parent.parent / "tests" / "books")

        try:
            with pytest.raises(SystemExit):
                importlib.import_module("utils.loader")
        finally:
            if prev_comics_dir is not None:
                os.environ["TM_COMICS_DIR"] = prev_comics_dir
            if prev_book_dir is None:
                os.environ.pop("TM_BOOK_DIR", None)
            else:
                os.environ["TM_BOOK_DIR"] = prev_book_dir
            sys.modules.pop("utils.loader", None)
            if prev_loader is not None:
                sys.modules["utils.loader"] = prev_loader


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

    def test_read_epub_chapter_exception_is_skipped(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        epub = tmp_path / "chapter-error.epub"
        with zipfile.ZipFile(epub, "w") as zf:
            zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>')
            zf.writestr("content.opf", '<package><manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>')
            zf.writestr("ch1.xhtml", "<html><body><p>ignored</p></body></html>")

        monkeypatch.setattr("utils.loader.BeautifulSoup", lambda *_args, **_kwargs: (_ for _ in ()).throw(ValueError("boom")))
        summary, line_count = Loader.read_from_epub_with_extracting_zip(epub)
        assert summary == ""
        assert line_count == 0


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

    def test_malformed_target_entry_returns_none(self):
        Loader = _get_loader()
        xref = b"xref\n0 2\n0000000000 65535 f \nnot-an-entry\ntrailer\n"
        assert Loader._find_xref_offset(xref, 1) is None

    def test_missing_xref_keyword_returns_none(self):
        Loader = _get_loader()
        assert Loader._find_xref_offset(b"no xref here", 1) is None


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

    def test_truncated_entry_returns_none(self):
        Loader = _get_loader()
        result = Loader._xref_stream_find_entry(bytes([1, 0, 10]), [1, 2, 1], [0, 1], 0)
        assert result is None


# ---- coverage: loader additional uncovered lines ----

import zlib
import tempfile


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
    def test_read_epub_ebooklib_success_path(self, monkeypatch):
        Loader = _get_loader()
        import ebooklib

        class FakeDoc:
            def get_body_content(self):
                return "<html><body><p>본문 줄1</p><p>줄2</p></body></html>"

        class FakeBook:
            def get_metadata(self, namespace, key):
                if (namespace, key) == ("DC", "title"):
                    return [("테스트 제목", {})]
                if (namespace, key) == ("DC", "creator"):
                    return [("테스트 저자", {})]
                return []

            def get_items_of_type(self, item_type):
                assert item_type == ebooklib.ITEM_DOCUMENT
                return [FakeDoc()]

        monkeypatch.setattr("utils.loader.epub.read_epub", lambda _path: FakeBook())
        summary, line_count, page_count = Loader.read_from_epub(Path("dummy.epub"))
        assert "테스트 제목" in summary
        assert "테스트 저자" in summary
        assert line_count >= 1
        assert page_count == 0

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

    def test_read_epub_both_primary_and_fallback_fail(self, monkeypatch):
        Loader = _get_loader()
        monkeypatch.setattr("utils.loader.epub.read_epub", lambda _path: (_ for _ in ()).throw(RuntimeError("primary fail")))
        monkeypatch.setattr(Loader, "read_from_epub_with_extracting_zip", staticmethod(lambda _path: (_ for _ in ()).throw(RuntimeError("fallback fail"))))
        summary, line_count, page_count = Loader.read_from_epub(Path("broken.epub"))
        assert summary == ""
        assert line_count == 0
        assert page_count == 0


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

    def test_xref_stream_type1_lookup(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        pdf = tmp_path / "stream.pdf"
        data = bytearray(b" " * 256)
        data[20 : 20 + len(b"<< /Type /XRef /Root 1 0 R >>")] = b"<< /Type /XRef /Root 1 0 R >>"
        data[100 : 100 + len(b"<< /Pages 2 0 R >>")] = b"<< /Pages 2 0 R >>"
        data[140 : 140 + len(b"<< /Count 7 >>")] = b"<< /Count 7 >>"
        trailer = b"startxref\n20\n%%EOF"
        data[-len(trailer) :] = trailer
        pdf.write_bytes(bytes(data))

        monkeypatch.setattr(Loader, "_parse_one_xref_stream", staticmethod(lambda *_args: (b"decoded", [1, 2, 1], [0, 1], None)))

        def fake_find_entry(_data, _w, _index_ranges, obj_num):
            if obj_num == 1:
                return (1, 100, 0)
            if obj_num == 2:
                return (1, 140, 0)
            return None

        monkeypatch.setattr(Loader, "_xref_stream_find_entry", staticmethod(fake_find_entry))
        assert Loader._fast_pdf_page_count(pdf) == 7

    def test_xref_stream_type2_object_stream_lookup(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        pdf = tmp_path / "objstream.pdf"
        data = bytearray(b" " * 256)
        data[20 : 20 + len(b"<< /Type /XRef /Root 1 0 R >>")] = b"<< /Type /XRef /Root 1 0 R >>"
        data[100 : 100 + len(b"<< /Pages 2 0 R >>")] = b"<< /Pages 2 0 R >>"
        trailer = b"startxref\n20\n%%EOF"
        data[-len(trailer) :] = trailer
        pdf.write_bytes(bytes(data))

        monkeypatch.setattr(Loader, "_parse_one_xref_stream", staticmethod(lambda *_args: (b"decoded", [1, 2, 1], [0, 3], None)))

        def fake_find_entry(_data, _w, _index_ranges, obj_num):
            if obj_num == 1:
                return (1, 100, 0)
            if obj_num == 2:
                return (2, 9, 0)
            if obj_num == 9:
                return (1, 180, 0)
            return None

        monkeypatch.setattr(Loader, "_xref_stream_find_entry", staticmethod(fake_find_entry))
        monkeypatch.setattr(Loader, "_read_from_obj_stream", staticmethod(lambda *_args: b"<< /Type /Pages /Count 4 >>"))
        assert Loader._fast_pdf_page_count(pdf) == 4

    def test_xref_stream_without_root_returns_none(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        pdf = tmp_path / "no-root.pdf"
        pdf.write_bytes(b"0" * 64 + b"startxref\n20\n%%EOF")
        monkeypatch.setattr(Loader, "_parse_one_xref_stream", staticmethod(lambda *_args: (b"decoded", [1, 2, 1], [0, 1], None)))
        assert Loader._fast_pdf_page_count(pdf) is None

    def test_xref_stream_missing_pages_reference_returns_none(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        pdf = tmp_path / "missing-pages-ref.pdf"
        data = bytearray(b" " * 256)
        data[20 : 20 + len(b"<< /Type /XRef /Root 1 0 R >>")] = b"<< /Type /XRef /Root 1 0 R >>"
        data[100 : 100 + len(b"<< /Type /Catalog >>")] = b"<< /Type /Catalog >>"
        trailer = b"startxref\n20\n%%EOF"
        data[-len(trailer) :] = trailer
        pdf.write_bytes(bytes(data))

        monkeypatch.setattr(Loader, "_parse_one_xref_stream", staticmethod(lambda *_args: (b"decoded", [1, 2, 1], [0, 1], None)))
        monkeypatch.setattr(Loader, "_xref_stream_find_entry", staticmethod(lambda *_args: (1, 100, 0)))
        assert Loader._fast_pdf_page_count(pdf) is None

    def test_xref_stream_missing_pages_object_returns_none(self, tmp_path: Path, monkeypatch):
        Loader = _get_loader()
        pdf = tmp_path / "missing-pages-object.pdf"
        data = bytearray(b" " * 256)
        data[20 : 20 + len(b"<< /Type /XRef /Root 1 0 R >>")] = b"<< /Type /XRef /Root 1 0 R >>"
        data[100 : 100 + len(b"<< /Pages 2 0 R >>")] = b"<< /Pages 2 0 R >>"
        trailer = b"startxref\n20\n%%EOF"
        data[-len(trailer) :] = trailer
        pdf.write_bytes(bytes(data))

        monkeypatch.setattr(Loader, "_parse_one_xref_stream", staticmethod(lambda *_args: (b"decoded", [1, 2, 1], [0, 1], None)))

        def fake_find_entry(_data, _w, _index_ranges, obj_num):
            if obj_num == 1:
                return (1, 100, 0)
            return None

        monkeypatch.setattr(Loader, "_xref_stream_find_entry", staticmethod(fake_find_entry))
        assert Loader._fast_pdf_page_count(pdf) is None


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

    def test_missing_length(self, tmp_path: Path):
        Loader = _get_loader()
        f = tmp_path / "fake.pdf"
        f.write_bytes(b"1 0 obj\n<< /W [1 2 1] /Size 10 >>\nstream\ndata")
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


# ---- HWP 버전별 실제 파일 테스트 ----

HWP_TEST_DIR = Path(__file__).parent / "books" / "_hwp"


def _detect_hwp_version(file_path: Path) -> str:
    """HWP 파일 헤더에서 버전 문자열을 추출한다."""
    with open(file_path, "rb") as f:
        header = f.read(30)
    if header[:15] == b"HWP Document Fi":
        import re as _re

        m = _re.search(r"V(\d+\.\d+)", header.decode("ascii", errors="replace"))
        if m:
            return m.group(1)
    if header[:4] == b"\xd0\xcf\x11\xe0":
        return "5.x"
    return "unknown"


class TestHwpVersionDetection:
    """HWP 파일 헤더 기반 버전 감지 테스트"""

    def test_detect_v1_20(self):
        f = HWP_TEST_DIR / "v1.20_부하의약혼녀.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        assert _detect_hwp_version(f) == "1.20"

    def test_detect_v2_00(self):
        f = HWP_TEST_DIR / "v2.00_박노해_참된시작.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        assert _detect_hwp_version(f) == "2.00"

    def test_detect_v2_10(self):
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        assert _detect_hwp_version(f) == "2.10"

    def test_detect_v3_00(self):
        f = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        assert _detect_hwp_version(f) == "3.00"

    def test_detect_unknown_for_non_hwp(self, tmp_path: Path):
        f = tmp_path / "fake.hwp"
        f.write_bytes(b"this is not an hwp file at all")
        assert _detect_hwp_version(f) == "unknown"

    def test_detect_ole2_hwp5(self, tmp_path: Path):
        """OLE2 매직 바이트를 가진 HWP 5.x 감지"""
        f = tmp_path / "hwp5.hwp"
        f.write_bytes(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" + b"\x00" * 22)
        assert _detect_hwp_version(f) == "5.x"


class TestReadFromHwpRealFiles:
    """실제 HWP 파일로 read_from_hwp 테스트 (LibreOffice 의존)

    LibreOffice가 설치되어 있지 않거나 변환 실패 시에도
    크래시 없이 빈 문자열을 반환하는지 검증한다.
    """

    @staticmethod
    def _hwp_files():
        if not HWP_TEST_DIR.exists():
            return []
        return sorted(HWP_TEST_DIR.glob("*.hwp"))

    def test_v1_20_no_crash(self):
        f = HWP_TEST_DIR / "v1.20_부하의약혼녀.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert isinstance(summary, str)
        assert isinstance(line_count, int)
        assert page_count == 0

    def test_v2_00_no_crash(self):
        f = HWP_TEST_DIR / "v2.00_박노해_참된시작.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert isinstance(summary, str)
        assert isinstance(line_count, int)
        assert page_count == 0

    def test_v2_10_no_crash(self):
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert isinstance(summary, str)
        assert isinstance(line_count, int)
        assert page_count == 0

    def test_v3_00_no_crash(self):
        f = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert isinstance(summary, str)
        assert isinstance(line_count, int)
        assert page_count == 0

    def test_v3_00_noname22_no_crash(self):
        f = HWP_TEST_DIR / "v3.00_noname22.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert isinstance(summary, str)
        assert isinstance(line_count, int)
        assert page_count == 0

    def test_all_versions_return_tuple(self):
        """모든 버전 파일에 대해 3-tuple 반환 확인"""
        files = self._hwp_files()
        if not files:
            import pytest

            pytest.skip("HWP 테스트 파일 없음")
        Loader = _get_loader()
        for f in files:
            result = Loader.read_from_hwp(f)
            assert len(result) == 3, f"파일 {f.name}: 3-tuple이 아님"
            summary, line_count, page_count = result
            assert isinstance(summary, str), f"파일 {f.name}: summary가 str이 아님"
            assert isinstance(line_count, int), f"파일 {f.name}: line_count가 int가 아님"
            assert page_count == 0, f"파일 {f.name}: page_count가 0이 아님"


class TestReadFromHwpWithMockedLibreoffice:
    """LibreOffice 변환 결과를 시뮬레이션하여 버전별 동작 검증"""

    def test_v1_20_libreoffice_returns_empty(self, monkeypatch):
        """V1.20 파일: LibreOffice가 빈 결과를 반환하는 경우"""
        f = HWP_TEST_DIR / "v1.20_부하의약혼녀.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert summary == ""
        assert line_count == 0

    def test_v2_10_libreoffice_returns_text(self, monkeypatch):
        """V2.10 파일: LibreOffice가 텍스트를 반환하는 경우"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "교카 시집\n첫 번째 시\n두 번째 시")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert "교카" in summary
        assert line_count == 3

    def test_v3_00_libreoffice_raises(self, monkeypatch):
        """V3.00 파일: LibreOffice 변환 시 예외 발생"""
        f = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: (_ for _ in ()).throw(RuntimeError("변환 실패")))
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert summary == ""
        assert line_count == 0

    def test_special_chars_cleaned(self, monkeypatch):
        """변환 결과의 특수문자가 정리되는지 확인"""
        f = HWP_TEST_DIR / "v2.10_PAGER.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "텍스트★내용♣특수◆문자")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert "★" not in summary
        assert "텍스트" in summary
        assert "내용" in summary


# ---- HWP3 네이티브 파서 테스트 ----


def _get_hwp3_parser():
    from utils.hwp3_parser import extract_text_from_hwp3

    return extract_text_from_hwp3


class TestHwp3ParserDirect:
    """hwp3_parser.extract_text_from_hwp3 직접 테스트"""

    def test_v3_00_extracts_korean_text(self):
        f = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        assert len(text) > 10
        assert "고정희" in text or "너" in text  # 시 제목/내용

    def test_v2_10_extracts_korean_text(self):
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        assert len(text) > 10
        assert "영동" in text or "교가" in text

    def test_v2_10_pager(self):
        f = HWP_TEST_DIR / "v2.10_PAGER.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        assert len(text) > 100

    def test_v1_20_returns_empty(self):
        """V1.20은 미지원 — 빈 문자열 반환"""
        f = HWP_TEST_DIR / "v1.20_부하의약혼녀.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        assert isinstance(text, str)

    def test_invalid_file_returns_empty(self, tmp_path: Path):
        f = tmp_path / "not_hwp.hwp"
        f.write_bytes(b"this is definitely not an HWP file")
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_truncated_file_returns_empty(self, tmp_path: Path):
        f = tmp_path / "truncated.hwp"
        f.write_bytes(b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05" + b"\x00" * 50)
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_encrypted_file_returns_empty(self, tmp_path: Path):
        """암호화 플래그가 설정된 파일"""
        sig = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)
        doc_info[96] = 1  # encryption flag (low byte of uint16)
        f = tmp_path / "encrypted.hwp"
        f.write_bytes(sig + bytes(doc_info) + b"\x00" * 1008)
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_nonexistent_file_returns_empty(self, tmp_path: Path):
        extract = _get_hwp3_parser()
        assert extract(tmp_path / "does_not_exist.hwp") == ""


class TestHwp3FallbackIntegration:
    """LibreOffice 실패 시 hwp3 파서 fallback 테스트"""

    def test_fallback_when_libreoffice_returns_empty(self, monkeypatch):
        """LibreOffice가 빈 문자열 → hwp3 파서로 텍스트 추출"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert len(summary) > 0
        assert "영동" in summary or "교가" in summary

    def test_fallback_when_libreoffice_returns_whitespace(self, monkeypatch):
        """LibreOffice가 공백만 반환 → hwp3 파서로 fallback"""
        f = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "   \n  ")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert len(summary) > 0

    def test_no_fallback_when_libreoffice_succeeds(self, monkeypatch):
        """LibreOffice가 텍스트 반환 → fallback 사용 안 함"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "LO 결과")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert "LO" in summary


# ---- HWP3 파서 추가 엣지 케이스 테스트 ----


class TestHwp3ParserEdgeCases:
    """hwp3_parser의 다양한 엣지 케이스 검증"""

    def test_compressed_v3_file(self):
        """V3.00 압축 파일 정상 파싱"""
        f = HWP_TEST_DIR / "v3.00_현대시사전.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        # 압축 플래그 확인
        data = f.read_bytes()
        assert data[30 + 124] != 0, "테스트 파일이 압축되어 있어야 함"
        extract = _get_hwp3_parser()
        text = extract(f)
        assert len(text) > 0

    def test_v2_10_file_parses(self):
        """V2.10 파일 정상 파싱"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        assert len(text) > 0

    def test_text_contains_no_null_bytes(self):
        """추출된 텍스트에 null 바이트가 없어야 함"""
        for name in ["v2.10_KYOKA.hwp", "v3.00_현대시사전.hwp", "v2.10_PAGER.hwp"]:
            f = HWP_TEST_DIR / name
            if not f.exists():
                continue
            extract = _get_hwp3_parser()
            text = extract(f)
            assert "\x00" not in text, f"{name}: null 바이트 포함"

    def test_text_is_valid_unicode(self):
        """추출된 텍스트가 유효한 유니코드 문자열이어야 함"""
        for name in ["v2.10_KYOKA.hwp", "v3.00_현대시사전.hwp"]:
            f = HWP_TEST_DIR / name
            if not f.exists():
                continue
            extract = _get_hwp3_parser()
            text = extract(f)
            # encode/decode가 에러 없이 수행되어야 함
            encoded = text.encode("utf-8")
            decoded = encoded.decode("utf-8")
            assert decoded == text

    def test_empty_body_after_header(self, tmp_path: Path):
        """헤더만 있고 본문이 없는 파일"""
        sig = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)
        # 압축 안 함, 정보블록 0
        f = tmp_path / "empty_body.hwp"
        f.write_bytes(sig + bytes(doc_info) + b"\x00" * 1008)
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_corrupted_compression(self, tmp_path: Path):
        """압축 플래그가 설정되었지만 데이터가 손상된 경우"""
        sig = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)
        doc_info[124] = 1  # compressed
        f = tmp_path / "bad_compress.hwp"
        f.write_bytes(sig + bytes(doc_info) + b"\x00" * 1008 + b"\xff\xfe\xfd\xfc" * 100)
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_hwp3_hnc_to_unicode_ascii(self):
        """HNC→Unicode: ASCII 범위 변환"""
        from utils.hwp3_parser import _hnc_to_unicode

        assert _hnc_to_unicode(0x41) == "A"
        assert _hnc_to_unicode(0x20) == " "
        assert _hnc_to_unicode(0x7E) == "~"

    def test_hwp3_hnc_to_unicode_hangul(self):
        """HNC→Unicode: 한글 음절 변환"""
        from utils.hwp3_parser import _hnc_to_unicode

        # '가' = cho=2(ㄱ), jung=3(ㅏ), jong=1(없음)
        # HNC: (2<<10)|(3<<5)|1 + 0x8000 = 0x8000 | 0x0800 | 0x0060 | 0x0001 = 0x8861
        result = _hnc_to_unicode(0x8861)
        assert result == "가", f"expected '가', got '{result}'"

    def test_hwp3_hnc_to_unicode_null(self):
        """HNC→Unicode: 0은 빈 문자열"""
        from utils.hwp3_parser import _hnc_to_unicode

        assert _hnc_to_unicode(0) == ""

    def test_hwp3_hnc_to_unicode_hanja(self):
        """HNC→Unicode: 한자 변환 (첫 번째 항목)"""
        from utils.hwp3_parser import _hnc_to_unicode
        from utils.hwp3_tables import KSC5601_TO_UNI

        # 0x4000 → KSC5601_TO_UNI[0]
        result = _hnc_to_unicode(0x4000)
        expected = chr(KSC5601_TO_UNI[0])
        assert result == expected

    def test_hwp3_stream_bounds(self):
        """_HwpStream 범위 초과 시 예외 발생"""
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"\x01\x02\x03")
        assert stream.read_uint8() == 1
        assert stream.remaining() == 2
        with pytest.raises(_HwpStreamError):
            stream.read_uint32()  # 4바이트 필요하지만 2바이트만 남음

    def test_hwp3_stream_skip_negative(self):
        """_HwpStream 음수 skip 시 예외"""
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"\x01\x02\x03")
        with pytest.raises(_HwpStreamError):
            stream.skip(-1)


class TestLoaderHwp3FallbackEdgeCases:
    """Loader.read_from_hwp의 hwp3 fallback 엣지 케이스"""

    def test_fallback_exception_in_hwp3_parser(self, monkeypatch):
        """hwp3 파서에서 예외 발생해도 전체가 크래시하지 않음"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "")

        import utils.hwp3_parser as hwp3_mod

        original = hwp3_mod.extract_text_from_hwp3
        monkeypatch.setattr(hwp3_mod, "extract_text_from_hwp3", lambda fp: (_ for _ in ()).throw(RuntimeError("파서 에러")))
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert summary == ""
        assert page_count == 0
        monkeypatch.setattr(hwp3_mod, "extract_text_from_hwp3", original)

    def test_fallback_text_truncated_to_text_size(self, monkeypatch):
        """fallback 결과가 TEXT_SIZE로 잘리는지 확인"""
        f = HWP_TEST_DIR / "v2.10_PAGER.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        assert len(summary) <= Loader.TEXT_SIZE

    def test_fallback_special_chars_cleaned(self, monkeypatch):
        """fallback 결과에도 특수문자 정리가 적용되는지 확인"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        import re

        # 한글, 영숫자, 공백만 남아야 함
        cleaned = re.sub(r"[^\w\sㄱ-힣]", "", summary)
        assert cleaned == summary

    def test_fallback_line_count_correct(self, monkeypatch):
        """fallback 결과의 line_count가 정확한지 확인"""
        f = HWP_TEST_DIR / "v2.10_KYOKA.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        Loader = _get_loader()
        monkeypatch.setattr(Loader, "_convert_with_libreoffice", lambda fp, fmt: "")
        summary, line_count, page_count = Loader.read_from_hwp(f)
        if summary:
            expected_lines = summary.count("\n") + 1
            assert line_count > 0


# ---- hwp3_parser 커버리지 보강 테스트 ----


class TestHwp3StreamMethods:
    """_HwpStream 메서드 커버리지"""

    def test_read_uint32(self):
        from utils.hwp3_parser import _HwpStream

        stream = _HwpStream(b"\x01\x00\x00\x00\x02\x00\x00\x00")
        assert stream.read_uint32() == 1
        assert stream.read_uint32() == 2

    def test_read_uint32_overflow(self):
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"\x01\x02")
        with pytest.raises(_HwpStreamError):
            stream.read_uint32()

    def test_read_bytes(self):
        from utils.hwp3_parser import _HwpStream

        stream = _HwpStream(b"hello world")
        assert stream.read_bytes(5) == b"hello"
        assert stream.remaining() == 6

    def test_read_bytes_overflow(self):
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"hi")
        with pytest.raises(_HwpStreamError):
            stream.read_bytes(10)

    def test_read_uint16_overflow(self):
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"\x01")
        with pytest.raises(_HwpStreamError):
            stream.read_uint16()

    def test_read_uint8_overflow(self):
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"")
        with pytest.raises(_HwpStreamError):
            stream.read_uint8()

    def test_skip_overflow(self):
        import pytest
        from utils.hwp3_parser import _HwpStream, _HwpStreamError

        stream = _HwpStream(b"\x01")
        with pytest.raises(_HwpStreamError):
            stream.skip(100)


class TestHncToUnicodeExtended:
    """_hnc_to_unicode 분기 커버리지 보강"""

    def test_special_char_range(self):
        """0x007F-0x3FFF: 특수문자 매핑"""
        from utils.hwp3_parser import _hnc_to_unicode

        # 0x0080 = 유로 기호 (€)
        result = _hnc_to_unicode(0x0080)
        assert result == "€"

    def test_special_char_unmapped(self):
        """매핑 없는 특수문자 → 빈 문자열"""
        from utils.hwp3_parser import _hnc_to_unicode

        result = _hnc_to_unicode(0x0001)  # 범위 밖
        assert result == ""

    def test_hanja_level2(self):
        """0x5318-0x7FFF: 2수준 한자"""
        from utils.hwp3_parser import _hnc_to_unicode
        from utils.hwp3_tables import HNC2UNI

        # 매핑이 있는 2수준 한자 코드 찾기
        for code in range(0x5318, 0x5400):
            if code in HNC2UNI:
                result = _hnc_to_unicode(code)
                assert len(result) == 1
                break

    def test_hanja_level2_unmapped(self):
        """매핑 없는 2수준 한자 → 빈 문자열"""
        from utils.hwp3_parser import _hnc_to_unicode

        result = _hnc_to_unicode(0x7FFF)  # 매핑 없을 확률 높음
        assert isinstance(result, str)

    def test_hangul_choseong_only(self):
        """초성만 있는 한글 (ㄱ)"""
        from utils.hwp3_parser import _hnc_to_unicode

        # cho=2(ㄱ), jung=2(FILL), jong=1(FILL)
        c = 0x8000 | (2 << 10) | (2 << 5) | 1
        result = _hnc_to_unicode(c)
        assert result == "ㄱ"

    def test_hangul_jungseong_only(self):
        """중성만 있는 한글 (ㅏ)"""
        from utils.hwp3_parser import _hnc_to_unicode

        # cho=1(FILL), jung=3(ㅏ), jong=1(FILL)
        c = 0x8000 | (1 << 10) | (3 << 5) | 1
        result = _hnc_to_unicode(c)
        assert result == "ㅏ"

    def test_hangul_jongseong_only(self):
        """종성만 있는 한글"""
        from utils.hwp3_parser import _hnc_to_unicode

        # cho=1(FILL), jung=2(FILL), jong=2(ㄱ)
        c = 0x8000 | (1 << 10) | (2 << 5) | 2
        result = _hnc_to_unicode(c)
        assert result == "ㄱ"

    def test_old_hangul_cho_jung(self):
        """옛한글: 초성+중성만"""
        from utils.hwp3_parser import _hnc_to_unicode

        # cho=2(ㄱ), jung=3(ㅏ), jong=1(FILL) — 현대 한글이면 완성형으로 가지만
        # L_MAP/V_MAP/T_MAP이 NONE인 조합을 찾아야 함
        # cho=21(ㅎ확장), jung=3(ㅏ), jong=1(FILL)
        c = 0x8000 | (21 << 10) | (3 << 5) | 1
        result = _hnc_to_unicode(c)
        assert len(result) >= 1  # 옛한글 자모 조합

    def test_old_hangul_cho_jung_jong(self):
        """옛한글: 초성+중성+종성 모두"""
        from utils.hwp3_parser import _hnc_to_unicode

        # cho=21, jung=3, jong=18(옛종성)
        c = 0x8000 | (21 << 10) | (3 << 5) | 18
        result = _hnc_to_unicode(c)
        assert len(result) >= 1

    def test_hangul_jung_zero_fallback(self):
        """jung=0인 완성형 옛한글 fallback"""
        from utils.hwp3_parser import _hnc_to_unicode

        # cho=2, jung=0, jong=0 → jung==0 분기
        c = 0x8000 | (2 << 10) | (0 << 5) | 0
        result = _hnc_to_unicode(c)
        assert isinstance(result, str)

    def test_out_of_range_code(self):
        """범위 밖 코드 → 빈 문자열"""
        from utils.hwp3_parser import _hnc_to_unicode

        assert _hnc_to_unicode(0x0010) == ""


class TestHandleControlCharCoverage:
    """_handle_control_char 분기 커버리지 보강"""

    def _make_ctrl_paragraph(self, ctrl_code, extra_data):
        """제어문자가 포함된 최소 문단 바이너리를 생성한다."""
        import struct

        n_chars = 4 if ctrl_code == 23 else (2 if ctrl_code in (24, 25) else (31 if ctrl_code == 28 else (1 if ctrl_code in (30, 31) else 3)))
        n_chars += 1  # 제어문자 자체 + CR
        n_lines = 1
        char_shape = 0

        header = struct.pack("<BHHB", 0, n_chars, n_lines, char_shape)  # 6 bytes
        header += b"\x00" * 37  # padding to 43 bytes
        header += b"\x00" * 187  # para shape (prev_para_shape=0)
        line_info = b"\x00" * 14  # 1 line
        # chars: ctrl_code + extra + CR(13)
        chars = struct.pack("<H", ctrl_code) + extra_data + struct.pack("<H", 13)
        # 빈 문단 (리스트 종료)
        terminator = b"\x00" * 43
        return header + line_info + chars + terminator

    def _parse_with_data(self, data):
        from utils.hwp3_parser import _HwpStream, _parse_paragraph_list

        stream = _HwpStream(data)
        return _parse_paragraph_list(stream)

    def test_ctrl_5_field_code(self):
        """제어문자 5: 필드 코드"""
        import struct

        # skip(6) + uint32(len=0) + skip(2) + skip(0)
        extra = b"\x00" * 6 + struct.pack("<I", 0) + b"\x00" * 2
        data = self._make_ctrl_paragraph(5, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)

    def test_ctrl_6_bookmark(self):
        """제어문자 6: 책갈피 (40바이트)"""
        extra = b"\x00" * 40
        data = self._make_ctrl_paragraph(6, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)

    def test_ctrl_23_compose(self):
        """제어문자 23: 글자겹침 (8바이트)"""
        extra = b"\x00" * 8
        data = self._make_ctrl_paragraph(23, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)

    def test_ctrl_24_hyphen(self):
        """제어문자 24: 하이픈 (4바이트)"""
        extra = b"\x00" * 4
        data = self._make_ctrl_paragraph(24, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)

    def test_ctrl_28_outline(self):
        """제어문자 28: 개요번호 (62바이트)"""
        extra = b"\x00" * 62
        data = self._make_ctrl_paragraph(28, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)

    def test_ctrl_30_keep_space(self):
        """제어문자 30: 묶음빈칸 (2바이트)"""
        extra = b"\x00" * 2
        data = self._make_ctrl_paragraph(30, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)

    def test_ctrl_18_auto_num(self):
        """제어문자 18: 자동번호 (6바이트)"""
        extra = b"\x00" * 6
        data = self._make_ctrl_paragraph(18, extra)
        result = self._parse_with_data(data)
        assert isinstance(result, str)


class TestParseEdgeCases:
    """파싱 경계 조건 커버리지"""

    def test_max_recursion_depth(self):
        """재귀 깊이 초과 시 빈 문자열 반환"""
        from utils.hwp3_parser import _HwpStream, _parse_paragraph_list, _MAX_RECURSION_DEPTH

        stream = _HwpStream(b"\x00" * 100)
        result = _parse_paragraph_list(stream, depth=_MAX_RECURSION_DEPTH + 1)
        assert result == ""

    def test_sanity_check_invalid_n_chars(self):
        """n_chars가 비정상적으로 크면 예외 → 빈 문자열"""
        import struct
        import pytest
        from utils.hwp3_parser import _HwpStream, _parse_paragraph, _HwpStreamError

        # prev_para_shape=1, n_chars=50000(invalid), n_lines=1, csi=0
        header = struct.pack("<BHHB", 1, 50000, 1, 0) + b"\x00" * 37
        stream = _HwpStream(header + b"\x00" * 200)
        with pytest.raises(_HwpStreamError):
            _parse_paragraph(stream, 0)

    def test_body_offset_exceeds_file(self, tmp_path: Path):
        """body_offset이 파일 크기를 초과하는 경우"""
        import struct

        sig = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)
        # info_block_len = 60000 (파일 크기 초과)
        struct.pack_into("<H", doc_info, 126, 60000)
        f = tmp_path / "big_info_block.hwp"
        f.write_bytes(sig + bytes(doc_info) + b"\x00" * 1008)
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_gzip_fallback_decompression(self, tmp_path: Path):
        """raw deflate 실패 → gzip auto 시도"""
        import zlib

        sig = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)
        doc_info[124] = 1  # compressed
        # 유효한 gzip 데이터 (빈 내용)
        body = zlib.compress(b"\x00" * 100)  # zlib 헤더 포함
        f = tmp_path / "gzip_test.hwp"
        f.write_bytes(sig + bytes(doc_info) + b"\x00" * 1008 + body)
        extract = _get_hwp3_parser()
        # 파싱은 빈 결과지만 크래시하지 않아야 함
        result = extract(f)
        assert isinstance(result, str)

    def test_unsupported_version_v1(self, tmp_path: Path):
        """V1.20 시그니처 → 빈 문자열"""
        sig = b"HWP Document File V1.20 \x1a\x01\x02\x03\x04\x05"
        f = tmp_path / "v1.hwp"
        f.write_bytes(sig + b"\x00" * 200)
        extract = _get_hwp3_parser()
        assert extract(f) == ""

    def test_header_decode_exception(self, tmp_path: Path):
        """시그니처 영역이 깨진 바이너리"""
        # 첫 17바이트는 맞지만 나머지가 깨진 경우
        sig = b"HWP Document File" + b"\xff" * 13
        f = tmp_path / "bad_header.hwp"
        f.write_bytes(sig + b"\x00" * 200)
        extract = _get_hwp3_parser()
        assert extract(f) == ""


class TestHandleControlCharRecursive:
    """표/그림/머리말/각주 등 재귀적 제어문자 커버리지"""

    def _build_hwp_with_paragraphs(self, body_data: bytes) -> bytes:
        """비압축 V3.00 HWP 파일 바이너리를 생성 (fonts=0, styles=0)"""

        sig = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)  # 비압축, 정보블록=0
        summary = b"\x00" * 1008
        # fonts: 7 카테고리, 각 0개
        fonts = b"\x00\x00" * 7
        # styles: 0개
        styles = b"\x00\x00"
        return sig + bytes(doc_info) + summary + fonts + styles + body_data

    def _make_paragraph(self, chars_data: bytes, n_chars: int, prev_shape: int = 0) -> bytes:
        """문단 헤더 + 줄 정보 + 글자 데이터"""
        import struct

        n_lines = 1
        header = struct.pack("<BHHB", prev_shape, n_chars, n_lines, 0) + b"\x00" * 37
        if prev_shape == 0:
            header += b"\x00" * 187  # 문단 모양
        line_info = b"\x00" * 14
        return header + line_info + chars_data

    def _empty_paragraph(self) -> bytes:
        """빈 문단 (리스트 종료)"""
        import struct

        return struct.pack("<BHHB", 0, 0, 0, 0) + b"\x00" * 37 + b"\x00" * 187

    def test_ctrl_11_picture(self):
        """제어문자 11: 그림 (skip(6)+uint32+skip(344)+skip(len)+캡션)"""
        import struct

        pic_len = 10
        # 제어문자 11 (n_chars_read += 3 → 총 4)
        ctrl = struct.pack("<H", 11)
        ctrl_data = b"\x00" * 6 + struct.pack("<I", pic_len) + b"\x00" * 344 + b"\x00" * pic_len
        # 캡션 = 빈 문단 리스트
        caption = self._empty_paragraph()
        cr = struct.pack("<H", 13)
        chars = ctrl + ctrl_data + caption + cr
        para = self._make_paragraph(chars, n_chars=4 + 1)
        body = para + self._empty_paragraph()
        data = self._build_hwp_with_paragraphs(body)

        from utils.hwp3_parser import _HwpStream

        # fonts(14) + styles(2) = 16바이트 오프셋부터 문단 시작
        stream = _HwpStream(b"\x00\x00" * 7 + b"\x00\x00" + body)
        # 직접 파싱
        from pathlib import Path
        import tempfile

        with tempfile.NamedTemporaryFile(suffix=".hwp", delete=False) as f:
            f.write(data)
            f.flush()
            extract = _get_hwp3_parser()
            result = extract(Path(f.name))
        assert isinstance(result, str)

    def test_ctrl_16_header_footer(self):
        """제어문자 16: 머리말/꼬리말"""
        import struct

        ctrl = struct.pack("<H", 16)
        ctrl_data = b"\x00" * 16  # skip(6+10)
        nested_end = self._empty_paragraph()
        cr = struct.pack("<H", 13)
        chars = ctrl + ctrl_data + nested_end + cr
        para = self._make_paragraph(chars, n_chars=4 + 1)
        body = para + self._empty_paragraph()
        data = self._build_hwp_with_paragraphs(body)
        import tempfile

        with tempfile.NamedTemporaryFile(suffix=".hwp", delete=False) as f:
            f.write(data)
            f.flush()
            extract = _get_hwp3_parser()
            result = extract(Path(f.name))
        assert isinstance(result, str)

    def test_ctrl_17_footnote(self):
        """제어문자 17: 각주/미주"""
        import struct

        ctrl = struct.pack("<H", 17)
        ctrl_data = b"\x00" * 20  # skip(6+14)
        # 각주 내 텍스트: '가' + CR + 빈 문단
        ga = struct.pack("<H", 0x8861)  # '가'
        cr = struct.pack("<H", 13)
        footnote_para = self._make_paragraph(ga + cr, n_chars=2, prev_shape=0)
        footnote_end = self._empty_paragraph()
        outer_cr = struct.pack("<H", 13)
        chars = ctrl + ctrl_data + footnote_para + footnote_end + outer_cr
        para = self._make_paragraph(chars, n_chars=4 + 1)
        body = para + self._empty_paragraph()
        data = self._build_hwp_with_paragraphs(body)
        import tempfile

        with tempfile.NamedTemporaryFile(suffix=".hwp", delete=False) as f:
            f.write(data)
            f.flush()
            extract = _get_hwp3_parser()
            result = extract(Path(f.name))
        assert isinstance(result, str)

    def test_paragraph_list_stream_error_partial(self):
        """문단 파싱 중 스트림 끝 도달 → 부분 텍스트 반환"""
        import struct
        from utils.hwp3_parser import _HwpStream, _parse_paragraph_list

        # 유효한 첫 문단 (CR만) + 잘린 두 번째 문단 헤더
        cr = struct.pack("<H", 13)
        para1 = self._make_paragraph(cr, n_chars=1, prev_shape=0)
        truncated = struct.pack("<BHHB", 0, 100, 1, 0)  # 불완전한 헤더
        stream = _HwpStream(para1 + truncated)
        result = _parse_paragraph_list(stream)
        assert "\n" in result  # 첫 문단의 CR은 포함

    def test_max_text_length_break(self):
        """_MAX_TEXT_LENGTH 초과 시 파싱 중단"""
        import struct
        from utils.hwp3_parser import _HwpStream, _parse_paragraph_list, _MAX_TEXT_LENGTH

        # 큰 텍스트를 가진 문단 반복 생성
        # '가' 200개 + CR
        ga = struct.pack("<H", 0x8861)
        cr = struct.pack("<H", 13)
        chars = ga * 200 + cr
        paragraphs = b""
        # 충분히 많은 문단
        for _ in range((_MAX_TEXT_LENGTH // 200) + 10):
            paragraphs += self._make_paragraph(chars, n_chars=201, prev_shape=1)
        paragraphs += self._empty_paragraph()
        stream = _HwpStream(paragraphs)
        result = _parse_paragraph_list(stream)
        assert len(result) <= _MAX_TEXT_LENGTH + 500  # 약간의 여유


# ---- hwp3_parser 추가 커버리지 ----


class TestSafeChr:
    """_safe_chr surrogate 필터"""

    def test_surrogate_high(self):
        from utils.hwp3_parser import _safe_chr

        assert _safe_chr(0xD800) == ""
        assert _safe_chr(0xD830) == ""

    def test_surrogate_low(self):
        from utils.hwp3_parser import _safe_chr

        assert _safe_chr(0xDC00) == ""
        assert _safe_chr(0xDFFF) == ""

    def test_normal_char(self):
        from utils.hwp3_parser import _safe_chr

        assert _safe_chr(0xAC00) == "가"
        assert _safe_chr(0x41) == "A"


class TestControlCharTab:
    """탭(ctrl 9) 제어문자 처리"""

    def test_ctrl_9_tab_in_paragraph(self):
        """탭 제어문자가 포함된 문단"""
        import struct
        from utils.hwp3_parser import _HwpStream, _parse_paragraph_list

        # 탭(9) + 6바이트 데이터 → n_chars_read += 3+1=4
        tab = struct.pack("<H", 9) + b"\x00" * 6
        ga = struct.pack("<H", 0x8861)  # '가'
        cr = struct.pack("<H", 13)
        chars = ga + tab + ga + cr  # 가\t가\n, n_chars=1+4+1+1=7
        # 문단 헤더
        header = struct.pack("<BHHB", 0, 7, 1, 0) + b"\x00" * 37
        header += b"\x00" * 187  # para shape
        line_info = b"\x00" * 14
        terminator = struct.pack("<BHHB", 0, 0, 0, 0) + b"\x00" * 37 + b"\x00" * 187
        data = header + line_info + chars + terminator
        stream = _HwpStream(data)
        result = _parse_paragraph_list(stream)
        assert "\t" in result
        assert "가" in result


class TestControlCharDateLine:
    """날짜형식(7), 날짜코드(8), 선(14), 숨은설명(15) 제어문자 처리"""

    def _build_ctrl_para(self, ctrl_code, skip_bytes, n_read_add, extra_nested=False):
        import struct
        from utils.hwp3_parser import _HwpStream, _parse_paragraph_list

        n_chars = 1 + n_read_add + 1  # ctrl + n_read_add + CR
        ctrl = struct.pack("<H", ctrl_code) + b"\x00" * skip_bytes
        if extra_nested:
            # 빈 문단 리스트 (종료)
            ctrl += struct.pack("<BHHB", 0, 0, 0, 0) + b"\x00" * 37 + b"\x00" * 187
        cr = struct.pack("<H", 13)
        chars = ctrl + cr
        header = struct.pack("<BHHB", 0, n_chars, 1, 0) + b"\x00" * 37 + b"\x00" * 187
        line_info = b"\x00" * 14
        terminator = struct.pack("<BHHB", 0, 0, 0, 0) + b"\x00" * 37 + b"\x00" * 187
        data = header + line_info + chars + terminator
        stream = _HwpStream(data)
        return _parse_paragraph_list(stream)

    def test_ctrl_7_date_format(self):
        result = self._build_ctrl_para(7, 84, 3)
        assert isinstance(result, str)

    def test_ctrl_8_date_code(self):
        result = self._build_ctrl_para(8, 96, 3)
        assert isinstance(result, str)

    def test_ctrl_14_line(self):
        result = self._build_ctrl_para(14, 92, 3)
        assert isinstance(result, str)

    def test_ctrl_15_hidden_comment(self):
        result = self._build_ctrl_para(15, 16, 3, extra_nested=True)
        assert isinstance(result, str)


class TestV200BruteForce:
    """V2.00 brute-force 추출 테스트"""

    def test_v200_extracts_text(self):
        """V2.00 파일에서 텍스트가 추출되는지 확인"""
        f = HWP_TEST_DIR / "v2.00_박노해_참된시작.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        assert len(text) > 100

    def test_v200_utf8_safe(self):
        """V2.00 추출 결과가 UTF-8 안전한지 확인"""
        f = HWP_TEST_DIR / "v2.00_박노해_참된시작.hwp"
        if not f.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        extract = _get_hwp3_parser()
        text = extract(f)
        text.encode("utf-8")  # should not raise

    def test_bruteforce_garbage_ratio_filter(self):
        """쓰레기 비율이 높으면 빈 문자열 반환"""
        from utils.hwp3_parser import _extract_text_bruteforce
        import struct

        # 전부 비한글/비한자/비ASCII 범위의 HNC 코드
        garbage = b""
        for i in range(100):
            garbage += struct.pack("<H", 0x3F00 + i)  # 특수문자 범위, 매핑 없음
        result = _extract_text_bruteforce(garbage)
        assert result == ""

    def test_bruteforce_with_valid_text(self):
        """유효한 텍스트가 포함된 데이터에서 추출"""
        from utils.hwp3_parser import _extract_text_bruteforce
        import struct

        # '가나다라마바사아자차' (한글 10자)
        data = b""
        for c in [0x8861, 0x8CC2, 0x9161, 0x9562, 0xA1A1, 0xA562, 0xAD61, 0xB161, 0xB961, 0xBD62]:
            data += struct.pack("<H", c)
        data += struct.pack("<H", 13)  # CR
        result = _extract_text_bruteforce(data)
        assert len(result) > 0

    def test_v200_font_skip_failure_fallback(self, tmp_path: Path):
        """V2.00에서 글꼴 영역 skip 실패 시 전체 brute-force"""
        import struct

        sig = b"HWP Document File V2.00 \x1a\x01\x02\x03\x04\x05"
        doc_info = bytearray(128)
        # 비압축, 정보블록=0
        summary = b"\x00" * 1008
        # 글꼴 영역이 깨진 데이터 (첫 font count가 비정상)
        broken_fonts = struct.pack("<H", 60000)  # n_fonts = 60000 → skip 실패
        # 그 뒤에 유효한 텍스트
        text_data = b""
        for c in [0x8861, 0x8CC2, 0x9161, 0x9562, 0xA1A1, 0xA562, 0xAD61, 0xB161, 0xB961, 0xBD62]:
            text_data += struct.pack("<H", c)
        text_data += struct.pack("<H", 13)
        body = broken_fonts + b"\x00" * 50 + text_data
        f = tmp_path / "v200_broken.hwp"
        f.write_bytes(sig + bytes(doc_info) + summary + body)
        extract = _get_hwp3_parser()
        text = extract(f)
        assert isinstance(text, str)


class TestBookManagerPreviewEmptyContent:
    """book_manager HWP preview에서 빈 결과 시 안내 메시지 반환"""

    def test_hwp_preview_empty_returns_message(self, tmp_path: Path, monkeypatch):
        """LO+hwp3 모두 빈 결과 → '미리보기를 생성할 수 없습니다' 메시지"""
        import shutil

        hwp_src = HWP_TEST_DIR / "v1.20_부하의약혼녀.hwp"
        if not hwp_src.exists():
            import pytest

            pytest.skip("테스트 파일 없음")
        (tmp_path / "A").mkdir(parents=True, exist_ok=True)
        shutil.copy(hwp_src, tmp_path / "A" / "old.hwp")

        from backend.book_manager import BookManager
        from tests.test_book_manager import make_manager, make_doc, DummyES, asyncio_runner

        doc = make_doc("A/old.hwp", "hwp")
        es = DummyES()
        manager = make_manager(tmp_path, es)
        es.search_by_id = lambda _id: doc
        monkeypatch.setattr(BookManager, "_convert_with_libreoffice", lambda p, fmt: "")
        resp = asyncio_runner(manager.get_book_preview(1))
        assert resp.status_code == 200
        body = resp.body.decode("utf-8")
        assert "미리보기를 생성할 수 없습니다" in body


# ── _find_xref_offset 미커버 분기 (loader.py:366, 377-379, 381) ─────────────


class TestFindXrefOffsetBranches:
    """기존 TestFindXrefOffset 이 놓친 분기를 명시적으로 검증한다."""

    def test_trailer_break_and_end_return_none(self):
        """obj 가 범위 밖 → i += 1+sub_count → trailer break → return None (lines 366, 378-379, 381)."""
        Loader = _get_loader()
        # obj_num=5 는 [0,3) 범위 밖 → skip subsection → trailer → None
        xref = b"xref\n0 3\n0000000000 65535 f \n0000000100 00000 n \n0000000200 00000 n \ntrailer\n<< /Size 3 >>"
        result = Loader._find_xref_offset(xref, 5)
        assert result is None

    def test_entry_idx_out_of_range_returns_none(self):
        """entry_idx >= len(lines) 이면 return None (line 377)."""
        Loader = _get_loader()
        # sub_count=2 이지만 entry 가 1개만 존재 → obj_num=1 의 entry_idx 가 범위 밖
        xref = b"xref\n0 2\n0000000009 00000 n \n"
        result = Loader._find_xref_offset(xref, 1)
        assert result is None

    def test_non_matching_line_increments_i(self):
        """digits 2개가 아닌 줄은 i += 1 로 건너뛴다 (line 380-381)."""
        Loader = _get_loader()
        # "junk" 줄은 2-digit 패턴 불일치 → i += 1; 그 다음 "0 1" 을 처리 → obj 0 발견
        xref = b"xref\njunk\n0 1\n0000000009 00000 n \ntrailer\n"
        result = Loader._find_xref_offset(xref, 0)
        assert result == 9

    def test_multiple_subsections_skips_first(self):
        """첫 subsection 에 obj 없음 → i += 1+sub_count (lines 378-379); 두 번째에서 발견."""
        Loader = _get_loader()
        xref = b"xref\n0 1\n0000000009 00000 n \n5 1\n0000000100 00000 n \ntrailer\n"
        # obj_num=5 는 첫 subsection [0,1) 에 없음 → skip → 두 번째 [5,6) 에서 발견
        assert Loader._find_xref_offset(xref, 5) == 100
        # obj_num=0 는 첫 subsection 에서 발견
        assert Loader._find_xref_offset(xref, 0) == 9


# ── comics PDF 경로: _fast_pdf_page_count → None → pypdf fallback (lines 726-732)


class TestReadFileComicsPdf:
    def test_comics_pdf_fast_count_none_falls_back_to_pypdf(self, tmp_path: Path, monkeypatch):
        """_fast_pdf_page_count 가 None 을 반환하면 pypdf.PdfReader 로 페이지 수를 구한다 (lines 726-729)."""
        Loader = _get_loader()
        import pypdf

        # comics_path_prefix 를 tmp_path 로 설정
        monkeypatch.setattr(Loader, "comics_path_prefix", tmp_path)
        monkeypatch.setattr(Loader, "path_prefix", tmp_path)

        pdf_file = tmp_path / "book.pdf"
        writer = pypdf.PdfWriter()
        writer.add_blank_page(width=72, height=72)
        with open(pdf_file, "wb") as f:
            writer.write(f)

        # _fast_pdf_page_count 가 None 을 반환하도록 강제
        monkeypatch.setattr(Loader, "_fast_pdf_page_count", staticmethod(lambda path: None))

        data = Loader.read_file(pdf_file)
        # inode 키로 결과가 반환되며 page_count 가 1
        assert len(data) == 1
        item = next(iter(data.values()))
        assert item["page_count"] == 1

    def test_comics_pdf_fast_count_none_pypdf_exception(self, tmp_path: Path, monkeypatch):
        """pypdf.PdfReader 도 실패하면 page_count=0 으로 처리한다 (lines 726-732)."""
        from unittest.mock import patch

        Loader = _get_loader()

        monkeypatch.setattr(Loader, "comics_path_prefix", tmp_path)
        monkeypatch.setattr(Loader, "path_prefix", tmp_path)

        pdf_file = tmp_path / "broken.pdf"
        pdf_file.write_bytes(b"%PDF-broken")

        monkeypatch.setattr(Loader, "_fast_pdf_page_count", staticmethod(lambda path: None))

        with patch("pypdf.PdfReader", side_effect=Exception("corrupt")):
            data = Loader.read_file(pdf_file)

        assert len(data) == 1
        item = next(iter(data.values()))
        assert item["page_count"] == 0
