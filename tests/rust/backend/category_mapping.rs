//! 원본: tests/test_category_mapping.py
//!
//! `CategoryMapping`(MySQL 기반 카테고리 키워드·숨김·재적재 락·분류 제안 테이블) 테스트.
//! Python은 `patch.dict(sys.modules, {"pymysql": ...})`로 드라이버를 가짜로 바꾸고, 인스턴스의 `_get_connection`을 대입으로 갈아끼운다.
//! Rust는 `backend::db`의 `DbConnection`/`ConnectionProvider`로 가짜 연결을 주입한다(설계: docs/rust-test-seams.md 3.4).
//! 생성자 세 가지: `with_connector(provider)`는 초기화(`init_db`)를 실행하고, `with_connection_provider(provider)`는 초기화를 건너뛴다.
//!
//! 가정한 API (아직 구현 전이다. Python과 이름·시그니처가 같다고 본다):
//! - `backend::db`: 드라이버 추상화 (가짜 연결이 구현한다)
//!   - `enum DbValue { Null, Int(i64), Text(String), DateTime(chrono::NaiveDateTime) }` (`Clone + PartialEq + Debug`, `From<&str>`, `From<i64>`)
//!   - `type Row = BTreeMap<String, DbValue>`, `enum DbError { IntegrityError(String), Other(String) }`
//!   - `trait DbCursor { fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError>;
//!      fn executemany(&mut self, sql: &str, seq: &[Vec<DbValue>]) -> Result<(), DbError>;
//!      fn fetchall(&mut self) -> Option<Vec<Row>>; fn fetchone(&mut self) -> Option<Row>; fn rowcount(&self) -> u64; }`
//!   - `trait DbConnection: Send { fn cursor(&mut self) -> Box<dyn DbCursor>; fn commit(&mut self) -> Result<(), DbError>; fn rollback(&mut self) -> Result<(), DbError>; fn close(&mut self); }`
//!   - `type ConnectionProvider = Arc<dyn Fn() -> Result<Box<dyn DbConnection>, DbError> + Send + Sync>`
//! - `backend::category_mapping::CategoryMapping`
//!   - `with_connector(provider: ConnectionProvider) -> Result<CategoryMapping, DbError>` (`init_db` 실행. 마이그레이션의 인덱스 삭제 예외는 삼킨다)
//!   - `with_connection_provider(provider: ConnectionProvider) -> CategoryMapping` (`init_db` 생략)
//!   - `content_type`은 기본값이 없어 매번 마지막 인자로 넘긴다(`delete_category`만 `(category, content_type, prefix)`). 아래 목록은 `"book"`을 넘기는 모양이다.
//!   - `get_all_mappings(&self, ct) -> Result<HashMap<String, Vec<String>>, DbError>`, `get_keywords(&self, &str, ct) -> Result<Vec<String>, DbError>`
//!   - `add_keyword(&self, &str, &str, ct) -> bool`, `remove_keyword(&self, &str, &str, ct) -> bool`, `set_keywords(&self, &str, &[&str], ct) -> bool`
//!   - `update_all_mappings(&self, &HashMap<String, Vec<String>>, ct) -> bool`
//!   - `delete_category(&self, &str, ct, prefix: bool) -> bool`, `rename_category(&self, &str, &str, ct) -> bool`
//!   - `get_hidden_categories(&self, ct) -> Vec<String>`, `set_hidden(&self, &str, bool, ct) -> bool`
//!   - `get_latest_excluded_categories(&self, ct) -> Vec<String>`, `set_latest_excluded(&self, &str, bool, ct) -> bool`
//!   - `search_by_keyword(&self, &str, ct) -> Vec<String>`
//!   - `acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> (bool, Option<String>, Option<Row>)`
//!   - `release_reload_lock(&self, content_type: &str, category: Option<&str>)`
//!   - `heartbeat_reload_lock(&self, content_type: &str, category: Option<&str>, counts: &[(&str, i64)]) -> Result<(), ValueError>`
//!   - `complete_reload_lock(&self, content_type: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &[(&str, i64)]) -> Result<(), ValueError>`
//!   - `get_reload_status(&self, content_type: &str, category: Option<&str>) -> Option<Row>`
//!   - `add_classify_proposal_items(&self, items: &[Value], category: &str, ct)`, `get_classify_proposal_items(&self, ct).unwrap() -> Vec<Value>`
//!   - `clear_classify_proposal_items(&self, ct)`, `delete_classify_proposal_item(&self, file_path: &str, ct).unwrap() -> u64`
//!   - `merge_classify_proposal_status(&self, status: &Value, ct)`
//!   - `pub const RELOAD_LOCK_HEARTBEAT_STALE_SECONDS: i64`, `struct ValueError(pub String)`
//! - `chrono`가 dev-dependency에 있다고 본다 (`datetime.now()` 대응).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use backend::category_mapping::{CategoryMapping, RELOAD_LOCK_HEARTBEAT_STALE_SECONDS, ValueError};
use backend::db::{ConnectionProvider, DbConnection, DbCursor, DbError, DbValue, Row};
use chrono::{Duration, Local, NaiveDateTime};
use serde_json::{Value, json};

// TODO(common): FakeCursor/FakeConn은 refresh_token_store.rs, view_history_store.rs의 가짜 DbConnection과 겹친다. common/db_fake.rs로 합친다.

// ---- 가짜 DbConnection (Python FakeCursor/FakeConn 대응) ----

type SideEffect = Arc<dyn Fn(&str, &[DbValue]) -> Result<(), DbError> + Send + Sync>;

struct CursorState {
    rows: Vec<Row>,
    rowcount: u64,
    executed: Vec<(String, Vec<DbValue>)>,
    executed_many: Vec<(String, Vec<Vec<DbValue>>)>,
    side_effect: Option<SideEffect>,
    fetchone_rows: Vec<Option<Row>>,
    fetchone_index: usize,
}

/// 복제본이 상태를 공유한다. 테스트가 쥔 핸들로 실행 기록을 읽는다.
#[derive(Clone)]
struct FakeCursor {
    st: Arc<Mutex<CursorState>>,
}

impl FakeCursor {
    fn new() -> Self {
        Self {
            st: Arc::new(Mutex::new(CursorState {
                rows: Vec::new(),
                rowcount: 1,
                executed: Vec::new(),
                executed_many: Vec::new(),
                side_effect: None,
                fetchone_rows: vec![Some(row([("cnt", 0_i64.into())]))],
                fetchone_index: 0,
            })),
        }
    }

    fn with_rows(self, rows: Vec<Row>) -> Self {
        self.st.lock().unwrap().rows = rows;
        self
    }

