#!/usr/bin/env python
"""pdfplumber dependency pinning.

utils/loader.py 사용처:
- loader.py: read_from_pdf() fallback 파서
    import pdfplumber
    with file_path.open("rb") as fp, pdfplumber.open(fp) as pdf:
        len(pdf.pages)
        page.extract_text()
        page.close()

박제 API:
- pdfplumber.open(file_obj) -> pdf   # 경로가 아닌 파일 객체를 넘긴다(fd 소유권을 직접 쥐기 위함)
- pdf.pages -> list[Page]
- page.extract_text() -> str | None
- page.close() -> None
"""

import tempfile
import unittest
from pathlib import Path


class TestPdfPlumberDependencyPinning(unittest.TestCase):
    def test_symbols_importable(self):
        """import pdfplumber; pdfplumber.open"""
        import pdfplumber

        self.assertTrue(callable(pdfplumber.open))

    def test_open_pdf_and_extract_text(self):
        """pdfplumber.open(file_obj) -> pdf.pages -> extract_text() -> page.close()"""
        import pdfplumber
        import pypdf

        # Create a minimal valid PDF
        writer = pypdf.PdfWriter()
        writer.add_blank_page(width=72, height=72)
        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            pdf_path = Path(f.name)
            writer.write(pdf_path)

        try:
            # loader.py와 동일하게 파일 객체를 넘긴다 (경로가 아님)
            with pdf_path.open("rb") as fp, pdfplumber.open(fp) as pdf:
                self.assertGreater(len(pdf.pages), 0)
                page = pdf.pages[0]
                text = page.extract_text()
                self.assertTrue(text is None or isinstance(text, str))
                page.close()
        finally:
            pdf_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
