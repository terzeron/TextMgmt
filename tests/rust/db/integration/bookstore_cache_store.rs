//! 서점 검색 결과 캐시를 실제 MySQL 컨테이너에 쓰고 되읽는다. Docker가 없으면 fixture가 panic한다.

#[path = "../../shared/mysql_fixture.rs"]
mod mysql_fixture;

use db::bookstore_cache_store::{BookstoreCacheStore, CachedLookup};

fn lookup(status: &str, rows_json: &str) -> CachedLookup {
    CachedLookup {
        status: status.to_string(),
        rows_json: rows_json.to_string(),
        method: "isbn".to_string(),
        query: "9780306406157".to_string(),
        isbn13: Some("9780306406157".to_string()),
        fetched_at: 1_000,
    }
}

#[test]
fn inode_and_isbn_rows_round_trip_and_overwrite() {
    mysql_fixture::mysql_container();
    let _guard = mysql_fixture::db_lock();
    let store = BookstoreCacheStore::new().expect("스토어 연결");

    store.put_inode(42, "yes24", &lookup("found", r#"[{"category":"SF"}]"#)).unwrap();
    assert_eq!(store.get_by_inode(42, "yes24", 5_000).unwrap(), Some(lookup("found", r#"[{"category":"SF"}]"#)));
    store.put_inode(42, "yes24", &lookup("not_found", "[]")).unwrap();
    assert_eq!(store.get_by_inode(42, "yes24", 5_000).unwrap().unwrap().status, "not_found");
    assert!(store.get_by_inode(42, "aladin", 5_000).unwrap().is_none());

    store.put_isbn("9780306406157", "kyobo", &lookup("found", "[]")).unwrap();
    let hit = store.get_by_isbn("9780306406157", "kyobo", 5_000).unwrap().unwrap();
    assert_eq!((hit.status.as_str(), hit.rows_json.as_str()), ("found", "[]"));
}
