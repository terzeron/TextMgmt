#!/usr/bin/env python
"""pdfplumber dependency pinning.

utils/loader.py 사용처:
- loader.py: read_from_pdf() fallback 파서
    import pdfplumber
    with pdfplumber.open(file_path) as pdf:
        len(pdf.pages)
        page.extract_text()

박제 API:
- pdfplumber.open(file_path) -> pdf
- pdf.pages -> list[Page]
- page.extract_text() -> str | None
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
        """pdfplumber.open() -> pdf.pages -> extract_text()"""
        import pdfplumber
        import pypdf

        # Create a minimal valid PDF
        writer = pypdf.PdfWriter()
        writer.add_blank_page(width=72, height=72)
        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            pdf_path = Path(f.name)
            writer.write(pdf_path)

        try:
            with pdfplumber.open(pdf_path) as pdf:
                self.assertGreater(len(pdf.pages), 0)
                page = pdf.pages[0]
                text = page.extract_text()
                self.assertTrue(text is None or isinstance(text, str))
        finally:
            pdf_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
