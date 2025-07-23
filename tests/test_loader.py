#!/usr/bin/env python

import sys
import os
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
        content = self.loader.read_from_text(self.txt_file_path)
        assert content
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE

    def test_read_from_epub_with_extracting_zip(self):
        content = self.loader.read_from_epub_with_extracting_zip(self.epub_file_path)
        assert content
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE

    def test_read_from_epub(self):
        content = self.loader.read_from_epub(self.epub_file_path)
        assert content
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE

    def test_read_from_pdf(self):
        content = self.loader.read_from_pdf(self.pdf_file_path)
        assert content is not None
        assert isinstance(content, str)
        assert 0 <= len(content) <= Loader.TEXT_SIZE

    def test_read_from_html(self):
        content = self.loader.read_from_html(self.html_file_path)
        assert content
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE

    def test_read_from_docx(self):
        content = self.loader.read_from_docx(self.docx_file_path)
        assert content
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE

    def test_read_from_rtf(self):
        content = self.loader.read_from_rtf(self.rtf_file_path)
        assert content
        assert isinstance(content, str)
        assert 0 < len(content) <= Loader.TEXT_SIZE

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


if __name__ == "__main__":
    unittest.main()