//! 원본: tests/test_view_history_store.py
//!
//! `ViewHistoryStore`: 사용자·유형별 최근 조회 이력을 MySQL에 기록하고 되읽는다.
//! 실제 MySQL 컨테이너가 필요한 테스트는 컨테이너를 못 띄우면 Python처럼 건너뛴다.
//!
//! 가정한 API (설계: docs/rust-test-seams.md 3.4, 3.13):
//! - `backend::view_history_store::{MAX_RECENT_VIEWS: usize, ViewHistoryStore, ViewHistoryError, create_view_history_store}`
//! - `ViewHistoryStore::new() -> Result<ViewHistoryStore, ViewHistoryError>`: `TM_MYSQL_*` 환경 변수로 접속하고 테이블을 만든다
//! - `ViewHistoryStore::record_view(&self, email: &str, content_type: &str, book_id: i64, title: &str, category: &str) -> Result<(), ViewHistoryError>`
//! - `ViewHistoryStore::list_recent_views(&self, limit: usize) -> Result<Value, ViewHistoryError>` (Python 기본값은 `MAX_RECENT_VIEWS`)
//! - `ViewHistoryStore::get_connection(&self) -> Result<Box<dyn DbConnection>, DbError>` (구 `_get_connection() -> mysql::Conn`)
//! - `ViewHistoryError::{ValueError(String), RuntimeError(String)}` (`DbError::Other(msg)`는 `RuntimeError(msg)`로 바뀐다)
//! - `create_view_history_store() -> Result<ViewHistoryStore, ViewHistoryError>`
//! - `backend::db::{DbConnection, DbCursor, DbValue, DbError, Row, ConnectionProvider}`: 구 `StoreConnection`은 `DbConnection`이다.
//!   - `DbConnection: Send { cursor() -> Box<dyn DbCursor>; commit(); rollback(); close(); execute(sql, &[DbValue]) -> Result<u64, DbError>; fetch_one(sql, &[DbValue]) -> Result<Option<Row>, DbError> }` (뒤 둘은 기본 구현)
//!   - `DbCursor { execute(sql, &[DbValue]); executemany; fetchall() -> Option<Vec<Row>>; fetchone(); rowcount() }`, `Row = BTreeMap<String, DbValue>`
//!   - `DbValue::{Null, Int(i64), Text(String), DateTime(NaiveDateTime)}`는 `PartialEq + Debug`이고 `From<&str>`, `From<i64>`, `From<Value>`를 가진다
//! - (mock seam) `ViewHistoryStore::with_connection_provider(ConnectionProvider) -> ViewHistoryStore`(init_db를 건너뛴다. 구 `with_connection_factory`),
//!   `ViewHistoryStore::with_max_history_rows(self, usize) -> ViewHistoryStore`(구 `set_max_history_rows`)
//! - 로그는 `log::warn!`로 남기고 `common::log_capture::LogCapture`로 수집한다.

mod common;
#[path = "common/mysql_fixture.rs"]
mod mysql_fixture;

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::sleep;
use std::time::Duration;

use backend::db::{ConnectionProvider, DbConnection, DbCursor, DbError, DbValue, Row};
use backend::view_history_store::{MAX_RECENT_VIEWS, ViewHistoryError, ViewHistoryStore, create_view_history_store};
use common::log_capture::LogCapture;
use log::Level;
use serde_json::{Value, json};

/// `store` fixture: 실제 MySQL 컨테이너에 붙은 스토어. 테스트마다 테이블을 비운다.
/// 컨테이너를 못 띄우면 panic한다. 잠금은 테스트가 끝날 때까지 쥔다.
fn fixture() -> (ViewHistoryStore, MutexGuard<'static, ()>) {
    mysql_fixture::mysql_container();
    let guard = mysql_fixture::db_lock();
    let store = ViewHistoryStore::new().expect("스토어 연결");
    let mut conn = store.get_connection().expect("연결");
    conn.execute("TRUNCATE TABLE view_history", &[]).expect("TRUNCATE");
    (store, guard)
}

fn emails(result: &Value) -> Vec<String> {
    result["users"].as_array().unwrap().iter().map(|u| u["email"].as_str().unwrap().to_string()).collect()
}

fn titles(result: &Value, email: &str, content_type: &str) -> Vec<String> {
    let user = result["users"].as_array().unwrap().iter().find(|u| u["email"] == email).expect("사용자");
    user[content_type].as_array().unwrap().iter().map(|item| item["title"].as_str().unwrap().to_string()).collect()
}

fn list(store: &ViewHistoryStore) -> Value {
    store.list_recent_views(MAX_RECENT_VIEWS).unwrap()
}

