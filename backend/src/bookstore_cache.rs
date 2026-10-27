//! 서점 응답 캐시: 같은 책을 두 번 묻지 않는다. Python `backend/bookstore_cache.py` 대응.
//!
//! 키는 두 가지다. `isbn:<isbn>` 이 가장 확실하고, `title_author:<제목>\0<저자>` 는 ISBN 을 모를 때 쓴다.
//! 파일은 JSONL 이다. 줄 하나를 덧붙이는 것은 건수와 무관하게 일정해서, 한 건마다 전체를 다시 쓰지 않는다.
//!
//! 캐시 파일은 신뢰할 수 없는 입력처럼 읽는다. 읽기 전에 크기를 보고, 줄 길이에 상한을 두며, 심볼릭 링크는 따르지 않는다.
//! 로그에는 제목·저자를 남기지 않는다.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::classifier::pyjson;
use crate::classifier::tokenize::is_py_space;

pub const STORE_NAMES: [&str; 3] = ["yes24", "aladin", "kyobo"];
/// 캐시 파일 최대 크기. 11만 건이 수십 MB 다.
pub const MAX_CACHE_FILE_BYTES: u64 = 2 << 30;
/// 줄 하나(서점 3곳의 응답)의 최대 크기.
pub const MAX_CACHE_LINE_BYTES: u64 = 4 << 20;

/// Python `str.strip()`.
fn strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

fn isbn_key(isbn: &str) -> String {
    format!("isbn:{}", strip(isbn))
}

fn title_author_key(title: &str, author: &str) -> String {
    format!("title_author:{}\0{}", strip(title), strip(author))
}

fn store_text(stores: &Value, name: &str, field: &str) -> String {
    stores.get(name).and_then(|s| s.get(field)).and_then(Value::as_str).map(|s| strip(s).to_string()).unwrap_or_default()
}

/// 서점 응답에서 값을 고른다. 먼저 답한 쪽을 쓴다 - 서점끼리 다를 일이 드물다.
fn first_from_stores(stores: &Value, field: &str) -> String {
    STORE_NAMES.iter().map(|n| store_text(stores, n, field)).find(|v| !v.is_empty()).unwrap_or_default()
}

fn python_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

