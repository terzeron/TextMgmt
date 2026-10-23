//! 파일 생성 시각. 원본: `utils/file_time.py`.
//!
//! Linux에서는 statx의 btime(표준 라이브러리 `created()`가 같은 호출을 쓴다)을 먼저 보고, 없으면
//! macOS의 birthtime, 그것도 없으면 ctime으로 내려간다. 시각은 Python `datetime.fromtimestamp`처럼
//! 시간대 정보가 없는 로컬 시각이다.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, NaiveDateTime, Timelike};

pub const CREATED_TIME_SOURCE_STATX_BTIME: &str = "statx_btime";
pub const CREATED_TIME_SOURCE_ST_BIRTHTIME: &str = "st_birthtime";
pub const CREATED_TIME_SOURCE_ST_CTIME: &str = "st_ctime";

/// `os.stat_result` 중 생성 시각에 쓰는 필드. 플랫폼에 없는 필드는 `None`이다.
#[derive(Debug, Clone, PartialEq)]
pub struct StatResult {
    pub birthtime_ns: Option<i128>,
    pub birthtime: Option<f64>,
    pub ctime: f64,
}

impl StatResult {
    pub fn from_metadata(m: &std::fs::Metadata) -> Self {
        let ctime = m.ctime() as f64 + m.ctime_nsec() as f64 / 1e9;
        // Python의 st_birthtime은 macOS 등에만 있고 Linux에는 없다.
        #[cfg(target_os = "macos")]
        let birthtime_ns = m.created().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as i128);
        #[cfg(not(target_os = "macos"))]
        let birthtime_ns = None;
        StatResult { birthtime_ns, birthtime: None, ctime }
    }
}

fn local_naive(sec: i64, micro: u32) -> NaiveDateTime {
    let utc = DateTime::from_timestamp(sec, micro * 1000).unwrap_or_default();
    utc.with_timezone(&Local).naive_local()
}

/// 초와 나노초로 로컬 시각을 만든다. 마이크로초는 짝수 쪽 반올림이고 올림이 초를 넘기면 다음 초로 간다.
pub fn datetime_from_parts(sec: i64, nsec: u32) -> NaiveDateTime {
    let (q, r) = (nsec / 1000, nsec % 1000);
    let micro = if r > 500 || (r == 500 && q % 2 == 1) { q + 1 } else { q };
    if micro >= 1_000_000 { local_naive(sec + 1, micro - 1_000_000) } else { local_naive(sec, micro) }
}

/// Python `datetime.fromtimestamp(float)`와 같은 변환.
pub fn datetime_from_float(ts: f64) -> NaiveDateTime {
    let whole = ts.trunc();
    let mut sec = whole as i64;
    let mut micro = ((ts - whole) * 1e6).round_ties_even() as i64;
    if micro >= 1_000_000 {
        sec += 1;
        micro -= 1_000_000;
    } else if micro < 0 {
        sec -= 1;
        micro += 1_000_000;
    }
    local_naive(sec, micro as u32)
}

/// Linux btime. `fetch`가 시각을 못 주면(미지원 파일시스템, 호출 실패) `None`이다.
pub fn linux_btime_with(path: &Path, is_linux: bool, fetch: &dyn Fn(&Path) -> io::Result<SystemTime>) -> Option<NaiveDateTime> {
    if !is_linux {
        return None;
    }
    let since_epoch = fetch(path).ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(datetime_from_parts(since_epoch.as_secs() as i64, since_epoch.subsec_nanos()))
}

pub fn linux_statx_btime(path: &Path) -> Option<NaiveDateTime> {
    linux_btime_with(path, cfg!(target_os = "linux"), &|p| std::fs::metadata(p)?.created())
}

pub fn stat_created_time_with_source(st: &StatResult) -> (NaiveDateTime, &'static str) {
    if let Some(ns) = st.birthtime_ns {
        return (datetime_from_float(ns as f64 / 1e9), CREATED_TIME_SOURCE_ST_BIRTHTIME);
    }
    if let Some(t) = st.birthtime {
        return (datetime_from_float(t), CREATED_TIME_SOURCE_ST_BIRTHTIME);
    }
    (datetime_from_float(st.ctime), CREATED_TIME_SOURCE_ST_CTIME)
}

pub fn stat_created_time(st: &StatResult) -> NaiveDateTime {
    stat_created_time_with_source(st).0
}

/// Python `datetime.isoformat()`: 마이크로초가 0이면 소수부를 생략한다.
fn iso(dt: NaiveDateTime) -> String {
    let base = dt.format("%Y-%m-%dT%H:%M:%S").to_string();
    match dt.nanosecond() / 1000 {
        0 => base,
        micro => format!("{base}.{micro:06}"),
    }
}

pub fn stat_created_time_iso(st: &StatResult) -> String {
    iso(stat_created_time(st))
}

/// btime 조회 방법을 바꿔 끼울 수 있는 본체. `stat`이 없으면 경로를 stat한다.
pub fn path_created_time_with(path: &Path, stat: Option<&StatResult>, btime: &dyn Fn(&Path) -> Option<NaiveDateTime>) -> io::Result<(NaiveDateTime, &'static str)> {
    if let Some(t) = btime(path) {
        return Ok((t, CREATED_TIME_SOURCE_STATX_BTIME));
    }
    match stat {
        Some(st) => Ok(stat_created_time_with_source(st)),
        None => Ok(stat_created_time_with_source(&StatResult::from_metadata(&std::fs::metadata(path)?))),
    }
}

pub fn path_created_time_with_source(path: &Path, stat: Option<&StatResult>) -> io::Result<(NaiveDateTime, &'static str)> {
    path_created_time_with(path, stat, &linux_statx_btime)
}

pub fn path_created_time(path: &Path, stat: Option<&StatResult>) -> io::Result<NaiveDateTime> {
    Ok(path_created_time_with_source(path, stat)?.0)
}

pub fn path_created_time_iso_with(path: &Path, stat: Option<&StatResult>, btime: &dyn Fn(&Path) -> Option<NaiveDateTime>) -> io::Result<String> {
    Ok(iso(path_created_time_with(path, stat, btime)?.0))
}

pub fn path_created_time_iso(path: &Path, stat: Option<&StatResult>) -> io::Result<String> {
    path_created_time_iso_with(path, stat, &linux_statx_btime)
}
