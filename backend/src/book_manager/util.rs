//! 하위 모듈이 함께 쓰는 작은 도구. Python `posixpath`, `urllib.parse.unquote`, `datetime.fromisoformat`에 대응한다.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;

use crate::book::Book;

/// `posixpath.normpath`: `//`, `.`, `..`를 접는다. 빈 문자열은 `"."`이다.
pub fn posix_normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".to_string(),
        (false, false) => joined,
    }
}

/// `posixpath.join(base, rel)`: `rel`이 절대 경로면 `rel`만 쓴다.
pub fn posix_join(base: &str, rel: &str) -> String {
    if rel.starts_with('/') || base.is_empty() {
        rel.to_string()
    } else if base.ends_with('/') {
        format!("{base}{rel}")
    } else {
        format!("{base}/{rel}")
    }
}

/// `posixpath.dirname`.
pub fn posix_dirname(path: &str) -> String {
    match path.rfind('/') {
        None => String::new(),
        Some(i) => {
            let head = &path[..=i];
            let trimmed = head.trim_end_matches('/');
            if trimmed.is_empty() { head.to_string() } else { trimmed.to_string() }
        }
    }
}

/// `urllib.parse.unquote`: `%XX`를 풀고 UTF-8이 아닌 바이트는 대체 문자로 바꾼다.
pub fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// `unquote(href).split("#")[0].split("?")[0]`.
pub fn clean_href(href: &str) -> String {
    let decoded = unquote(href);
    let no_fragment = decoded.split('#').next().unwrap_or("");
    no_fragment.split('?').next().unwrap_or("").to_string()
}

/// ZIP 안 경로: `normpath(join(dir, href))`, `dir`이 비면 `normpath(href)`.
pub fn zip_path(dir: &str, href: &str) -> String {
    if dir.is_empty() { posix_normpath(href) } else { posix_normpath(&posix_join(dir, href)) }
}

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

/// `OSError.strerror`에 가까운 문구: `"... (os error N)"` 꼬리를 뗀다.
pub fn strerror(e: &io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

/// 상대 경로 문자열. 접두사를 못 떼면 `None`.
pub fn relative_to(path: &Path, prefix: &Path) -> Option<String> {
    path.strip_prefix(prefix).ok().map(|p| p.to_string_lossy().into_owned())
}

/// 마이크로초 ISO 시각 문자열.
pub fn iso(t: NaiveDateTime) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
}

/// 이 프로세스 안에서 겹치지 않는 임시 경로 이름 조각.
pub fn unique_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// 지울 때 알아서 정리되는 임시 디렉터리. `tempfile`은 개발 의존이라 직접 만든다.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> io::Result<TempDir> {
        let path = std::env::temp_dir().join(format!("{prefix}{}", unique_token()));
        std::fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