    fn with_rowcount(self, rowcount: u64) -> Self {
        self.st.lock().unwrap().rowcount = rowcount;
        self
    }

    /// Python `fetchone_rows or [{"cnt": 0}]`처럼 빈 목록이면 기본값을 유지한다.
    fn with_fetchone_rows(self, rows: Vec<Option<Row>>) -> Self {
        if !rows.is_empty() {
            self.st.lock().unwrap().fetchone_rows = rows;
        }
        self
    }

    fn with_side_effect(self, f: impl Fn(&str, &[DbValue]) -> Result<(), DbError> + Send + Sync + 'static) -> Self {
        self.st.lock().unwrap().side_effect = Some(Arc::new(f));
        self
    }

    fn executed(&self) -> Vec<(String, Vec<DbValue>)> {
        self.st.lock().unwrap().executed.clone()
    }

    fn executed_many(&self) -> Vec<(String, Vec<Vec<DbValue>>)> {
        self.st.lock().unwrap().executed_many.clone()
    }

    fn ran(&self, needle: &str) -> bool {
        self.executed().iter().any(|(sql, _)| sql.contains(needle))
    }
}

impl DbCursor for FakeCursor {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError> {
        let effect = {
            let mut st = self.st.lock().unwrap();
            st.executed.push((sql.to_string(), params.to_vec()));
            st.side_effect.clone()
        };
        match effect {
            Some(f) => f(sql, params),
            None => Ok(()),
        }
    }

    fn executemany(&mut self, sql: &str, seq: &[Vec<DbValue>]) -> Result<(), DbError> {
        let effect = {
            let mut st = self.st.lock().unwrap();
            st.executed_many.push((sql.to_string(), seq.to_vec()));
            st.side_effect.clone()
        };
        if let Some(effect) = effect {
            for params in seq {
                effect(sql, params)?;
            }
        }
        Ok(())
    }

    fn fetchall(&mut self) -> Option<Vec<Row>> {
        Some(self.st.lock().unwrap().rows.clone())
    }

    fn fetchone(&mut self) -> Option<Row> {
        let mut st = self.st.lock().unwrap();
        if st.fetchone_index < st.fetchone_rows.len() {
            let found = st.fetchone_rows[st.fetchone_index].clone();
            st.fetchone_index += 1;
            return found;
        }
        Some(row([("cnt", 0_i64.into())]))
    }

    fn rowcount(&self) -> u64 {
        self.st.lock().unwrap().rowcount
    }
}

#[derive(Clone)]
struct FakeConn {
    cursor: FakeCursor,
    committed: Arc<AtomicBool>,
    rolled_back: Arc<AtomicBool>,
}

impl FakeConn {
    fn new(cursor: &FakeCursor) -> Self {
        Self { cursor: cursor.clone(), committed: Arc::new(AtomicBool::new(false)), rolled_back: Arc::new(AtomicBool::new(false)) }
    }

    fn committed(&self) -> bool {
        self.committed.load(Ordering::SeqCst)
    }

    fn rolled_back(&self) -> bool {
        self.rolled_back.load(Ordering::SeqCst)
    }
}

impl DbConnection for FakeConn {
    fn cursor(&mut self) -> Box<dyn DbCursor> {
        Box::new(self.cursor.clone())
    }

