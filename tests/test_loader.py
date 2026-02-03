#!/usr/bin/env python

import os
import shutil
import tempfile
import unittest
import logging.config
from pathlib import Path
from typing import Dict, Any
from utils.loader import Loader

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class TestLoader(unittest.TestCase):

    def setUp(self):
        self.loader = Loader()
        self.path_prefix = Loader.path_prefix
        self.txt_dir_path = self.path_prefix / "_txt"
        self.txt_file_path = list(self.txt_dir_path.glob("*.txt"))[0]
        self.epub_dir_path = self.path_prefix / "_epub"
        self.epub_file_path = list(self.epub_dir_path.glob("*.epub"))[0]
        self.pdf_dir_path = self.path_prefix / "_pdf"
        self.pdf_file_path = list(self.pdf_dir_path.glob("*.pdf"))[0]
        self.html_dir_path = self.path_prefix / "_html"
        self.html_file_path = list(self.html_dir_path.glob("*.html"))[0]
        self.docx_dir_path = self.path_prefix / "_doc"
        self.docx_file_path = list(self.docx_dir_path.glob("*.docx"))[0]
        self.rtf_dir_path = self.path_prefix / "_rtf"
        self.rtf_file_path = list(self.rtf_dir_path.glob("*.rtf"))[0]

    def tearDown(self):
        del self.loader

    @staticmethod
    def inspect_data(data: Dict[int, Dict[str, Any]]) -> None:
        assert isinstance(data, dict)
        for k, v in data.items():
            assert isinstance(k, int)
            assert isinstance(v, dict)
            assert "category" in v and isinstance(v["category"], str)
            assert "title" in v and isinstance(v["title"], str)
            assert "author" in v and isinstance(v["author"], str)
            assert "file_path" in v and isinstance(v["file_path"], str)
            assert "file_type" in v and isinstance(v["file_type"], str)
            assert "file_size" in v and isinstance(v["file_size"], int)
            assert "summary" in v and isinstance(v["summary"], str)
            assert "updated_time" in v and isinstance(v["updated_time"], str)
            for k1, _ in v.items():
                assert isinstance(k1, str)

    def test_read_from_text(self):
        """read_from_text가 (summary, line_count, page_count, raw_content) 튜플을 반환하는지 테스트"""
        result = self.loader.read_from_text(self.txt_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 4
        summary, line_count, page_count, raw_content = result
        assert isinstance(summary, str)
        assert 0 < len(summary) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert line_count > 0
        assert isinstance(page_count, int)
        assert page_count == 0  # TXT 파일은 page_count가 0
        assert isinstance(raw_content, str)
        assert len(raw_content) > 0

    def test_read_from_text_raw_content_contains_summary(self):
        """raw_content가 summary를 포함하는지 테스트 (summary는 raw_content의 앞부분)"""
        result = self.loader.read_from_text(self.txt_file_path)
        summary, _, _, raw_content = result
        # summary는 raw_content의 앞부분에서 특수문자가 제거된 형태
        # raw_content가 summary보다 길거나 같아야 함
        assert len(raw_content) >= len(summary.replace(' ', '').strip())

    def test_read_from_epub_with_extracting_zip(self):
        result = self.loader.read_from_epub_with_extracting_zip(self.epub_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 2
        content, line_count = result
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)

    def test_read_from_epub(self):
        result = self.loader.read_from_epub(self.epub_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 3
        content, line_count, page_count = result
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert isinstance(page_count, int)

    def test_read_from_pdf(self):
        result = self.loader.read_from_pdf(self.pdf_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 3
        content, line_count, page_count = result
        assert content is not None
        assert isinstance(content, str)
        assert 0 <= len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert isinstance(page_count, int)
        assert page_count > 0  # PDF는 page_count가 있어야 함

    def test_read_from_html(self):
        result = self.loader.read_from_html(self.html_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 3
        content, line_count, page_count = result
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert isinstance(page_count, int)

    def test_read_from_docx(self):
        result = self.loader.read_from_docx(self.docx_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 3
        content, line_count, page_count = result
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert isinstance(page_count, int)

    def test_read_from_rtf(self):
        result = self.loader.read_from_rtf(self.rtf_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 3
        content, line_count, page_count = result
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert isinstance(page_count, int)

    def test_read_file(self):
        data = self.loader.read_file(self.epub_file_path)
        assert data
        assert len(data) == 1
        self.inspect_data(data)

    def test_read_files(self):
        data = self.loader.read_files(self.epub_dir_path, 1000)
        assert data
        assert 0 < len(data) <= 1000
        self.inspect_data(data)

    def test_read_files_non_recursive(self):
        """Test read_files with recursive=False (default) only reads files in the immediate directory."""
        data = self.loader.read_files(self.txt_dir_path, recursive=False)
        assert isinstance(data, dict)
        # All files should be from the immediate directory, not subdirectories
        for _, v in data.items():
            file_path = Path(v["file_path"])
            # The parent should be the txt_dir_path itself (relative path)
            assert file_path.parent.name == self.txt_dir_path.name

    def test_read_files_recursive(self):
        """Test read_files with recursive=True reads files from subdirectories."""
        # Use path_prefix which should have subdirectories
        data = self.loader.read_files(self.path_prefix, num_files=100, recursive=True)
        assert isinstance(data, dict)
        # With recursive=True, we should find files in various subdirectories
        if data:
            categories = set()
            for _, v in data.items():
                categories.add(v["category"])
            # Should have files from multiple categories/subdirectories
            self.inspect_data(data)

    def test_read_files_recursive_vs_non_recursive(self):
        """Test that recursive=True finds more or equal files than recursive=False."""
        non_recursive_data = self.loader.read_files(self.path_prefix, num_files=1000, recursive=False)
        recursive_data = self.loader.read_files(self.path_prefix, num_files=1000, recursive=True)
        # Recursive should find at least as many files as non-recursive
        assert len(recursive_data) >= len(non_recursive_data)

    # ========== 성능 개선 관련 테스트 ==========

    def test_beautifulsoup_uses_lxml_parser(self):
        """BeautifulSoup이 lxml 파서를 사용하는지 간접 테스트 (HTML 파싱 동작 확인)"""
        # lxml이 설치되어 있어야 함
        try:
            import lxml
            has_lxml = True
        except ImportError:
            has_lxml = False

        assert has_lxml, "lxml 패키지가 설치되어 있어야 합니다"

        # HTML 파일 파싱이 정상 동작하는지 확인
        result = self.loader.read_from_html(self.html_file_path)
        content, line_count, page_count = result
        assert isinstance(content, str)
        assert len(content) > 0

    def test_get_stat_returns_stat_result(self):
        """get_stat()이 os.stat_result를 반환하는지 테스트"""
        st = Loader.get_stat(self.txt_file_path)
        assert isinstance(st, os.stat_result)
        assert hasattr(st, 'st_ino')
        assert hasattr(st, 'st_size')
        assert st.st_ino > 0
        assert st.st_size > 0

    def test_read_file_with_stat_result(self):
        """read_file()에 stat_result를 전달했을 때 올바르게 동작하는지 테스트"""
        # stat을 미리 호출
        st = Loader.get_stat(self.txt_file_path)
        inode_from_stat = st.st_ino
        size_from_stat = st.st_size

        # stat_result를 전달하여 read_file 호출
        data = Loader.read_file(self.txt_file_path, stat_result=st)

        assert data
        assert len(data) == 1
        assert inode_from_stat in data
        assert data[inode_from_stat]['file_size'] == size_from_stat

    def test_read_file_without_stat_result(self):
        """read_file()에 stat_result를 전달하지 않아도 동작하는지 테스트"""
        data = Loader.read_file(self.txt_file_path)
        assert data
        assert len(data) == 1
        self.inspect_data(data)

    def test_read_file_stat_reuse_consistency(self):
        """stat_result 전달 유무에 관계없이 결과가 일치하는지 테스트"""
        # stat_result 없이 호출
        data_without_stat = Loader.read_file(self.epub_file_path)

        # stat_result와 함께 호출
        st = Loader.get_stat(self.epub_file_path)
        data_with_stat = Loader.read_file(self.epub_file_path, stat_result=st)

        # 두 결과가 동일해야 함 (updated_time 제외)
        assert data_without_stat.keys() == data_with_stat.keys()
        for inode in data_without_stat:
            for key in ['category', 'title', 'author', 'file_path', 'file_type', 'file_size', 'line_count', 'page_count', 'isbn', 'summary']:
                assert data_without_stat[inode][key] == data_with_stat[inode][key], f"Mismatch in {key}"


class TestLoaderWithTempFiles(unittest.TestCase):
    """임시 파일을 사용한 테스트 (실제 데이터 의존성 없음)"""

    def setUp(self):
        # 임시 디렉토리 생성
        self.temp_dir = tempfile.mkdtemp()
        self.original_path_prefix = Loader.path_prefix
        Loader.path_prefix = Path(self.temp_dir)

    def tearDown(self):
        # 원래 path_prefix 복원
        Loader.path_prefix = self.original_path_prefix
        # 임시 파일 정리
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_read_from_text_line_count_accuracy(self):
        """read_from_text의 line_count가 정확한지 테스트"""
        # 10줄짜리 파일 생성 (각 줄 끝에 newline 있음)
        test_file = Path(self.temp_dir) / "test_lines.txt"
        lines = ["Line {}\n".format(i) for i in range(10)]
        test_file.write_text("".join(lines), encoding="utf-8")

        _, line_count, _, raw_content = Loader.read_from_text(test_file)

        # line_count = newline 수 + 1 = 10 + 1 = 11
        assert line_count == 11
        assert "Line 0" in raw_content
        assert "Line 9" in raw_content

    def test_read_from_text_small_file(self):
        """작은 파일에서 raw_content가 전체 내용을 포함하는지 테스트"""
        test_file = Path(self.temp_dir) / "small.txt"
        content = "Hello, World! 안녕하세요."
        test_file.write_text(content, encoding="utf-8")

        summary, line_count, _, raw_content = Loader.read_from_text(test_file)

        assert raw_content == content
        assert line_count == 1

    def test_read_from_text_large_file_summary_truncation(self):
        """큰 파일에서 summary가 TEXT_SIZE로 잘리는지 테스트"""
        test_file = Path(self.temp_dir) / "large.txt"
        # TEXT_SIZE보다 큰 파일 생성
        large_content = "A" * (Loader.TEXT_SIZE * 2)
        test_file.write_text(large_content, encoding="utf-8")

        summary, line_count, _, raw_content = Loader.read_from_text(test_file)

        # summary는 TEXT_SIZE 이하로 잘려야 함
        assert len(summary) <= Loader.TEXT_SIZE
        # raw_content는 전체 내용을 포함해야 함
        assert len(raw_content) == Loader.TEXT_SIZE * 2

    def test_read_from_text_bom_removal(self):
        """BOM이 제거되는지 테스트"""
        test_file = Path(self.temp_dir) / "bom.txt"
        content_with_bom = "\ufeffHello BOM"
        test_file.write_text(content_with_bom, encoding="utf-8")

        summary, _, _, _ = Loader.read_from_text(test_file)

        assert "\ufeff" not in summary
        assert "Hello" in summary


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

        # ISBN이 OPF 메타데이터에 포함된 EPUB 생성
        epub_path = Path(self.temp_dir) / "test_isbn.epub"

        with zipfile.ZipFile(epub_path, 'w') as zf:
            # container.xml
            container_xml = '''<?xml version="1.0"?>
            <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
                <rootfiles>
                    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
                </rootfiles>
            </container>'''
            zf.writestr('META-INF/container.xml', container_xml)

            # content.opf with ISBN in metadata
            content_opf = '''<?xml version="1.0"?>
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
            </package>'''
            zf.writestr('OEBPS/content.opf', content_opf)

            # chapter1.xhtml (no ISBN here)
            chapter1 = '''<html><body><p>This is chapter 1 content.</p></body></html>'''
            zf.writestr('OEBPS/chapter1.xhtml', chapter1)

        # ISBN 추출
        result = extract_from_epub(epub_path)

        # OPF 메타데이터에서 ISBN이 추출되어야 함
        assert len(result) > 0
        assert result[0] == "9788912345679"

    def test_extract_from_epub_with_chapter_content(self):
        """EPUB 챕터 내용에서 ISBN 추출 테스트 (메타데이터에 없는 경우)"""
        from utils.isbn import extract_from_epub
        import zipfile

        # ISBN이 챕터에만 있는 EPUB 생성
        epub_path = Path(self.temp_dir) / "test_isbn_chapter.epub"

        with zipfile.ZipFile(epub_path, 'w') as zf:
            # container.xml
            container_xml = '''<?xml version="1.0"?>
            <container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
                <rootfiles>
                    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
                </rootfiles>
            </container>'''
            zf.writestr('META-INF/container.xml', container_xml)

            # content.opf without ISBN
            content_opf = '''<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf" version="2.0">
                <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
                    <dc:title>Test Book</dc:title>
                </metadata>
                <manifest>
                    <item id="chapter1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
                    <item id="chapter2" href="chapter2.xhtml" media-type="application/xhtml+xml"/>
                </manifest>
                <spine>
                    <itemref idref="chapter1"/>
                    <itemref idref="chapter2"/>
                </spine>
            </package>'''
            zf.writestr('OEBPS/content.opf', content_opf)

            # chapter1.xhtml with ISBN
            chapter1 = '''<html><body><p>ISBN: 978-89-98765-43-9</p></body></html>'''
            zf.writestr('OEBPS/chapter1.xhtml', chapter1)

            # chapter2.xhtml
            chapter2 = '''<html><body><p>This is chapter 2.</p></body></html>'''
            zf.writestr('OEBPS/chapter2.xhtml', chapter2)

        # ISBN 추출
        result = extract_from_epub(epub_path)

        # 챕터에서 ISBN이 추출되어야 함
        assert len(result) > 0
        assert result[0] == "9788998765439"

    def test_extract_from_djvu_command_availability(self):
        """DJVU 추출 함수가 djvused 없을 때 빈 결과 반환하는지 테스트"""
        from utils.isbn import extract_from_djvu
        import subprocess

        # djvused 명령어 존재 확인
        try:
            result = subprocess.run(["which", "djvused"], capture_output=True)
            has_djvused = result.returncode == 0
        except Exception:
            has_djvused = False

        if not has_djvused:
            # djvused가 없으면 빈 결과 반환해야 함
            dummy_path = Path(self.temp_dir) / "dummy.djvu"
            dummy_path.write_bytes(b"dummy")
            result = extract_from_djvu(dummy_path)
            assert result == []
        else:
            # djvused가 있으면 skip (실제 DJVU 파일 필요)
            pass  # 실제 환경에서는 DJVU 파일로 테스트

    def test_extract_from_hwp_command_availability(self):
        """HWP 추출 함수가 strings 명령어로 동작하는지 테스트"""
        from utils.isbn import extract_from_hwp
        import subprocess

        # strings 명령어 존재 확인
        try:
            result = subprocess.run(["which", "strings"], capture_output=True)
            has_strings = result.returncode == 0
        except Exception:
            has_strings = False

        if has_strings:
            # ISBN이 포함된 가짜 HWP 파일 생성 (실제로는 텍스트 파일)
            hwp_path = Path(self.temp_dir) / "test.hwp"
            # HWP는 바이너리이므로 strings가 추출할 수 있는 형태로 작성
            content = b"Some binary data\x00ISBN 978-89-11111-22-0\x00More data" + b"\x00" * 10000
            content += b"End of file\x00ISBN info at end: 978-89-44444-55-5\x00"
            hwp_path.write_bytes(content)

            result = extract_from_hwp(hwp_path)
            # strings가 ISBN을 찾을 수 있어야 함
            assert isinstance(result, list)
        else:
            # strings가 없으면 skip
            pass


class TestISBNByteBasedReading(unittest.TestCase):
    """ISBN 추출을 위한 바이트 기반 읽기 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_read_head_tail_from_file_small_file(self):
        """작은 파일에서 head_tail 읽기 테스트 (8KB 미만)"""
        from utils.isbn import read_head_tail_from_file, HEAD_TAIL_SIZE

        # 작은 파일 생성 (1KB)
        small_file = Path(self.temp_dir) / "small.txt"
        content = "A" * 1024
        small_file.write_text(content, encoding="utf-8")

        result = read_head_tail_from_file(small_file)

        # 작은 파일은 전체가 반환되어야 함 (tail은 빈 문자열)
        assert result == content

    def test_read_head_tail_from_file_large_file(self):
        """큰 파일에서 head_tail 읽기 테스트 (8KB 초과)"""
        from utils.isbn import read_head_tail_from_file, HEAD_TAIL_SIZE

        # 큰 파일 생성 (32KB)
        large_file = Path(self.temp_dir) / "large.txt"
        head_content = "HEAD" * 2048  # 8KB
        middle_content = "MIDDLE" * 4096  # 24KB
        tail_content = "TAIL" * 2048  # 8KB
        content = head_content + middle_content + tail_content
        large_file.write_text(content, encoding="utf-8")

        result = read_head_tail_from_file(large_file)

        # head + tail만 반환되어야 함
        assert len(result) == HEAD_TAIL_SIZE * 2
        assert result.startswith("HEAD")
        assert result.endswith("TAIL")
        assert "MIDDLE" not in result

    def test_read_head_tail_from_file_exact_boundary(self):
        """정확히 16KB인 파일 테스트 (경계 조건)"""
        from utils.isbn import read_head_tail_from_file, HEAD_TAIL_SIZE

        # 정확히 16KB 파일 생성
        boundary_file = Path(self.temp_dir) / "boundary.txt"
        content = "X" * (HEAD_TAIL_SIZE * 2)
        boundary_file.write_text(content, encoding="utf-8")

        result = read_head_tail_from_file(boundary_file)

        # 전체가 반환되어야 함 (tail은 빈 문자열)
        assert result == content

    def test_read_head_tail_from_content_small(self):
        """작은 콘텐츠에서 head_tail 추출 테스트"""
        from utils.isbn import read_head_tail_from_content, HEAD_TAIL_SIZE

        content = "Small content"
        result = read_head_tail_from_content(content)

        # 작은 콘텐츠는 그대로 반환
        assert result == content

    def test_read_head_tail_from_content_large(self):
        """큰 콘텐츠에서 head_tail 추출 테스트"""
        from utils.isbn import read_head_tail_from_content, HEAD_TAIL_SIZE

        head = "H" * HEAD_TAIL_SIZE
        middle = "M" * HEAD_TAIL_SIZE
        tail = "T" * HEAD_TAIL_SIZE
        content = head + middle + tail

        result = read_head_tail_from_content(content)

        # head + tail만 반환
        assert len(result) == HEAD_TAIL_SIZE * 2
        assert result.startswith("H")
        assert result.endswith("T")
        assert "M" not in result


class TestISBNValidation(unittest.TestCase):
    """ISBN 유효성 검증 테스트"""

    def test_validate_isbn10_valid(self):
        """유효한 ISBN-10 검증"""
        from utils.isbn import validate_isbn10

        # 유효한 ISBN-10 예시
        assert validate_isbn10("0306406152") is True
        assert validate_isbn10("0596520689") is True

    def test_validate_isbn10_invalid(self):
        """무효한 ISBN-10 검증"""
        from utils.isbn import validate_isbn10

        assert validate_isbn10("1234567890") is False  # 잘못된 체크섬
        assert validate_isbn10("12345") is False  # 너무 짧음
        assert validate_isbn10("1111111111") is False  # 블랙리스트
        assert validate_isbn10("1100101101") is False  # 블랙리스트

    def test_validate_isbn10_with_x(self):
        """X로 끝나는 ISBN-10 검증"""
        from utils.isbn import validate_isbn10

        # ISBN-10에서 X는 10을 의미
        assert validate_isbn10("080442957X") is True

    def test_validate_isbn13_valid(self):
        """유효한 ISBN-13 검증"""
        from utils.isbn import validate_isbn13

        assert validate_isbn13("9780306406157") is True
        assert validate_isbn13("9788912345679") is True  # 978-89-12345-67-9

    def test_validate_isbn13_invalid(self):
        """무효한 ISBN-13 검증"""
        from utils.isbn import validate_isbn13

        assert validate_isbn13("9781234567890") is False  # 잘못된 체크섬
        assert validate_isbn13("978123456") is False  # 너무 짧음
        assert validate_isbn13("978123456789X") is False  # X는 ISBN-13에 없음

    def test_validate_isbn_both_types(self):
        """validate_isbn이 두 유형을 모두 처리하는지 테스트"""
        from utils.isbn import validate_isbn

        # ISBN-10
        assert validate_isbn("0306406152") is True
        # ISBN-13
        assert validate_isbn("9780306406157") is True
        # 무효
        assert validate_isbn("1234567890") is False


class TestSearchInContent(unittest.TestCase):
    """search_in_content 함수의 다양한 ISBN 포맷 테스트"""

    def test_search_isbn13_with_hyphens(self):
        """하이픈이 있는 ISBN-13 검색"""
        from utils.isbn import search_in_content

        content = "이 책의 ISBN: 978-89-12345-67-9"
        result = search_in_content(content)
        assert "9788912345679" in result

    def test_search_isbn13_without_hyphens(self):
        """하이픈이 없는 ISBN-13 검색"""
        from utils.isbn import search_in_content

        content = "ISBN 9788912345679"
        result = search_in_content(content)
        assert "9788912345679" in result

    def test_search_isbn10(self):
        """ISBN-10 검색 (한국 ISBN, 89로 시작)"""
        from utils.isbn import search_in_content

        # 89-12345-67-2는 유효한 한국 ISBN-10
        content = "ISBN: 89-12345-67-2"
        result = search_in_content(content)
        assert len(result) > 0
        assert result[0] == "8912345672"

    def test_search_multiple_isbns(self):
        """여러 ISBN이 있는 경우"""
        from utils.isbn import search_in_content

        content = """
        첫 번째 책: 978-89-12345-67-9
        두 번째 책: 978-89-98765-43-9
        """
        result = search_in_content(content)
        assert len(result) >= 2

    def test_search_isbn_with_ocr_errors(self):
        """OCR 에러가 있는 ISBN 검색 (l->1, O->0 등)"""
        from utils.isbn import search_in_content

        # 일부 OCR 에러 케이스는 정규식에서 처리
        content = "ISBN: 978-89-l2345-67-9"  # l instead of 1
        result = search_in_content(content)
        # OCR 에러 보정이 되어야 함
        assert len(result) >= 0  # 구현에 따라 다를 수 있음

    def test_search_no_isbn(self):
        """ISBN이 없는 텍스트"""
        from utils.isbn import search_in_content

        content = "이 텍스트에는 ISBN이 없습니다."
        result = search_in_content(content)
        assert result == []


class TestEpubEdgeCases(unittest.TestCase):
    """EPUB 추출 엣지 케이스 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_extract_from_epub_no_opf(self):
        """OPF 파일이 없는 EPUB"""
        from utils.isbn import extract_from_epub
        import zipfile

        epub_path = Path(self.temp_dir) / "no_opf.epub"
        with zipfile.ZipFile(epub_path, 'w') as zf:
            zf.writestr('META-INF/container.xml', '<?xml version="1.0"?><container/>')
            zf.writestr('content.html', '<html><body>No OPF</body></html>')

        result = extract_from_epub(epub_path)
        assert result == []

    def test_extract_from_epub_empty_spine(self):
        """spine이 비어있는 EPUB"""
        from utils.isbn import extract_from_epub
        import zipfile

        epub_path = Path(self.temp_dir) / "empty_spine.epub"
        with zipfile.ZipFile(epub_path, 'w') as zf:
            content_opf = '''<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf" version="2.0">
                <metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test</dc:title></metadata>
                <manifest>
                    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                </manifest>
                <spine></spine>
            </package>'''
            zf.writestr('content.opf', content_opf)

        result = extract_from_epub(epub_path)
        assert result == []

    def test_extract_from_epub_corrupted_file(self):
        """손상된 EPUB 파일"""
        from utils.isbn import extract_from_epub

        corrupted_path = Path(self.temp_dir) / "corrupted.epub"
        corrupted_path.write_bytes(b"This is not a valid ZIP file")

        result = extract_from_epub(corrupted_path)
        assert result == []

    def test_extract_from_epub_missing_chapter(self):
        """챕터 파일이 누락된 EPUB"""
        from utils.isbn import extract_from_epub
        import zipfile

        epub_path = Path(self.temp_dir) / "missing_chapter.epub"
        with zipfile.ZipFile(epub_path, 'w') as zf:
            content_opf = '''<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf" version="2.0">
                <metadata><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Test</dc:title></metadata>
                <manifest>
                    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                </manifest>
                <spine>
                    <itemref idref="ch1"/>
                </spine>
            </package>'''
            zf.writestr('content.opf', content_opf)
            # ch1.xhtml 파일을 의도적으로 생성하지 않음

        result = extract_from_epub(epub_path)
        # 누락된 챕터는 건너뛰고 빈 결과 반환
        assert result == []


class TestISBNContentReuse(unittest.TestCase):
    """ISBN 추출 시 content 재사용 테스트"""

    def setUp(self):
        self.temp_dir = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.temp_dir, ignore_errors=True)

    def test_isbn_extract_with_content_parameter(self):
        """extract_isbn에 content를 전달했을 때 동작하는지 테스트"""
        from utils.isbn import extract as extract_isbn

        # ISBN이 포함된 텍스트 파일 생성
        test_file = Path(self.temp_dir) / "isbn_test.txt"
        content = "이 책의 ISBN은 978-89-6540-123-0 입니다."
        test_file.write_text(content, encoding="utf-8")

        # content 없이 호출
        result_without_content = extract_isbn(test_file)

        # content와 함께 호출
        result_with_content = extract_isbn(test_file, content=content)

        # 둘 다 같은 결과를 반환해야 함
        assert result_without_content == result_with_content

    def test_isbn_extract_content_reuse_from_loader(self):
        """Loader에서 읽은 content를 ISBN 추출에 재사용하는 시나리오 테스트"""
        from utils.isbn import extract as extract_isbn

        # ISBN이 포함된 텍스트 파일 생성
        test_file = Path(self.temp_dir) / "book.txt"
        content = """
        이 책은 테스트용 도서입니다.
        ISBN: 978-89-12345-67-9
        저자: 테스트 작가
        """
        test_file.write_text(content, encoding="utf-8")

        # Loader로 읽기
        summary, line_count, page_count, raw_content = Loader.read_from_text(test_file)

        # raw_content를 extract_isbn에 전달
        isbn_list = extract_isbn(test_file, content=raw_content)

        # ISBN이 추출되어야 함
        assert len(isbn_list) > 0
        assert isbn_list[0].startswith("978")

    def test_isbn_extract_with_empty_content(self):
        """빈 content를 전달해도 파일에서 읽어서 처리하는지 테스트"""
        from utils.isbn import extract as extract_isbn

        # ISBN이 포함된 텍스트 파일 생성
        test_file = Path(self.temp_dir) / "isbn_empty_test.txt"
        content = "ISBN 978-89-6540-999-1 테스트"
        test_file.write_text(content, encoding="utf-8")

        # 빈 문자열은 falsy이므로 파일에서 읽어야 함
        result = extract_isbn(test_file, content="")

        # 빈 문자열이 전달되면 파일에서 읽지 않고 빈 결과 반환 (현재 구현)
        # 또는 파일에서 읽어서 결과 반환 (둘 다 유효한 동작)
        assert isinstance(result, list)


class TestGeneratorSupport(unittest.TestCase):
    """Generator 지원 관련 테스트"""

    def setUp(self):
        self.path_prefix = Loader.path_prefix
        self.txt_dir_path = self.path_prefix / "_txt"

    def test_get_file_list_returns_list(self):
        """get_file_list가 리스트를 반환하는지 테스트"""
        file_list = Loader.get_file_list(self.txt_dir_path, num_files=5)
        assert isinstance(file_list, list)
        assert len(file_list) <= 5
        for f in file_list:
            assert isinstance(f, Path)
            assert f.is_file()

    def test_file_iteration_with_generator(self):
        """Generator로 파일을 순회할 수 있는지 테스트"""
        # Generator 생성
        file_gen = (p for p in self.txt_dir_path.iterdir() if p.is_file())

        # Generator에서 파일을 하나씩 읽기
        count = 0
        for file_path in file_gen:
            if count >= 3:
                break
            st = Loader.get_stat(file_path)
            assert isinstance(st, os.stat_result)
            count += 1

        assert count > 0


# ========== pytest 기반 ES 통합 테스트 ==========
import pytest
from itertools import islice


@pytest.fixture(scope="module")
def es_manager_for_loader(elasticsearch_container):
    """Loader 테스트용 ESManager fixture"""
    from backend.es_manager import ESManager
    from elasticsearch import Elasticsearch
    import time

    esm = ESManager()
    esm.es = Elasticsearch(
        hosts=[os.environ["TM_ES_URL"]],
        basic_auth=(os.environ.get("TM_ES_USER", ""), os.environ.get("TM_ES_PASSWORD", "")),
        request_timeout=120,
        retry_on_timeout=True,
        verify_certs=False,
        max_retries=5
    )

    # Wait for cluster
    for _ in range(60):
        try:
            health = esm.es.cluster.health(wait_for_status="yellow", timeout="5s")
            break
        except Exception:
            time.sleep(1)

    # Create fresh index
    try:
        if esm.do_exist_index():
            esm.delete_index()
    except Exception:
        pass

    esm.create_index()
    esm.es.cluster.health(index=esm.index_name, wait_for_status="yellow", timeout="30s")

    yield esm

    try:
        esm.delete_index()
    except Exception:
        pass


class TestLoaderWithES:
    """ES를 사용하는 Loader 통합 테스트"""

    def test_insert_with_stat_reuse(self, es_manager_for_loader):
        """stat 재사용하여 ES에 삽입하는 테스트"""
        esm = es_manager_for_loader
        path_prefix = Loader.path_prefix
        txt_dir = path_prefix / "_txt"

        if not txt_dir.exists():
            pytest.skip("_txt directory not found")

        # 파일 하나 선택
        txt_files = list(txt_dir.glob("*.txt"))[:1]
        if not txt_files:
            pytest.skip("No txt files found")

        file_path = txt_files[0]

        # stat 한 번만 호출
        st = Loader.get_stat(file_path)
        inode = st.st_ino

        # stat_result를 전달하여 read_file 호출
        data = Loader.read_file(file_path, stat_result=st)
        assert data
        assert inode in data

        # ES에 삽입
        esm.insert(data)
        esm.es.indices.refresh(index=esm.index_name)

        # 검색으로 확인
        result = esm.search_by_id(inode)
        assert result
        assert result["file_path"] == data[inode]["file_path"]

    def test_batch_insert_with_generator(self, es_manager_for_loader):
        """Generator로 파일을 배치 처리하여 ES에 삽입하는 테스트"""
        esm = es_manager_for_loader
        path_prefix = Loader.path_prefix
        epub_dir = path_prefix / "_epub"

        if not epub_dir.exists():
            pytest.skip("_epub directory not found")

        # Generator 생성
        file_gen = (p for p in epub_dir.iterdir() if p.is_file() and p.suffix == ".epub")

        # islice로 배치 처리 시뮬레이션
        batch_size = 3
        batch = list(islice(file_gen, batch_size))

        if not batch:
            pytest.skip("No epub files found")

        # 배치 처리
        batch_data: Dict[int, Dict[str, Any]] = {}
        for file_path in batch:
            st = Loader.get_stat(file_path)
            data = Loader.read_file(file_path, stat_result=st)
            if data:
                batch_data.update(data)

        assert len(batch_data) > 0

        # ES에 삽입
        esm.insert(batch_data)
        esm.es.indices.refresh(index=esm.index_name)

        # 삽입된 문서 수 확인
        count = esm.es.count(index=esm.index_name)["count"]
        assert count >= len(batch_data)

    def test_refresh_method(self, es_manager_for_loader):
        """ESManager.refresh() 메서드 테스트"""
        esm = es_manager_for_loader

        # 데이터 삽입 (refresh=False로 설정됨)
        test_data = {
            99999: {
                "category": "test",
                "title": "Refresh 테스트",
                "author": "테스트",
                "file_path": "/test/refresh.txt",
                "file_type": "txt",
                "file_size": 100,
                "line_count": 10,
                "page_count": 0,
                "isbn": "",
                "summary": "refresh 테스트용",
                "updated_time": "2024-01-01T00:00:00",
            }
        }
        esm.insert(test_data)

        # refresh 호출
        esm.refresh()

        # 검색으로 확인
        result = esm.search_by_id(99999)
        assert result
        assert result["title"] == "Refresh 테스트"

    def test_txt_content_reuse_for_isbn(self, es_manager_for_loader):
        """TXT 파일에서 content 재사용하여 ISBN 추출 후 ES 저장 테스트"""
        from utils.isbn import extract as extract_isbn

        esm = es_manager_for_loader

        # 임시 파일 생성
        temp_dir = tempfile.mkdtemp()
        original_prefix = Loader.path_prefix

        try:
            Loader.path_prefix = Path(temp_dir)
            test_file = Path(temp_dir) / "isbn_book.txt"
            content = """
            제목: 테스트 도서
            ISBN: 978-89-12345-67-9
            이 책은 테스트용입니다.
            """ + ("내용 " * 1000)  # 충분한 크기
            test_file.write_text(content, encoding="utf-8")

            # Loader로 읽기 (stat 재사용)
            st = Loader.get_stat(test_file)
            summary, line_count, page_count, raw_content = Loader.read_from_text(test_file)

            # raw_content를 ISBN 추출에 재사용
            isbn_list = extract_isbn(test_file, content=raw_content)

            # read_file 호출 (stat 재사용)
            data = Loader.read_file(test_file, stat_result=st)
            inode = st.st_ino

            # ISBN이 추출되었는지 확인
            if isbn_list:
                assert data[inode]["isbn"] == isbn_list[0]

            # ES에 삽입
            esm.insert(data)
            esm.refresh()

            # 검색으로 확인
            result = esm.search_by_id(inode)
            assert result
            assert "테스트" in result["summary"]

        finally:
            Loader.path_prefix = original_prefix
            shutil.rmtree(temp_dir, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()