import sys

from tests import test_runner


def test_run_coverage_report_prints_report(monkeypatch, tmp_path):
    """run_coverage_report reads .coverage and prints report."""
    calls = []

    def fake_run(cmd, **kwargs):
        calls.append(cmd)

        class Result:
            returncode = 0
            stdout = "Name  Stmts   Miss  Cover\nTOTAL  100  0  100%\n"
            stderr = ""

        return Result()

    coverage_file = tmp_path / ".coverage"
    import sqlite3

    sqlite3.connect(coverage_file).close()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: tmp_path / ".coverage.last")

    ok = test_runner.run_coverage_report()

    assert ok is True
    assert len(calls) == 1
    assert "coverage" in calls[0]
    assert "report" in calls[0]


def test_run_coverage_report_no_data(monkeypatch, tmp_path):
    """run_coverage_report returns False when no .coverage exists."""
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: tmp_path / ".coverage")
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: tmp_path / ".coverage.last")

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

    ok = test_runner.run_coverage_report()
    assert ok is True
    assert str(backup_file) in calls[0]


def test_run_coverage_report_caches_data(monkeypatch, tmp_path):
    """run_coverage_report copies .coverage to .coverage.last on success."""
    coverage_file = tmp_path / ".coverage"
    backup_file = tmp_path / ".coverage.last"
    import sqlite3

    sqlite3.connect(coverage_file).close()

    def fake_run(cmd, **kwargs):
        class Result:
            returncode = 0
            stdout = "Name  Stmts   Miss  Cover\nTOTAL  100  0  100%\n"
            stderr = ""

        return Result()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: coverage_file)
    monkeypatch.setattr(test_runner, "get_coverage_backup_file", lambda: backup_file)

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

    test_runner.run_coverage_report()
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
