//! ES 문서와 `Book`·시각 문자열 사이의 변환.

use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime};
use common::json_field_reader::str_field;
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

fn int_of(info: &Value, key: &str) -> i64 {
    info.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// ES 문서에서 `Book`을 만든다. `Book::new`와 달리 이 매니저의 책 루트를 쓰고, 빠진 필드는 기본값으로 채운다
/// (문서 하나가 목록 전체를 막지 않게 한다).
pub fn book_from_doc(path_prefix: &Path, book_id: i64, info: &Value, score: f64) -> Book {
    let updated_text = str_field(info, "updated_time").to_string();
    let created_text = info.get("created_time").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| updated_text.clone());
    let epoch = NaiveDateTime::default();
    Book {
        book_id,
        category: str_field(info, "category").to_string(),
        title: str_field(info, "title").to_string(),
        author: str_field(info, "author").to_string(),
        file_path: path_prefix.join(str_field(info, "file_path")),
        file_type: str_field(info, "file_type").to_string(),
        file_size: int_of(info, "file_size"),
        line_count: int_of(info, "line_count"),
        page_count: int_of(info, "page_count"),
        isbn: str_field(info, "isbn").to_string(),
        summary: str_field(info, "summary").to_string(),
        created_time: parse_iso(&created_text).unwrap_or(epoch),
        updated_time: parse_iso(&updated_text).unwrap_or(epoch),
        score,
    }
}

/// `Book`을 만드는 데 필요한 ES 필드를 검증한다.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_iso_accepts_iso_8601_variants() {
        for text in ["2024-01-02T03:04:05.123456", "2024-01-02 03:04:05", "2024-01-02T03:04", "2024-01-02 03:04", "2024-01-02"] {
            assert!(parse_iso(text).is_some(), "{text}");
        }
        assert_eq!(crate::book_metadata::iso_micros(&parse_iso("2024-01-02").unwrap()), "2024-01-02T00:00:00.000000");
        assert_eq!(parse_iso("02/01/2024"), None);
    }

    #[test]
    fn book_from_doc_fills_missing_fields_and_falls_back_to_updated_time() {
        let info = json!({"category": "A", "title": "t", "file_path": "A/t.epub", "file_size": 3, "updated_time": "2024-01-02T03:04:05"});
        let book = book_from_doc(Path::new("/lib"), 7, &info, 1.5);
        assert_eq!((book.book_id, book.category.as_str(), book.author.as_str(), book.file_size, book.score), (7, "A", "", 3, 1.5));
        assert_eq!(book.file_path, Path::new("/lib/A/t.epub"));
        assert_eq!(book.created_time, book.updated_time, "created_time이 없으면 updated_time을 쓴다");
        let broken = book_from_doc(Path::new("/lib"), 1, &json!({"updated_time": "nope"}), 0.0);
        assert_eq!(broken.updated_time, NaiveDateTime::default());
    }

    #[test]
    fn try_book_from_doc_names_the_first_missing_required_field() {
        let full = json!({"category": "A", "title": "t", "author": "a", "file_path": "A/t.epub", "file_type": "epub", "updated_time": "2024-01-02", "file_size": 1});
        assert!(try_book_from_doc(Path::new("/lib"), 1, &full, 0.0).is_ok());
        let mut no_title = full.clone();
        no_title.as_object_mut().unwrap().remove("title");
        assert_eq!(try_book_from_doc(Path::new("/lib"), 1, &no_title, 0.0).unwrap_err(), "'title'");
        let mut bad_size = full;
        bad_size["file_size"] = json!("big");
        assert_eq!(try_book_from_doc(Path::new("/lib"), 1, &bad_size, 0.0).unwrap_err(), "'file_size'");
    }
}
