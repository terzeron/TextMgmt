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
    # 클래스 레벨에서 한 번만 파일 목록을 검색하여 성능 개선
    _path_prefix: Path
    _txt_dir_path: Path
    _txt_file_path: Path
    _epub_dir_path: Path
    _epub_file_path: Path
    _pdf_dir_path: Path
    _pdf_file_path: Path
    _html_dir_path: Path
    _html_file_path: Path
    _docx_dir_path: Path
    _docx_file_path: Path
    _rtf_dir_path: Path
    _rtf_file_path: Path
    _doc_dir_path: Path
    _doc_file_path: Path

    @classmethod
    def setUpClass(cls):
        """클래스 레벨에서 한 번만 실행 - 파일 목록 캐싱"""
        cls._path_prefix = Loader.path_prefix
        cls._txt_dir_path = cls._path_prefix / "_txt"
        cls._txt_file_path = list(cls._txt_dir_path.glob("*.txt"))[0]
        cls._epub_dir_path = cls._path_prefix / "_epub"
        cls._epub_file_path = list(cls._epub_dir_path.glob("*.epub"))[0]
        cls._pdf_dir_path = cls._path_prefix / "_pdf"
        cls._pdf_file_path = list(cls._pdf_dir_path.glob("*.pdf"))[0]
        cls._html_dir_path = cls._path_prefix / "_html"
        cls._html_file_path = list(cls._html_dir_path.glob("*.html"))[0]
        cls._docx_dir_path = cls._path_prefix / "_doc"
        cls._docx_file_path = list(cls._docx_dir_path.glob("*.docx"))[0]
        cls._rtf_dir_path = cls._path_prefix / "_rtf"
        cls._rtf_file_path = list(cls._rtf_dir_path.glob("*.rtf"))[0]
        cls._doc_dir_path = cls._path_prefix / "_doc"
        cls._doc_file_path = list(cls._doc_dir_path.glob("*.doc"))[0]

    def setUp(self):
        self.loader = Loader()
        # 클래스 변수를 인스턴스 변수로 복사 (기존 코드 호환성 유지)
        self.path_prefix = self._path_prefix
        self.txt_dir_path = self._txt_dir_path
        self.txt_file_path = self._txt_file_path
        self.epub_dir_path = self._epub_dir_path
        self.epub_file_path = self._epub_file_path
        self.pdf_dir_path = self._pdf_dir_path
        self.pdf_file_path = self._pdf_file_path
        self.html_dir_path = self._html_dir_path
        self.html_file_path = self._html_file_path
        self.docx_dir_path = self._docx_dir_path
        self.docx_file_path = self._docx_file_path
        self.rtf_dir_path = self._rtf_dir_path
        self.rtf_file_path = self._rtf_file_path
        self.doc_dir_path = self._doc_dir_path
        self.doc_file_path = self._doc_file_path

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

    def test_read_from_doc(self):
        result = self.loader.read_from_doc(self.doc_file_path)
        assert isinstance(result, tuple)
        assert len(result) == 3
        content, line_count, page_count = result
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE
        assert isinstance(line_count, int)
        assert line_count > 0
        assert isinstance(page_count, int)
        assert page_count == 0  # DOC은 page_count가 0

    def test_read_file_doc(self):
        """read_file이 .doc 파일을 정상 처리하는지 테스트"""
        data = self.loader.read_file(self.doc_file_path)
        assert data
        assert len(data) == 1
        self.inspect_data(data)
        for _, v in data.items():
            assert v["file_type"] == "doc"

    def test_read_file(self):
        data = self.loader.read_file(self.epub_file_path)
        assert data
        assert len(data) == 1
        self.inspect_data(data)

    def test_read_files(self):
        data = self.loader.read_files(self.epub_dir_path, 5)
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
        # txt 디렉토리만 사용하여 빠른 테스트
        data = self.loader.read_files(self.txt_dir_path, num_files=5, recursive=True)
        assert isinstance(data, dict)
        if data:
            self.inspect_data(data)

    def test_read_files_recursive_vs_non_recursive(self):
        """Test that recursive=True finds more or equal files than recursive=False."""
        # txt 디렉토리만 사용하여 빠른 테스트 (epub, docx 등은 파싱이 느림)
        non_recursive_data = self.loader.read_files(self.txt_dir_path, num_files=5, recursive=False)
        recursive_data = self.loader.read_files(self.txt_dir_path, num_files=5, recursive=True)
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

    def test_read_file_stat_reuse(self):
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
def es_manager_for_loader(es_client, es_index):
    """Loader 테스트용 ESManager fixture (공유된 ES 클라이언트 및 인덱스 사용)."""
    from backend.es_manager import ESManager

    esm = ESManager()
    esm.es = es_client

    yield esm


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