    fn commit(&mut self) -> Result<(), DbError> {
        self.committed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), DbError> {
        self.rolled_back.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn close(&mut self) {}
}

fn row<const N: usize>(pairs: [(&str, DbValue); N]) -> Row {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// `{"cnt": n}`을 `times`번 반복한 fetchone 응답.
fn rep(n: i64, times: usize) -> Vec<Option<Row>> {
    (0..times).map(|_| Some(row([("cnt", n.into())]))).collect()
}

/// 가짜 `DbConnection`으로 CategoryMapping을 만든다. 생성자(`with_connector`)가 information_schema 조회와 마이그레이션을 `cursor`로 실행한다.
/// 초기화를 건너뛰고 연결만 갈아끼우려면 `CategoryMapping::with_connection_provider`를 쓴다(구 `set_connection_provider`).
fn build_cm(cursor: &FakeCursor) -> CategoryMapping {
    let c = cursor.clone();
    let connector: ConnectionProvider = Arc::new(move || Ok(Box::new(FakeConn::new(&c)) as Box<dyn DbConnection>));
    CategoryMapping::with_connector(connector).unwrap()
}

/// 호출마다 새 FakeConn을 주는 `ConnectionProvider`. 연결 핸들은 `conns`에 쌓는다.
fn recording_provider(cursor: &FakeCursor) -> (ConnectionProvider, Arc<Mutex<Vec<FakeConn>>>) {
    let conns = Arc::new(Mutex::new(Vec::new()));
    let (c, sink) = (cursor.clone(), conns.clone());
    let provider: ConnectionProvider = Arc::new(move || {
        let conn = FakeConn::new(&c);
        sink.lock().unwrap().push(conn.clone());
        Ok(Box::new(conn) as Box<dyn DbConnection>)
    });
    (provider, conns)
}

fn provider_for_cursor(cursor: &FakeCursor) -> ConnectionProvider {
    recording_provider(cursor).0
}

/// 같은 FakeConn을 매번 돌려주는 `ConnectionProvider`. 테스트가 committed/rolled_back을 읽는다.
fn provider_of(conn: &FakeConn) -> ConnectionProvider {
    let conn = conn.clone();
    Arc::new(move || Ok(Box::new(conn.clone()) as Box<dyn DbConnection>))
}

fn fail_with(msg: &'static str) -> impl Fn(&str, &[DbValue]) -> Result<(), DbError> + Send + Sync + 'static {
    move |_sql, _params| Err(DbError::Other(msg.to_string()))
}

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn reload_row(category: DbValue, status: &str, at: NaiveDateTime) -> Row {
    row([
        ("category", category),
        ("status", status.into()),
        ("started_at", DbValue::DateTime(at)),
        ("updated_at", DbValue::DateTime(at)),
        ("indexed_count", 0_i64.into()),
        ("deleted_count", 0_i64.into()),
        ("failed_count", 0_i64.into()),
        ("before_count", 0_i64.into()),
        ("after_count", 0_i64.into()),
        ("error", DbValue::Null),
    ])
}

fn lock_row(lock_key: &str, status: &str, updated_at: NaiveDateTime) -> Row {
    row([("lock_key", lock_key.into()), ("status", status.into()), ("updated_at", DbValue::DateTime(updated_at))])
}

mod test_category_mapping {
    use super::*;

    #[test]
    fn test_init() {
        let cursor = FakeCursor::new().with_fetchone_rows(rep(0, 3));
        // 생성자가 패닉하지 않고 끝나면 Python의 `assert cm is not None`과 같다.
        let _cm = build_cm(&cursor);
    }

    #[test]
    fn test_init_creates_latest_excluded_categories_table() {
        let cursor = FakeCursor::new().with_fetchone_rows(rep(1, 3));
        let _cm = build_cm(&cursor);
        assert!(cursor.ran("CREATE TABLE IF NOT EXISTS latest_excluded_categories"));
    }

    #[test]
    fn test_get_all_and_keywords() {
        let cursor = FakeCursor::new().with_rows(vec![row([("category", "c1".into()), ("keyword", "k1".into())]), row([("category", "c1".into()), ("keyword", "k2".into())])]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let mappings = cm.get_all_mappings("book").unwrap();
        assert_eq!(mappings, BTreeMap::from([("c1".to_string(), strs(&["k1", "k2"]))]));
        let keywords = cm.get_keywords("c1", "book").unwrap();
        assert_eq!(keywords, strs(&["k1", "k2"]));
    }

    #[test]
    fn test_get_all_mappings_keeps_category_order() {
        // 키워드 동률 판정과 /mapping 응답이 SQL의 ORDER BY category 순서를 따라야 한다(Python dict와 같다).
        let categories = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let rows = categories.iter().map(|c| row([("category", (*c).into()), ("keyword", "k".into())])).collect();
        let cursor = FakeCursor::new().with_rows(rows);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let order: Vec<String> = cm.get_all_mappings("book").unwrap().into_keys().collect();
        assert_eq!(order, strs(&categories));
    }

    #[test]
    fn test_get_keywords_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("query failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        assert!(cm.get_keywords("fiction", "book").unwrap_err().to_string().contains("query failed"));
    }

    #[test]
    fn test_get_all_mappings_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("query failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        assert!(cm.get_all_mappings("book").unwrap_err().to_string().contains("query failed"));
    }

    #[test]
    fn test_get_classify_proposal_items_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("query failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        assert!(cm.get_classify_proposal_items("book").unwrap_err().to_string().contains("query failed"));
    }

    #[test]
    fn test_get_classify_proposal_items_propagates_invalid_json_like_python() {
        let cursor = FakeCursor::new().with_rows(vec![row([("payload", "{".into())])]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        assert!(cm.get_classify_proposal_items("book").unwrap_err().to_string().contains("payload"));
    }

    #[test]
    fn test_add_and_remove_keyword() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.add_keyword("cat", " kw ", "book").unwrap());
        assert!(cm.remove_keyword("cat", "kw", "book").unwrap());
    }

    #[test]
    fn test_remove_keyword_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("delete unavailable"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.remove_keyword("cat", "kw", "book").unwrap_err().to_string().contains("delete unavailable"));
    }

    #[test]
    fn test_add_keyword_propagates_non_integrity_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("insert unavailable"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.add_keyword("cat", "kw", "book").unwrap_err().to_string().contains("insert unavailable"));
    }

    #[test]
    fn test_add_keyword_empty() {
        let cursor = FakeCursor::new();
        let cm = build_cm(&cursor);
        assert!(!cm.add_keyword("cat", "  ", "book").unwrap());
    }

    #[test]
    fn test_add_keyword_duplicate() {
        let cursor_fail = FakeCursor::new().with_side_effect(|_sql, _params| Err(DbError::IntegrityError("dup".to_string())));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor_fail));
        assert!(!cm.add_keyword("cat", "kw", "book").unwrap());
    }

    #[test]
    fn test_set_keywords_success_and_fail() {
        let cursor = FakeCursor::new();
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        assert!(cm.set_keywords("cat", &["a", "a", " ", "b"], "book"));
        assert!(conn.committed());

        let cursor_fail = FakeCursor::new().with_side_effect(fail_with("fail"));
        let conn_fail = FakeConn::new(&cursor_fail);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn_fail));
        assert!(!cm.set_keywords("cat", &["a"], "book"));
        assert!(conn_fail.rolled_back());
    }

    #[test]
    fn test_update_all_mappings() {
        let cursor = FakeCursor::new();
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        assert!(cm.update_all_mappings(&HashMap::from([("c1".to_string(), strs(&["k1", "k2"]))]), "book"));
        assert!(conn.committed());
    }

    #[test]
    fn test_delete_and_hidden() {
        let cursor = FakeCursor::new().with_rowcount(1);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.delete_category("cat", "book", false).unwrap());
        assert!(cm.delete_category("cat", "book", true).unwrap());

        let cursor_hidden = FakeCursor::new().with_rows(vec![row([("category", "c1".into())])]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor_hidden));
        assert_eq!(cm.get_hidden_categories("book").unwrap(), strs(&["c1"]));
    }

    #[test]
    fn test_get_and_set_latest_excluded_categories() {
        let cursor_latest = FakeCursor::new().with_rows(vec![row([("category", "c1".into())])]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor_latest));
        assert_eq!(cm.get_latest_excluded_categories("book").unwrap(), strs(&["c1"]));

        let cursor_set = FakeCursor::new();
        let conn = FakeConn::new(&cursor_set);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        assert!(cm.set_latest_excluded("cat", true, "book").unwrap());
        assert!(cm.set_latest_excluded("cat", false, "book").unwrap());
        assert!(conn.committed());
        assert!(cursor_set.ran("INSERT IGNORE INTO latest_excluded_categories"));
        assert!(cursor_set.ran("DELETE FROM latest_excluded_categories"));
    }

    #[test]
    fn test_categories_search_and_rename() {
        let cursor = FakeCursor::new().with_rows(vec![row([("category", "a".into())]), row([("category", "b".into())])]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert_eq!(cm.search_by_keyword("key", "book").unwrap(), strs(&["a", "b"]));

        let cursor_rename = FakeCursor::new();
        let conn = FakeConn::new(&cursor_rename);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        assert!(cm.rename_category("old", "new", "book"));
        assert!(cursor_rename.ran("UPDATE latest_excluded_categories SET category"));

        let cursor_fail = FakeCursor::new().with_side_effect(fail_with("fail"));
        let conn_fail = FakeConn::new(&cursor_fail);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn_fail));
        assert!(!cm.rename_category("old", "new", "book"));
    }

    #[test]
    fn test_mapping_category_list_reads_propagate_database_errors_like_python() {
        for read in ["hidden", "latest", "search"] {
            let cursor = FakeCursor::new().with_side_effect(fail_with("query failed"));
            let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
            let error = match read {
                "hidden" => cm.get_hidden_categories("book").unwrap_err(),
                "latest" => cm.get_latest_excluded_categories("book").unwrap_err(),
                _ => cm.search_by_keyword("key", "book").unwrap_err(),
            };
            assert!(error.to_string().contains("query failed"), "{read}: {error}");
        }
    }

    #[test]
    fn test_set_hidden() {
        let cursor = FakeCursor::new();
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        assert!(cm.set_hidden("cat", true, "book").unwrap());
        assert!(cm.set_hidden("cat", false, "book").unwrap());

        let cursor_fail = FakeCursor::new().with_side_effect(fail_with("fail"));
        let conn_fail = FakeConn::new(&cursor_fail);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn_fail));
        assert!(!cm.set_hidden("cat", true, "book").unwrap());
    }

    #[test]
    fn test_unhide_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("unhide failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.set_hidden("cat", false, "book").unwrap_err().to_string().contains("unhide failed"));
    }

    #[test]
    fn test_set_latest_excluded_failure() {
        let cursor_fail = FakeCursor::new().with_side_effect(fail_with("fail"));
        let conn_fail = FakeConn::new(&cursor_fail);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn_fail));
        assert!(!cm.set_latest_excluded("cat", true, "book").unwrap());
    }

    #[test]
    fn test_unexclude_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("unexclude failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.set_latest_excluded("cat", false, "book").unwrap_err().to_string().contains("unexclude failed"));
    }

    #[test]
    fn test_migrate_drop_index_exception_category_keywords() {
        // unique_category_keyword 인덱스 삭제가 실패해도 생성자가 예외를 삼켜야 한다.
        let cursor = FakeCursor::new().with_fetchone_rows(rep(0, 2)).with_side_effect(|sql, _params| {
            if sql.contains("DROP INDEX unique_category_keyword") {
                return Err(DbError::Other("index not found".to_string()));
            }
            Ok(())
        });
        let _cm = build_cm(&cursor);
    }

    #[test]
    fn test_migrate_drop_index_exception_hidden_categories() {
        // hidden_categories의 category 인덱스 삭제가 실패해도 생성자가 예외를 삼켜야 한다.
        let cursor = FakeCursor::new().with_fetchone_rows(rep(0, 2)).with_side_effect(|sql, _params| {
            if sql.contains("DROP INDEX category") {
                return Err(DbError::Other("index not found".to_string()));
            }
            Ok(())
        });
        let _cm = build_cm(&cursor);
    }

    #[test]
    fn test_update_all_mappings_exception() {
        // update_all_mappings는 예외 시 rollback하고 false를 돌려준다.
        let cursor_fail = FakeCursor::new().with_side_effect(|sql, _params| {
            if sql.contains("DELETE") {
                return Err(DbError::Other("db error".to_string()));
            }
            Ok(())
        });
        let conn_fail = FakeConn::new(&cursor_fail);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn_fail));
        assert!(!cm.update_all_mappings(&HashMap::from([("c1".to_string(), strs(&["k1"]))]), "book"));
        assert!(conn_fail.rolled_back());
    }

    #[test]
    fn test_get_all_mappings_empty_and_remove_keyword_not_found() {
        let cursor = FakeCursor::new().with_rows(Vec::new()).with_rowcount(0);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.get_all_mappings("book").unwrap().is_empty());
        assert!(!cm.remove_keyword("cat", "kw", "book").unwrap());
    }

    #[test]
    fn test_delete_category_not_found() {
        let cursor = FakeCursor::new().with_rowcount(0);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(!cm.delete_category("missing", "book", false).unwrap());
    }

    #[test]
    fn test_delete_category_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("delete failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.delete_category("cat", "book", false).unwrap_err().to_string().contains("delete failed"));
    }

    #[test]
    fn test_set_hidden_false_always_succeeds() {
        let cursor = FakeCursor::new().with_rowcount(0);
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        assert!(cm.set_hidden("cat", false, "book").unwrap());
        assert!(conn.committed());
    }

    #[test]
    fn test_migrate_skips_when_content_type_already_exists() {
        // information_schema 조회 순서: 1~3 content_type 컬럼, 4 apply_status,
        // 5 idx_content_file, 6 idx_content_type, 7~16 reload_locks 컬럼, 17 lock_key.
        // 6번만 0(없음)이라 지울 중복 인덱스가 없고, 나머지는 이미 있어 손댈 게 없다.
        let mut rows = rep(1, 5);
        rows.extend(rep(0, 1));
        rows.extend(rep(1, 11));
        let cursor = FakeCursor::new().with_fetchone_rows(rows);
        let _cm = build_cm(&cursor);
        let alter_queries: Vec<String> = cursor.executed().into_iter().map(|(sql, _)| sql).filter(|sql| sql.contains("ALTER TABLE")).collect();
        assert!(alter_queries.is_empty(), "{alter_queries:?}");
    }

    #[test]
    fn test_migrate_reload_locks_adds_reload_source() {
        // 16번째 조회가 reload_source 컬럼 존재 여부다(위 테스트의 순서 주석 참고).
        let mut rows = rep(1, 15);
        rows.extend(rep(0, 1));
        rows.extend(rep(1, 1));
        let cursor = FakeCursor::new().with_fetchone_rows(rows);
        let _cm = build_cm(&cursor);
        assert!(cursor.ran("ADD COLUMN reload_source"));
        assert!(cursor.ran("SET reload_source = %s WHERE category IS NOT NULL"));
    }

    #[test]
    fn test_migrate_reload_locks_adds_lock_key_and_switches_primary_key() {
        let mut rows = rep(1, 14);
        rows.extend(rep(0, 1));
        let cursor = FakeCursor::new().with_fetchone_rows(rows);
        let _cm = build_cm(&cursor);
        let executed_sql: Vec<String> = cursor.executed().into_iter().map(|(sql, _)| sql).collect();
        assert!(executed_sql.iter().any(|s| s.contains("ADD COLUMN lock_key")));
        assert!(executed_sql.iter().any(|s| s.starts_with("UPDATE reload_locks SET lock_key")));
        assert!(executed_sql.iter().any(|s| s.contains("DROP PRIMARY KEY") && s.contains("ADD PRIMARY KEY (content_type, lock_key)")));
    }

    #[test]
    fn test_acquire_reload_lock_success() {
        let cursor = FakeCursor::new().with_rows(Vec::new());
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, None).unwrap();
        assert!(acquired);
        assert!(error.is_none());
        assert!(blocking.is_none());
        assert!(cursor.ran("INSERT INTO reload_locks"));
    }

    #[test]
    fn test_acquire_reload_lock_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("lock query failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.acquire_reload_lock("book", None, None).unwrap_err().to_string().contains("lock query failed"));
    }

    #[test]
    fn test_acquire_reload_lock_stores_reload_source() {
        let cursor = FakeCursor::new().with_rows(Vec::new());
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, Some("mismatch")).unwrap();
        assert!(acquired);
        assert!(error.is_none());
        assert!(blocking.is_none());
        let (sql, params) = cursor.executed().into_iter().find(|(sql, _)| sql.contains("INSERT INTO reload_locks")).unwrap();
        assert!(sql.contains("reload_source"));
        assert!(sql.contains("reload_source = VALUES(reload_source)"));
        assert_eq!(params, vec![DbValue::from("book"), DbValue::from("__all__"), DbValue::Null, DbValue::from("mismatch")]);
    }

    #[test]
    fn test_acquire_reload_lock_defaults_unknown_reload_source_to_bulk() {
        let cursor = FakeCursor::new().with_rows(Vec::new());
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, Some("unknown")).unwrap();
        assert!(acquired);
        assert!(error.is_none());
        assert!(blocking.is_none());
        let (_sql, params) = cursor.executed().into_iter().find(|(sql, _)| sql.contains("INSERT INTO reload_locks")).unwrap();
        assert_eq!(params, vec![DbValue::from("book"), DbValue::from("__all__"), DbValue::Null, DbValue::from("bulk")]);
    }

    #[test]
    fn test_acquire_reload_lock_ignores_finished_lock_rows() {
        let cursor = FakeCursor::new().with_rows(vec![lock_row("__all__", "done", now())]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, None).unwrap();
        assert!(acquired);
        assert!(error.is_none());
        assert!(blocking.is_none());
        assert!(cursor.ran("INSERT INTO reload_locks"));
    }

    #[test]
    fn test_acquire_reload_lock_already_in_progress() {
        let at = now();
        let cursor = FakeCursor::new().with_rows(vec![lock_row("__all__", "running", at)]).with_fetchone_rows(vec![Some(reload_row(DbValue::Null, "running", at))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, None).unwrap();
        assert!(!acquired);
        assert!(error.is_some());
        assert!(error.unwrap().contains("진행 중"));
        assert_eq!(blocking.unwrap()["status"], DbValue::from("running"));
        assert!(!cursor.ran("INSERT INTO reload_locks"));
    }

    #[test]
    fn test_acquire_reload_lock_replaces_stale_lock() {
        let old_updated_at = now() - Duration::seconds(RELOAD_LOCK_HEARTBEAT_STALE_SECONDS + 60);
        let cursor = FakeCursor::new().with_rows(vec![lock_row("__all__", "running", old_updated_at)]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, None).unwrap();
        assert!(acquired);
        assert!(error.is_none());
        assert!(blocking.is_none());
        let inserts = cursor.executed().iter().filter(|(sql, _)| sql.contains("INSERT INTO reload_locks")).count();
        assert_eq!(inserts, 1);
    }

    #[test]
    fn test_acquire_reload_lock_different_categories_are_independent() {
        let cursor = FakeCursor::new().with_rows(Vec::new());
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", Some("1_fiction"), None).unwrap();
        assert!(acquired);
        assert!(error.is_none());
        assert!(blocking.is_none());
        let select_sql = cursor.executed().into_iter().map(|(sql, _)| sql).find(|sql| sql.trim().to_uppercase().starts_with("SELECT")).unwrap();
        assert!(select_sql.contains("IN (%s, %s)"), "{select_sql}");
    }

    #[test]
    fn test_acquire_reload_lock_category_blocked_by_running_bulk_lock() {
        let at = now();
        let cursor = FakeCursor::new().with_rows(vec![lock_row("__all__", "running", at)]).with_fetchone_rows(vec![Some(reload_row(DbValue::Null, "running", at))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", Some("1_fiction"), None).unwrap();
        assert!(!acquired);
        assert!(error.unwrap().contains("다른 재적재 작업"));
        assert_eq!(blocking.unwrap()["category"], DbValue::Null);
        assert!(!cursor.ran("INSERT INTO reload_locks"));
    }

    #[test]
    fn test_acquire_reload_lock_bulk_blocked_by_running_category_lock() {
        let at = now();
        let cursor = FakeCursor::new().with_rows(vec![lock_row("1_fiction", "running", at)]).with_fetchone_rows(vec![Some(reload_row("1_fiction".into(), "running", at))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let (acquired, error, blocking) = cm.acquire_reload_lock("book", None, None).unwrap();
        assert!(!acquired);
        assert!(error.unwrap().contains("다른 재적재 작업"));
        assert_eq!(blocking.unwrap()["category"], DbValue::from("1_fiction"));
        assert!(!cursor.ran("INSERT INTO reload_locks"));
    }

    #[test]
    fn test_release_reload_lock() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        cm.release_reload_lock("book", None).unwrap();
        assert!(cursor.ran("DELETE FROM reload_locks"));
    }

    #[test]
    fn test_release_reload_lock_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("release failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.release_reload_lock("book", None).unwrap_err().to_string().contains("release failed"));
    }

    #[test]
    fn test_heartbeat_reload_lock_touches_updated_at_only() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        cm.heartbeat_reload_lock("book", None, &[]).unwrap();
        let (sql, params) = cursor.executed().last().cloned().unwrap();
        assert!(sql.contains("updated_at = NOW()"));
        assert!(!sql.contains("indexed_count"));
        assert_eq!(params, vec![DbValue::from("book"), DbValue::from("__all__")]);
    }

    #[test]
    fn test_heartbeat_reload_lock_scopes_to_category() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        cm.heartbeat_reload_lock("book", Some("1_fiction"), &[]).unwrap();
        let (_sql, params) = cursor.executed().last().cloned().unwrap();
        assert_eq!(params, vec![DbValue::from("book"), DbValue::from("1_fiction")]);
    }

    #[test]
    fn test_heartbeat_reload_lock_updates_counts() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        cm.heartbeat_reload_lock("book", None, &[("indexed_count", 3), ("deleted_count", 1)]).unwrap();
        let (sql, params) = cursor.executed().last().cloned().unwrap();
        assert!(sql.contains("indexed_count = %s"));
        assert!(sql.contains("deleted_count = %s"));
        assert_eq!(params, vec![DbValue::from(3_i64), DbValue::from(1_i64), DbValue::from("book"), DbValue::from("__all__")]);
    }

    #[test]
    fn test_heartbeat_reload_lock_rejects_unknown_field() {
        let cm = build_cm(&FakeCursor::new());
        let result = cm.heartbeat_reload_lock("book", None, &[("bogus_count", 1)]);
        assert!(matches!(result, Err(ValueError(_))), "{result:?}");
    }

    #[test]
    fn test_complete_reload_lock_records_final_status() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        cm.complete_reload_lock("book", "done", None, None, &[("indexed_count", 5), ("deleted_count", 2), ("failed_count", 0), ("before_count", 7), ("after_count", 0)]).unwrap();
        let (sql, params) = cursor.executed().last().cloned().unwrap();
        assert!(sql.contains("status = %s"));
        assert_eq!(params[0], DbValue::from("done"));
        assert_eq!(params[params.len() - 2], DbValue::from("book"));
        assert_eq!(params[params.len() - 1], DbValue::from("__all__"));
    }

    #[test]
    fn test_complete_reload_lock_scopes_to_category() {
        let cursor = FakeCursor::new();
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        cm.complete_reload_lock("book", "done", None, Some("1_fiction"), &[("indexed_count", 5)]).unwrap();
        let (_sql, params) = cursor.executed().last().cloned().unwrap();
        assert_eq!(params[params.len() - 2], DbValue::from("book"));
        assert_eq!(params[params.len() - 1], DbValue::from("1_fiction"));
    }

    #[test]
    fn test_complete_reload_lock_rejects_unknown_field() {
        let cm = build_cm(&FakeCursor::new());
        let result = cm.complete_reload_lock("book", "done", None, None, &[("bogus_count", 1)]);
        assert!(matches!(result, Err(ValueError(_))), "{result:?}");
    }

    #[test]
    fn test_get_reload_status_none_when_no_row() {
        let cursor = FakeCursor::new().with_fetchone_rows(vec![None]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.get_reload_status("book", None).unwrap().is_none());
    }

    #[test]
    fn test_get_reload_status_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("query failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.get_reload_status("book", None).unwrap_err().to_string().contains("query failed"));
    }

    #[test]
    fn test_get_reload_status_returns_running_row() {
        let at = now();
        let status_row = row([
            ("category", "A".into()),
            ("reload_source", "mismatch".into()),
            ("status", "running".into()),
            ("started_at", DbValue::DateTime(at)),
            ("updated_at", DbValue::DateTime(at)),
            ("indexed_count", 1_i64.into()),
            ("deleted_count", 0_i64.into()),
            ("failed_count", 0_i64.into()),
            ("before_count", 2_i64.into()),
            ("after_count", 0_i64.into()),
            ("error", DbValue::Null),
        ]);
        let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(status_row)]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let status = cm.get_reload_status("book", None).unwrap().unwrap();
        assert_eq!(status["status"], DbValue::from("running"));
        assert_eq!(status["category"], DbValue::from("A"));
        assert_eq!(status["reload_source"], DbValue::from("mismatch"));
        assert_eq!(status["indexed_count"], DbValue::from(1_i64));
    }

    #[test]
    fn test_get_reload_status_reports_stale_running_as_failed() {
        let old_updated_at = now() - Duration::seconds(RELOAD_LOCK_HEARTBEAT_STALE_SECONDS + 60);
        let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(reload_row("A".into(), "running", old_updated_at))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        let status = cm.get_reload_status("book", None).unwrap().unwrap();
        assert_eq!(status["status"], DbValue::from("failed"));
        // Python `assert status["error"]`: None이나 빈 문자열이면 실패한다.
        assert!(!matches!(&status["error"], DbValue::Null) && status["error"] != DbValue::from(""), "{:?}", status["error"]);
    }

    fn classify_status_row(status: &str, payload: Value, age_seconds: i64) -> Row {
        row([("status", status.into()), ("payload", serde_json::to_string(&payload).unwrap().into()), ("age_seconds", age_seconds.into())])
    }

    #[test]
    fn test_get_classify_proposal_status_marks_stale_running_failed_and_preserves_progress() {
        let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(classify_status_row("running", json!({"remaining_count": 19, "error": "기존 오류"}), RELOAD_LOCK_HEARTBEAT_STALE_SECONDS + 1))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        let status = cm.get_classify_proposal_status("book", RELOAD_LOCK_HEARTBEAT_STALE_SECONDS).unwrap();

        assert_eq!(status["status"], "failed");
        assert_eq!(status["error"], "백엔드가 다시 시작되어 진행 중이던 작업이 중단되었습니다. 남은 항목을 확인한 뒤 진행하세요.");
        assert!(!status["error"].as_str().unwrap().contains("다시 실행"));
        assert_eq!(status["remaining_count"], 19);
    }

    #[test]
    fn test_get_classify_proposal_status_treats_missing_heartbeat_as_stale() {
        for state in ["running", "applying"] {
            let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(row([("status", state.into()), ("payload", json!({"progress": 3}).to_string().into()), ("age_seconds", DbValue::Null)]))]);
            let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

            let status = cm.get_classify_proposal_status("book", RELOAD_LOCK_HEARTBEAT_STALE_SECONDS).unwrap();

            assert_eq!(status["status"], "failed", "state={state}");
            assert_eq!(status["progress"], 3, "state={state}");
        }
    }

    #[test]
    fn test_get_classify_proposal_status_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("query failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.get_classify_proposal_status("book", 300).unwrap_err().to_string().contains("query failed"));
    }

    #[test]
    fn test_get_classify_proposal_status_propagates_invalid_json_like_python() {
        let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(row([("status", "running".into()), ("payload", "{".into()), ("age_seconds", 0.into())]))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.get_classify_proposal_status("book", 300).unwrap_err().to_string().contains("payload"));
    }

    #[test]
    fn test_try_start_classify_proposal_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("status row insert failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.try_start_classify_proposal(&serde_json::json!({"status": "running"}), "book", 300).unwrap_err().to_string().contains("status row insert failed"));
    }

    #[test]
    fn test_get_classify_proposal_status_marks_stale_applying_failed_and_preserves_applied_count() {
        let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(classify_status_row("applying", json!({"applied_count": 1}), RELOAD_LOCK_HEARTBEAT_STALE_SECONDS + 1))]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        let status = cm.get_classify_proposal_status("book", RELOAD_LOCK_HEARTBEAT_STALE_SECONDS).unwrap();

        assert_eq!(status["status"], "failed");
        assert_eq!(status["applied_count"], 1);
    }

    #[test]
    fn test_get_classify_proposal_status_leaves_recent_and_terminal_states_untouched() {
        for (state, age) in [("running", RELOAD_LOCK_HEARTBEAT_STALE_SECONDS), ("applying", 0), ("done", 9999), ("failed", 9999), ("idle", 9999)] {
            let cursor = FakeCursor::new().with_fetchone_rows(vec![Some(classify_status_row(state, json!({"progress": 3}), age))]);
            let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

            let status = cm.get_classify_proposal_status("book", RELOAD_LOCK_HEARTBEAT_STALE_SECONDS).unwrap();

            assert_eq!(status["status"], state, "state={state}");
            assert_eq!(status["progress"], 3, "state={state}");
        }
    }
}

