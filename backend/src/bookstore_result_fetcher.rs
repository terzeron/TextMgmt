//! 분류 제안의 서점 조회. 캐시를 먼저 보고, 없을 때만 서점을 부른다.
//!
//! 순서는 inode 캐시 → ISBN 캐시 → 서점 검색(ISBN → 저자명+작품명)이다. inode가 ISBN보다 우선이다.
//! - 저자명+작품명으로 얻은 inode 행은 그때 쓴 검색어가 지금 검색어와 같을 때만 쓴다(이름을 바로잡은 경우 다시 검색).
//! - inode 행에 남은 ISBN과 지금 ISBN이 둘 다 있는데 다르면 그 행을 쓰지 않는다(inode 재사용 방어).
//! - ISBN 검색 결과는 두 캐시에, 이름 검색 결과는 inode 캐시에만 적는다. 차단은 적지 않는다.
//! - 한 서점이 연속 `BLOCK_LIMIT`번 차단되면 이 조회 객체가 사는 동안(제안 하나) 그 서점을 더 부르지 않는다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use book_manager::bookstore_evidence::{BookstoreLookup, LookupOutcome, LookupStatus, StoreResult, StoreRow};
use book_manager::bookstore_query_builder::BookQuery;
use bookstore::store_clients::BookstoreSearch;
use db::DbError;
use db::bookstore_cache_store::{BookstoreCacheStore, CachedLookup};
use serde::Serialize;
use serde_json::Value;
use sysdeps::Clock;

/// 연속 차단 상한.
pub const BLOCK_LIMIT: usize = 20;
/// 서점 한 곳에서 쓰는 결과 행 수. 서점 검색 API 응답과 같다.
const MAX_ROWS: usize = 4;

