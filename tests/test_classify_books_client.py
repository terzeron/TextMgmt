import json
from pathlib import Path
from types import SimpleNamespace

import pytest


class FakeClassifier:
    def __init__(self, library_root, delay):
        self.cache = {}
        self.calls = []
        self.saved = 0

    def classify_file(self, path, **kwargs):
        self.calls.append((path.name, kwargs))
        name = path.stem
        if name == "error":
            raise RuntimeError("classification failed")
        target = {
            "skip": None,
            "same": "0_telegram",
            "move": "3_판타지",
            "duplicate": "3_판타지",
            "conflict": "3_판타지",
            "linked": "3_판타지",
        }.get(name, "3_판타지")
        return target, "test", "fixture", {}

    def save_cache(self):
        self.saved += 1


def make_source(root, names):
    source = root / "0_telegram"
    source.mkdir(parents=True, exist_ok=True)
    for name in names:
        path = source / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(name, encoding="utf-8")
    return source


def test_direct_classifier_moves_skips_cleans_conflicts_and_logs_failures(tmp_path, monkeypatch, caplog):
    from utils import classify_books_client as module

    source = make_source(tmp_path, ["skip.epub", "same.pdf", "move.epub", "duplicate.pdf", "conflict.pdf", "linked.epub", "error.txt", ".hidden.epub", "ignored.png"])
    target = tmp_path / "3_판타지"
    target.mkdir()
    (target / "duplicate.pdf").write_text("existing")
    (target / "conflict.pdf").write_text("existing")
    linked_target = target / "linked.epub"
    linked_target.write_text("shared")
    (source / "linked.epub").unlink()
    (source / "linked.epub").symlink_to(linked_target)
    monkeypatch.setattr(module, "BookClassifierService", FakeClassifier)

    result = module.run_direct_classifier(tmp_path, "0_telegram", clean_existing=True, use_bookstore=False)

    assert result == 0
    assert not (source / "move.epub").exists()
    assert (target / "move.epub").exists()
    assert not (source / "duplicate.pdf").exists()
    assert (target / "duplicate.pdf").read_text() == "existing"
    assert not (source / "conflict.pdf").exists()
    assert (source / "linked.epub").is_symlink()
    assert (source / "skip.epub").exists() and (source / "same.pdf").exists()
    assert "classification failed" in caplog.text


def test_direct_classifier_filters_cached_entries_offset_and_limit(tmp_path, monkeypatch):
    from utils import classify_books_client as module

    make_source(tmp_path, ["cached.epub", "seen.pdf", "new.txt"])
    instances = []

    class CachedClassifier(FakeClassifier):
        def __init__(self, library_root, delay):
            super().__init__(library_root, delay)
            self.cache = {"cached.epub": {"target_category": None}, "seen.pdf": {"target_category": "3_판타지"}}
            instances.append(self)

    monkeypatch.setattr(module, "BookClassifierService", CachedClassifier)
    assert module.run_direct_classifier(tmp_path, "0_telegram", uncached_only=True, offset=1, limit=1, dry_run=True, re_evaluate=True) == 0
    assert [name for name, _ in instances[0].calls] == ["seen.pdf"]
    assert instances[0].calls[0][1]["use_bookstore"] is False
    assert instances[0].saved == 0


def test_direct_classifier_reports_missing_directory(tmp_path):
    from utils.classify_books_client import run_direct_classifier

    assert run_direct_classifier(tmp_path, "missing") == 1


def test_api_client_handles_auth_poll_retry_success_and_error(monkeypatch, caplog):
    from utils import classify_books_client as module

    class Response:
        def __init__(self, status, data=None, text=""):
            self.status_code, self.data, self.text = status, data or {}, text

        def json(self):
            return self.data

    class Client:
        def __init__(self, *, timeout):
            assert timeout == 30.0
            self.responses = [Response(200, {"status": "success"}), Response(503), Response(200, {"result": {"status": "done", "processed_count": 2}})]
            self.calls = []

        def __enter__(self):
            return self

        def __exit__(self, *_args):
            return False

        def post(self, *args, **kwargs):
            self.calls.append(("post", args, kwargs))
            return self.responses.pop(0)

        def get(self, *args, **kwargs):
            self.calls.append(("get", args, kwargs))
            return self.responses.pop(0)

    client = Client(timeout=30)
    monkeypatch.setattr(module, "httpx", SimpleNamespace(Client=lambda **kwargs: client))
    monkeypatch.setattr(module.time, "sleep", lambda _: None)
    assert module.run_api_client("http://example/", "0_telegram", token="secret") == 0
    assert client.calls[0][1][0] == "http://example/categories/auto-classify"
    assert client.calls[0][2]["headers"]["Authorization"] == "Bearer secret"
    assert "상태 조회 실패" in caplog.text

    for response in (Response(500, text="bad"), Response(200, {"status": "error", "error": "rejected"})):
        class OnePostClient(Client):
            def __init__(self, *, timeout):
                self.response = response
            def post(self, *_args, **_kwargs):
                return self.response
        monkeypatch.setattr(module, "httpx", SimpleNamespace(Client=OnePostClient))
        assert module.run_api_client("http://example", "cat") == 1

    class FailedClient(Client):
        def __init__(self, *, timeout):
            self.responses = [Response(200, {"status": "success"}), Response(200, {"result": {"status": "failed", "error": "worker"}})]
            self.calls = []
    monkeypatch.setattr(module, "httpx", SimpleNamespace(Client=FailedClient))
    assert module.run_api_client("http://example", "cat") == 1
    assert "작업 실패" in caplog.text


