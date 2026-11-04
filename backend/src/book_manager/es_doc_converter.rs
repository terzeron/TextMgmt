//! ES 문서와 `Book`·시각 문자열 사이의 변환. Python `Book.__init__`, `datetime.fromisoformat`에 대응한다.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;

use crate::book_metadata::Book;

/// `datetime.fromisoformat`이 받는 흔한 꼴(날짜만, 'T' 또는 공백 구분, 소수 초).
pub fn parse_iso(text: &str) -> Option<NaiveDateTime> {
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(text, fmt) {
            return Some(parsed);
        }
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

fn str_of(info: &Value, key: &str) -> String {
    info.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn int_of(info: &Value, key: &str) -> i64 {
    info.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// ES 문서에서 `Book`을 만든다. `Book::new`와 달리 이 매니저의 책 루트를 쓰고, 빠진 필드는 기본값으로 채운다
/// (Python은 `KeyError`를 낸다. 문서 하나가 목록 전체를 막지 않게 한다).
pub fn book_from_doc(path_prefix: &Path, book_id: i64, info: &Value, score: f64) -> Book {
    let updated_text = str_of(info, "updated_time");
    let created_text = info.get("created_time").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| updated_text.clone());
    let epoch = NaiveDateTime::default();
    Book {
        book_id,
        category: str_of(info, "category"),
        title: str_of(info, "title"),
        author: str_of(info, "author"),
        file_path: path_prefix.join(str_of(info, "file_path")),
        file_type: str_of(info, "file_type"),
        file_size: int_of(info, "file_size"),
        line_count: int_of(info, "line_count"),
        page_count: int_of(info, "page_count"),
        isbn: str_of(info, "isbn"),
        summary: str_of(info, "summary"),
        created_time: parse_iso(&created_text).unwrap_or(epoch),
        updated_time: parse_iso(&updated_text).unwrap_or(epoch),
        score,
    }
}

/// Python `Book.__init__`가 필수로 읽는 ES 필드를 검증한다.
pub fn try_book_from_doc(path_prefix: &Path, book_id: i64, info: &Value, score: f64) -> Result<Book, String> {
    for key in ["category", "title", "author", "file_path", "file_type", "updated_time"] {
        if info.get(key).and_then(Value::as_str).is_none() {
            return Err(format!("'{key}'"));
        }
    }
    if info.get("file_size").and_then(Value::as_i64).is_none() {
        return Err("'file_size'".to_string());
    }
    Ok(book_from_doc(path_prefix, book_id, info, score))
}

/// 마이크로초 ISO 시각 문자열.
pub fn iso(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}
