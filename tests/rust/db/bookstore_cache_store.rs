//! 서점 검색 결과 캐시. 연결을 대역으로 바꿔 SQL 인자와 만료 판정을 본다.

use std::sync::{Arc, Mutex};

use db::bookstore_cache_store::{BookstoreCacheStore, CachedLookup, NOT_FOUND_TTL_SECONDS};
use db::{DbConnection, DbError, DbValue, Row};

type Log = Arc<Mutex<Vec<(String, Vec<DbValue>)>>>;

struct FakeConn {
    log: Log,
    rows: Vec<Row>,
}

impl DbConnection for FakeConn {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<u64, DbError> {
        self.log.lock().unwrap().push((sql.to_string(), params.to_vec()));
        Ok(1)
    }
    fn fetch_all(&mut self, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, DbError> {
        self.log.lock().unwrap().push((sql.to_string(), params.to_vec()));
        Ok(self.rows.clone())
    }
    fn commit(&mut self) -> Result<(), DbError> {
        Ok(())
    }
    fn rollback(&mut self) -> Result<(), DbError> {
        Ok(())
    }
    fn close(&mut self) {}
}

fn store_with(rows: Vec<Row>) -> (BookstoreCacheStore, Log) {
    let log: Log = Arc::default();
    let provider_log = log.clone();
    let store = BookstoreCacheStore::with_connection_provider(Arc::new(move || Ok(Box::new(FakeConn { log: provider_log.clone(), rows: rows.clone() }) as Box<dyn DbConnection>)));
    (store, log)
}

fn lookup(status: &str) -> CachedLookup {
    CachedLookup {
        status: status.to_string(),
        rows_json: "[]".to_string(),
        method: "title_author".to_string(),
        query: "저자\u{1f}제목".to_string(),
        isbn13: Some("9780306406157".to_string()),
        fetched_at: 1_000,
    }
}

fn row(status: &str, expires_at: Option<i64>) -> Row {
    let mut r = Row::new();
    r.insert("status".into(), DbValue::Text(status.into()));
    r.insert("rows_json".into(), DbValue::Text("[]".into()));
    r.insert("method".into(), DbValue::Text("isbn".into()));
    r.insert("query".into(), DbValue::Text("9780306406157".into()));
    r.insert("isbn13".into(), DbValue::Null);
    r.insert("fetched_at".into(), DbValue::Int(1_000));
    r.insert("expires_at".into(), expires_at.map_or(DbValue::Null, DbValue::Int));
    r
}

#[test]
fn not_found_expires_after_thirty_days_and_found_never() {
    assert_eq!(NOT_FOUND_TTL_SECONDS, 30 * 24 * 60 * 60);
    let (store, log) = store_with(vec![]);
    store.put_inode(7, "yes24", &lookup("not_found")).unwrap();
    store.put_inode(7, "aladin", &lookup("found")).unwrap();
    let log = log.lock().unwrap();
    assert!(log[0].0.contains("bookstore_inode_cache"));
    assert_eq!(log[0].1.last(), Some(&DbValue::Int(1_000 + NOT_FOUND_TTL_SECONDS)));
    assert_eq!(log[1].1.last(), Some(&DbValue::Null));
    assert_eq!(&log[0].1[..2], &[DbValue::Int(7), DbValue::Text("yes24".into())]);
}

#[test]
fn isbn_rows_are_keyed_by_isbn_and_store() {
    let (store, log) = store_with(vec![]);
    store.put_isbn("9780306406157", "kyobo", &lookup("found")).unwrap();
    let log = log.lock().unwrap();
    assert!(log[0].0.contains("bookstore_isbn_cache"));
    assert_eq!(&log[0].1[..2], &[DbValue::Text("9780306406157".into()), DbValue::Text("kyobo".into())]);
}

#[test]
fn expired_rows_read_as_missing() {
    let (store, _) = store_with(vec![row("not_found", Some(2_000))]);
    assert!(store.get_by_inode(7, "yes24", 1_999).unwrap().is_some());
    assert!(store.get_by_inode(7, "yes24", 2_000).unwrap().is_none());
    let (store, _) = store_with(vec![row("found", None)]);
    let hit = store.get_by_isbn("9780306406157", "yes24", i64::MAX).unwrap().unwrap();
    assert_eq!((hit.status.as_str(), hit.method.as_str(), hit.isbn13), ("found", "isbn", None));
}

#[test]
fn missing_rows_read_as_none() {
    let (store, _) = store_with(vec![]);
    assert!(store.get_by_inode(7, "yes24", 0).unwrap().is_none());
}
