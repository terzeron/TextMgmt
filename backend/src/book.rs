//! 책 한 권의 메타데이터. Python `backend/book.py`에 대응한다.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::{NaiveDate, NaiveDateTime, Timelike};
use serde::Serialize;
use serde_json::ser::{Formatter, Serializer};
use serde_json::{Value, json};

/// `TM_BOOK_DIR`에서 책 루트를 읽는다. 없으면 Python의 import 시점 `sys.exit(-1)`에 해당하는 오류를 돌려준다.
pub fn path_prefix_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    match get("TM_BOOK_DIR") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Err("The environment variable TM_BOOK_DIR is not set.".to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct Book {
    pub book_id: i64,
    pub category: String,
    pub title: String,
    pub author: String,
    pub file_path: PathBuf,
    pub file_type: String,
    pub file_size: i64,
    pub line_count: i64,
    pub page_count: i64,
    pub isbn: String,
    pub summary: String,
    pub created_time: NaiveDateTime,
    pub updated_time: NaiveDateTime,
    pub score: f64,
}

fn required_str(info: &Value, key: &str) -> String {
    info.get(key).and_then(Value::as_str).unwrap_or_else(|| panic!("info에 문자열 {key}가 없다")).to_string()
}

fn optional_str(info: &Value, key: &str) -> String {
    info.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn optional_int(info: &Value, key: &str) -> i64 {
    info.get(key).and_then(Value::as_i64).unwrap_or(0)
}

/// `datetime.fromisoformat`이 받는 흔한 꼴(날짜만, 'T' 또는 공백 구분, 소수 초)을 읽는다.
fn parse_iso(text: &str) -> NaiveDateTime {
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(text, fmt) {
            return parsed;
        }
    }
    match NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        Ok(date) => date.and_hms_opt(0, 0, 0).expect("자정"),
        Err(_) => panic!("ISO 형식이 아닌 시각: {text}"),
    }
}

impl Book {
    /// 책 루트. `TM_BOOK_DIR`을 처음 쓸 때 한 번 읽는다. 없으면 Python처럼 종료 오류로 본다.
    pub fn path_prefix() -> PathBuf {
        static PREFIX: OnceLock<PathBuf> = OnceLock::new();
        PREFIX.get_or_init(|| path_prefix_from_env(&|name| std::env::var(name).ok()).unwrap_or_else(|msg| panic!("{msg}"))).clone()
    }

    pub fn new(book_id: i64, info: &Value, score: f64) -> Book {
        let updated_text = required_str(info, "updated_time");
        let created_text = info.get("created_time").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&updated_text).to_string();
        Book {
            book_id,
            category: required_str(info, "category"),
            title: required_str(info, "title"),
            author: required_str(info, "author"),
            file_path: Self::path_prefix().join(required_str(info, "file_path")),
            file_type: required_str(info, "file_type"),
            file_size: info.get("file_size").and_then(Value::as_i64).expect("info에 정수 file_size가 없다"),
            line_count: optional_int(info, "line_count"),
            page_count: optional_int(info, "page_count"),
            isbn: optional_str(info, "isbn"),
            summary: optional_str(info, "summary"),
            created_time: parse_iso(&created_text),
            updated_time: parse_iso(&updated_text),
            score,
        }
    }

    pub fn dict(&self) -> Value {
        let prefix = Self::path_prefix();
        let relative = self.file_path.strip_prefix(&prefix).unwrap_or(&self.file_path);
        json!({
            "book_id": self.book_id,
            "category": self.category,
            "title": self.title,
            "author": self.author,
            "file_path": relative.to_string_lossy(),
            "file_type": self.file_type,
            "file_size": self.file_size,
            "line_count": self.line_count,
            "page_count": self.page_count,
            "isbn": self.isbn,
            "created_time": self.created_time.format("%Y-%m-%dT%H:%M:%S.%6f").to_string(),
            "updated_time": self.updated_time.format("%Y-%m-%dT%H:%M:%S.%6f").to_string(),
            "score": self.score,
        })
    }

    /// Python `json.dumps` 형식: 구분자 ", "와 ": ", 비ASCII는 `\uXXXX`로 이스케이프한다.
    pub fn json(&self) -> String {
        let mut out = Vec::new();
        let mut ser = Serializer::with_formatter(&mut out, PythonFormatter);
        self.dict().serialize(&mut ser).expect("메모리에 쓰기");
        String::from_utf8(out).expect("ASCII")
    }
}

/// Python `str(datetime)`: 마이크로초가 있을 때만 소수부를 붙인다.
fn python_datetime_str(t: &NaiveDateTime) -> String {
    let micros = t.nanosecond() / 1000;
    if micros == 0 { t.format("%Y-%m-%d %H:%M:%S").to_string() } else { format!("{}.{micros:06}", t.format("%Y-%m-%d %H:%M:%S")) }
}

impl fmt::Display for Book {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{category: {}, title: {}, author: {}, file_path: {}, file_type: {}, file_size: {}, updated_time: {}}}",
            self.category,
            self.title,
            self.author,
            self.file_path.display(),
            self.file_type,
            self.file_size,
            python_datetime_str(&self.updated_time)
        )
    }
}

struct PythonFormatter;

impl Formatter for PythonFormatter {
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { writer.write_all(b", ") }
    }

    fn begin_object_key<W: ?Sized + io::Write>(&mut self, writer: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { writer.write_all(b", ") }
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }

    fn write_string_fragment<W: ?Sized + io::Write>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()> {
        let mut units = [0u16; 2];
        for ch in fragment.chars() {
            if ch.is_ascii() {
                writer.write_all(&[ch as u8])?;
            } else {
                for unit in ch.encode_utf16(&mut units) {
                    write!(writer, "\\u{unit:04x}")?;
                }
            }
        }
        Ok(())
    }
}