// ---- classify_proposal_items 전용 인메모리 대역 ----

/// classify_proposal_items에 대해 INSERT(executemany)/DELETE/SELECT를 흉내내는 인메모리 저장소.
/// 공용 FakeCursor는 고정된 rows만 돌려줘, "여러 번 나눠 넣어도 get이 전체를 순서대로 돌려준다"는
/// 시나리오(테일 플러시 검증 포함)를 표현할 수 없어서 별도로 둔다.
#[derive(Clone)]
struct ClassifyItemsFakeCursor {
    store: Arc<Mutex<Vec<Vec<DbValue>>>>,
    result: Arc<Mutex<Vec<Row>>>,
}

impl ClassifyItemsFakeCursor {
    fn new(store: &Arc<Mutex<Vec<Vec<DbValue>>>>) -> Self {
        Self { store: store.clone(), result: Arc::new(Mutex::new(Vec::new())) }
    }
}

impl DbCursor for ClassifyItemsFakeCursor {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError> {
        if sql.starts_with("DELETE FROM classify_proposal_items") {
            let content_type = &params[0];
            self.store.lock().unwrap().retain(|r| &r[0] != content_type);
        } else if sql.starts_with("SELECT payload, apply_status, apply_error FROM classify_proposal_items") {
            let content_type = &params[0];
            let rows = self
                .store
                .lock()
                .unwrap()
                .iter()
                .filter(|r| &r[0] == content_type)
                .map(|r| row([("payload", r[3].clone()), ("apply_status", "pending".into()), ("apply_error", DbValue::Null)]))
                .collect();
            *self.result.lock().unwrap() = rows;
        } else {
            panic!("unexpected SQL: {sql}");
        }
        Ok(())
    }

