//! 책 한 권의 메타데이터.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `TM_BOOK_DIR`에서 책 루트를 읽는다. 없으면 프로세스를 끝내야 하는 설정 오류를 돌려준다.
pub fn path_prefix_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    match get("TM_BOOK_DIR") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Err("The environment variable TM_BOOK_DIR is not set.".to_string()),
    }
}

/// 쪽 단위 도서 목록. `next_cursor`는 커서 방식 목록에서만 채운다.
#[derive(Debug, Clone, Default)]
pub struct BookPage {
    pub books: Vec<Book>,
    pub total: i64,
    pub next_cursor: Option<String>,
}

/// `get_book` 실패. 없음과 조회 실패를 나눈다. viewer 접근 검사는 없음을 404, 조회 실패를 503으로 응답한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookLookupError {
    NotFound(i64),
    /// ES 조회나 문서 변환 실패. 메시지는 API 응답의 `error`로 나간다.
    Failed(String),
}

impl fmt::Display for BookLookupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(book_id) => write!(f, "No book found by '{book_id}'"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BookLookupError {}

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

/// 응답에 싣는 책 한 권. 경로는 책 루트 기준 상대 경로, 시각은 마이크로초까지 적은 ISO 문자열이다.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookView {
    pub book_id: i64,
    pub category: String,
    pub title: String,
    pub author: String,
    pub file_path: String,
    pub file_type: String,
    pub file_size: i64,
    pub line_count: i64,
    pub page_count: i64,
    pub isbn: String,
    /// 만화 색인에는 생성 시각이 없다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_time: Option<String>,
    pub updated_time: String,
    pub score: f64,
}

/// `2020-01-02T03:04:05.123456` 모양.
pub fn iso_micros(time: &NaiveDateTime) -> String {
    time.format("%Y-%m-%dT%H:%M:%S.%6f").to_string()
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
    // 소수 초 구분자로 쉼표도 허용한다.
    let normalized;
    let text = if text.contains(',') {
        normalized = text.replacen(',', ".", 1);
        normalized.as_str()
    } else {
        text
    };
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return parsed.naive_local();
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%d %H:%M:%S%.f%z", "%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%d %H:%M:%S%.f%:z"] {
        if let Ok(parsed) = DateTime::<FixedOffset>::parse_from_str(text, fmt) {
            return parsed.naive_local();
        }
    }
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
    /// 책 루트. `TM_BOOK_DIR`을 처음 쓸 때 한 번 읽는다. 없으면 종료 오류로 본다.
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

    /// 응답에 싣는 모양. 경로는 `TM_BOOK_DIR` 기준이다.
    pub fn view(&self) -> BookView {
        self.view_with_prefix(&Self::path_prefix())
    }

    /// `view`와 같으나 manager가 정한 책 루트를 기준으로 경로를 상대화한다.
    pub(crate) fn view_with_prefix(&self, prefix: &Path) -> BookView {
        let relative = self.file_path.strip_prefix(prefix).expect("file_path is not under TM_BOOK_DIR");
        BookView {
            book_id: self.book_id,
            category: self.category.clone(),
            title: self.title.clone(),
            author: self.author.clone(),
            file_path: relative.to_string_lossy().into_owned(),
            file_type: self.file_type.clone(),
            file_size: self.file_size,
            line_count: self.line_count,
            page_count: self.page_count,
            isbn: self.isbn.clone(),
            created_time: Some(iso_micros(&self.created_time)),
            updated_time: iso_micros(&self.updated_time),
            score: self.score,
        }
    }
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
            self.updated_time
        )
    }
}
