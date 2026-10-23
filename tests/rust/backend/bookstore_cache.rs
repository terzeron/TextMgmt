//! 원본: tests/test_bookstore_cache.py
//!
//! 서점 응답 캐시 테스트. 같은 책을 두 번 묻지 않게 하는 것이 목적이다.
//! 그래서 확인할 것도 두 가지다. 어떤 키로 같은 책을 알아보는가, 그리고 어떤 응답을 남기면 안 되는가.
//!
//! 가정한 API:
//! - `backend::bookstore_cache::BookstoreResponseCache::new(path: impl AsRef<Path>) -> Self`
//!   - `get(&self, isbn: &str, title: &str, author: &str) -> Option<Value>` (Python 기본값 `""`는 빈 문자열로 넘긴다)
//!   - `put(&mut self, stores: &Value, title: &str, author: &str, isbn: &str)`
//!   - `path(&self) -> &Path`
//! - 경고 로그는 `log` crate 파사드로 남긴다. 테스트는 `common::log_capture::LogCapture`(RAII, 스레드 로컬)로 caplog를 대신한다.
//! - `backend::bookstore::BookstoreSearch` 트레이트: `search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String)`
//!   (`SearchRow`는 `(title, author, category, detail_url, search_url, isbn)` 문자열 6-튜플)
//! - `backend::book_classifier::BookClassifierService`
//!   - `new(library_root: &Path, cache_file: Option<&Path>, classifier: Option<Arc<dyn Classify>>) -> Self`
//!   - `with_delay(self, f64) -> Self`, `with_bookstore_cache_file(self, &Path) -> Self`
//!   - pub 필드 `yes24`, `aladin`, `kyobo: Box<dyn BookstoreSearch>`
//!   - `query_bookstores(&mut self, search_title: &str, raw_author: &str, raw_title: &str, isbn: &str) -> (Value, Value, Value)`

mod common;

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use backend::book_classifier::BookClassifierService;
use backend::bookstore::{BookstoreSearch, SearchRow};
use backend::bookstore_cache::BookstoreResponseCache;
use common::log_capture::LogCapture;
use serde_json::{Value, json};
use tempfile::TempDir;

fn stores() -> Value {
    json!({
        "yes24": {"title": "달빛조각사", "author": "남희성", "cat": "소설 > 판타지", "mapped": "3_판타지", "isbn": "9791162203484"},
        "aladin": {"title": "달빛조각사", "author": "남희성", "cat": "국내도서 > 판타지", "mapped": "3_판타지", "isbn": "9791162203484"},
        "kyobo": {"title": "", "author": "", "cat": "", "mapped": null, "isbn": ""},
    })
}

fn cache(tmp: &TempDir) -> BookstoreResponseCache {
    BookstoreResponseCache::new(tmp.path().join("bookstore_cache.jsonl"))
}

#[test]
fn test_get_returns_nothing_before_anything_is_stored() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(cache(&tmp).get("", "달빛조각사", "남희성"), None);
}

#[test]
fn test_put_then_get_by_title() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&stores(), "달빛조각사", "남희성", "");

    assert_eq!(cache.get("", "달빛조각사", "남희성"), Some(stores()));
}

/// ISBN 은 서점 응답에서 얻는다. 제목 표기가 달라도 같은 책을 찾아야 한다.
#[test]
fn test_put_indexes_by_isbn_taken_from_the_store_answers() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&stores(), "달빛조각사", "남희성", "");

    assert_eq!(cache.get("9791162203484", "", ""), Some(stores()));
    assert_eq!(cache.get("9791162203484", "전혀 다른 제목", ""), Some(stores()));
}

#[test]
fn test_put_infers_author_from_store_answers() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&stores(), "달빛조각사", "", "");

    assert_eq!(cache.get("", "달빛조각사", "남희성"), Some(stores()));
}