    fn executemany(&mut self, _sql: &str, seq: &[Vec<DbValue>]) -> Result<(), DbError> {
        self.store.lock().unwrap().extend(seq.iter().cloned());
        Ok(())
    }

    fn fetchall(&mut self) -> Option<Vec<Row>> {
        Some(self.result.lock().unwrap().clone())
    }

    fn fetchone(&mut self) -> Option<Row> {
        Some(row([("cnt", 0_i64.into())]))
    }

    fn rowcount(&self) -> u64 {
        0
    }
}

struct ClassifyItemsFakeConn {
    cursor: ClassifyItemsFakeCursor,
}

impl DbConnection for ClassifyItemsFakeConn {
    fn cursor(&mut self) -> Box<dyn DbCursor> {
        Box::new(self.cursor.clone())
    }

    fn commit(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    fn close(&mut self) {}
}

fn classify_items_provider(cursor: &ClassifyItemsFakeCursor) -> ConnectionProvider {
    let cursor = cursor.clone();
    Arc::new(move || Ok(Box::new(ClassifyItemsFakeConn { cursor: cursor.clone() }) as Box<dyn DbConnection>))
}

fn text_row(content_type: &str, category: &str, file_path: &str, payload: &str) -> Vec<DbValue> {
    vec![content_type.into(), category.into(), file_path.into(), payload.into()]
}

mod test_classify_proposal_items {
    use super::*;

