#!/usr/bin/env python
"""pypdf dependency pinning.

backend/utils 사용처:
- backend/book_manager.py:538-547  PDF 미리보기 생성
    from pypdf import PdfReader, PdfWriter
    reader = PdfReader(str(book.file_path))
    writer = PdfWriter()
    writer.add_page(reader.pages[i])
    writer.write(buf)            # buf = io.BytesIO()
- backend/book_manager.py:1218-1244  동일 패턴(분할 추출)
- utils/isbn.py:269-278  reader = pypdf.PdfReader(f); reader.pages[i].extract_text()
- utils/loader.py:188,697,729  len(pypdf.PdfReader(f).pages)

박제 API:
- pypdf.PdfReader(str_path | file_obj) -> reader
- reader.pages : 시퀀스, len() 가능, 인덱싱 가능
- page.extract_text() -> str
- pypdf.PdfWriter() -> writer
- writer.add_page(page)
- writer.write(io.BytesIO())
"""

import io
import tempfile
import unittest
from pathlib import Path


class TestPyPDFDependencyPinning(unittest.TestCase):
    def _make_pdf_bytes(self, page_count: int = 3) -> io.BytesIO:
        from pypdf import PdfWriter

        writer = PdfWriter()
        for _ in range(page_count):
            writer.add_blank_page(width=612, height=792)
        buf = io.BytesIO()
        writer.write(buf)
        buf.seek(0)
        return buf

    def test_symbols_importable(self):
        """from pypdf import PdfReader, PdfWriter"""
        from pypdf import PdfReader, PdfWriter

        self.assertTrue(callable(PdfReader))
        self.assertTrue(callable(PdfWriter))

    def test_reader_from_str_path(self):
        """book_manager.py: PdfReader(str(book.file_path))"""
        from pypdf import PdfReader

        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            pdf_path = Path(f.name)
            f.write(self._make_pdf_bytes(2).getvalue())
        try:
            reader = PdfReader(str(pdf_path))
            self.assertEqual(len(reader.pages), 2)
        finally:
            pdf_path.unlink(missing_ok=True)

    def test_reader_from_file_object(self):
        """isbn.py / loader.py: pypdf.PdfReader(f) (열린 파일 객체)"""
        import pypdf

        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            pdf_path = Path(f.name)
            f.write(self._make_pdf_bytes(4).getvalue())
        try:
            with open(pdf_path, "rb") as infile:
                reader = pypdf.PdfReader(infile)
                self.assertEqual(len(reader.pages), 4)
        finally:
            pdf_path.unlink(missing_ok=True)

    def test_pages_indexable_and_extract_text(self):
        """isbn.py: reader.pages[i].extract_text() -> str"""
        from pypdf import PdfReader

        reader = PdfReader(self._make_pdf_bytes(1))
        page = reader.pages[0]
        text = page.extract_text()
        self.assertIsInstance(text, str)

    def test_writer_add_page_and_write_bytesio(self):
        """book_manager.py: writer.add_page(reader.pages[i]); writer.write(io.BytesIO())"""
        from pypdf import PdfReader, PdfWriter

        reader = PdfReader(self._make_pdf_bytes(3))
        writer = PdfWriter()
        pages_to_extract = min(len(reader.pages), 2)
        for i in range(pages_to_extract):
            writer.add_page(reader.pages[i])
        buf = io.BytesIO()
        writer.write(buf)
        preview_bytes = buf.getvalue()
        self.assertGreater(len(preview_bytes), 0)
        # 생성된 미리보기가 다시 읽혀 페이지 수가 일치
        self.assertEqual(len(PdfReader(io.BytesIO(preview_bytes)).pages), 2)


if __name__ == "__main__":
    unittest.main()
