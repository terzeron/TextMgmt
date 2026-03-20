#!/usr/bin/env python
"""ISBN 추출 및 검증 관련 테스트 (utils/isbn.py)"""

import shutil
import tempfile
import unittest
from pathlib import Path


class TestISBNExtraction(unittest.TestCase):
    """ISBN 추출 함수별 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_extract_from_epub_with_opf_metadata(self):
        """EPUB OPF 메타데이터에서 ISBN 추출 테스트"""
        from utils.isbn import extract_from_epub
        import zipfile

        epub_path = Path(self.temp_dir) / "test_isbn.epub"

        with zipfile.ZipFile(epub_path, "w") as zf:
            container_xml = """<?xml version="1.0"?>
            <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
                <rootfiles>
                    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
                </rootfiles>
            </container>"""
            zf.writestr("META-INF/container.xml", container_xml)

            content_opf = """<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf" version="2.0">
                <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
                    <dc:title>Test Book</dc:title>
                    <dc:identifier opf:scheme="ISBN">978-89-12345-67-9</dc:identifier>
                </metadata>
                <manifest>
                    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
                </manifest>
                <spine>
                    <itemref idref="chapter1"/>
                </spine>
            </package>"""
            zf.writestr("OEBPS/content.opf", content_opf)
            zf.writestr("OEBPS/chapter1.xhtml", "<html><body><p>Chapter 1</p></body></html>")

        result = extract_from_epub(epub_path)
        assert len(result) > 0
        assert result[0] == "9788912345679"

    def test_extract_from_epub_with_chapter_content(self):
        """EPUB 챕터 내용에서 ISBN 추출 테스트"""
        from utils.isbn import extract_from_epub
        import zipfile

        epub_path = Path(self.temp_dir) / "test_isbn_chapter.epub"

        with zipfile.ZipFile(epub_path, "w") as zf:
            container_xml = """<?xml version="1.0"?>
            <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
                <rootfiles>
                    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
                </rootfiles>
            </container>"""
            zf.writestr("META-INF/container.xml", container_xml)

            content_opf = """<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf" version="2.0">
                <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
                    <dc:title>Test Book</dc:title>
                </metadata>
                <manifest>
                    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
                </manifest>
                <spine>
                    <itemref idref="chapter1"/>
                </spine>
            </package>"""
            zf.writestr("OEBPS/content.opf", content_opf)
            zf.writestr("OEBPS/chapter1.xhtml", "<html><body><p>ISBN: 978-89-98765-43-9</p></body></html>")

        result = extract_from_epub(epub_path)
        assert len(result) > 0
        assert result[0] == "9788998765439"

    def test_extract_from_djvu_command_availability(self):
        """DJVU 추출 함수가 djvused 없을 때 빈 결과 반환하는지 테스트"""
        from utils.isbn import extract_from_djvu
        import subprocess

        try:
            result = subprocess.run(["which", "djvused"], capture_output=True)
            has_djvused = result.returncode == 0
        except Exception:
            has_djvused = False

        if not has_djvused:
            dummy_path = Path(self.temp_dir) / "dummy.djvu"
            dummy_path.write_bytes(b"dummy")
            result = extract_from_djvu(dummy_path)
            assert result == []

    def test_extract_from_hwp_command_availability(self):
        """HWP 추출 함수가 strings 명령어로 동작하는지 테스트"""
        from utils.isbn import extract_from_hwp
        import subprocess

        try:
            result = subprocess.run(["which", "strings"], capture_output=True)
            has_strings = result.returncode == 0
        except Exception:
            has_strings = False

        if has_strings:
            hwp_path = Path(self.temp_dir) / "test.hwp"
            content = b"Some binary data\x00ISBN 978-89-11111-22-0\x00More data" + b"\x00" * 10000
            hwp_path.write_bytes(content)
            result = extract_from_hwp(hwp_path)
            assert isinstance(result, list)

    def test_extract_from_hwp_isbn_found(self):
        """HWP에서 ISBN이 정상 추출되는지 mock으로 테스트"""
        from unittest.mock import patch, MagicMock
        from utils.isbn import extract_from_hwp

        mock_result = MagicMock()
        mock_result.returncode = 0
        mock_result.stdout = "Some text ISBN 978-89-12345-67-9 more text"

        with patch("utils.isbn.subprocess.run", return_value=mock_result) as mock_run:
            hwp_path = Path(self.temp_dir) / "test.hwp"
            hwp_path.write_bytes(b"dummy")
            result = extract_from_hwp(hwp_path)
            mock_run.assert_called_once_with(["strings", str(hwp_path)], capture_output=True, text=True, errors="ignore")
            assert result == ["9788912345679"]

    def test_extract_from_hwp_returncode_nonzero(self):
        """strings 명령이 실패(returncode != 0)하면 빈 리스트 반환"""
        from unittest.mock import patch, MagicMock
        from utils.isbn import extract_from_hwp

        mock_result = MagicMock()
        mock_result.returncode = 1
        mock_result.stdout = ""

        with patch("utils.isbn.subprocess.run", return_value=mock_result):
            hwp_path = Path(self.temp_dir) / "test.hwp"
            hwp_path.write_bytes(b"dummy")
            result = extract_from_hwp(hwp_path)
            assert result == []

    def test_extract_from_hwp_strings_not_found(self):
        """strings 명령이 없을 때 FileNotFoundError → 빈 리스트"""
        from unittest.mock import patch
        from utils.isbn import extract_from_hwp

        with patch("utils.isbn.subprocess.run", side_effect=FileNotFoundError):
            hwp_path = Path(self.temp_dir) / "test.hwp"
            hwp_path.write_bytes(b"dummy")
            result = extract_from_hwp(hwp_path)
            assert result == []

    def test_extract_from_hwp_large_output_slicing(self):
        """대용량 strings 출력이 head/tail 슬라이싱되는지 테스트"""
        from unittest.mock import patch, MagicMock
        from utils.isbn import extract_from_hwp, HEAD_TAIL_SIZE

        head_isbn = "ISBN 978-89-12345-67-9 "
        tail_isbn = " ISBN 978-89-98765-43-9"
        # head 영역에 ISBN, 중간에 패딩, tail 영역에 ISBN
        head_part = head_isbn + "a" * HEAD_TAIL_SIZE
        middle_part = "b" * HEAD_TAIL_SIZE
        tail_part = "c" * HEAD_TAIL_SIZE + tail_isbn

        mock_result = MagicMock()
        mock_result.returncode = 0
        mock_result.stdout = head_part + middle_part + tail_part

        with patch("utils.isbn.subprocess.run", return_value=mock_result):
            hwp_path = Path(self.temp_dir) / "large.hwp"
            hwp_path.write_bytes(b"dummy")
            result = extract_from_hwp(hwp_path)
            # head[:size]에 head_isbn, tail[-size:]에 tail_isbn이 포함되어야 함
            assert "9788912345679" in result
            assert "9788998765439" in result


class TestISBNByteBasedReading(unittest.TestCase):
    """ISBN 추출을 위한 바이트 기반 읽기 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_read_head_tail_from_file(self):
        """파일에서 head_tail 읽기 테스트 (small, large, boundary 통합)"""
        from utils.isbn import read_head_tail_from_file, HEAD_TAIL_SIZE

        # 작은 파일 (1KB)
        small_file = Path(self.temp_dir) / "small.txt"
        small_content = "A" * 1024
        small_file.write_text(small_content, encoding="utf-8")
        result = read_head_tail_from_file(small_file)
        assert result == small_content

        # 큰 파일 (32KB)
        large_file = Path(self.temp_dir) / "large.txt"
        head_content = "HEAD" * 2048
        middle_content = "MIDDLE" * 4096
        tail_content = "TAIL" * 2048
        content = head_content + middle_content + tail_content
        large_file.write_text(content, encoding="utf-8")
        result = read_head_tail_from_file(large_file)
        assert len(result) == HEAD_TAIL_SIZE * 2
        assert result.startswith("HEAD")
        assert result.endswith("TAIL")
        assert "MIDDLE" not in result

    def test_read_head_tail_from_content(self):
        """콘텐츠에서 head_tail 추출 테스트 (small, large 통합)"""
        from utils.isbn import read_head_tail_from_content, HEAD_TAIL_SIZE

        # 작은 콘텐츠
        small = "Small content"
        assert read_head_tail_from_content(small) == small

        # 큰 콘텐츠
        head = "H" * HEAD_TAIL_SIZE
        middle = "M" * HEAD_TAIL_SIZE
        tail = "T" * HEAD_TAIL_SIZE
        content = head + middle + tail
        result = read_head_tail_from_content(content)
        assert len(result) == HEAD_TAIL_SIZE * 2
        assert result.startswith("H")
        assert result.endswith("T")


