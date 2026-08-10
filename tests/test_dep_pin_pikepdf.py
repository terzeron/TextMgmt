#!/usr/bin/env python
"""pikepdf dependency pinning.

backend 사용처:
- backend/book_manager.py:440-471  validate_pdf()
    import pikepdf
    pdf = pikepdf.open(book.file_path)
    issues = pdf.check_pdf_syntax()                  # 구문 검사
    docinfo = pdf.docinfo                            # 메타데이터
    if docinfo.get("/Title"): str(docinfo["/Title"])
    publication["page_count"] = len(pdf.pages)

utils 사용처:
- utils/loader.py: _pdf_text_by_pikepdf_repair() — 손상 PDF 구조 복구
    import pikepdf
    with file_path.open("rb") as fp, pikepdf.open(fp) as pdf:
        pdf.save(buffer)          # BytesIO로 저장 (디스크 미경유)

박제 API:
- pikepdf.open(path) -> pdf
- pikepdf.open(file_obj) -> pdf   # 파일 객체도 받는다(fd 소유권을 호출자가 쥐기 위함)
- pdf : context manager (with 구문)
- pdf.save(BytesIO) -> None       # 열기 시 자동 복구된 구조를 그대로 직렬화
- pdf.check_pdf_syntax() -> 반복 가능(list) — 메시지 문자열들
- pdf.docinfo : .get("/Title")/[...] 접근, str() 변환 가능
- pdf.pages : len() 가능
- pdf.close()
"""

import io
import tempfile
import unittest
from pathlib import Path


class TestPikePDFDependencyPinning(unittest.TestCase):
    def _make_pdf_file(self, page_count: int = 2, title: str | None = None) -> Path:
        from pypdf import PdfWriter

        f = tempfile.NamedTemporaryFile(suffix=".pdf", delete=False)
        writer = PdfWriter()
        for _ in range(page_count):
            writer.add_blank_page(width=612, height=792)
        if title is not None:
            writer.add_metadata({"/Title": title})
        writer.write(f)
        f.close()
        return Path(f.name)

    def test_open_and_close(self):
        """pikepdf.open(path) / pdf.close()"""
        import pikepdf

        pdf_path = self._make_pdf_file()
        try:
            pdf = pikepdf.open(pdf_path)
            self.assertIsNotNone(pdf)
            pdf.close()
        finally:
            pdf_path.unlink(missing_ok=True)

    def test_pages_len(self):
        """book_manager.py:471  len(pdf.pages)"""
        import pikepdf

        pdf_path = self._make_pdf_file(2)
        try:
            pdf = pikepdf.open(pdf_path)
            self.assertEqual(len(pdf.pages), 2)
            pdf.close()
        finally:
            pdf_path.unlink(missing_ok=True)

    def test_check_pdf_syntax_returns_iterable(self):
        """book_manager.py:457  issues = pdf.check_pdf_syntax() — 반복하여 message 추출"""
        import pikepdf

        pdf_path = self._make_pdf_file()
        try:
            pdf = pikepdf.open(pdf_path)
            self.assertTrue(hasattr(pdf, "check_pdf_syntax"), "pikepdf.Pdf.check_pdf_syntax 제거됨")
            issues = pdf.check_pdf_syntax()
            # source가 [{...} for msg in issues] 로 순회하므로 반복 가능해야 함
            messages = [msg for msg in issues]
            self.assertIsInstance(messages, list)
            pdf.close()
        finally:
            pdf_path.unlink(missing_ok=True)

    def test_open_file_object_and_save_to_buffer(self):
        """loader.py: pikepdf.open(file_obj) -> with 구문 -> pdf.save(BytesIO)

        경로가 아닌 파일 객체를 받아야 하고, 복구본을 디스크가 아닌 메모리로
        직렬화할 수 있어야 한다.
        """
        import pikepdf

        pdf_path = self._make_pdf_file(3)
        try:
            buffer = io.BytesIO()
            with pdf_path.open("rb") as fp, pikepdf.open(fp) as pdf:
                self.assertEqual(len(pdf.pages), 3)
                pdf.save(buffer)
            self.assertGreater(buffer.tell(), 0, "save()가 아무것도 쓰지 않음")

            # 저장된 바이트가 다시 열리는 온전한 PDF여야 한다
            buffer.seek(0)
            with pikepdf.open(buffer) as reopened:
                self.assertEqual(len(reopened.pages), 3)
        finally:
            pdf_path.unlink(missing_ok=True)

    def test_docinfo_get_and_str(self):
        """book_manager.py:462-466  docinfo.get('/Title'); str(docinfo['/Title']) (읽기 경로)"""
        import pikepdf

        pdf_path = self._make_pdf_file(title="Pin Test Title")
        try:
            pdf = pikepdf.open(pdf_path)
            docinfo = pdf.docinfo
            # source: if docinfo.get("/Title"): publication["title"] = str(docinfo["/Title"])
            self.assertIsNotNone(docinfo.get("/Title"))
            self.assertEqual(str(docinfo["/Title"]), "Pin Test Title")
            # 미존재 키는 None 반환 (source가 truthiness 로 분기)
            self.assertIsNone(docinfo.get("/NonExistentKey"))
            pdf.close()
        finally:
            pdf_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
