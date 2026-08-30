from datetime import datetime
import ctypes
import os
import sys


AT_FDCWD = -100
AT_STATX_SYNC_AS_STAT = 0x0000
STATX_BTIME = 0x00000800
CREATED_TIME_SOURCE_STATX_BTIME = "statx_btime"
CREATED_TIME_SOURCE_ST_BIRTHTIME = "st_birthtime"
CREATED_TIME_SOURCE_ST_CTIME = "st_ctime"


class _StatxTimestamp(ctypes.Structure):
    _fields_ = [
        ("tv_sec", ctypes.c_int64),
        ("tv_nsec", ctypes.c_uint32),
        ("__reserved", ctypes.c_int32),
    ]


class _Statx(ctypes.Structure):
    _fields_ = [
        ("stx_mask", ctypes.c_uint32),
        ("stx_blksize", ctypes.c_uint32),
        ("stx_attributes", ctypes.c_uint64),
        ("stx_nlink", ctypes.c_uint32),
        ("stx_uid", ctypes.c_uint32),
        ("stx_gid", ctypes.c_uint32),
        ("stx_mode", ctypes.c_uint16),
        ("__spare0", ctypes.c_uint16 * 1),
        ("stx_ino", ctypes.c_uint64),
        ("stx_size", ctypes.c_uint64),
        ("stx_blocks", ctypes.c_uint64),
        ("stx_attributes_mask", ctypes.c_uint64),
        ("stx_atime", _StatxTimestamp),
        ("stx_btime", _StatxTimestamp),
        ("stx_ctime", _StatxTimestamp),
        ("stx_mtime", _StatxTimestamp),
        ("stx_rdev_major", ctypes.c_uint32),
        ("stx_rdev_minor", ctypes.c_uint32),
        ("stx_dev_major", ctypes.c_uint32),
        ("stx_dev_minor", ctypes.c_uint32),
        ("stx_mnt_id", ctypes.c_uint64),
        ("stx_dio_mem_align", ctypes.c_uint32),
        ("stx_dio_offset_align", ctypes.c_uint32),
        ("stx_subvol", ctypes.c_uint64),
        ("stx_atomic_write_unit_min", ctypes.c_uint32),
        ("stx_atomic_write_unit_max", ctypes.c_uint32),
        ("stx_atomic_write_segments_max", ctypes.c_uint32),
        ("stx_dio_read_offset_align", ctypes.c_uint32),
        ("stx_atomic_write_unit_max_opt", ctypes.c_uint32),
        ("__spare2", ctypes.c_uint32 * 1),
        ("__spare3", ctypes.c_uint64 * 8),
    ]


def _timestamp_to_datetime(timestamp: _StatxTimestamp) -> datetime:
    return datetime.fromtimestamp(timestamp.tv_sec + timestamp.tv_nsec / 1_000_000_000)


def _linux_statx_btime(path: str | bytes | os.PathLike) -> datetime | None:
    if sys.platform != "linux" or ctypes.sizeof(_Statx) != 256:
        return None
    try:
        statx = ctypes.CDLL(None, use_errno=True).statx
    except (AttributeError, OSError):
        return None

    statx.argtypes = [
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_uint,
        ctypes.POINTER(_Statx),
    ]
    statx.restype = ctypes.c_int

    statxbuf = _Statx()
    if statx(AT_FDCWD, os.fsencode(path), AT_STATX_SYNC_AS_STAT, STATX_BTIME, ctypes.byref(statxbuf)) != 0:
        return None
    if not statxbuf.stx_mask & STATX_BTIME:
        return None
    return _timestamp_to_datetime(statxbuf.stx_btime)


def stat_created_time_with_source(stat_result: os.stat_result) -> tuple[datetime, str]:
    birthtime_ns = getattr(stat_result, "st_birthtime_ns", None)
    if birthtime_ns is not None:
        return datetime.fromtimestamp(birthtime_ns / 1_000_000_000), CREATED_TIME_SOURCE_ST_BIRTHTIME

    birthtime = getattr(stat_result, "st_birthtime", None)
    if birthtime is not None:
        return datetime.fromtimestamp(birthtime), CREATED_TIME_SOURCE_ST_BIRTHTIME

    return datetime.fromtimestamp(stat_result.st_ctime), CREATED_TIME_SOURCE_ST_CTIME


def stat_created_time(stat_result: os.stat_result) -> datetime:
    created_time, _source = stat_created_time_with_source(stat_result)
    return created_time


def stat_created_time_iso(stat_result: os.stat_result) -> str:
    return stat_created_time(stat_result).isoformat()


def path_created_time_with_source(path: str | bytes | os.PathLike, stat_result: os.stat_result | None = None) -> tuple[datetime, str]:
    linux_btime = _linux_statx_btime(path)
    if linux_btime is not None:
        return linux_btime, CREATED_TIME_SOURCE_STATX_BTIME
    return stat_created_time_with_source(stat_result or os.stat(path))


def path_created_time(path: str | bytes | os.PathLike, stat_result: os.stat_result | None = None) -> datetime:
    created_time, _source = path_created_time_with_source(path, stat_result)
    return created_time


def path_created_time_iso(path: str | bytes | os.PathLike, stat_result: os.stat_result | None = None) -> str:
    return path_created_time(path, stat_result).isoformat()
