import sys
from pathlib import Path

from tests import test_runner


def test_run_coverage_report_prints_report(monkeypatch, tmp_path):
    """run_coverage_report combines shards and prints report."""
    calls = []

    def fake_run(cmd, **kwargs):
        calls.append(cmd)
        if "combine" in cmd:
            import sqlite3

            sqlite3.connect(coverage_file).close()

        class Result:
            returncode = 0
            stdout = "Name  Stmts   Miss  Cover\nTOTAL  100  0  100%\n"
            stderr = ""

        return Result()

    coverage_file = tmp_path / ".coverage"
    shards_dir = tmp_path / ".coverage_out"
    shards_dir.mkdir()
    shard_file = shards_dir / ".coverage.tests__test_sample.py"
    import sqlite3

    sqlite3.connect(coverage_file).close()
    sqlite3.connect(shard_file).close()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: tmp_path / ".coverage.last")
    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: shards_dir)

    ok = test_runner.run_coverage_report()

    assert ok is True
    assert len(calls) == 2
    assert calls[0][-2:] == ["--keep", str(shards_dir)] or "combine" in calls[0]
    assert "report" in calls[1]


def test_run_coverage_report_no_data(monkeypatch, tmp_path):
    """run_coverage_report returns False when no .coverage exists."""
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: tmp_path / ".coverage")
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: tmp_path / ".coverage.last")
    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: tmp_path / ".coverage_out")

    ok = test_runner.run_coverage_report()
    assert ok is False


def test_run_coverage_report_falls_back_to_backup(monkeypatch, tmp_path):
    """run_coverage_report uses .coverage.last when .coverage is missing."""
    calls = []

    def fake_run(cmd, **kwargs):
        calls.append(kwargs.get("env", {}).get("COVERAGE_FILE", ""))

        class Result:
            returncode = 0
            stdout = "Name  Stmts   Miss  Cover\nTOTAL  100  0  100%\n"
            stderr = ""

        return Result()

    backup_file = tmp_path / ".coverage.last"
    import sqlite3

    sqlite3.connect(backup_file).close()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: tmp_path / ".coverage")
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: backup_file)
    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: tmp_path / ".coverage_out")

    ok = test_runner.run_coverage_report()
    assert ok is True
    assert str(backup_file) in calls[0]


def test_run_coverage_report_caches_data(monkeypatch, tmp_path):
    """run_coverage_report copies .coverage to .coverage.last on success."""
    coverage_file = tmp_path / ".coverage"
    backup_file = tmp_path / ".coverage.last"
    shards_dir = tmp_path / ".coverage_out"
    shards_dir.mkdir()
    shard_file = shards_dir / ".coverage.tests__test_sample.py"
    import sqlite3

    sqlite3.connect(coverage_file).close()
    sqlite3.connect(shard_file).close()

    def fake_run(cmd, **kwargs):
        if "combine" in cmd:
            import sqlite3

            sqlite3.connect(coverage_file).close()

        class Result:
            returncode = 0
            stdout = "Name  Stmts   Miss  Cover\nTOTAL  100  0  100%\n"
            stderr = ""

        return Result()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: backup_file)
    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: shards_dir)

    test_runner.run_coverage_report()
    assert backup_file.exists()


def test_run_coverage_report_removes_corrupted(monkeypatch, tmp_path):
    """run_coverage_report removes corrupted .coverage files."""
    coverage_file = tmp_path / ".coverage"
    coverage_file.write_text("not a sqlite db")

    def fake_run(cmd, **kwargs):
        class Result:
            returncode = 0
            stdout = "No data to report."
            stderr = ""

        return Result()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: tmp_path / ".coverage.last")
    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: tmp_path / ".coverage_out")

    test_runner.run_coverage_report()
    assert not coverage_file.exists()


def test_prepare_coverage_data_removes_corrupted_shard(monkeypatch, tmp_path):
    """prepare_coverage_data removes corrupted shard files."""
    shards_dir = tmp_path / ".coverage_out"
    shards_dir.mkdir()
    shard = shards_dir / ".coverage.tests__test_sample.py"
    shard.write_text("garbage")

    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: tmp_path / ".coverage")
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: tmp_path / ".coverage.last")
    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: shards_dir)

    test_runner.prepare_coverage_data()
    assert not shard.exists()


def test_reset_coverage_shard_replaces_existing(monkeypatch, tmp_path):
    """reset_coverage_shard deletes old shard and combined coverage."""
    shards_dir = tmp_path / ".coverage_out"
    coverage_file = tmp_path / ".coverage"
    shard = shards_dir / ".coverage.tests__test_sample.py"
    shards_dir.mkdir()
    shard.write_text("old")
    coverage_file.write_text("old")

    monkeypatch.setattr(test_runner, "get_coverage_shards_dir", lambda: shards_dir)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)
    monkeypatch.setattr(test_runner, "get_coverage_shard_file", lambda _p: shard)

    result = test_runner.reset_coverage_shard(Path("/not/used.py"))

    assert result == shard
    assert not shard.exists()
    assert not coverage_file.exists()


def test_prepare_coverage_data_keeps_valid(monkeypatch, tmp_path):
    """prepare_coverage_data keeps valid .coverage file."""
    coverage_file = tmp_path / ".coverage"
    import sqlite3

    sqlite3.connect(coverage_file).close()
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)

    test_runner.prepare_coverage_data()
    assert coverage_file.exists()


def test_prepare_coverage_data_removes_corrupted(monkeypatch, tmp_path):
    """prepare_coverage_data removes corrupted .coverage file."""
    coverage_file = tmp_path / ".coverage"
    coverage_file.write_text("garbage")
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)

    test_runner.prepare_coverage_data()
    assert not coverage_file.exists()


def test_maybe_run_coverage_runs_when_success(monkeypatch):
    called = {"count": 0}

    def fake_run_coverage():
        called["count"] += 1
        return True

    monkeypatch.setattr(test_runner, "run_coverage_report", fake_run_coverage)

    assert test_runner.maybe_run_coverage(True) is True
    assert called["count"] == 1


def test_maybe_run_coverage_skips_on_failure(monkeypatch):
    called = {"count": 0}

    def fake_run_coverage():
        called["count"] += 1
        return True

    monkeypatch.setattr(test_runner, "run_coverage_report", fake_run_coverage)

    assert test_runner.maybe_run_coverage(False) is False
    assert called["count"] == 0