    #[test]
    fn test_init_creates_classify_proposal_items_table() {
        let cursor = FakeCursor::new().with_fetchone_rows(rep(0, 14));
        let _cm = build_cm(&cursor);
        assert!(cursor.ran("CREATE TABLE IF NOT EXISTS classify_proposal_items"));
    }

    #[test]
    fn test_add_then_get_returns_items_in_insertion_order() {
        let cursor = FakeCursor::new();
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        let items = vec![json!({"file_path": "a.epub", "target_category": "3_SF"}), json!({"file_path": "b.epub", "target_category": "3_SF"})];
        cm.add_classify_proposal_items(&items, "0_inbox", "book").unwrap();

        assert!(conn.committed());
        let many = cursor.executed_many();
        assert_eq!(many.len(), 1);
        let (sql, rows) = &many[0];
        assert!(sql.contains("INSERT INTO classify_proposal_items"));
        // Python json.dumps(ensure_ascii=False)는 serde_json::to_string에 대응한다 (구현도 같은 직렬화를 쓴다고 본다).
        let expected = vec![text_row("book", "0_inbox", "a.epub", &serde_json::to_string(&items[0]).unwrap()), text_row("book", "0_inbox", "b.epub", &serde_json::to_string(&items[1]).unwrap())];
        assert_eq!(rows, &expected);

        let payload = |i: usize| DbValue::from(serde_json::to_string(&items[i]).unwrap().as_str());
        let cursor_get = FakeCursor::new().with_rows(vec![
            row([("payload", payload(0)), ("apply_status", "pending".into()), ("apply_error", DbValue::Null)]),
            row([("payload", payload(1)), ("apply_status", "pending".into()), ("apply_error", DbValue::Null)]),
        ]);
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor_get));
        let fetched = cm.get_classify_proposal_items("book").unwrap();
        let paths: Vec<&str> = fetched.iter().map(|i| i["file_path"].as_str().unwrap()).collect();
        assert_eq!(paths, vec!["a.epub", "b.epub"]);
        assert!(cursor_get.ran("ORDER BY id ASC"));
    }

    #[test]
    fn test_add_empty_items_is_noop() {
        let cursor = FakeCursor::new();
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        cm.add_classify_proposal_items(&[], "0_inbox", "book").unwrap();
        assert!(cursor.executed_many().is_empty());
        assert!(!conn.committed());
    }

    #[test]
    fn test_clear_scopes_delete_to_given_content_type() {
        let cursor = FakeCursor::new();
        let conn = FakeConn::new(&cursor);
        let cm = CategoryMapping::with_connection_provider(provider_of(&conn));
        cm.clear_classify_proposal_items("comic").unwrap();

        assert!(conn.committed());
        let (sql, params) = cursor.executed().last().cloned().unwrap();
        assert!(sql.contains("DELETE FROM classify_proposal_items WHERE content_type = %s"));
        assert_eq!(params, vec![DbValue::from("comic")]);
    }

    #[test]
    fn test_clear_only_removes_matching_content_type() {
        // 다른 content_type의 행은 clear 뒤에도 남아 있어야 한다.
        let store = Arc::new(Mutex::new(vec![text_row("book", "0_inbox", "a.epub", "{}"), text_row("comic", "0_inbox", "b.epub", "{}")]));
        let cursor = ClassifyItemsFakeCursor::new(&store);
        let cm = CategoryMapping::with_connection_provider(classify_items_provider(&cursor));
        cm.clear_classify_proposal_items("book").unwrap();

        let types: Vec<DbValue> = store.lock().unwrap().iter().map(|r| r[0].clone()).collect();
        assert_eq!(types, vec![DbValue::from("comic")]);
    }

    #[test]
    fn test_add_across_multiple_calls_then_get_returns_all_in_order_no_tail_dropped() {
        // 100건 단위로 나눠 add를 여러 번 불러도(100+100+50), get은 250건 전체를 순서대로 돌려준다.
        // 마지막 잔여분(꼬리)이 누락되면 관리자가 목록 끝의 책들을 못 보고 승인하게 되므로,
        // 이 테일 플러시를 DB 계층에서 직접 확인한다.
        let store = Arc::new(Mutex::new(Vec::new()));
        let cursor = ClassifyItemsFakeCursor::new(&store);
        let cm = CategoryMapping::with_connection_provider(classify_items_provider(&cursor));
        let batch = |range: std::ops::Range<usize>| -> Vec<Value> { range.map(|i| json!({"file_path": format!("{i}.epub")})).collect() };
        cm.add_classify_proposal_items(&batch(0..100), "0_inbox", "book").unwrap();
        cm.add_classify_proposal_items(&batch(100..200), "0_inbox", "book").unwrap();
        cm.add_classify_proposal_items(&batch(200..250), "0_inbox", "book").unwrap();

        let items = cm.get_classify_proposal_items("book").unwrap();
        assert_eq!(items.len(), 250);
        let paths: Vec<String> = items.iter().map(|i| i["file_path"].as_str().unwrap().to_string()).collect();
        let expected: Vec<String> = (0..250).map(|i| format!("{i}.epub")).collect();
        assert_eq!(paths, expected);
    }
}

