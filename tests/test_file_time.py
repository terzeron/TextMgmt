from datetime import datetime
from types import SimpleNamespace

import pytest

from utils import file_time


class FakeStatxFn:
    def __init__(self, *, mask=file_time.STATX_BTIME, sec=1_700_000_000, nsec=123_456_000, result=0):
        self.mask = mask
        self.sec = sec
        self.nsec = nsec
        self.result = result
        self.calls = []
        self.argtypes = None
        self.restype = None

    def __call__(self, dirfd, pathname, flags, mask, statxbuf):
        self.calls.append((dirfd, pathname, flags, mask))
        if self.result != 0:
            return self.result
        buf = statxbuf._obj
        buf.stx_mask = self.mask
        buf.stx_btime.tv_sec = self.sec
        buf.stx_btime.tv_nsec = self.nsec
        return 0


def test_linux_statx_btime_uses_btime_when_mask_is_present(monkeypatch: pytest.MonkeyPatch):
    fake_statx = FakeStatxFn()
    monkeypatch.setattr(file_time.ctypes, "CDLL", lambda *args, **kwargs: SimpleNamespace(statx=fake_statx))

    result = file_time._linux_statx_btime("book.txt")

    assert result == datetime.fromtimestamp(1_700_000_000.123456)
    assert fake_statx.calls == [
        (
            file_time.AT_FDCWD,
            b"book.txt",
            file_time.AT_STATX_SYNC_AS_STAT,
            file_time.STATX_BTIME,
        )
    ]


def test_linux_statx_btime_returns_none_when_btime_mask_is_missing(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(file_time.ctypes, "CDLL", lambda *args, **kwargs: SimpleNamespace(statx=FakeStatxFn(mask=0)))

    assert file_time._linux_statx_btime("book.txt") is None


def test_path_created_time_prefers_linux_btime(monkeypatch: pytest.MonkeyPatch):
    expected = datetime(2024, 1, 2, 3, 4, 5, 123456)
    monkeypatch.setattr(file_time, "_linux_statx_btime", lambda _path: expected)
    stat_result = SimpleNamespace(st_ctime=datetime(2026, 1, 1).timestamp())

    assert file_time.path_created_time("book.txt", stat_result) == expected
    assert file_time.path_created_time_with_source("book.txt", stat_result) == (
        expected,
        file_time.CREATED_TIME_SOURCE_STATX_BTIME,
    )


def test_path_created_time_falls_back_to_ctime_when_btime_is_unavailable(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(file_time, "_linux_statx_btime", lambda _path: None)
    stat_result = SimpleNamespace(st_ctime=datetime(2025, 5, 6, 7, 8, 9).timestamp())

    assert file_time.path_created_time("book.txt", stat_result) == datetime(2025, 5, 6, 7, 8, 9)
    assert file_time.path_created_time_with_source("book.txt", stat_result) == (
        datetime(2025, 5, 6, 7, 8, 9),
        file_time.CREATED_TIME_SOURCE_ST_CTIME,
    )


def test_stat_created_time_keeps_platform_birthtime_priority():
    stat_result = SimpleNamespace(
        st_birthtime_ns=1_600_000_000_500_000_000,
        st_birthtime=1_700_000_000,
        st_ctime=1_800_000_000,
    )

    assert file_time.stat_created_time(stat_result) == datetime.fromtimestamp(1_600_000_000.5)
    assert file_time.stat_created_time_with_source(stat_result) == (
        datetime.fromtimestamp(1_600_000_000.5),
        file_time.CREATED_TIME_SOURCE_ST_BIRTHTIME,
    )
