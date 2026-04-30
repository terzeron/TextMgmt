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

    def test_init_uses_optional_defaults_and_score(self):
        info = {
            "category": "cat2",
            "title": "title2",
            "author": "author2",
            "file_path": "cat2/file.pdf",
            "file_type": "pdf",
            "file_size": 20,
            "updated_time": "2022-02-02T00:00:00.000000",
        }
        c = Comics(3, info, score=2.5)
        assert c.line_count == 0
        assert c.page_count == 0
        assert c.isbn == ""
        assert c.summary == ""
        assert c.score == 2.5
        assert c.dict()["file_path"] == "cat2/file.pdf"


def test_comics_manager_missing_env(monkeypatch):
    """comics_manager.py line 21: RuntimeError when TM_COMICS_DIR is not set"""
    monkeypatch.delenv("TM_COMICS_DIR", raising=False)
    from backend.comics_manager import ComicsManager
    import pytest

    with pytest.raises(RuntimeError, match="TM_COMICS_DIR"):
        ComicsManager()


def test_comics_manager_init(monkeypatch, tmp_path: Path):
    from backend import comics_manager as comics_manager_mod

    class DummyESManager:
        def __init__(self, index_name):
            self.index_name = index_name
            self.created = False

        def create_index(self):
            self.created = True

    monkeypatch.setenv("TM_COMICS_DIR", str(tmp_path))
    monkeypatch.setenv("TM_ES_COMICS_INDEX", "test_comics")
    monkeypatch.setattr(comics_manager_mod, "ESManager", DummyESManager)

    manager = comics_manager_mod.ComicsManager()
    assert manager.path_prefix == tmp_path
    assert manager.es_manager.index_name == "test_comics"
    assert manager.es_manager.created is True
    assert manager._mismatch_cache is None
    assert manager._mismatch_cache_time == 0.0


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
