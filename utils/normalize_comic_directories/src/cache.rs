//! 서점별 검색 결과 캐시. 질의와 서점마다 한 줄씩 JSONL로 덧붙이고, 빈 결과는 일정 시간 뒤 만료한다.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

pub const STORE_FIELDS: [&str; 5] = ["title", "author", "category", "url", "isbn"];
/// 빈 결과를 캐시에 두는 기본 시간: 7일.
pub const DEFAULT_NEGATIVE_CACHE_SECONDS: u64 = 7 * 24 * 60 * 60;

/// 서점 검색 결과 한 줄.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreRow {
    pub title: String,
    pub author: String,
    pub category: String,
    pub url: String,
    pub isbn: String,
}

impl StoreRow {
    fn to_json(&self) -> Value {
        json!({"title": self.title, "author": self.author, "category": self.category, "url": self.url, "isbn": self.isbn})
    }

    fn from_json(value: &Value) -> Option<StoreRow> {
        let object = value.as_object()?;
        let field = |name: &str| object.get(name).and_then(Value::as_str).map(str::to_owned);
        Some(StoreRow { title: field("title")?, author: field("author")?, category: field("category")?, url: field("url")?, isbn: field("isbn")? })
    }
}

struct Entry {
    results: Vec<StoreRow>,
    cached_at: f64,
}

pub struct BookstoreSearchCache {
    path: PathBuf,
    negative_ttl_seconds: u64,
    entries: HashMap<(String, String), Entry>,
    pub hits: usize,
    pub searches: usize,
    pub search_seconds: f64,
    pub last_request_completed_at: Option<Instant>,
}

fn now_seconds() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// `cached_at`은 숫자(초)이거나 ISO 문자열이다. 읽지 못하면 0이다.
fn parse_cached_at(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(text)) => {
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(text) {
                return t.timestamp() as f64 + f64::from(t.timestamp_subsec_nanos()) / 1e9;
            }
            for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
                if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(text, format) {
                    // 시간대가 없으면 Python `datetime.timestamp()`처럼 지역 시간으로 본다.
                    if let Some(t) = naive.and_local_timezone(chrono::Local).earliest() {
                        return t.timestamp() as f64 + f64::from(t.timestamp_subsec_nanos()) / 1e9;
                    }
                }
            }
            0.0
        }
        _ => 0.0,
    }
}

impl BookstoreSearchCache {
    /// `refresh`면 파일을 비운다. 아니면 있는 파일을 읽는다.
    pub fn new(path: &Path, refresh: bool, negative_ttl_seconds: u64) -> Self {
        match Self::try_new(path, refresh, negative_ttl_seconds) {
            Ok(cache) => cache,
            Err(e) => {
                eprintln!("Cache write failed: {e}");
                Self::empty(path, negative_ttl_seconds)
            }
        }
    }

    /// `refresh` 실패를 호출자에게 돌려준다. CLI는 Python 원본처럼 초기화에 실패하면 중단한다.
    pub fn try_new(path: &Path, refresh: bool, negative_ttl_seconds: u64) -> std::io::Result<Self> {
        let mut cache = Self::empty(path, negative_ttl_seconds);
        if refresh {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, "")?;
        } else if path.exists() {
            cache.load()?;
        }
        Ok(cache)
    }

    fn empty(path: &Path, negative_ttl_seconds: u64) -> Self {
        BookstoreSearchCache { path: path.to_owned(), negative_ttl_seconds, entries: HashMap::new(), hits: 0, searches: 0, search_seconds: 0.0, last_request_completed_at: None }
    }

    fn load(&mut self) -> std::io::Result<()> {
        let text = match fs::read(&self.path) {
            Ok(bytes) => String::from_utf8(bytes).map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
            Err(e) => {
                eprintln!("Cache read failed: {e}");
                return Ok(());
            }
        };
        for line in text.lines() {
            let Ok(item) = serde_json::from_str::<Value>(line) else { continue };
            let text_of = |name: &str| item.get(name).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).unwrap_or_default();
            let (query, store) = (text_of("query"), text_of("store"));
            let Some(rows) = item.get("results").and_then(Value::as_array) else { continue };
            let results: Option<Vec<StoreRow>> = rows.iter().map(StoreRow::from_json).collect();
            let Some(results) = results else { continue };
            if !query.is_empty() && !store.is_empty() {
                self.entries.insert((query, store), Entry { results, cached_at: parse_cached_at(item.get("cached_at")) });
            }
        }
        Ok(())
    }

    /// 항목이 있는지(만료 여부와 무관). 한도 계산에 쓴다.
    pub fn contains(&self, query: &str, store: &str) -> bool {
        self.entries.contains_key(&(query.to_owned(), store.to_owned()))
    }

    /// 캐시된 결과. 만료된 빈 결과는 지우고 `None`을 돌려준다.
    pub fn get(&mut self, query: &str, store: &str) -> Option<Vec<StoreRow>> {
        let key = (query.to_owned(), store.to_owned());
        let entry = self.entries.get(&key)?;
        if entry.results.is_empty() && now_seconds() - entry.cached_at > self.negative_ttl_seconds as f64 {
            self.entries.remove(&key);
            return None;
        }
        self.hits += 1;
        Some(entry.results.clone())
    }

    pub fn put(&mut self, query: &str, store: &str, results: Vec<StoreRow>) {
        let cached_at = now_seconds();
        let item = json!({"query": query, "store": store, "cached_at": cached_at, "results": results.iter().map(StoreRow::to_json).collect::<Vec<_>>()});
        let write = || -> std::io::Result<()> {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
            writeln!(file, "{item}")
        };
        match write() {
            Ok(()) => {
                self.entries.insert((query.to_owned(), store.to_owned()), Entry { results, cached_at });
            }
            Err(e) => eprintln!("Cache write failed: {e}"),
        }
    }

    /// 마지막 요청이 끝난 뒤 `delay`초가 지날 때까지 기다린다.
    pub fn wait_for_request_slot(&self, delay: f64) {
        let Some(last) = self.last_request_completed_at else { return };
        if delay <= 0.0 {
            return;
        }
        let remaining = delay - last.elapsed().as_secs_f64();
        if remaining > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(remaining));
        }
    }
}
