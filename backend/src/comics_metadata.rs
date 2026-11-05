//! 만화 한 권의 메타데이터.

use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;

use chrono::NaiveDateTime;
use serde_json::Value;

use crate::book_metadata::{BookView, iso_micros};

/// `TM_COMICS_DIR`에서 만화 루트를 읽는다. 없으면 프로세스를 끝내야 하는 설정 오류를 돌려준다.
pub fn path_prefix_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    match get("TM_COMICS_DIR") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Err("The environment variable TM_COMICS_DIR is not set.".to_string()),
    }
}

const TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.f";

#[derive(Debug, Clone)]
pub struct Comics {
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

impl Comics {
    /// 만화 루트. `TM_COMICS_DIR`을 처음 쓸 때 한 번 읽는다. 없으면 종료 오류로 본다.
    pub fn path_prefix() -> PathBuf {
        static PREFIX: OnceLock<PathBuf> = OnceLock::new();
        PREFIX.get_or_init(|| path_prefix_from_env(&|name| std::env::var(name).ok()).unwrap_or_else(|msg| panic!("{msg}"))).clone()
    }

    pub fn new(book_id: i64, info: &Value, score: f64) -> Comics {
        let updated_text = required_str(info, "updated_time");
        let updated_time = NaiveDateTime::parse_from_str(&updated_text, TIME_FORMAT).unwrap_or_else(|e| panic!("updated_time 형식 오류 {updated_text}: {e}"));
        Comics {
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
            updated_time,
            score,
        }
    }

    /// 응답에 싣는 모양. 경로는 `TM_COMICS_DIR` 기준이고, 생성 시각은 없다.
    pub fn view(&self) -> BookView {
        let prefix = Self::path_prefix();
        let relative = self.file_path.strip_prefix(&prefix).expect("file_path is not under TM_COMICS_DIR");
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
            created_time: None,
            updated_time: iso_micros(&self.updated_time),
            score: self.score,
        }
    }
}

impl fmt::Display for Comics {
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
