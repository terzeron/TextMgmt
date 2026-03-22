import unittest
import sys
import os
from pathlib import Path

os.environ["TM_COMICS_DIR"] = "/tmp/comics"
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from backend.comics import Comics


class TestComics(unittest.TestCase):
    def test_init_and_dict(self):
        info = {
            "category": "cat",
            "title": "title",
            "author": "author",
            "file_path": "cat/file.epub",
            "file_type": "epub",
            "file_size": 10,
            "updated_time": "2021-01-01T00:00:00.000000",
        }
        c = Comics(1, info, score=1.5)
        data = c.dict()
        assert data["book_id"] == 1
        assert data["category"] == "cat"
        assert data["file_path"] == "cat/file.epub"
        assert data["score"] == 1.5

    def test_json_and_str(self):
        info = {
            "category": "cat",
            "title": "title",
            "author": "author",
            "file_path": "cat/file.epub",
            "file_type": "epub",
            "file_size": 10,
            "updated_time": "2021-01-01T00:00:00.000000",
        }
        c = Comics(2, info)
        assert "category" in c.json()
        assert "title" in str(c)