#[test]
fn test_authorless_response_is_only_indexed_by_isbn() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    let answers = json!({"yes24": {"cat": "소설", "isbn": "9788900000000"}, "aladin": {}, "kyobo": {}});
    cache.put(&answers, "달빛조각사", "", "");

    assert_eq!(cache.get("9788900000000", "", ""), Some(answers));
    assert_eq!(cache.get("", "달빛조각사", "남희성"), None);
}

/// 제목은 같아도 다른 책일 수 있다. ISBN 을 알면 그쪽이 맞다.
#[test]
fn test_get_prefers_isbn_over_title() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    let other = json!({"yes24": {"cat": "소설 > 무협", "mapped": "3_무협", "isbn": "9788900000000"}, "aladin": {}, "kyobo": {}});
    cache.put(&stores(), "달빛조각사", "남희성", "");
    cache.put(&other, "달빛조각사", "다른저자", "9788900000000");

    assert_eq!(cache.get("9788900000000", "달빛조각사", "다른저자"), Some(other));
}

/// 서점이 다 빈손인 것은 진짜 없어서일 수도, 망이 끊겨서일 수도 있다.
/// 구분할 방법이 없으므로 남기지 않는다. 남기면 일시적인 실패가 영구 오답이 된다.
#[test]
fn test_empty_answers_are_not_cached() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&json!({"yes24": {"cat": "", "mapped": null}, "aladin": {}, "kyobo": {}}), "없는책", "누구", "");

    assert_eq!(cache.get("", "없는책", "누구"), None);
}

/// 제안 생성은 매번 서비스를 새로 만든다. 파일에 남지 않으면 캐시가 아니다.
#[test]
fn test_entries_survive_a_new_instance() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bookstore_cache.jsonl");
    BookstoreResponseCache::new(&path).put(&stores(), "달빛조각사", "남희성", "");

    assert_eq!(BookstoreResponseCache::new(&path).get("", "달빛조각사", "남희성"), Some(stores()));
}

#[test]
fn test_later_entry_wins_for_the_same_key() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bookstore_cache.jsonl");
    let mut cache = BookstoreResponseCache::new(&path);
    cache.put(&stores(), "달빛조각사", "남희성", "");
    let newer = json!({"yes24": {"cat": "소설 > 판타지 > 현대판타지", "mapped": "3_판타지", "isbn": ""}, "aladin": {}, "kyobo": {}});
    cache.put(&newer, "달빛조각사", "남희성", "");

    assert_eq!(BookstoreResponseCache::new(&path).get("", "달빛조각사", "남희성"), Some(newer));
}

/// 줄 단위로 덧붙이므로 쓰다 죽으면 마지막 줄이 깨질 수 있다. 나머지는 살아야 한다.
#[test]
fn test_a_broken_line_does_not_kill_the_whole_cache() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bookstore_cache.jsonl");
    BookstoreResponseCache::new(&path).put(&stores(), "달빛조각사", "남희성", "");
    let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
    f.write_all(b"\n").unwrap();
    writeln!(f, "{}", json!({"keys": ["title:잘못된레코드"], "stores": []})).unwrap();
    f.write_all("{\"keys\": [\"title:잘린책\"], \"stor\n".as_bytes()).unwrap();

    assert_eq!(BookstoreResponseCache::new(&path).get("", "달빛조각사", "남희성"), Some(stores()));
}

#[test]
fn test_missing_file_is_not_an_error() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(BookstoreResponseCache::new(tmp.path().join("없는파일.jsonl")).get("", "아무책", "저자"), None);
}

#[test]
fn test_unreadable_cache_file_is_ignored() {
    let logs = LogCapture::start(log::Level::Debug);
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("cache.jsonl");
    fs::create_dir(&path).unwrap();

    assert_eq!(BookstoreResponseCache::new(&path).get("", "아무책", "저자"), None);
    assert!(logs.text().contains("서점 캐시를 읽지 못했다"), "로그: {}", logs.text());
}

#[test]
fn test_answers_without_any_cache_key_are_not_persisted() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&json!({"yes24": {"cat": "소설"}, "aladin": {}, "kyobo": {}}), "", "", "");

    assert!(!cache.path().exists());
}