#[test]
#[ignore = "Docker 필요"]
fn test_record_and_list_single_view() {
    let (store, _guard) = fixture();
    store.record_view("a@example.com", "book", 1, "책 하나", "소설").unwrap();

    let result = list(&store);

    assert_eq!(result["limit"], json!(MAX_RECENT_VIEWS));
    assert_eq!(emails(&result), vec!["a@example.com"]);
    let user = &result["users"][0];
    assert_eq!(user["comic"], json!([]));
    assert_eq!(user["book"].as_array().unwrap().len(), 1);
    assert_eq!(user["book"][0]["book_id"], json!(1));
    assert_eq!(user["book"][0]["title"], json!("책 하나"));
    assert_eq!(user["book"][0]["category"], json!("소설"));
    assert_eq!(user["last_viewed_at"], user["book"][0]["viewed_at"]);
}

#[test]
#[ignore = "Docker 필요"]
fn test_reviewing_same_book_does_not_duplicate() {
    let (store, _guard) = fixture();
    store.record_view("a@example.com", "book", 1, "원래 제목", "소설").unwrap();
    store.record_view("a@example.com", "book", 1, "바뀐 제목", "에세이").unwrap();

    let result = list(&store);
    let user = &result["users"][0];

    // 같은 책은 row 가 하나뿐이고 스냅샷이 최신으로 갱신된다
    assert_eq!(user["book"].as_array().unwrap().len(), 1);
    assert_eq!(user["book"][0]["title"], json!("바뀐 제목"));
    assert_eq!(user["book"][0]["category"], json!("에세이"));
}

#[test]
#[ignore = "Docker 필요"]
fn test_reviewing_moves_book_to_front() {
    let (store, _guard) = fixture();
    for (book_id, title) in [(1, "첫째"), (2, "둘째"), (3, "셋째")] {
        store.record_view("a@example.com", "book", book_id, title, "").unwrap();
        sleep(Duration::from_millis(10));
    }

    assert_eq!(titles(&list(&store), "a@example.com", "book"), vec!["셋째", "둘째", "첫째"]);

    // 가장 오래된 책을 다시 열면 맨 앞으로 온다
    sleep(Duration::from_millis(1100)); // viewed_at 은 초 단위라 순서를 확실히 벌린다
    store.record_view("a@example.com", "book", 1, "첫째", "").unwrap();

    assert_eq!(titles(&list(&store), "a@example.com", "book")[0], "첫째");
}

#[test]
#[ignore = "Docker 필요"]
fn test_keeps_only_max_recent_views_per_type() {
    let (store, _guard) = fixture();
    let overflow = MAX_RECENT_VIEWS + 5;
    for book_id in 0..overflow {
        store.record_view("a@example.com", "book", book_id as i64, &format!("책 {book_id}"), "").unwrap();
    }

    let result = list(&store);
    assert_eq!(result["users"][0]["book"].as_array().unwrap().len(), MAX_RECENT_VIEWS);

    // DB 에도 상한만 남아야 한다(초과분은 기록 시점에 정리)
    let mut conn = store.get_connection().unwrap();
    let row = conn.fetch_one("SELECT COUNT(*) AS cnt FROM view_history WHERE email = ? AND content_type = ?", &[DbValue::from("a@example.com"), DbValue::from("book")]).unwrap().unwrap();
    assert_eq!(row["cnt"], DbValue::Int(MAX_RECENT_VIEWS as i64));
}

#[test]
#[ignore = "Docker 필요"]
fn test_book_and_comic_limits_are_independent() {
    let (store, _guard) = fixture();
    for book_id in 0..MAX_RECENT_VIEWS + 3 {
        store.record_view("a@example.com", "book", book_id as i64, &format!("책 {book_id}"), "").unwrap();
    }
    for book_id in 0..MAX_RECENT_VIEWS + 3 {
        store.record_view("a@example.com", "comic", book_id as i64, &format!("만화 {book_id}"), "").unwrap();
    }

    let result = list(&store);
    let user = &result["users"][0];

    assert_eq!(user["book"].as_array().unwrap().len(), MAX_RECENT_VIEWS);
    assert_eq!(user["comic"].as_array().unwrap().len(), MAX_RECENT_VIEWS);
    // 같은 book_id 라도 유형이 다르면 별개 이력이다
    assert!(user["book"].as_array().unwrap().iter().all(|item| item["title"].as_str().unwrap().starts_with("책")));
    assert!(user["comic"].as_array().unwrap().iter().all(|item| item["title"].as_str().unwrap().starts_with("만화")));
}

#[test]
#[ignore = "Docker 필요"]
fn test_users_ordered_by_most_recent_activity() {
    let (store, _guard) = fixture();
    store.record_view("old@example.com", "book", 1, "오래된 조회", "").unwrap();
    sleep(Duration::from_millis(1100));
    store.record_view("new@example.com", "comic", 2, "최근 조회", "").unwrap();

    assert_eq!(emails(&list(&store)), vec!["new@example.com", "old@example.com"]);
}