mod test_classify_proposal_delete_and_merge {
    use super::*;

    fn make_cm(cursor: &FakeCursor) -> (CategoryMapping, Arc<Mutex<Vec<FakeConn>>>) {
        let (provider, conns) = recording_provider(cursor);
        let cm = CategoryMapping::with_connection_provider(provider);
        (cm, conns)
    }

    #[test]
    fn test_delete_classify_proposal_item_returns_deleted_count_and_commits() {
        let cursor = FakeCursor::new().with_rowcount(1);
        let (cm, conns) = make_cm(&cursor);

        assert_eq!(cm.delete_classify_proposal_item("/lib/a.epub", "comic").unwrap(), 1);

        let (sql, params) = cursor.executed().last().cloned().unwrap();
        assert!(sql.starts_with("DELETE FROM classify_proposal_items"));
        assert_eq!(params, vec![DbValue::from("comic"), DbValue::from("/lib/a.epub")]);
        assert!(conns.lock().unwrap().last().unwrap().committed());
    }

    #[test]
    fn test_delete_classify_proposal_item_missing_row_returns_zero() {
        let cursor = FakeCursor::new().with_rowcount(0);
        let (cm, _) = make_cm(&cursor);

        assert_eq!(cm.delete_classify_proposal_item("/lib/none.epub", "book").unwrap(), 0);
    }