fn python_repr_string(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') { '"' } else { '\'' };
    let mut out = String::with_capacity(value.len() + 2);
    out.push(quote);
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch == quote => {
                out.push('\\');
                out.push(ch);
            }
            ch if ch.is_control() && (ch as u32) <= 0xff => out.push_str(&format!("\\x{:02x}", ch as u32)),
            ch if ch.is_control() && (ch as u32) <= 0xffff => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch if ch.is_control() => out.push_str(&format!("\\U{:08x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push(quote);
    out
}

fn python_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => format!("[{}]", values.iter().map(python_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(values) => format!("{{{}}}", values.iter().map(|(key, value)| format!("{}: {}", python_repr_string(key), python_repr(value))).collect::<Vec<_>>().join(", ")),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::String(value) => python_repr_string(value),
        other => python_str(other),
    }
}

/// Python `str(value or fallback)` for JSON metadata.
fn metadata_text(record: &Value, key: &str, fallback: String) -> String {
    let Some(value) = record.get(key) else { return fallback };
    if !python_truthy(value) {
        return fallback;
    }
    strip(&python_str(value)).to_owned()
}

/// 서점 중 한 곳이라도 카테고리를 줬는가.
fn has_any_answer(stores: &Value) -> bool {
    STORE_NAMES.iter().any(|n| {
        stores.get(n).and_then(|s| s.get("cat")).is_some_and(|cat| match cat {
            Value::Null => false,
            Value::Bool(value) => *value,
            Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
            Value::String(value) => !value.is_empty(),
            Value::Array(value) => !value.is_empty(),
            Value::Object(value) => !value.is_empty(),
        })
    })
}

#[derive(Debug)]
struct Entry {
    stores: Value,
    isbn: String,
    author: String,
}

/// 서점 3곳의 응답을 ISBN 또는 제목·저자 조합으로 찾아 쓰는 캐시.
#[derive(Debug)]
pub struct BookstoreResponseCache {
    path: PathBuf,
    entries: HashMap<String, Arc<Entry>>,
}

impl BookstoreResponseCache {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let mut cache = Self { path: path.as_ref().to_path_buf(), entries: HashMap::new() };
        cache.load();
        cache
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&mut self) {
        if !self.path.exists() {
            return;
        }
        if let Err(e) = self.read_file() {
            log::warn!("서점 캐시를 읽지 못했다: {e}");
        }
    }

    fn read_file(&mut self) -> io::Result<()> {
        if fs::symlink_metadata(&self.path)?.file_type().is_symlink() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "심볼릭 링크는 따르지 않는다"));
        }
        let file = fs::File::open(&self.path)?;
        let len = file.metadata()?.len();
        if len > MAX_CACHE_FILE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("캐시가 너무 크다 ({len}바이트)")));
        }
        let mut reader = BufReader::new(file);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            let n = (&mut reader).take(MAX_CACHE_LINE_BYTES + 1).read_until(b'\n', &mut buf)?;
            if n == 0 {
                return Ok(());
            }
            if buf.len() as u64 > MAX_CACHE_LINE_BYTES {
                // 이 줄만 버린다. 줄 끝까지 읽어 넘긴다.
                let mut rest = Vec::new();
                reader.read_until(b'\n', &mut rest)?;
                log::debug!("서점 캐시의 너무 긴 줄을 건너뛴다");
                continue;
            }
            if buf.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            // 쓰다 죽으면 마지막 줄이 잘린다. 그 줄만 버리고 나머지는 쓴다.
            let Ok(record) = serde_json::from_slice::<Value>(&buf) else {
                log::debug!("서점 캐시의 깨진 줄을 건너뛴다");
                continue;
            };
            let Some(stores) = record.get("stores").filter(|s| s.is_object()).cloned() else { continue };
            let entry =
                Arc::new(Entry { isbn: metadata_text(&record, "isbn", first_from_stores(&stores, "isbn")), author: metadata_text(&record, "author", first_from_stores(&stores, "author")), stores });
            for key in record.get("keys").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                // 나중 줄이 이긴다. 같은 책을 다시 물었으면 새 답이 맞다.
                self.entries.insert(key.to_string(), Arc::clone(&entry));
            }
        }
    }

    /// ISBN 과 제목·저자 키를 확인하고 저장된 식별 정보까지 검증한다.
    pub fn get(&self, isbn: &str, title: &str, author: &str) -> Option<Value> {
        let (requested_isbn, requested_author) = (strip(isbn), strip(author));
        let mut candidates: Vec<&Arc<Entry>> = Vec::new();
        if !requested_isbn.is_empty()
            && let Some(hit) = self.entries.get(&isbn_key(requested_isbn))
        {
            candidates.push(hit);
        }
        if !strip(title).is_empty()
            && !requested_author.is_empty()
            && let Some(hit) = self.entries.get(&title_author_key(title, requested_author))
            && !candidates.iter().any(|c| c.stores == hit.stores && c.isbn == hit.isbn && c.author == hit.author)
        {
            candidates.push(hit);
        }
        candidates.into_iter().find(|e| (requested_isbn.is_empty() || e.isbn == requested_isbn) && (requested_author.is_empty() || e.author == requested_author)).map(|e| e.stores.clone())
    }

    /// 서점 응답을 남긴다. 전부 빈손이면 남기지 않는다.
    ///
    /// 다 비었을 때 그것이 '정말 없는 책' 인지 '지금 망이 끊긴 것' 인지 구분할 방법이 없다.
    /// 남기면 일시적인 실패가 영구 오답이 된다.
    pub fn put(&mut self, stores: &Value, title: &str, author: &str, isbn: &str) {
        if !has_any_answer(stores) {
            return;
        }
        let effective_isbn = Some(first_from_stores(stores, "isbn")).filter(|v| !v.is_empty()).unwrap_or_else(|| strip(isbn).to_string());
        let effective_author = if strip(author).is_empty() { first_from_stores(stores, "author") } else { strip(author).to_string() };
        let mut keys = Vec::new();
        if !effective_isbn.is_empty() {
            keys.push(isbn_key(&effective_isbn));
        }
        if !strip(title).is_empty() && !effective_author.is_empty() {
            keys.push(title_author_key(title, &effective_author));
        }
        if keys.is_empty() {
            return;
        }
        let entry = Arc::new(Entry { stores: stores.clone(), isbn: effective_isbn.clone(), author: effective_author.clone() });
        for key in &keys {
            self.entries.insert(key.clone(), Arc::clone(&entry));
        }
        let cached_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false);
        self.append(&json!({"keys": keys, "isbn": effective_isbn, "author": effective_author, "stores": stores, "cached_at": cached_at}));
    }

    fn append(&self, record: &Value) {
        let write = || -> io::Result<()> {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }
            if fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "심볼릭 링크는 따르지 않는다"));
            }
            let mut line = pyjson::dumps(record, false);
            line.push('\n');
            // 줄 하나를 한 번의 쓰기로 덧붙인다(O_APPEND). 다른 프로세스의 줄과 섞이지 않는다.
            OpenOptions::new().create(true).append(true).open(&self.path)?.write_all(line.as_bytes())
        };
        // 캐시를 못 써도 판정은 계속돼야 한다. 다음 번에 서점을 다시 부를 뿐이다.
        if let Err(e) = write() {
            log::warn!("서점 캐시에 쓰지 못했다: {e}");
        }
    }
}