#[test]
fn test_cache_write_failure_keeps_in_memory_answer() {
    let logs = LogCapture::start(log::Level::Debug);
    let tmp = TempDir::new().unwrap();
    let parent = tmp.path().join("not-a-directory");
    fs::write(&parent, "file blocks directory creation").unwrap();
    let mut cache = BookstoreResponseCache::new(parent.join("cache.jsonl"));
    cache.put(&stores(), "달빛조각사", "남희성", "");

    assert_eq!(cache.get("", "달빛조각사", "남희성"), Some(stores()));
    assert!(logs.text().contains("서점 캐시에 쓰지 못했다"), "로그: {}", logs.text());
}

#[test]
fn test_title_key_ignores_surrounding_space() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&stores(), "  달빛조각사 ", " 남희성 ", "");

    assert_eq!(cache.get("", "달빛조각사", "남희성"), Some(stores()));
}

#[test]
fn test_title_lookup_requires_matching_author() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&stores(), "달빛조각사", "남희성", "");

    assert_eq!(cache.get("", "달빛조각사", "다른 저자"), None);
}

#[test]
fn test_isbn_lookup_rejects_mismatched_saved_isbn_or_author() {
    let tmp = TempDir::new().unwrap();
    let mut cache = cache(&tmp);
    cache.put(&stores(), "달빛조각사", "남희성", "9791162203484");

    assert_eq!(cache.get("9780000000000", "달빛조각사", "남희성"), None);
    assert_eq!(cache.get("9791162203484", "달빛조각사", "다른 저자"), None);
}

/// 서점 대역. 호출한 서점 이름을 `calls`에 쌓는다.
struct Store {
    name: &'static str,
    calls: Arc<Mutex<Vec<String>>>,
}

impl BookstoreSearch for Store {
    fn search(&self, _isbn: &str, title: &str, _author: &str) -> (Vec<SearchRow>, String, String) {
        self.calls.lock().unwrap().push(self.name.to_owned());
        (vec![("달빛조각사".into(), "남희성".into(), "소설 > 판타지".into(), "http://x".into(), "http://s".into(), "9791162203484".into())], title.to_owned(), "title".into())
    }
}

fn service_with_stores(tmp: &Path, calls: &Arc<Mutex<Vec<String>>>) -> BookClassifierService {
    let mut service = BookClassifierService::new(tmp, Some(&tmp.join("cache.json")), None).with_delay(0.0).with_bookstore_cache_file(&tmp.join("bs.jsonl"));
    service.yes24 = Box::new(Store { name: "y", calls: Arc::clone(calls) });
    service.aladin = Box::new(Store { name: "a", calls: Arc::clone(calls) });
    service.kyobo = Box::new(Store { name: "k", calls: Arc::clone(calls) });
    service
}

/// 같은 제목을 두 번 물으면 두 번째는 서점을 안 부른다. 한 건에 3.6초가 걸린다.
#[test]
fn test_query_bookstores_uses_the_cache_instead_of_calling_stores_again() {
    let tmp = TempDir::new().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut service = service_with_stores(tmp.path(), &calls);

    let first = service.query_bookstores("달빛조각사", "남희성", "달빛조각사", "");
    assert_eq!(calls.lock().unwrap().len(), 3);

    let second = service.query_bookstores("달빛조각사", "남희성", "달빛조각사", "");
    assert_eq!(calls.lock().unwrap().len(), 3);
    assert_eq!(second, first);
}

/// 제목 표기가 달라도 ISBN 이 같으면 같은 책이다.
#[test]
fn test_query_bookstores_finds_the_cache_by_isbn_when_the_title_differs() {
    let tmp = TempDir::new().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut service = service_with_stores(tmp.path(), &calls);

    service.query_bookstores("달빛조각사", "남희성", "달빛조각사", "");
    assert_eq!(calls.lock().unwrap().len(), 3);

    service.query_bookstores("달빛 조각사 1부", "남희성", "달빛 조각사 1부", "9791162203484");
    assert_eq!(calls.lock().unwrap().len(), 3);
}
