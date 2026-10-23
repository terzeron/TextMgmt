//! 파일 생성 시각 테스트 (tests/test_file_time.py 이식).

use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime};
use file_time::*;

fn local(sec: i64, micro: u32) -> NaiveDateTime {
    DateTime::from_timestamp(sec, micro * 1000).unwrap().with_timezone(&Local).naive_local()
}

fn at(secs: u64, nanos: u32) -> SystemTime {
    UNIX_EPOCH + Duration::new(secs, nanos)
}

fn ctime_only(secs: f64) -> StatResult {
    StatResult { birthtime_ns: None, birthtime: None, ctime: secs }
}

fn naive(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, s).unwrap()
}

fn local_ts(dt: NaiveDateTime) -> f64 {
    dt.and_local_timezone(Local).unwrap().timestamp() as f64
}

#[test]
fn statx_btime_is_used_when_the_source_answers() {
    let got = linux_btime_with(Path::new("book.txt"), true, &|_| Ok(at(1_700_000_000, 123_456_000)));
    assert_eq!(got, Some(local(1_700_000_000, 123_456)));
}

#[test]
fn statx_btime_is_none_when_unsupported_failing_or_not_linux() {
    let unsupported = |_: &Path| -> io::Result<SystemTime> { Err(io::Error::from(io::ErrorKind::Unsupported)) };
    assert_eq!(linux_btime_with(Path::new("b"), true, &unsupported), None);
    assert_eq!(linux_btime_with(Path::new("b"), false, &|_| Ok(at(1, 0))), None);
}

#[test]
fn microseconds_round_half_even_and_carry_into_the_next_second() {
    assert_eq!(datetime_from_parts(1_700_000_000, 123_456_499), local(1_700_000_000, 123_456));
    assert_eq!(datetime_from_parts(1_700_000_000, 123_456_500), local(1_700_000_000, 123_456));
    assert_eq!(datetime_from_parts(1_700_000_000, 123_457_500), local(1_700_000_000, 123_458));
    assert_eq!(datetime_from_parts(1_700_000_000, 999_999_600), local(1_700_000_001, 0));
}

#[test]
fn path_created_time_prefers_btime_then_falls_back_to_ctime() {
    let expected = naive(2024, 1, 2, 3, 4, 5);
    let stat = ctime_only(local_ts(naive(2026, 1, 1, 0, 0, 0)));
    let (t, src) = path_created_time_with(Path::new("b"), Some(&stat), &|_| Some(expected)).unwrap();
    assert_eq!((t, src), (expected, CREATED_TIME_SOURCE_STATX_BTIME));

    let stat = ctime_only(local_ts(naive(2025, 5, 6, 7, 8, 9)));
    let (t, src) = path_created_time_with(Path::new("b"), Some(&stat), &|_| None).unwrap();
    assert_eq!((t, src), (naive(2025, 5, 6, 7, 8, 9), CREATED_TIME_SOURCE_ST_CTIME));
}

#[test]
fn stat_created_time_keeps_platform_birthtime_priority() {
    let all = StatResult { birthtime_ns: Some(1_600_000_000_500_000_000), birthtime: Some(1_700_000_000.0), ctime: 1_800_000_000.0 };
    assert_eq!(stat_created_time_with_source(&all), (local(1_600_000_000, 500_000), CREATED_TIME_SOURCE_ST_BIRTHTIME));
    assert_eq!(stat_created_time(&all), local(1_600_000_000, 500_000));

    let float_only = StatResult { birthtime_ns: None, birthtime: Some(1_700_000_000.0), ctime: 1_800_000_000.0 };
    assert_eq!(stat_created_time_with_source(&float_only), (local(1_700_000_000, 0), CREATED_TIME_SOURCE_ST_BIRTHTIME));

    let none = ctime_only(1_800_000_000.25);
    assert_eq!(stat_created_time_with_source(&none), (local(1_800_000_000, 250_000), CREATED_TIME_SOURCE_ST_CTIME));
}

#[test]
fn iso_helpers_drop_zero_microseconds_like_python() {
    let stat = ctime_only(local_ts(naive(2025, 1, 2, 3, 4, 5)));
    assert_eq!(stat_created_time_iso(&stat), "2025-01-02T03:04:05");
    assert_eq!(path_created_time_iso_with(Path::new("b"), Some(&stat), &|_| None).unwrap(), "2025-01-02T03:04:05");
    let with_micro = ctime_only(local_ts(naive(2025, 1, 2, 3, 4, 5)) + 0.5);
    assert_eq!(stat_created_time_iso(&with_micro), "2025-01-02T03:04:05.500000");
}

#[test]
fn real_files_report_a_creation_time_close_to_now() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.txt");
    std::fs::write(&p, "x").unwrap();
    let (t, source) = path_created_time_with_source(&p, None).unwrap();
    assert!([CREATED_TIME_SOURCE_STATX_BTIME, CREATED_TIME_SOURCE_ST_BIRTHTIME, CREATED_TIME_SOURCE_ST_CTIME].contains(&source));
    let drift = (Local::now().naive_local() - t).num_seconds().abs();
    assert!(drift < 60, "{t} {source}");
    assert_eq!(path_created_time(&p, None).unwrap(), t);
    assert!(path_created_time_iso(&p, None).unwrap().starts_with(&t.format("%Y-%m-%dT%H:%M:%S").to_string()));
}

#[test]
fn missing_paths_are_errors_when_no_stat_is_given() {
    assert!(path_created_time_with_source(Path::new("/nonexistent/x"), None).is_err());
}

#[test]
fn stat_result_is_built_from_real_metadata() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.txt");
    std::fs::write(&p, "x").unwrap();
    let st = StatResult::from_metadata(&std::fs::metadata(&p).unwrap());
    assert!(st.ctime > 1_000_000_000.0);
}

#[test]
fn float_timestamps_carry_and_borrow_across_the_second_boundary() {
    // 마이크로초가 올림으로 100만에 닿으면 다음 초로 넘어간다.
    assert_eq!(datetime_from_float(1_700_000_000.999_999_8), local(1_700_000_001, 0));
    // 1970년 이전은 음수 마이크로초를 앞 초에서 빌린다.
    assert_eq!(datetime_from_float(-0.5), local(-1, 500_000));
}
