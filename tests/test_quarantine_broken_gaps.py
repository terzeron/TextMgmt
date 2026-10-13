#!/usr/bin/env python
"""quarantine_broken.py 를 스크립트로 직접 실행하는 경로를 확인한다."""

import runpy
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent.parent / "utils" / "quarantine_broken.py"
REPO_ROOT = SCRIPT.parent.parent


def test_script_run_adds_repo_root_to_sys_path_and_exits_via_main(monkeypatch: pytest.MonkeyPatch):
    """저장소 루트가 sys.path 에 없어도 `python utils/quarantine_broken.py` 로 돌 수 있다."""
    without_root = [entry for entry in sys.path if Path(entry or ".").resolve() != REPO_ROOT]
    monkeypatch.setattr(sys, "path", without_root)
    monkeypatch.setattr(sys, "argv", ["quarantine_broken.py", "--help"])

    with pytest.raises(SystemExit) as exc_info:
        runpy.run_path(str(SCRIPT), run_name="__main__")

    assert exc_info.value.code == 0
    assert str(REPO_ROOT) in sys.path