def test_api_client_handles_missing_dependency_and_transport_exception(monkeypatch):
    from utils import classify_books_client as module

    monkeypatch.setattr(module, "httpx", None)
    assert module.run_api_client("http://example", "cat") == 1

    class BrokenClient:
        def __init__(self, **_kwargs):
            raise OSError("offline")
    monkeypatch.setattr(module, "httpx", SimpleNamespace(Client=BrokenClient))
    assert module.run_api_client("http://example", "cat") == 1


def test_main_dispatches_api_and_direct_options(monkeypatch):
    from utils import classify_books_client as module

    captured = {}
    monkeypatch.setattr(module, "run_api_client", lambda **kwargs: captured.update(kwargs) or 7)
    monkeypatch.setattr(module.sys, "argv", ["client", "--api-url", "http://api", "--category", "books", "--token", "t", "--no-recursive", "--no-bookstore"])
    assert module.main() == 7
    assert captured["recursive"] is False and captured["use_bookstore"] is False and captured["token"] == "t"

    monkeypatch.setattr(module, "run_direct_classifier", lambda **kwargs: captured.update(kwargs) or 9)
    monkeypatch.setattr(module.sys, "argv", ["client", "--library-root", "/tmp/books", "--category", "input", "--offset", "3", "--uncached-only", "--re-evaluate", "--no-clean-existing", "--no-content-meta"])
    assert module.main() == 9
    assert captured["library_root"] == Path("/tmp/books")
    assert captured["offset"] == 3 and captured["uncached_only"] and captured["re_evaluate"]
    assert captured["clean_existing"] is False and captured["use_content_meta"] is False


def test_module_entrypoint_help(monkeypatch, capsys):
    import runpy
    import sys

    monkeypatch.setattr(sys, "argv", ["utils.classify_books_client", "--help"])
    with pytest.raises(SystemExit) as exited:
        runpy.run_module("utils.classify_books_client", run_name="__main__")
    assert exited.value.code == 0
    assert "--library-root" in capsys.readouterr().out


def test_direct_classifier_conflict_without_cleaning_is_left_in_place(tmp_path, monkeypatch, caplog):
    from utils import classify_books_client as module

    source = make_source(tmp_path, ["conflict.pdf"])
    target = tmp_path / "3_판타지"
    target.mkdir()
    (target / "conflict.pdf").write_text("existing")
    monkeypatch.setattr(module, "BookClassifierService", FakeClassifier)

    assert module.run_direct_classifier(tmp_path, "0_telegram", clean_existing=False) == 0

    assert (source / "conflict.pdf").exists()
    assert "[CONFLICT]" in caplog.text


def test_direct_classifier_dry_run_records_move_and_duplicate_clean(tmp_path, monkeypatch, caplog):
    from utils import classify_books_client as module

    source = make_source(tmp_path, ["move.epub", "duplicate.pdf"])
    target = tmp_path / "3_판타지"
    target.mkdir()
    (target / "duplicate.pdf").write_text("existing")
    monkeypatch.setattr(module, "BookClassifierService", FakeClassifier)

    assert module.run_direct_classifier(tmp_path, "0_telegram", dry_run=True) == 0

    assert (source / "move.epub").exists() and (source / "duplicate.pdf").exists()
    assert "[DRY-RUN MOVE]" in caplog.text and "[DRY-RUN CLEAN]" in caplog.text


