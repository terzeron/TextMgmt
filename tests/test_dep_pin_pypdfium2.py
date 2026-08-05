#!/usr/bin/env python
"""pypdfium2 dependency pinning.

utils/loader.py 사용처:
- loader.py: read_from_pdf() fallback 파서
    import pypdfium2
    pdf = pypdfium2.PdfDocument(file_path)
    len(pdf)
    textpage = pdf[i].get_textpage()
    textpage.get_text_range()

박제 API:
- pypdfium2.PdfDocument(file_path) -> pdf
- len(pdf) -> int
- pdf[i].get_textpage() -> textpage
- textpage.get_text_range() -> str
"""

import tempfile
import unittest
from pathlib import Path


class TestPyPdfium2DependencyPinning(unittest.TestCase):
    def test_symbols_importable(self):
        """import pypdfium2; pypdfium2.PdfDocument"""
        import pypdfium2

        self.assertTrue(callable(pypdfium2.PdfDocument))

    def test_pdfdocument_open_and_get_text(self):
        """pypdfium2.PdfDocument(pdf_path) -> page.get_textpage().get_text_range()"""
        import pypdfium2
        import pypdf

        # Create a minimal valid PDF
        writer = pypdf.PdfWriter()
        writer.add_blank_page(width=72, height=72)
        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            pdf_path = Path(f.name)
            writer.write(pdf_path)

        try:
            pdf = pypdfium2.PdfDocument(pdf_path)
            self.assertGreater(len(pdf), 0)
            page = pdf[0]
            textpage = page.get_textpage()
            text = textpage.get_text_range()
            self.assertIsInstance(text, str)
        finally:
            pdf_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