/// 서점 결과 캐시. 운영은 `BookstoreCacheStore`이고 테스트는 대역이다.
pub trait CacheStore: Send + Sync {
    fn get_by_inode(&self, inode: i64, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError>;
    fn get_by_isbn(&self, isbn13: &str, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError>;
    fn put_inode(&self, inode: i64, store: &str, lookup: &CachedLookup) -> Result<(), DbError>;
    fn put_isbn(&self, isbn13: &str, store: &str, lookup: &CachedLookup) -> Result<(), DbError>;
}

impl CacheStore for BookstoreCacheStore {
    fn get_by_inode(&self, inode: i64, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError> {
        BookstoreCacheStore::get_by_inode(self, inode, store, now)
    }
    fn get_by_isbn(&self, isbn13: &str, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError> {
        BookstoreCacheStore::get_by_isbn(self, isbn13, store, now)
    }
    fn put_inode(&self, inode: i64, store: &str, lookup: &CachedLookup) -> Result<(), DbError> {
        BookstoreCacheStore::put_inode(self, inode, store, lookup)
    }
    fn put_isbn(&self, isbn13: &str, store: &str, lookup: &CachedLookup) -> Result<(), DbError> {
        BookstoreCacheStore::put_isbn(self, isbn13, store, lookup)
    }
}

/// 서점 한 곳의 집계.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StoreCounts {
    pub found: usize,
    pub not_found: usize,
    pub blocked: usize,
    pub error: usize,
    pub cache_hit: usize,
    pub skipped: usize,
}

struct StoreSlot {
    name: String,
    search: Box<dyn BookstoreSearch>,
    consecutive_blocks: AtomicUsize,
    counts: Mutex<StoreCounts>,
}

impl StoreSlot {
    fn count(&self, f: impl FnOnce(&mut StoreCounts)) {
        f(&mut self.counts.lock().unwrap_or_else(|e| e.into_inner()));
    }
}

pub struct BookstoreResultFetcher {
    stores: Vec<StoreSlot>,
    cache: Option<Arc<dyn CacheStore>>,
    clock: Arc<dyn Clock>,
}

fn result(store: &str, status: LookupStatus, rows: Vec<StoreRow>) -> StoreResult {
    StoreResult { store: store.to_string(), status, rows }
}

fn rows_of(lookup: &CachedLookup) -> Vec<StoreRow> {
    serde_json::from_str(&lookup.rows_json).unwrap_or_default()
}

fn from_cached(store: &str, lookup: &CachedLookup) -> StoreResult {
    let status = if lookup.status == "found" { LookupStatus::Found } else { LookupStatus::NotFound };
    result(store, status, rows_of(lookup))
}

/// 캐시 오류는 조회를 막지 않는다. 경고만 남기고 캐시 없이 계속한다.
fn ok_or_warn<T: Default>(outcome: Result<T, DbError>, what: &str) -> T {
    outcome.unwrap_or_else(|error| {
        log::warn!("서점 캐시 {what} 실패: {error}");
        T::default()
    })
}

impl BookstoreResultFetcher {
    pub fn new(stores: Vec<(String, Box<dyn BookstoreSearch>)>, cache: Option<Arc<dyn CacheStore>>, clock: Arc<dyn Clock>) -> Self {
        let stores = stores.into_iter().map(|(name, search)| StoreSlot { name, search, consecutive_blocks: AtomicUsize::new(0), counts: Mutex::default() }).collect();
        Self { stores, cache, clock }
    }

    pub fn counts(&self) -> BTreeMap<String, StoreCounts> {
        self.stores.iter().map(|s| (s.name.clone(), s.counts.lock().unwrap_or_else(|e| e.into_inner()).clone())).collect()
    }

    fn put_inode(&self, inode: i64, store: &str, lookup: &CachedLookup) {
        if let Some(cache) = &self.cache {
            ok_or_warn(cache.put_inode(inode, store, lookup), "기록");
        }
    }

    fn put_isbn(&self, isbn13: &str, store: &str, lookup: &CachedLookup) {
        if let Some(cache) = &self.cache {
            ok_or_warn(cache.put_isbn(isbn13, store, lookup), "기록");
        }
    }

    /// 서점을 불러 결과 행을 얻는다. 차단이면 `None`이다.
    fn call(&self, slot: &StoreSlot, isbn: &str, title: &str, author: &str) -> Option<(Vec<StoreRow>, String)> {
        let (rows, _, method) = slot.search.search(isbn, title, author);
        if rows.is_empty() && slot.search.blocked() {
            slot.consecutive_blocks.fetch_add(1, Ordering::SeqCst);
            slot.count(|c| c.blocked += 1);
            return None;
        }
        slot.consecutive_blocks.store(0, Ordering::SeqCst);
        let rows = rows.into_iter().take(MAX_ROWS).map(|r| StoreRow { title: r.title, author: r.author, category: r.category }).collect();
        Some((rows, method))
    }

    fn lookup_store(&self, slot: &StoreSlot, inode: i64, query: &BookQuery, now: i64) -> (StoreResult, bool) {
        let name = slot.name.as_str();
        // 1. inode 캐시
        if let Some(row) = self.cache.as_ref().and_then(|c| ok_or_warn(c.get_by_inode(inode, name, now), "조회")) {
            let isbn_conflict = matches!((&query.isbn13, &row.isbn13), (Some(now_isbn), Some(then_isbn)) if now_isbn != then_isbn);
            let same_query = row.method == "isbn" || row.query == query.title_key();
            if !isbn_conflict && same_query {
                slot.count(|c| c.cache_hit += 1);
                return (from_cached(name, &row), false);
            }
        }
        // 2. ISBN 캐시. "결과 없음"이 캐시돼 있으면 ISBN 검색은 건너뛰고 이름 검색으로 간다.
        let mut isbn_known_missing = false;
        if let (Some(cache), Some(isbn)) = (&self.cache, &query.isbn13)
            && let Some(row) = ok_or_warn(cache.get_by_isbn(isbn, name, now), "조회")
        {
            if row.status == "found" {
                self.put_inode(inode, name, &CachedLookup { method: "isbn".to_string(), query: isbn.clone(), isbn13: Some(isbn.clone()), ..row.clone() });
                slot.count(|c| c.cache_hit += 1);
                return (from_cached(name, &row), false);
            }
            isbn_known_missing = true;
        }
        // 3. 서점 검색
        if slot.consecutive_blocks.load(Ordering::SeqCst) >= BLOCK_LIMIT {
            slot.count(|c| c.skipped += 1);
            return (result(name, LookupStatus::Blocked, Vec::new()), false);
        }
        let mut network_used = false;
        if let Some(isbn) = query.isbn13.as_ref().filter(|_| !isbn_known_missing) {
            network_used = true;
            let Some((rows, _)) = self.call(slot, isbn, "", "") else {
                return (result(name, LookupStatus::Blocked, Vec::new()), true);
            };
            let found = !rows.is_empty();
            let entry = CachedLookup {
                status: if found { "found" } else { "not_found" }.to_string(),
                rows_json: serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string()),
                method: "isbn".to_string(),
                query: isbn.clone(),
                isbn13: Some(isbn.clone()),
                fetched_at: now,
            };
            self.put_isbn(isbn, name, &entry);
            if found {
                self.put_inode(inode, name, &entry);
                slot.count(|c| c.found += 1);
                return (result(name, LookupStatus::Found, rows), true);
            }
        }
        if !query.title.is_empty() {
            let Some((rows, method)) = self.call(slot, "", &query.title, &query.author) else {
                return (result(name, LookupStatus::Blocked, Vec::new()), true);
            };
            let found = !rows.is_empty();
            let entry = CachedLookup {
                status: if found { "found" } else { "not_found" }.to_string(),
                rows_json: serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string()),
                method: if method == "title" { method } else { "title_author".to_string() },
                query: query.title_key(),
                isbn13: query.isbn13.clone(),
                fetched_at: now,
            };
            self.put_inode(inode, name, &entry);
            slot.count(|c| if found { c.found += 1 } else { c.not_found += 1 });
            let status = if found { LookupStatus::Found } else { LookupStatus::NotFound };
            return (result(name, status, rows), true);
        }
        slot.count(|c| c.not_found += 1);
        (result(name, LookupStatus::NotFound, Vec::new()), network_used)
    }
}

impl BookstoreLookup for BookstoreResultFetcher {
    fn lookup(&self, inode: i64, query: &BookQuery) -> LookupOutcome {
        let now = self.clock.now() as i64;
        let outcomes: Vec<(StoreResult, bool)> = std::thread::scope(|scope| {
            let handles: Vec<_> = self.stores.iter().map(|slot| scope.spawn(move || self.lookup_store(slot, inode, query, now))).collect();
            handles
                .into_iter()
                .zip(&self.stores)
                .map(|(h, slot)| {
                    h.join().unwrap_or_else(|_| {
                        log::warn!("서점 조회 스레드 실패: {}", slot.name);
                        slot.count(|c| c.error += 1);
                        (result(&slot.name, LookupStatus::Error, Vec::new()), false)
                    })
                })
                .collect()
        });
        let network_used = outcomes.iter().any(|(_, used)| *used);
        LookupOutcome { results: outcomes.into_iter().map(|(r, _)| r).collect(), network_used }
    }

    fn summary(&self) -> Option<Value> {
        serde_json::to_value(self.counts()).ok()
    }
}
