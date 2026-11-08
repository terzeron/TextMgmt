//! 분류 제안의 서점 조회: inode 캐시 → ISBN 캐시 → 서점 검색 순서와 캐시 기록 규칙.
//! 서점·캐시·시계는 대역이다. 실제 서점과 DB에 붙지 않는다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use backend::bookstore_result_fetcher::{BLOCK_LIMIT, BookstoreResultFetcher, CacheStore};
use book_manager::bookstore_evidence::{BookstoreLookup, LookupStatus};
use book_manager::bookstore_query_builder::BookQuery;
use bookstore::store_clients::{BookstoreSearch, SearchRow};
use db::DbError;
use db::bookstore_cache_store::CachedLookup;
use sysdeps::Clock;

const ISBN: &str = "9780306406157";
const OTHER_ISBN: &str = "9780131103627";

type Calls = Arc<Mutex<Vec<(String, String, String)>>>;

/// ISBN 검색과 이름 검색에 정해 둔 카테고리를 돌려주는 서점.
struct FakeStore {
    calls: Calls,
    by_isbn: Vec<&'static str>,
    by_title: Vec<&'static str>,
    blocked: bool,
}

impl BookstoreSearch for FakeStore {
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        self.calls.lock().unwrap().push((isbn.to_string(), title.to_string(), author.to_string()));
        if self.blocked {
            return (Vec::new(), String::new(), "unknown".to_string());
        }
        let (cats, method) = if isbn.is_empty() { (&self.by_title, "title_author") } else { (&self.by_isbn, "isbn") };
        (cats.iter().map(|c| SearchRow { category: c.to_string(), ..SearchRow::default() }).collect(), String::new(), method.to_string())
    }

    fn blocked(&self) -> bool {
        self.blocked
    }
}

#[derive(Default)]
struct MemCache {
    inode: Mutex<HashMap<(i64, String), CachedLookup>>,
    isbn: Mutex<HashMap<(String, String), CachedLookup>>,
}

