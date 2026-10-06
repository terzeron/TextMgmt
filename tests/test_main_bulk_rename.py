"""POST /categories/bulk-rename-files 라우트 테스트 — ES/MySQL 없이 manager 를 mock 한다."""

import time
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from fastapi.testclient import TestClient

import backend.main as main_module

ADMIN_PAYLOAD = {"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": int(time.time()) + 3600}
URL = "/categories/bulk-rename-files"


@pytest.fixture(autouse=True)
def override_auth():
    main_module.app.dependency_overrides[main_module.require_auth] = lambda: ADMIN_PAYLOAD
    main_module.app.dependency_overrides[main_module.require_admin] = lambda: ADMIN_PAYLOAD
    yield
    main_module.app.dependency_overrides.clear()


@pytest.fixture
def client():
    return TestClient(main_module.app)


@pytest.fixture
def mock_bm():
    m = AsyncMock()
    m.es_manager = MagicMock()
    prev = main_module.book_manager._instance
    object.__setattr__(main_module.book_manager, "_instance", m)
    yield m
    object.__setattr__(main_module.book_manager, "_instance", prev)


def _book(path: Path, book_id: int = 1):
    b = MagicMock()
    b.file_path = path
    b.book_id = book_id
    b.category = "cat"
    b.author = "author"
    b.file_type = "txt"
    return b


def _setup(mock_bm, paths):
    """paths 를 한 카테고리의 책으로 돌려주는 manager. update_book 은 파일을 옮기지 않는다."""
    mock_bm.get_categories.return_value = (["cat"], None)
    mock_bm.get_books_in_category_paged.return_value = ([_book(p, i) for i, p in enumerate(paths, 1)], len(paths), "", None)

    mock_bm.update_book.return_value = ({"ok": True}, None)


def _touch(directory: Path, *names):
    out = []
    for name in names:
        p = directory / name
        p.write_text("x")
        out.append(p)
    return out


@pytest.mark.parametrize("category", ["", "_root"])
def test_requires_directory_category(client, mock_bm, category):
    resp = client.post(URL, json={"category": category, "pattern": "a", "replacement": "b"})
    assert resp.status_code == 400


@pytest.mark.parametrize("pattern", ["", "a" * 257])
def test_rejects_pattern_length(client, mock_bm, pattern):
    resp = client.post(URL, json={"category": "cat", "pattern": pattern, "replacement": "b"})
    assert resp.json()["status"] == "failure"
    assert resp.json()["changed_count"] == 0


def test_category_lookup_error(client, mock_bm):
    mock_bm.get_categories.return_value = ([], "es down")
    resp = client.post(URL, json={"category": "cat", "pattern": "a", "replacement": "b"})
    assert resp.json() == {"status": "failure", "error": "es down", "changed_count": 0}


def test_invalid_regex(client, mock_bm):
    mock_bm.get_categories.return_value = (["cat"], None)
    resp = client.post(URL, json={"category": "cat", "pattern": "(", "replacement": "b"})
    body = resp.json()
    assert body["status"] == "failure" and body["error"].startswith("정규표현식 오류")


def test_paging_follows_cursor_and_includes_subcategories(client, mock_bm, tmp_path):
    first, second = _touch(tmp_path, "old1.txt", "old2.txt")
    mock_bm.get_categories.return_value = (["cat", "cat/sub", "other"], None)
    mock_bm.get_books_in_category_paged.side_effect = [
        ([_book(first, 1)], 2, "next", None),
        ([_book(second, 2)], 2, "", None),
        ([], 0, "", None),
    ]
    mock_bm.update_book.return_value = ({}, None)
    resp = client.post(URL, json={"category": "cat", "pattern": "old", "replacement": "new"})
    assert resp.json()["result"]["changed_count"] == 2
    visited = [c.args[0] for c in mock_bm.get_books_in_category_paged.call_args_list]
    assert visited == ["cat", "cat", "cat/sub"]
    mock_bm.es_manager.refresh.assert_called_once()


def test_paging_error(client, mock_bm, tmp_path):
    mock_bm.get_categories.return_value = (["cat"], None)
    mock_bm.get_books_in_category_paged.return_value = ([], 0, "", "page failed")
    resp = client.post(URL, json={"category": "cat", "pattern": "a", "replacement": "b"})
    assert resp.json() == {"status": "failure", "error": "page failed", "changed_count": 0}