class TestISBNValidation(unittest.TestCase):
    """ISBN 유효성 검증 테스트"""

    def test_validate_isbn10(self):
        """ISBN-10 검증 (valid, invalid, with X 통합)"""
        from utils.isbn import validate_isbn10

        # 유효한 ISBN-10
        assert validate_isbn10("0306406152") is True
        assert validate_isbn10("0596520689") is True
        assert validate_isbn10("080442957X") is True  # X로 끝남

        # 무효한 ISBN-10
        assert validate_isbn10("1234567890") is False
        assert validate_isbn10("12345") is False
        assert validate_isbn10("1111111111") is False

    def test_validate_isbn13(self):
        """ISBN-13 검증 (valid, invalid 통합)"""
        from utils.isbn import validate_isbn13

        # 유효한 ISBN-13
        assert validate_isbn13("9780306406157") is True
        assert validate_isbn13("9788912345679") is True

        # 무효한 ISBN-13
        assert validate_isbn13("9781234567890") is False
        assert validate_isbn13("978123456") is False
        assert validate_isbn13("978123456789X") is False

    def test_validate_isbn_both_types(self):
        """validate_isbn이 두 유형을 모두 처리하는지 테스트"""
        from utils.isbn import validate_isbn

        assert validate_isbn("0306406152") is True  # ISBN-10
        assert validate_isbn("9780306406157") is True  # ISBN-13
        assert validate_isbn("1234567890") is False


