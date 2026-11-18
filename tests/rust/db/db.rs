//! `db` crate 루트 단위 테스트: `DbValue`의 JSON 변환.

use chrono::NaiveDate;
use db::DbValue;
use serde_json::{Value, json};

/// 날짜는 공백으로 잇는 `YYYY-MM-DD HH:MM:SS`다. 소수 초는 0이 아닐 때만 3·6자리 묶음으로 붙는다.
#[test]
fn datetimes_become_space_separated_strings_in_json() {
    let at = |micros| NaiveDate::from_ymd_opt(2026, 1, 2).unwrap().and_hms_micro_opt(3, 4, 5, micros).unwrap();
    assert_eq!(Value::from(&DbValue::DateTime(at(0))), json!("2026-01-02 03:04:05"));
    assert_eq!(Value::from(&DbValue::DateTime(at(500_000))), json!("2026-01-02 03:04:05.500"));
    assert_eq!(Value::from(&DbValue::DateTime(at(42))), json!("2026-01-02 03:04:05.000042"));
}
