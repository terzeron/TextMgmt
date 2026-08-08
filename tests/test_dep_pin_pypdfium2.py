#!/usr/bin/env python
"""pypdfium2 dependency pinning.

utils/loader.py 사용처:
- loader.py: read_from_pdf() 1단계 파서
    import pypdfium2
    with file_path.open("rb") as fp:
        pdf = pypdfium2.PdfDocument(fp)
        len(pdf)
        textpage = pdf[i].get_textpage()
        textpage.get_text_range()
        pdf.close()

박제 API:
- pypdfium2.PdfDocument(file_obj) -> pdf   # 경로가 아닌 파일 객체를 넘긴다.
      경로를 넘기면 문서 로드 실패 시 pdfium이 쥔 fd가 닫히지 않고 GC로도 회수되지
      않아, 손상 PDF마다 fd가 1개씩 영구 누적된다.
- len(pdf) -> int
- pdf[i].get_textpage() -> textpage
- textpage.get_text_range() -> str
- pdf.close() -> None
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
        """pypdfium2.PdfDocument(file_obj) -> page.get_textpage().get_text_range()"""
        import pypdfium2
        import pypdf

        # Create a minimal valid PDF
        writer = pypdf.PdfWriter()
        writer.add_blank_page(width=72, height=72)
        with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as f:
            pdf_path = Path(f.name)
            writer.write(pdf_path)

        try:
            # loader.py와 동일하게 파일 객체를 넘긴다 (경로가 아님)
            with pdf_path.open("rb") as fp:
                pdf = pypdfium2.PdfDocument(fp)
                self.assertGreater(len(pdf), 0)
                page = pdf[0]
                textpage = page.get_textpage()
                text = textpage.get_text_range()
                self.assertIsInstance(text, str)
                pdf.close()
        finally:
            pdf_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