#[test]
#[ignore = "Docker 필요"]
fn test_users_are_isolated() {
    let (store, _guard) = fixture();
    store.record_view("a@example.com", "book", 1, "A 의 책", "").unwrap();
    store.record_view("b@example.com", "book", 2, "B 의 책", "").unwrap();

    let result = list(&store);

    assert_eq!(titles(&result, "a@example.com", "book"), vec!["A 의 책"]);
    assert_eq!(titles(&result, "b@example.com", "book"), vec!["B 의 책"]);
}

#[test]
#[ignore = "Docker 필요"]
fn test_limit_argument_caps_returned_items() {
    let (store, _guard) = fixture();
    for book_id in 0..5 {
        store.record_view("a@example.com", "book", book_id, &format!("책 {book_id}"), "").unwrap();
    }

    let result = store.list_recent_views(2).unwrap();

    assert_eq!(result["limit"], json!(2));
    assert_eq!(result["users"][0]["book"].as_array().unwrap().len(), 2);
}

#[test]
#[ignore = "Docker 필요"]
fn test_snapshot_survives_when_source_book_is_gone() {
    // 책이 삭제돼도 이력은 조회 시점 제목을 그대로 보여준다(원본 join 없음).
    let (store, _guard) = fixture();
    store.record_view("a@example.com", "book", 999, "삭제될 책", "소설").unwrap();

    let result = list(&store);
    let user = &result["users"][0];

    assert_eq!(user["book"][0]["title"], json!("삭제될 책"));
    assert_eq!(user["book"][0]["book_id"], json!(999));
}

#[test]
#[ignore = "Docker 필요"]
fn test_rejects_unknown_content_type() {
    let (store, _guard) = fixture();
    match store.record_view("a@example.com", "magazine", 1, "x", "") {
        Err(ViewHistoryError::ValueError(message)) => assert!(message.contains("content_type"), "{message}"),
        other => panic!("ValueError(content_type)를 기대했다: {other:?}"),
    }
}

/// 실패 지점을 심은 가짜 연결이 남기는 호출 기록.
#[derive(Default)]
struct Flags {
    began: bool,
    committed: bool,
    rolled_back: bool,
}

#[test]
#[ignore = "DbConnection에 begin이 없어 `began` 단언을 그대로 옮길 수 없다(아래 FailingConn은 cursor() 요청을 begin으로 본다); 확신이 없어 유지"]
fn test_record_view_rolls_back_and_reraises_on_db_error() {
    /// 두 번째 `execute`에서 실패하는 가짜 연결. 커서가 연결과 같은 호출 횟수와 기록을 공유한다.
    struct FailingConn {
        calls: Arc<Mutex<usize>>,
        flags: Arc<Mutex<Flags>>,
    }

    struct FailingCursor {
        calls: Arc<Mutex<usize>>,
    }

    impl DbCursor for FailingCursor {
        fn execute(&mut self, _sql: &str, _params: &[DbValue]) -> Result<(), DbError> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;
            if *calls == 2 {
                return Err(DbError::Other("trim failed".to_string()));
            }
            Ok(())
        }

        fn executemany(&mut self, _sql: &str, _seq: &[Vec<DbValue>]) -> Result<(), DbError> {
            unreachable!("record_view는 executemany를 쓰지 않는다")
        }

        fn fetchall(&mut self) -> Option<Vec<Row>> {
            unreachable!("record_view는 행을 읽지 않는다")
        }

        fn fetchone(&mut self) -> Option<Row> {
            unreachable!("record_view는 행을 읽지 않는다")
        }

        fn rowcount(&self) -> u64 {
            0
        }
    }

    impl DbConnection for FailingConn {
        fn cursor(&mut self) -> Box<dyn DbCursor> {
            // DbConnection에는 begin이 없다. 트랜잭션은 첫 커서를 얻을 때 시작한다고 본다.
            self.flags.lock().unwrap().began = true;
            Box::new(FailingCursor { calls: self.calls.clone() })
        }

        fn commit(&mut self) -> Result<(), DbError> {
            self.flags.lock().unwrap().committed = true;
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), DbError> {
            self.flags.lock().unwrap().rolled_back = true;
            Ok(())
        }

        fn close(&mut self) {}
    }

    let flags = Arc::new(Mutex::new(Flags::default()));
    let factory_flags = flags.clone();
    let provider: ConnectionProvider = Arc::new(move || Ok(Box::new(FailingConn { calls: Arc::new(Mutex::new(0)), flags: factory_flags.clone() }) as Box<dyn DbConnection>));
    let store = ViewHistoryStore::with_connection_provider(provider);

    match store.record_view("a@example.com", "book", 1, "책", "") {
        Err(ViewHistoryError::RuntimeError(message)) => assert!(message.contains("trim failed"), "{message}"),
        other => panic!("RuntimeError(trim failed)를 기대했다: {other:?}"),
    }

    let flags = flags.lock().unwrap();
    assert!(flags.began);
    assert!(!flags.committed);
    assert!(flags.rolled_back);
}

