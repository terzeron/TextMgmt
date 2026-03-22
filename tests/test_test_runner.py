import sys

from tests import test_runner


def test_run_coverage_report_invokes_coverage(monkeypatch, tmp_path):
    calls = []

    def fake_run(cmd, **kwargs):
        calls.append((cmd, kwargs))

        class Result:
            returncode = 0

        return Result()

    monkeypatch.setattr(test_runner.subprocess, "run", fake_run)
    monkeypatch.setattr(test_runner, "get_coverage_file", lambda: tmp_path / ".coverage")
    (tmp_path / ".coverage").write_text("data")

    ok = test_runner.run_coverage_report()

    assert ok is True
    assert calls, "subprocess.run should be called"
    cmd, kwargs = calls[0]
    assert cmd[0] == sys.executable
    assert cmd[1:3] == ["-m", "coverage"]
    assert "report" in cmd
    assert kwargs.get("cwd") == test_runner.PROJECT_ROOT


def test_maybe_run_coverage_runs_when_success(monkeypatch):
    called = {"count": 0}

    def fake_run_coverage():
        called["count"] += 1
        return True

    monkeypatch.setattr(test_runner, "run_coverage_report", fake_run_coverage)

    assert test_runner.maybe_run_coverage(True) is True
    assert called["count"] == 1