    #[test]
    fn test_proposal_item_delete_methods_propagate_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("delete failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        assert!(cm.delete_classify_proposal_item("/lib/a.epub", "book").unwrap_err().to_string().contains("delete failed"));
        assert!(cm.delete_applied_classify_proposal_items("book").unwrap_err().to_string().contains("delete failed"));
    }

    #[test]
    fn test_clear_classify_proposal_items_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("clear failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.clear_classify_proposal_items("book").unwrap_err().to_string().contains("clear failed"));
    }

    #[test]
    fn test_set_classify_proposal_status_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("status write failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.set_classify_proposal_status(&json!({"status": "ready"}), "book").unwrap_err().to_string().contains("status write failed"));
    }

    #[test]
    fn test_merge_classify_proposal_status_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("status merge failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.merge_classify_proposal_status(&json!({"progress": 3}), "book").unwrap_err().to_string().contains("status merge failed"));
    }

    #[test]
    fn test_proposal_item_update_methods_propagate_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("item update failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));

        assert!(cm.update_classify_proposal_item_payloads(&[json!({"file_path": "a.epub"})], "book").unwrap_err().to_string().contains("item update failed"));
        assert!(cm.update_classify_proposal_item_status("a.epub", "moved", None, "book").unwrap_err().to_string().contains("item update failed"));
    }

    #[test]
    fn test_add_classify_proposal_items_propagates_database_errors_like_python() {
        let cursor = FakeCursor::new().with_side_effect(fail_with("item insert failed"));
        let cm = CategoryMapping::with_connection_provider(provider_for_cursor(&cursor));
        assert!(cm.add_classify_proposal_items(&[json!({"file_path": "a.epub"})], "inbox", "book").unwrap_err().to_string().contains("item insert failed"));
    }

    #[test]
    fn test_merge_classify_proposal_status_patches_payload_without_status() {
        let cursor = FakeCursor::new();
        let (cm, conns) = make_cm(&cursor);

        cm.merge_classify_proposal_status(&json!({"status": "ready", "progress": 3, "name": "한글"}), "book").unwrap();

        let (sql, params) = cursor.executed().last().cloned().unwrap();
        assert!(sql.contains("JSON_MERGE_PATCH"));
        let DbValue::Text(patch) = &params[0] else { panic!("첫 파라미터가 문자열이 아니다: {:?}", params[0]) };
        assert_eq!(serde_json::from_str::<Value>(patch).unwrap(), json!({"progress": 3, "name": "한글"}));
        assert_eq!(params[1], DbValue::from("book"));
        assert!(conns.lock().unwrap().last().unwrap().committed());
    }

    #[test]
    fn test_merge_classify_proposal_status_with_only_status_is_a_no_op() {
        let cursor = FakeCursor::new();
        let (cm, conns) = make_cm(&cursor);
        let before = cursor.executed().len();

        cm.merge_classify_proposal_status(&json!({"status": "ready"}), "book").unwrap();

        assert_eq!(cursor.executed().len(), before);
        assert!(conns.lock().unwrap().is_empty());
    }
}