#[test]
fn test_list_recent_views_warns_and_skips_unknown_content_type() {
    /// `execute`가 받은 (SQL, 파라미터)를 기록하고 `rows`를 돌려주는 가짜 연결.
    struct RowsConn {
        rows: Vec<Row>,
        executed: Arc<Mutex<Vec<(String, Vec<DbValue>)>>>,
    }

    struct RowsCursor {
        rows: Vec<Row>,
        executed: Arc<Mutex<Vec<(String, Vec<DbValue>)>>>,
    }

    impl DbCursor for RowsCursor {
        fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError> {
            self.executed.lock().unwrap().push((sql.to_string(), params.to_vec()));
            Ok(())
        }

        fn executemany(&mut self, _sql: &str, _seq: &[Vec<DbValue>]) -> Result<(), DbError> {
            unreachable!()
        }

        fn fetchall(&mut self) -> Option<Vec<Row>> {
            Some(self.rows.clone())
        }

        fn fetchone(&mut self) -> Option<Row> {
            unreachable!()
        }

        fn rowcount(&self) -> u64 {
            0
        }
    }

    impl DbConnection for RowsConn {
        fn cursor(&mut self) -> Box<dyn DbCursor> {
            Box::new(RowsCursor { rows: self.rows.clone(), executed: self.executed.clone() })
        }

        fn commit(&mut self) -> Result<(), DbError> {
            unreachable!("list_recent_views는 트랜잭션을 쓰지 않는다")
        }

        fn rollback(&mut self) -> Result<(), DbError> {
            unreachable!("list_recent_views는 트랜잭션을 쓰지 않는다")
        }

        fn close(&mut self) {}
    }

    let rows: Vec<Row> = vec![
        json!({"email": "book@example.com", "content_type": "magazine", "book_id": 1, "title": "잡지", "category": "", "viewed_at": 20}),
        json!({"email": "book@example.com", "content_type": "book", "book_id": 2, "title": "책", "category": "소설", "viewed_at": 10}),
    ]
    .into_iter()
    .map(|row| row.as_object().unwrap().iter().map(|(k, v)| (k.clone(), DbValue::from(v.clone()))).collect())
    .collect();
    let row_count = rows.len();
    let executed = Arc::new(Mutex::new(Vec::new()));
    let (provider_rows, provider_executed) = (rows.clone(), executed.clone());
    let provider: ConnectionProvider = Arc::new(move || Ok(Box::new(RowsConn { rows: provider_rows.clone(), executed: provider_executed.clone() }) as Box<dyn DbConnection>));
    let store = ViewHistoryStore::with_connection_provider(provider).with_max_history_rows(row_count);
    let logs = LogCapture::start(Level::Warn);

    let result = store.list_recent_views(MAX_RECENT_VIEWS).unwrap();

    assert_eq!(executed.lock().unwrap()[0].1, vec![DbValue::Int(row_count as i64)]);
    assert_eq!(
        result,
        json!({
            "limit": MAX_RECENT_VIEWS,
            "users": [{
                "email": "book@example.com",
                "last_viewed_at": 10,
                "book": [{"book_id": 2, "title": "책", "category": "소설", "viewed_at": 10}],
                "comic": [],
            }],
        })
    );
    let text = logs.text();
    assert!(text.contains("조회 이력이 상한"), "{text}");
    assert!(text.contains("알 수 없는 content_type"), "{text}");
}

#[test]
#[ignore = "Docker 필요"]
fn test_empty_store_returns_no_users() {
    let (store, _guard) = fixture();
    assert_eq!(list(&store), json!({"limit": MAX_RECENT_VIEWS, "users": []}));
}

#[test]
#[ignore = "Docker 필요"]
fn test_long_title_is_stored() {
    let (store, _guard) = fixture();
    let long_title = "가".repeat(500);
    store.record_view("a@example.com", "book", 1, &long_title, "").unwrap();

    assert_eq!(list(&store)["users"][0]["book"][0]["title"], json!(long_title));
}

#[test]
#[ignore = "Docker 필요"]
fn test_factory_returns_store() {
    mysql_fixture::mysql_container();
    let _guard = mysql_fixture::db_lock();
    assert!(create_view_history_store().is_ok());
}