@pytest.mark.parametrize("replacement", ["", "x/y", "x\\y"])
def test_rejects_unsafe_new_name(client, mock_bm, tmp_path, replacement):
    paths = _touch(tmp_path, "a")
    _setup(mock_bm, paths)
    # 이름 전체를 바꾸는 패턴이라 replacement 가 곧 새 이름이다. ".", ".." 도 막는다.
    resp = client.post(URL, json={"category": "cat", "pattern": "^a$", "replacement": replacement})
    body = resp.json()
    assert body["status"] == "failure" and body["changed_count"] == 0
    mock_bm.update_book.assert_not_called()


@pytest.mark.parametrize("replacement", [".", ".."])
def test_rejects_dot_names(client, mock_bm, tmp_path, replacement):
    paths = _touch(tmp_path, "a")
    _setup(mock_bm, paths)
    resp = client.post(URL, json={"category": "cat", "pattern": "^a$", "replacement": replacement})
    assert resp.json()["status"] == "failure"


def test_unchanged_names_are_skipped(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "keep.txt")
    _setup(mock_bm, paths)
    resp = client.post(URL, json={"category": "cat", "pattern": "zzz", "replacement": "b"})
    assert resp.json()["result"] == {"changed_count": 0, "failed_count": 0, "failures": []}
    mock_bm.update_book.assert_not_called()


def test_update_failure_is_reported(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "old.txt")
    _setup(mock_bm, paths)
    mock_bm.update_book.return_value = (None, "db error")
    resp = client.post(URL, json={"category": "cat", "pattern": "old", "replacement": "new"})
    assert resp.json()["result"] == {"changed_count": 0, "failed_count": 1, "failures": [{"file": "old.txt", "error": "db error"}]}


def test_existing_target_gets_next_serial(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "a1.txt", "ax.txt", "ax (1).txt", "ax (4).txt", "unrelated.txt")
    _setup(mock_bm, paths[:1])
    resp = client.post(URL, json={"category": "cat", "pattern": "a1", "replacement": "ax"})
    assert resp.json()["result"]["changed_count"] == 1
    assert mock_bm.update_book.call_args.args[4].name == "ax (5).txt"


def test_serial_skips_names_reserved_by_earlier_files(client, mock_bm, tmp_path):
    # update_book 이 파일을 옮기지 않으므로 iterdir 은 앞서 정한 이름을 못 본다. reserved 가 막아야 한다.
    paths = _touch(tmp_path, "a1.txt", "a2.txt", "a3.txt", "ax.txt")
    _setup(mock_bm, paths[:3])
    resp = client.post(URL, json={"category": "cat", "pattern": r"a\d", "replacement": "ax"})
    assert resp.json()["result"]["changed_count"] == 3
    targets = [c.args[4].name for c in mock_bm.update_book.call_args_list]
    assert targets == ["ax (1).txt", "ax (2).txt", "ax (3).txt"]


def test_same_new_name_in_one_batch_gets_serials(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "a1.txt", "a2.txt", "a3.txt")
    _setup(mock_bm, paths)
    resp = client.post(URL, json={"category": "cat", "pattern": r"a\d", "replacement": "ax"})
    assert resp.json()["result"]["changed_count"] == 3
    targets = [c.args[4].name for c in mock_bm.update_book.call_args_list]
    assert targets == ["ax.txt", "ax (1).txt", "ax (2).txt"]


def test_reserved_serial_skips_file_already_on_disk(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "a1.txt", "a2.txt", "ax (1).txt")
    _setup(mock_bm, paths[:2])
    resp = client.post(URL, json={"category": "cat", "pattern": r"a\d", "replacement": "ax"})
    assert resp.json()["result"]["changed_count"] == 2
    targets = [c.args[4].name for c in mock_bm.update_book.call_args_list]
    assert targets == ["ax.txt", "ax (2).txt"]


def test_name_without_extension_gets_serial(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "a1", "a2")
    _setup(mock_bm, paths)
    resp = client.post(URL, json={"category": "cat", "pattern": r"a\d", "replacement": "ax"})
    targets = [c.args[4].name for c in mock_bm.update_book.call_args_list]
    assert targets == ["ax", "ax (1)"]


def test_directory_scan_failure_is_reported(client, mock_bm, tmp_path):
    paths = _touch(tmp_path, "a1.txt", "ax.txt")
    _setup(mock_bm, paths[:1])
    with patch.object(Path, "iterdir", side_effect=OSError("denied")):
        resp = client.post(URL, json={"category": "cat", "pattern": "a1", "replacement": "ax"})
    assert resp.json()["result"] == {"changed_count": 0, "failed_count": 1, "failures": [{"file": "a1.txt", "error": "denied"}]}
    mock_bm.update_book.assert_not_called()