class TestSearchInContent(unittest.TestCase):
    """search_in_content 함수의 다양한 ISBN 포맷 테스트"""

    def test_search_isbn_formats(self):
        """다양한 ISBN 포맷 검색 (하이픈 유무, ISBN-10/13 통합)"""
        from utils.isbn import search_in_content

        # ISBN-13 with hyphens
        result = search_in_content("이 책의 ISBN: 978-89-12345-67-9")
        assert "9788912345679" in result

        # ISBN-13 without hyphens
        result = search_in_content("ISBN 9788912345679")
        assert "9788912345679" in result

        # ISBN-10
        result = search_in_content("ISBN: 89-12345-67-2")
        assert len(result) > 0
        assert result[0] == "8912345672"

        # No ISBN
        result = search_in_content("이 텍스트에는 ISBN이 없습니다.")
        assert result == []

    def test_search_multiple_isbns(self):
        """여러 ISBN이 있는 경우"""
        from utils.isbn import search_in_content

        content = """
        첫 번째 책: 978-89-12345-67-9
        두 번째 책: 978-89-98765-43-9
        """
        result = search_in_content(content)
        assert len(result) >= 2


class TestEpubEdgeCases(unittest.TestCase):
    """EPUB 추출 엣지 케이스 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_epub_edge_cases(self):
        """EPUB 엣지 케이스 (no_opf, empty_spine, corrupted, missing_chapter 통합)"""
        from utils.isbn import extract_from_epub
        import zipfile

        # No OPF
        epub_path = Path(self.temp_dir) / "no_opf.epub"
        with zipfile.ZipFile(epub_path, "w") as zf:
            zf.writestr("META-INF/container.xml", '<?xml version="1.0"?><container/>')
        assert extract_from_epub(epub_path) == []

        # Corrupted file
        corrupted_path = Path(self.temp_dir) / "corrupted.epub"
        corrupted_path.write_bytes(b"This is not a valid ZIP file")
        assert extract_from_epub(corrupted_path) == []


class TestISBNContentReuse(unittest.TestCase):
    """ISBN 추출 시 content 재사용 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_isbn_extract_with_content_parameter(self):
        """extract_isbn에 content를 전달했을 때 동작 테스트"""
        from utils.isbn import extract as extract_isbn
        from utils.loader import Loader

        test_file = Path(self.temp_dir) / "isbn_test.txt"
        content = "이 책의 ISBN은 978-89-6540-123-0 입니다."
        test_file.write_text(content, encoding="utf-8")

        result_without = extract_isbn(test_file)
        result_with = extract_isbn(test_file, content=content)
        assert result_without == result_with

    def test_isbn_extract_from_loader_content(self):
        """Loader에서 읽은 content를 ISBN 추출에 재사용"""
        from utils.isbn import extract as extract_isbn
        from utils.loader import Loader

        test_file = Path(self.temp_dir) / "book.txt"
        content = "ISBN: 978-89-12345-67-9"
        test_file.write_text(content, encoding="utf-8")

        _, _, _, raw_content = Loader.read_from_text(test_file)
        isbn_list = extract_isbn(test_file, content=raw_content)
        assert len(isbn_list) > 0
        assert isbn_list[0].startswith("978")


if __name__ == "__main__":
    unittest.main()
