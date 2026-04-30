import importlib
import sys
from pathlib import Path

import pytest


sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


class DummyESManager:
    def __init__(self):
        self.title_calls = []
        self.similar_calls = []

    def search_by_title(self, title, ext, size):
        self.title_calls.append((title, ext, size))
        return [
            (1, {"summary": "line1\nline2", "title": title}, 100.0),
        ]

    def search_similar_docs(self, summary=""):
        self.similar_calls.append(summary)
        return [
            (2, {"summary": "content summary", "title": "similar"}, 88.0),
        ]


def _load_search_module(monkeypatch, es_instance):
    import utils.search as search_mod

    monkeypatch.setattr(search_mod, "ESManager", lambda: es_instance)
    return search_mod


def test_print_usage_exits(capsys):
    import utils.search as search_mod

    with pytest.raises(SystemExit) as exc_info:
        search_mod.print_usage("prog")

    assert exc_info.value.code == 0
    out = capsys.readouterr().out
    assert "Usage:" in out
    assert "prog" in out


def test_main_requires_title(monkeypatch):
    import utils.search as search_mod

    monkeypatch.setattr(sys, "argv", ["search.py"])
    with pytest.raises(SystemExit):
        search_mod.main()


def test_main_searches_by_title_and_content(monkeypatch, capsys):
    es = DummyESManager()
    search_mod = _load_search_module(monkeypatch, es)

    monkeypatch.setattr(sys, "argv", ["search.py", "hello", "body", "epub", "7"])
    assert search_mod.main() == 0

    assert es.title_calls == [("hello", "epub", 7)]
    assert es.similar_calls == ["body"]

    out = capsys.readouterr().out
    assert "item:" in out
    assert "line1 line2" in out
    assert "content summary" in out


def test_main_with_only_title_uses_default_ext_and_size(monkeypatch):
    es = DummyESManager()
    search_mod = _load_search_module(monkeypatch, es)

    monkeypatch.setattr(sys, "argv", ["search.py", "hello"])
    assert search_mod.main() == 0
    assert es.title_calls == [("hello", "", 0)]
    assert es.similar_calls == []