impl CacheStore for MemCache {
    fn get_by_inode(&self, inode: i64, store: &str, _now: i64) -> Result<Option<CachedLookup>, DbError> {
        Ok(self.inode.lock().unwrap().get(&(inode, store.to_string())).cloned())
    }
    fn get_by_isbn(&self, isbn13: &str, store: &str, _now: i64) -> Result<Option<CachedLookup>, DbError> {
        Ok(self.isbn.lock().unwrap().get(&(isbn13.to_string(), store.to_string())).cloned())
    }
    fn put_inode(&self, inode: i64, store: &str, lookup: &CachedLookup) -> Result<(), DbError> {
        self.inode.lock().unwrap().insert((inode, store.to_string()), lookup.clone());
        Ok(())
    }
    fn put_isbn(&self, isbn13: &str, store: &str, lookup: &CachedLookup) -> Result<(), DbError> {
        self.isbn.lock().unwrap().insert((isbn13.to_string(), store.to_string()), lookup.clone());
        Ok(())
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> f64 {
        5_000.0
    }
    fn monotonic(&self) -> f64 {
        0.0
    }
    fn sleep(&self, _seconds: f64) {}
}

fn store(by_isbn: Vec<&'static str>, by_title: Vec<&'static str>, blocked: bool) -> (Box<dyn BookstoreSearch>, Calls) {
    let calls: Calls = Arc::default();
    (Box::new(FakeStore { calls: calls.clone(), by_isbn, by_title, blocked }), calls)
}

fn fetcher(s: Box<dyn BookstoreSearch>, cache: &Arc<MemCache>) -> BookstoreResultFetcher {
    BookstoreResultFetcher::new(vec![("yes24".to_string(), s)], Some(cache.clone() as Arc<dyn CacheStore>), Arc::new(FixedClock))
}

fn query(isbn: Option<&str>, author: &str, title: &str) -> BookQuery {
    BookQuery { isbn13: isbn.map(str::to_string), title: title.to_string(), author: author.to_string() }
}

fn cached(method: &str, query: &str, isbn13: Option<&str>, category: &str) -> CachedLookup {
    CachedLookup {
        status: "found".to_string(),
        rows_json: format!(r#"[{{"title":"","author":"","category":"{category}"}}]"#),
        method: method.to_string(),
        query: query.to_string(),
        isbn13: isbn13.map(str::to_string),
        fetched_at: 1_000,
    }
}

#[test]
fn inode_hit_skips_the_store() {
    let cache = Arc::new(MemCache::default());
    let q = query(None, "홍길동", "검과 마법");
    cache.put_inode(7, "yes24", &cached("title_author", &q.title_key(), None, "SF > 한국SF")).unwrap();
    let (s, calls) = store(vec![], vec!["다른"], false);

    let outcome = fetcher(s, &cache).lookup(7, &q);

    assert!(!outcome.network_used);
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(outcome.results[0].status, LookupStatus::Found);
    assert_eq!(outcome.results[0].rows[0].category, "SF > 한국SF");
}

#[test]
fn renamed_title_searches_again() {
    let cache = Arc::new(MemCache::default());
    cache.put_inode(7, "yes24", &cached("title_author", "홍길동\u{1f}옛 제목", None, "옛 > 분류")).unwrap();
    let (s, calls) = store(vec![], vec!["새 > 분류"], false);
    let q = query(None, "홍길동", "새 제목");

    let outcome = fetcher(s, &cache).lookup(7, &q);

    assert!(outcome.network_used);
    assert_eq!(calls.lock().unwrap().as_slice(), &[(String::new(), "새 제목".to_string(), "홍길동".to_string())]);
    assert_eq!(cache.get_by_inode(7, "yes24", 0).unwrap().unwrap().query, q.title_key());
}

#[test]
fn inode_row_with_a_different_isbn_is_ignored() {
    let cache = Arc::new(MemCache::default());
    cache.put_inode(7, "yes24", &cached("isbn", OTHER_ISBN, Some(OTHER_ISBN), "옛 > 분류")).unwrap();
    let (s, calls) = store(vec!["SF > 한국SF"], vec![], false);

    let outcome = fetcher(s, &cache).lookup(7, &query(Some(ISBN), "", "제목"));

    assert_eq!(calls.lock().unwrap().as_slice(), &[(ISBN.to_string(), String::new(), String::new())]);
    assert_eq!(outcome.results[0].rows[0].category, "SF > 한국SF");
}

#[test]
fn isbn_cache_hit_skips_the_store_and_fills_the_inode_cache() {
    let cache = Arc::new(MemCache::default());
    cache.put_isbn(ISBN, "yes24", &cached("isbn", ISBN, None, "SF > 한국SF")).unwrap();
    let (s, calls) = store(vec![], vec![], false);

    let outcome = fetcher(s, &cache).lookup(7, &query(Some(ISBN), "", "제목"));

    assert!(!outcome.network_used);
    assert!(calls.lock().unwrap().is_empty());
    let inode_row = cache.get_by_inode(7, "yes24", 0).unwrap().unwrap();
    assert_eq!((inode_row.method.as_str(), inode_row.isbn13.as_deref()), ("isbn", Some(ISBN)));
}

#[test]
fn copies_with_the_same_isbn_call_the_store_once() {
    let cache = Arc::new(MemCache::default());
    let (s, calls) = store(vec!["SF > 한국SF"], vec![], false);
    let f = fetcher(s, &cache);

    f.lookup(7, &query(Some(ISBN), "", "제목"));
    f.lookup(8, &query(Some(ISBN), "", "다른 이름"));

    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn isbn_results_go_to_both_caches_and_title_results_only_to_the_inode_cache() {
    let cache = Arc::new(MemCache::default());
    let (s, _) = store(vec!["SF > 한국SF"], vec![], false);
    fetcher(s, &cache).lookup(7, &query(Some(ISBN), "", "제목"));
    assert!(cache.get_by_isbn(ISBN, "yes24", 0).unwrap().is_some());
    assert!(cache.get_by_inode(7, "yes24", 0).unwrap().is_some());

    let cache = Arc::new(MemCache::default());
    let (s, _) = store(vec![], vec!["SF > 한국SF"], false);
    fetcher(s, &cache).lookup(8, &query(None, "홍길동", "제목"));
    assert!(cache.isbn.lock().unwrap().is_empty());
    let row = cache.get_by_inode(8, "yes24", 0).unwrap().unwrap();
    assert_eq!((row.method.as_str(), row.status.as_str()), ("title_author", "found"));
}

#[test]
fn blocked_results_are_not_cached() {
    let cache = Arc::new(MemCache::default());
    let (s, _) = store(vec!["x"], vec!["x"], true);

    let outcome = fetcher(s, &cache).lookup(7, &query(Some(ISBN), "홍길동", "제목"));

    assert_eq!(outcome.results[0].status, LookupStatus::Blocked);
    assert!(cache.inode.lock().unwrap().is_empty());
    assert!(cache.isbn.lock().unwrap().is_empty());
}

#[test]
fn a_store_blocked_too_many_times_in_a_row_is_skipped() {
    assert_eq!(BLOCK_LIMIT, 20);
    let cache = Arc::new(MemCache::default());
    let (s, calls) = store(vec![], vec!["x"], true);
    let f = fetcher(s, &cache);

    for inode in 0..BLOCK_LIMIT as i64 {
        f.lookup(inode, &query(None, "", &format!("제목{inode}")));
    }
    let before = calls.lock().unwrap().len();
    let outcome = f.lookup(999, &query(None, "", "마지막"));

    assert_eq!(calls.lock().unwrap().len(), before);
    assert_eq!(outcome.results[0].status, LookupStatus::Blocked);
    assert!(!outcome.network_used);
    assert_eq!(f.summary().unwrap()["yes24"]["skipped"], 1);
}

#[test]
fn summary_counts_each_outcome_per_store() {
    let cache = Arc::new(MemCache::default());
    let (s, _) = store(vec![], vec!["SF > 한국SF"], false);
    let f = fetcher(s, &cache);

    f.lookup(7, &query(None, "", "제목"));
    f.lookup(7, &query(None, "", "제목"));

    let summary = f.summary().unwrap();
    assert_eq!(summary["yes24"]["found"], 1);
    assert_eq!(summary["yes24"]["cache_hit"], 1);
}
