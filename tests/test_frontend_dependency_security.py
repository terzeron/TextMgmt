import json
from pathlib import Path


def test_frontend_lockfile_pins_patched_lodash():
    lockfile = Path(__file__).resolve().parent.parent / "frontend" / "package-lock.json"
    package_lock = json.loads(lockfile.read_text(encoding="utf-8"))
    lodash_entry = package_lock["packages"]["node_modules/lodash"]

    assert lodash_entry["version"] == "4.18.1"
    assert lodash_entry["resolved"].endswith("/lodash-4.18.1.tgz")
