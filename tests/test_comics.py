import unittest
import sys
import os
from pathlib import Path

os.environ["TM_COMICS_DIR"] = "/tmp/comics"
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from backend.comics import Comics


class TestComics(unittest.TestCase):
    def test_init_and_dict(self):
        info = {"category": "cat", "title": "title", "author": "author", "file_path": "cat/file.epub", "file_type": "epub", "file_size": 10, "updated_time": "2021-01-01T00:00:00.000000"}
        c = Comics(1, info, score=1.5)
        data = c.dict()
        assert data["book_id"] == 1
        assert data["category"] == "cat"
        assert data["file_path"] == "cat/file.epub"
        assert data["score"] == 1.5

    def test_json_and_str(self):
        info = {"category": "cat", "title": "title", "author": "author", "file_path": "cat/file.epub", "file_type": "epub", "file_size": 10, "updated_time": "2021-01-01T00:00:00.000000"}
        c = Comics(2, info)
        assert "category" in c.json()
        assert "title" in str(c)


def test_comics_manager_missing_env(monkeypatch):
    """comics_manager.py line 21: RuntimeError when TM_COMICS_DIR is not set"""
    monkeypatch.delenv("TM_COMICS_DIR", raising=False)
    from backend.comics_manager import ComicsManager
    import pytest

    with pytest.raises(RuntimeError, match="TM_COMICS_DIR"):
        ComicsManager()


# ---- merged from test_comics_env_guard.py ----


def test_comics_requires_comics_dir():
    import importlib

    prev = os.environ.pop("TM_COMICS_DIR", None)
    try:
        if "backend.comics" in sys.modules:
            del sys.modules["backend.comics"]
        import pytest

        with pytest.raises(SystemExit):
            importlib.import_module("backend.comics")
    finally:
        if prev is not None:
            os.environ["TM_COMICS_DIR"] = prev
