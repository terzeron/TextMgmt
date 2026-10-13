#!/usr/bin/env python
"""loader 재적재 경로의 드문 분기를 확인한다."""

from unittest.mock import MagicMock

from tests.test_loader import _setup_loader_env


def test_reload_ignores_stored_created_time_of_files_outside_the_batch(monkeypatch, tmp_path):
    """ES가 이번 배치에 없는 inode의 메타데이터를 같이 돌려줘도 무시한다."""
    _setup_loader_env(monkeypatch, tmp_path)
    file_path = tmp_path / "[author] book.txt"
    file_path.write_text("content", encoding="utf-8")
    inode = file_path.stat().st_ino
    monkeypatch.setattr("sys.argv", ["loader", "--reload", "book", str(file_path)])
    from utils.loader import main

    stranger = inode + 1_000_000
    inserted_data: dict = {}

    def capture_insert(data):
        inserted_data.update(data)
        return list(data)

    mock_es = MagicMock()
    mock_es.es.ping.return_value = True
    mock_es.get_existing_created_time_metadata.return_value = {
        stranger: {"created_time": "1999-01-01T00:00:00", "created_time_source": "statx_btime"},
        inode: {"created_time": "2020-01-01T00:00:00"},
    }
    mock_es.delete_by_file_paths.return_value = 0
    mock_es.insert.side_effect = capture_insert
    monkeypatch.setattr("utils.loader.ESManager", lambda index_name: mock_es)

    assert main() == 0
    assert set(inserted_data) == {inode}
    assert inserted_data[inode]["created_time"] == "2020-01-01T00:00:00"