def test_direct_classifier_saves_cache_and_reports_batches_at_boundaries(tmp_path, monkeypatch, caplog):
    from utils import classify_books_client as module

    make_source(tmp_path, [f"skip-{index:03}.epub" for index in range(500)])
    instances = []

    class SkippingClassifier(FakeClassifier):
        def __init__(self, library_root, delay):
            super().__init__(library_root, delay)
            instances.append(self)

        def classify_file(self, path, **kwargs):
            self.calls.append((path.name, kwargs))
            return None, "none", "uncategorized", {}

    monkeypatch.setattr(module, "BookClassifierService", SkippingClassifier)
    assert module.run_direct_classifier(tmp_path, "0_telegram", limit=0) == 0
    assert instances[0].saved == 2
    assert any("200/500 진행 중" in record.message for record in caplog.records)
    assert any("400/500 진행 중" in record.message for record in caplog.records)


def test_direct_classifier_collects_recursively_and_stops_at_limit(tmp_path, monkeypatch):
    from utils import classify_books_client as module

    make_source(tmp_path, ["one.epub", "nested/two.pdf", "nested/three.txt"])
    instances = []

    class RecordingClassifier(FakeClassifier):
        def __init__(self, library_root, delay):
            super().__init__(library_root, delay)
            instances.append(self)

        def classify_file(self, path, **kwargs):
            self.calls.append((path.name, kwargs))
            return None, "none", "skip", {}

    monkeypatch.setattr(module, "BookClassifierService", RecordingClassifier)
    assert module.run_direct_classifier(tmp_path, "0_telegram", recursive=False, limit=1, dry_run=True) == 0
    assert len(instances[0].calls) == 1


def test_direct_classifier_falls_back_when_samefile_check_fails(tmp_path, monkeypatch, caplog):
    from utils import classify_books_client as module

    source = make_source(tmp_path, ["duplicate.pdf"])
    target = tmp_path / "3_판타지"
    target.mkdir()
    (target / "duplicate.pdf").write_text("existing")

    def samefile_error(_self, _other):
        raise OSError("samefile unavailable")

    monkeypatch.setattr(module.Path, "samefile", samefile_error)
    monkeypatch.setattr(module, "BookClassifierService", FakeClassifier)
    assert module.run_direct_classifier(tmp_path, "0_telegram") == 0
    assert not (source / "duplicate.pdf").exists()


def test_direct_classifier_moving_batch_logs_progress_and_saves_cache(tmp_path, monkeypatch, caplog):
    from utils import classify_books_client as module

    source = make_source(tmp_path, [f"move-{index:03}.epub" for index in range(500)])
    instances = []

    class MovingClassifier(FakeClassifier):
        def __init__(self, library_root, delay):
            super().__init__(library_root, delay)
            instances.append(self)

        def classify_file(self, path, **kwargs):
            self.calls.append((path.name, kwargs))
            return "3_판타지", "fixture", "move", {}

    monkeypatch.setattr(module, "BookClassifierService", MovingClassifier)
    assert module.run_direct_classifier(tmp_path, "0_telegram", limit=0) == 0
    assert instances[0].saved == 2
    assert any("200/500 진행 중" in record.message for record in caplog.records)
    assert any("400/500 진행 중" in record.message for record in caplog.records)
    assert len(list((tmp_path / "3_판타지").glob("*.epub"))) == 500


def test_module_entrypoint_imports_without_httpx_and_adds_repo_path(monkeypatch, capsys):
    import builtins
    import runpy
    import sys

    original_import = builtins.__import__

    def no_httpx(name, *args, **kwargs):
        if name == "httpx":
            raise ImportError("disabled for this test")
        return original_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", no_httpx)
    repo = str(Path(__file__).resolve().parents[1])
    monkeypatch.setattr(sys, "path", [entry for entry in sys.path if entry != repo])
    monkeypatch.setattr(sys, "argv", ["utils.classify_books_client", "--help"])
    with pytest.raises(SystemExit) as exited:
        runpy.run_module("utils.classify_books_client", run_name="__main__")
    assert exited.value.code == 0
    assert "--api-url" in capsys.readouterr().out
    assert repo in sys.path


def test_uncached_only_skips_a_cached_unclassified_book(tmp_path, monkeypatch):
    from utils import classify_books_client as module

    make_source(tmp_path, ["cached.epub"])
    instances = []

    class CachedClassifier(FakeClassifier):
        def __init__(self, library_root, delay):
            super().__init__(library_root, delay)
            self.cache = {"cached.epub": {"target_category": None}}
            instances.append(self)

    monkeypatch.setattr(module, "BookClassifierService", CachedClassifier)
    assert module.run_direct_classifier(tmp_path, "0_telegram", uncached_only=True, dry_run=True) == 0
    assert instances[0].calls == []
