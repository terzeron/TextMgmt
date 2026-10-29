//! 원본: tests/test_refresh_token_store.py
//!
//! refresh token 저장소 MySQL 회전·재사용 탐지·세션 집계 테스트.
//! 빠른 동작 테스트는 메모리 MySQL 대역을 쓰고, 운영 SQL 검증은 Docker MySQL 통합 테스트로 확인한다.
//!
//! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.4, 3.9, 3.10):
//! - `backend::db`: `DbValue { Null, Int(i64), Text(String), DateTime(NaiveDateTime) }` (`From<&str>`, `From<i64>`, `Clone + PartialEq + Debug`),
//!   `type Row = BTreeMap<String, DbValue>`, `enum DbError { IntegrityError(String), Other(String) }`,
//!   `trait DbCursor { execute(&mut self, &str, &[DbValue]) -> Result<(), DbError>; executemany(..); fetchall(&mut self) -> Option<Vec<Row>>; fetchone(&mut self) -> Option<Row>; rowcount(&self) -> u64 }`,
//!   `trait DbConnection { cursor(&mut self) -> Box<dyn DbCursor>; commit(&mut self) -> Result<(), DbError>; rollback(&mut self) -> Result<(), DbError>; close(&mut self) }`,
//!   `type ConnectionProvider = Arc<dyn Fn() -> Result<Box<dyn DbConnection>, DbError> + Send + Sync>`
//! - `backend::refresh_token_store`
//!   - `trait RefreshTokenStoreApi`
//!     - `store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError>`
//!     - `rotate(&self, current_token_id: &str, new_token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<String, StoreError>`
//!     - `revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError>`
//!     - `get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError>`
//!     - `list_sessions(&self, q: &SessionQuery) -> Result<Value, StoreError>`, `revoke_session(&self, family_id: &str) -> Result<Value, StoreError>`
//!     - `_session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError>`
//!   - `struct SessionQuery { status: String, current_family_id: Option<String>, page: u32, page_size: u32, email: Option<String> }`
//!     (`Default`: status "active", page 1, page_size 50. Python 키워드 인자의 기본값에 대응한다)
//!   - `MySQLRefreshTokenStore::{new() -> Result<Self, StoreError> (TM_MYSQL_* 환경 변수), with_connector(ConnectionProvider) -> Result<Self, StoreError> (_init_db 실행),
//!     with_connection_provider(ConnectionProvider) -> Self (_init_db 생략, Python `__new__` + `_connect` 대입),
//!     _connect(&self) -> Result<Box<dyn DbConnection>, DbError>}` (`set_connection_provider`는 없다)
//!   - `enum AnyRefreshTokenStore { MySql(MySQLRefreshTokenStore) }`,
//!     `create_refresh_token_store() -> Result<AnyRefreshTokenStore, StoreError>` (TM_REFRESH_TOKEN_BACKEND),
//!     `create_refresh_token_store_with(ConnectionProvider) -> Result<AnyRefreshTokenStore, StoreError>`
//!   - `DEFAULT_ROTATION_GRACE_SECONDS: i64`, `_rotation_grace_seconds() -> i64`, `MAX_USER_AGENT_LENGTH: usize`
//!   - `summarize_user_agent(&str) -> String`, `_parse_browser_name_version_with(ua: &str, patterns: &[(regex::Regex, &str)]) -> Option<String>` (`_UA_BROWSER_PATTERNS` 대체)
//!   - `StoreError: Display`
//! - 로그 캡처는 `common::log_capture::LogCapture`(`log` crate 기반), 환경 변수는 `common::env::EnvGuard`, 실제 MySQL 컨테이너는 `common/mysql_fixture.rs`.
//! - dev-dependency: `tempfile`, `regex`, `testcontainers-modules`(mysql, blocking)

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{common, mysql_fixture};

use backend::db::{ConnectionProvider, DbConnection, DbCursor, DbError, DbValue, Row};
use backend::refresh_token_store::{
    _rotation_grace_seconds, AnyRefreshTokenStore, DEFAULT_ROTATION_GRACE_SECONDS, MAX_USER_AGENT_LENGTH, MySQLRefreshTokenStore, RefreshTokenStoreApi, Rotation, SessionQuery,
    create_refresh_token_store, create_refresh_token_store_with, summarize_user_agent,
};
use common::env::EnvGuard;
use common::log_capture::LogCapture;
use serde_json::{Value, json};

// ---- 시간 / 저장소 헬퍼 ----

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn fam(c: char) -> String {
    c.to_string().repeat(32)
}

/// 상태를 공유하지 않는 메모리 MySQL 저장소를 만든다.
fn memory_mysql_store() -> MySQLRefreshTokenStore {
    mem_store().0
}

fn mem_store() -> (MySQLRefreshTokenStore, MemDb) {
    let db = MemDb::default();
    (MySQLRefreshTokenStore::with_connector(db.provider()).unwrap(), db)
}

fn issue(s: &dyn RefreshTokenStoreApi, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64) {
    s.store_issued(token_id, family_id, email, issued_at, expires_at, None, None).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn issue_from(s: &dyn RefreshTokenStoreApi, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: &str, user_agent: &str) {
    s.store_issued(token_id, family_id, email, issued_at, expires_at, Some(client_ip), Some(user_agent)).unwrap();
}

fn rot(s: &dyn RefreshTokenStoreApi, current: &str, new: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64) -> &'static str {
    s.rotate(current, new, family_id, email, issued_at, expires_at, None, None).unwrap().as_str()
}

fn all() -> SessionQuery {
    SessionQuery { status: "all".to_string(), ..Default::default() }
}

fn items_of(page: &Value) -> Vec<Value> {
    page["items"].as_array().unwrap().clone()
}

// ---- 가짜 pymysql (Python _FakeCursor/_FakeConn/_BoomConn 대응) ----

struct ConnState {
    select_row: Option<Row>,
    existing_column_count: i64,
    executed: Vec<(String, Vec<DbValue>)>,
    committed: bool,
    rolled_back: bool,
    last_sql: String,
    boom: bool,
    fetchall_none: bool,
}

/// rotate 상태머신 검증용 pymysql 연결 대역. 복제본이 상태를 공유한다.
#[derive(Clone)]
struct FakeConn {
    st: Arc<Mutex<ConnState>>,
}

impl FakeConn {
    fn new(select_row: Option<Row>) -> Self {
        Self::with_columns(select_row, 1)
    }

    /// `existing_column_count` 1이면 client_ip/user_agent가 이미 있는 테이블(= ALTER 생략), 0이면 마이그레이션 대상.
    fn with_columns(select_row: Option<Row>, existing_column_count: i64) -> Self {
        Self {
            st: Arc::new(Mutex::new(ConnState {
                select_row,
                existing_column_count,
                executed: Vec::new(),
                committed: false,
                rolled_back: false,
                last_sql: String::new(),
                boom: false,
                fetchall_none: false,
            })),
        }
    }

    /// execute가 예외를 던지는 연결 (rollback/raise 경로 검증용).
    fn boom() -> Self {
        let conn = Self::new(None);
        conn.st.lock().unwrap().boom = true;
        conn
    }

    fn fetchall_none() -> Self {
        let conn = Self::new(None);
        conn.st.lock().unwrap().fetchall_none = true;
        conn
    }

    fn verbs(&self) -> Vec<String> {
        self.st.lock().unwrap().executed.iter().map(|(sql, _)| sql.split_whitespace().next().unwrap_or("").to_uppercase()).collect()
    }

    fn has_verb(&self, verb: &str) -> bool {
        self.verbs().iter().any(|v| v == verb)
    }

    fn committed(&self) -> bool {
        self.st.lock().unwrap().committed
    }

    fn rolled_back(&self) -> bool {
        self.st.lock().unwrap().rolled_back
    }

    fn last_params(&self) -> Vec<DbValue> {
        self.st.lock().unwrap().executed.last().map(|(_, p)| p.clone()).unwrap_or_default()
    }
}

struct FakeCursor {
    st: Arc<Mutex<ConnState>>,
}

impl DbCursor for FakeCursor {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError> {
        let mut st = self.st.lock().unwrap();
        if st.boom {
            return Err(DbError::Other("db down".to_string()));
        }
        st.last_sql = sql.to_string();
        st.executed.push((sql.to_string(), params.to_vec()));
        Ok(())
    }

    fn executemany(&mut self, _sql: &str, _seq: &[Vec<DbValue>]) -> Result<(), DbError> {
        Ok(())
    }

    fn fetchall(&mut self) -> Option<Vec<Row>> {
        let st = self.st.lock().unwrap();
        if st.fetchall_none {
            return None;
        }
        // 세션 집계(list_sessions)는 여러 row를 읽는다. select_row가 없으면 빈 결과.
        Some(st.select_row.iter().cloned().collect())
    }

    fn fetchone(&mut self) -> Option<Row> {
        let st = self.st.lock().unwrap();
        // 컬럼 존재 확인(information_schema)은 마이그레이션 전용 질의라 응답 모양이 다르다.
        if st.last_sql.contains("information_schema.columns") {
            return Some(row([("cnt", st.existing_column_count.into())]));
        }
        st.select_row.clone()
    }

    fn rowcount(&self) -> u64 {
        1
    }
}

impl DbConnection for FakeConn {
    fn cursor(&mut self) -> Box<dyn DbCursor> {
        Box::new(FakeCursor { st: self.st.clone() })
    }

    fn commit(&mut self) -> Result<(), DbError> {
        self.st.lock().unwrap().committed = true;
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), DbError> {
        self.st.lock().unwrap().rolled_back = true;
        Ok(())
    }

    fn close(&mut self) {}
}

fn row<const N: usize>(pairs: [(&str, DbValue); N]) -> Row {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn provider_of(conn: &FakeConn) -> ConnectionProvider {
    let conn = conn.clone();
    Arc::new(move || Ok(Box::new(conn.clone()) as Box<dyn DbConnection>))
}

/// pymysql.connect가 호출마다 새 FakeConn을 반환하도록 한다(_init_db와 rotate 분리).
fn install_fresh_conns(select_row: Option<Row>) -> (ConnectionProvider, Arc<Mutex<Vec<FakeConn>>>) {
    install_fresh_conns_with_columns(select_row, 1)
}

fn install_fresh_conns_with_columns(select_row: Option<Row>, existing_column_count: i64) -> (ConnectionProvider, Arc<Mutex<Vec<FakeConn>>>) {
    let conns = Arc::new(Mutex::new(Vec::new()));
    let sink = conns.clone();
    let provider: ConnectionProvider = Arc::new(move || {
        let conn = FakeConn::with_columns(select_row.clone(), existing_column_count);
        sink.lock().unwrap().push(conn.clone());
        Ok(Box::new(conn) as Box<dyn DbConnection>)
    });
    (provider, conns)
}

fn last_conn(conns: &Arc<Mutex<Vec<FakeConn>>>) -> FakeConn {
    conns.lock().unwrap().last().unwrap().clone()
}

/// _init_db를 건너뛰고, 연산마다 예외를 던지는 연결을 쓰는 스토어를 반환한다.
fn install_init_then_boom() -> (MySQLRefreshTokenStore, FakeConn) {
    // `set_connection_provider`는 없다. `_init_db`를 건너뛰는 `with_connection_provider`로 처음부터 예외 연결을 쥔다.
    let boom = FakeConn::boom();
    let store = MySQLRefreshTokenStore::with_connection_provider(provider_of(&boom));
    (store, boom)
}

fn mysql_row_for_rotate(family: &str, expires_at: i64, replaced_by: DbValue, revoked_at: DbValue, reason: Option<&str>) -> Row {
    let mut r = row([("family_id", family.into()), ("email", "e".into()), ("expires_at", expires_at.into()), ("replaced_by", replaced_by), ("revoked_at", revoked_at)]);
    if let Some(reason) = reason {
        r.insert("revoke_reason".to_string(), reason.into());
    }
    r
}

fn mysql_session_row(now: i64) -> Row {
    row([
        ("family_id", fam('a').as_str().into()),
        ("email", "admin@example.com".into()),
        ("created_at", (now - 100).into()),
        ("last_seen_at", (now - 10).into()),
        ("max_expires_at", (now + 2000).into()),
        ("token_count", 3_i64.into()),
        ("valid_token_count", 1_i64.into()),
        ("active_expires_at", (now + 2000).into()),
        ("terminal_reason", DbValue::Null),
        ("revoked_at", DbValue::Null),
        ("client_ip", "203.0.113.7".into()),
        ("user_agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".into()),
    ])
}

// ---- 메모리 MySQL 대역 (MySQL 저장소 동작을 재현하는 상태 있는 대역) ----

#[derive(Clone)]
struct Tok {
    jti: String,
    family_id: String,
    email: String,
    issued_at: i64,
    expires_at: i64,
    replaced_by: Option<String>,
    revoked_at: Option<i64>,
    revoke_reason: Option<String>,
    client_ip: Option<String>,
    user_agent: Option<String>,
}

/// refresh_tokens 테이블 한 개를 흉내 낸다. 저장소가 보내는 SQL 문장 종류를 앞부분으로 구분해 실행한다(트랜잭션은 즉시 반영).
#[derive(Clone, Default)]
struct MemDb {
    rows: Arc<Mutex<Vec<Tok>>>,
}

fn p_text(p: &[DbValue], i: usize) -> Option<String> {
    p.get(i).and_then(|v| v.as_str()).map(str::to_string)
}

fn p_int(p: &[DbValue], i: usize) -> i64 {
    p.get(i).and_then(|v| v.as_i64()).unwrap()
}

fn opt_text(v: &Option<String>) -> DbValue {
    v.as_deref().into()
}

impl MemDb {
    fn count(&self, jti: &str) -> usize {
        self.rows.lock().unwrap().iter().filter(|t| t.jti == jti).count()
    }

    fn provider(&self) -> ConnectionProvider {
        let db = self.clone();
        Arc::new(move || Ok(Box::new(MemConn { db: db.clone() }) as Box<dyn DbConnection>))
    }

    fn aggregate(&self, sql: &str, p: &[DbValue]) -> Vec<Row> {
        let now = p_int(p, 0);
        let mut next = 2;
        let mut email = None;
        let mut family = None;
        if sql.contains("email = %s") {
            email = p_text(p, next);
            next += 1;
        }
        if sql.contains("family_id = %s") {
            family = p_text(p, next);
        }
        let limit: usize = sql.rsplit("LIMIT").next().unwrap().trim().parse().unwrap();
        let rows = self.rows.lock().unwrap();
        let mut groups: Vec<((String, String), Vec<&Tok>)> = Vec::new();
        for t in rows.iter().filter(|t| email.as_ref().is_none_or(|e| *e == t.email) && family.as_ref().is_none_or(|f| *f == t.family_id)) {
            let key = (t.family_id.clone(), t.email.clone());
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(t),
                None => groups.push((key, vec![t])),
            }
        }
        groups.sort_by_key(|(_, v)| std::cmp::Reverse(v.iter().map(|t| t.issued_at).max().unwrap()));
        groups
            .into_iter()
            .take(limit)
            .map(|((family_id, email), v)| {
                let alive = |t: &&&Tok| t.revoked_at.is_none() && t.expires_at >= now;
                let latest = v.iter().max_by_key(|t| (t.issued_at, t.jti.clone())).unwrap();
                let terminal = v.iter().filter_map(|t| t.revoke_reason.clone()).filter(|r| r != "rotated").max();
                row([
                    ("family_id", family_id.as_str().into()),
                    ("email", email.as_str().into()),
                    ("created_at", v.iter().map(|t| t.issued_at).min().unwrap().into()),
                    ("last_seen_at", v.iter().map(|t| t.issued_at).max().unwrap().into()),
                    ("max_expires_at", v.iter().map(|t| t.expires_at).max().unwrap().into()),
                    ("token_count", (v.len() as i64).into()),
                    // MySQL 의 SUM() 은 DECIMAL 이라 문자열로 돌아온다.
                    ("valid_token_count", v.iter().filter(alive).count().to_string().into()),
                    ("active_expires_at", v.iter().filter(alive).map(|t| t.expires_at).max().unwrap_or(0).into()),
                    ("terminal_reason", terminal.into()),
                    ("revoked_at", v.iter().filter_map(|t| t.revoked_at).max().into()),
                    ("client_ip", opt_text(&latest.client_ip)),
                    ("user_agent", opt_text(&latest.user_agent)),
                ])
            })
            .collect()
    }
}

struct MemConn {
    db: MemDb,
}

struct MemCursor {
    db: MemDb,
    result: Option<Vec<Row>>,
}

impl DbCursor for MemCursor {
    fn execute(&mut self, sql: &str, p: &[DbValue]) -> Result<(), DbError> {
        let sql = sql.trim_start();
        self.result = None;
        if sql.starts_with("CREATE") || sql.starts_with("ALTER") {
            return Ok(());
        }
        if sql.contains("information_schema.columns") {
            self.result = Some(vec![row([("cnt", 1_i64.into())])]);
        } else if sql.contains("ROW_NUMBER") {
            self.result = Some(self.db.aggregate(sql, p));
        } else if sql.starts_with("SELECT family_id, email, expires_at") {
            let jti = p_text(p, 0).unwrap();
            let rows = self.db.rows.lock().unwrap();
            self.result = Some(
                rows.iter()
                    .filter(|t| t.jti == jti)
                    .map(|t| {
                        row([
                            ("family_id", t.family_id.as_str().into()),
                            ("email", t.email.as_str().into()),
                            ("expires_at", t.expires_at.into()),
                            ("replaced_by", opt_text(&t.replaced_by)),
                            ("revoked_at", t.revoked_at.into()),
                            ("revoke_reason", opt_text(&t.revoke_reason)),
                        ])
                    })
                    .collect(),
            );
        } else if sql.starts_with("SELECT replaced_by, revoked_at, revoke_reason") {
            let jti = p_text(p, 0).unwrap();
            let rows = self.db.rows.lock().unwrap();
            self.result = Some(
                rows.iter()
                    .filter(|t| t.jti == jti)
                    .map(|t| row([("replaced_by", opt_text(&t.replaced_by)), ("revoked_at", t.revoked_at.into()), ("revoke_reason", opt_text(&t.revoke_reason))]))
                    .collect(),
            );
        } else if sql.starts_with("INSERT INTO refresh_tokens") {
            let tok = Tok {
                jti: p_text(p, 0).unwrap(),
                family_id: p_text(p, 1).unwrap(),
                email: p_text(p, 2).unwrap(),
                issued_at: p_int(p, 3),
                expires_at: p_int(p, 4),
                replaced_by: None,
                revoked_at: None,
                revoke_reason: None,
                client_ip: p_text(p, 5),
                user_agent: p_text(p, 6),
            };
            let mut rows = self.db.rows.lock().unwrap();
            match rows.iter_mut().find(|t| t.jti == tok.jti) {
                Some(existing) if sql.contains("ON DUPLICATE KEY UPDATE") => *existing = tok,
                Some(_) => return Err(DbError::IntegrityError("중복 jti".to_string())),
                None => rows.push(tok),
            }
        } else if sql.starts_with("UPDATE refresh_tokens SET replaced_by") {
            let jti = p_text(p, 3).unwrap();
            for t in self.db.rows.lock().unwrap().iter_mut().filter(|t| t.jti == jti) {
                t.replaced_by = p_text(p, 0);
                t.revoked_at = Some(p_int(p, 1));
                t.revoke_reason = p_text(p, 2);
            }
        } else if sql.starts_with("UPDATE refresh_tokens") && sql.contains("COALESCE") {
            let family = p_text(p, 2).unwrap();
            for t in self.db.rows.lock().unwrap().iter_mut().filter(|t| t.family_id == family) {
                t.revoked_at = t.revoked_at.or(Some(p_int(p, 0)));
                if t.revoke_reason.as_deref().is_none_or(|r| r == "rotated") {
                    t.revoke_reason = p_text(p, 1);
                }
            }
        } else {
            return Err(DbError::Other(format!("MemDb 가 모르는 SQL: {sql}")));
        }
        Ok(())
    }

    fn executemany(&mut self, _sql: &str, _seq: &[Vec<DbValue>]) -> Result<(), DbError> {
        Ok(())
    }

    fn fetchall(&mut self) -> Option<Vec<Row>> {
        self.result.take()
    }

    fn fetchone(&mut self) -> Option<Row> {
        self.result.as_mut().and_then(|r| (!r.is_empty()).then(|| r.remove(0)))
    }

    fn rowcount(&self) -> u64 {
        self.result.as_ref().map_or(0, |r| r.len() as u64)
    }
}

impl DbConnection for MemConn {
    fn cursor(&mut self) -> Box<dyn DbCursor> {
        Box::new(MemCursor { db: self.db.clone(), result: None })
    }

    fn commit(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), DbError> {
        Ok(())
    }

    fn close(&mut self) {}
}

// ---- 회전 상태머신 (메모리 MySQL 대역) ----

#[test]
fn test_rotate_success() {
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now, now + 1000);

    let result = rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now + 1, now + 1001);

    assert_eq!(result, "ok");
}

#[test]
fn test_concurrent_reuse_within_grace_reissues_without_revoking_family() {
    // 방금 회전된 토큰이 grace window 내에 다시 제출되면(멀티탭 동시 refresh) 재사용
    // 공격이 아닌 정상 동시 요청으로 보고, 패밀리를 폐기하지 않고 후속 토큰을 재발급한다.
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now, now + 1000);
    assert_eq!(rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now + 1, now + 1001), "ok");

    // token-1 을 grace window 내에 한 번 더 제출 -> 폐기 대신 재발급("ok")
    assert_eq!(rot(&store, "token-1", "token-3", "family-1", "admin@example.com", now + 2, now + 1002), "ok");

    // 패밀리가 살아 있으므로 두 후속 토큰 모두 계속 사용 가능
    assert_eq!(rot(&store, "token-2", "token-4", "family-1", "admin@example.com", now + 3, now + 1003), "ok");
    assert_eq!(rot(&store, "token-3", "token-5", "family-1", "admin@example.com", now + 4, now + 1004), "ok");
}

#[test]
fn test_reuse_detection_revokes_family_after_grace() {
    // grace window 를 벗어난 뒤 회전 완료된 토큰이 재제출되면 실제 재사용 공격으로
    // 간주하고 패밀리 전체를 폐기한다.
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now - 1000, now + 1000);
    // 회전이 grace window 보다 한참 전에 발생(revoked_at = now-500)
    assert_eq!(rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now - 500, now + 1001), "ok");

    // 회전된 지 오래된 token-1 을 재제출 -> 진짜 재사용 -> 패밀리 폐기
    assert_eq!(rot(&store, "token-1", "token-3", "family-1", "admin@example.com", now, now + 1002), "reused");
    // 패밀리가 폐기되어 정상 후속 토큰 token-2 도 차단됨
    assert_eq!(rot(&store, "token-2", "token-4", "family-1", "admin@example.com", now + 1, now + 1003), "revoked");
}

#[test]
fn test_revoke_family_blocks_rotation() {
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now, now + 1000);

    store.revoke_family("family-1", "logout").unwrap();

    let result = rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now + 1, now + 1001);

    assert_eq!(result, "revoked");
}

#[test]
fn test_rotate_missing_token() {
    let store = memory_mysql_store();
    let now = now();
    let result = rot(&store, "missing-token", "token-2", "family-1", "admin@example.com", now, now + 1000);
    assert_eq!(result, "missing");
}

#[test]
fn test_rotate_rejects_family_or_email_mismatch() {
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now, now + 1000);

    let family_result = rot(&store, "token-1", "token-2", "other-family", "admin@example.com", now + 1, now + 1001);
    let email_result = rot(&store, "token-1", "token-3", "family-1", "other@example.com", now + 1, now + 1001);

    assert_eq!(family_result, "mismatch");
    assert_eq!(email_result, "mismatch");
}

#[test]
fn test_rotate_rejects_expired_token_without_creating_replacement() {
    let (store, db) = mem_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now - 100, now - 1);

    let result = rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now, now + 1000);

    assert_eq!(result, "expired");
    assert_eq!(db.count("token-2"), 0);
}

// --- N3: MySQL 공유 백엔드 (replica 간 일관성) ---

#[test]
fn test_factory_rejects_sqlite_and_unknown_backends() {
    let mut env = EnvGuard::new();
    for value in ["sqlite", "unknown"] {
        env.set("TM_REFRESH_TOKEN_BACKEND", value);
        let error = match create_refresh_token_store() {
            Err(error) => error,
            Ok(_) => panic!("backend={value}인데 저장소가 생성됐다"),
        };
        assert!(error.to_string().contains("TM_REFRESH_TOKEN_BACKEND=mysql"), "{error}");
    }
}

#[test]
fn test_mysql_rotation_observation_session_listing_and_revoke() {
    let (store, _) = mem_store();
    let issued_at = now();
    issue_from(&store, "old", &fam('s'), "reader@example.com", issued_at, issued_at + 3600, "203.0.113.8", "Mozilla/5.0 Chrome/120.0 Safari/537.36");

    assert_eq!(
        store.rotate("old", "new", &fam('s'), "reader@example.com", issued_at + 1, issued_at + 3601, Some("203.0.113.8"), Some("Mozilla/5.0 Chrome/120.0 Safari/537.36")).unwrap(),
        Rotation::Ok
    );
    let observation = store.get_token_observation("old").unwrap().unwrap();
    assert_eq!(observation["replaced_by_present"], true);
    assert_eq!(observation["revoke_reason"], "rotated");
    let page = store.list_sessions(&all()).unwrap();
    assert_eq!(page["summary"]["active"], 1);
    assert_eq!(page["items"][0]["token_count"], 2);
    assert_eq!(page["items"][0]["client_ip"], "203.0.113.8");

    let revoked = store.revoke_session(&fam('s')).unwrap();
    assert_eq!(revoked["found"], true);
    assert_eq!(revoked["revoked"], true);
    assert_eq!(revoked["session"]["status"], "revoked");
}

#[test]
fn test_mysql_rotation_reuse_after_grace_revokes_the_family() {
    let (store, _) = mem_store();
    let mut env = EnvGuard::new();
    env.set("TM_REFRESH_ROTATION_GRACE_SECONDS", "0");
    let issued_at = now();
    issue(&store, "old", &fam('r'), "reader@example.com", issued_at - 100, issued_at + 3600);

    assert_eq!(rot(&store, "old", "new", &fam('r'), "reader@example.com", issued_at - 100, issued_at + 3600), "ok");
    assert_eq!(rot(&store, "old", "replay", &fam('r'), "reader@example.com", issued_at, issued_at + 3600), "reused");
    let page = store.list_sessions(&all()).unwrap();
    assert_eq!(page["summary"]["revoked"], 1);
    assert_eq!(page["items"][0]["revoke_reason"], "reuse-detected");
}

#[test]
fn test_factory_rejects_sqlite_backend() {
    // Rust 백엔드는 SQLite 저장소를 지원하지 않는다. 명확한 오류를 돌려준다.
    let mut env = EnvGuard::new();
    for value in ["sqlite", "unknown"] {
        env.set("TM_REFRESH_TOKEN_BACKEND", value);
        let err = create_refresh_token_store_with(provider_of(&FakeConn::new(None))).err().unwrap();
        assert!(err.to_string().contains("TM_REFRESH_TOKEN_BACKEND=mysql"), "{err}");
    }
}

#[test]
fn test_factory_defaults_to_mysql() {
    let mut env = EnvGuard::new();
    env.remove("TM_REFRESH_TOKEN_BACKEND");
    let store = create_refresh_token_store_with(provider_of(&FakeConn::new(None))).unwrap();
    assert!(matches!(store, AnyRefreshTokenStore::MySql(_)));
}

#[test]
fn test_factory_selects_mysql_when_configured() {
    let mut env = EnvGuard::new();
    env.set("TM_REFRESH_TOKEN_BACKEND", "mysql");
    let store = create_refresh_token_store_with(provider_of(&FakeConn::new(None))).unwrap();
    assert!(matches!(store, AnyRefreshTokenStore::MySql(_)));
}

#[test]
fn test_mysql_rotate_ok_inserts_new_and_revokes_old() {
    let (provider, conns) = install_fresh_conns(Some(mysql_row_for_rotate("F", now() + 1000, DbValue::Null, DbValue::Null, None)));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns); // rotate 가 연 연결
    assert_eq!(result, "ok");
    assert!(rotated.committed() && !rotated.rolled_back());
    assert!(rotated.has_verb("INSERT") && rotated.has_verb("UPDATE"));
}

#[test]
fn test_mysql_rotate_detects_reuse_and_revokes_family() {
    // grace window 밖에서 회전된 토큰(revoked_at 이 오래 전, reason='rotated') 재사용
    let now = now();
    let (provider, conns) = install_fresh_conns(Some(mysql_row_for_rotate("F", now + 1000, "already-used".into(), (now - 1000).into(), Some("rotated"))));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns);
    assert_eq!(result, "reused");
    // 재사용 탐지 시 신규 토큰을 만들지 않고 family revoke(UPDATE)만 수행
    assert!(!rotated.has_verb("INSERT"));
    assert!(rotated.has_verb("UPDATE"));
    assert!(rotated.committed());
}

#[test]
fn test_mysql_rotate_within_grace_reissues_without_revoking_family() {
    // 방금(grace window 내) 회전된 토큰 재제출 -> 폐기 없이 후속 토큰 재발급
    let now = now();
    let (provider, conns) = install_fresh_conns(Some(mysql_row_for_rotate("F", now + 1000, "successor".into(), now.into(), Some("rotated"))));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns);
    assert_eq!(result, "ok");
    // 신규 토큰 INSERT 만 수행하고 family revoke(UPDATE) 는 하지 않음
    assert!(rotated.has_verb("INSERT"));
    assert!(!rotated.has_verb("UPDATE"));
    assert!(rotated.committed() && !rotated.rolled_back());
}

#[test]
fn test_mysql_rotate_missing_rolls_back() {
    let (provider, conns) = install_fresh_conns(None);
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns);
    assert_eq!(result, "missing");
    assert!(rotated.rolled_back() && !rotated.committed());
}

#[test]
fn test_rotation_grace_seconds_invalid_env_falls_back() {
    // TM_REFRESH_ROTATION_GRACE_SECONDS 가 정수가 아니면 기본값으로 폴백한다 (line 20-21).
    let mut env = EnvGuard::new();
    env.set("TM_REFRESH_ROTATION_GRACE_SECONDS", "not-an-int");
    assert_eq!(_rotation_grace_seconds(), DEFAULT_ROTATION_GRACE_SECONDS);
}

#[test]
fn test_mysql_store_issued_inserts_and_commits() {
    // MySQLRefreshTokenStore.store_issued 는 INSERT 후 commit 한다 (line 211-225).
    let (provider, conns) = install_fresh_conns(None);
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    issue(&store, "t", "F", "e", 1, 2);

    let issued = last_conn(&conns); // store_issued 가 연 연결
    assert!(issued.has_verb("INSERT"));
    assert!(issued.committed());
}

#[test]
fn test_mysql_rotate_mismatch_rolls_back() {
    // family/email 불일치 시 rollback 하고 mismatch 를 반환한다 (line 240-241).
    let (provider, conns) = install_fresh_conns(Some(mysql_row_for_rotate("OTHER", now() + 1000, DbValue::Null, DbValue::Null, None)));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns);
    assert_eq!(result, "mismatch");
    assert!(rotated.rolled_back() && !rotated.committed());
    assert!(!rotated.has_verb("INSERT"));
}

#[test]
fn test_mysql_rotate_expired_rolls_back() {
    // 만료된 토큰은 rollback 하고 expired 를 반환한다 (line 243-244).
    let (provider, conns) = install_fresh_conns(Some(mysql_row_for_rotate("F", now() - 1, DbValue::Null, DbValue::Null, None)));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns);
    assert_eq!(result, "expired");
    assert!(rotated.rolled_back() && !rotated.committed());
    assert!(!rotated.has_verb("INSERT"));
}

#[test]
fn test_mysql_rotate_revoked_token_revokes_family() {
    // 이미 폐기된(revoked_at 설정, replaced_by 없음) 토큰 재제출 시 family 를 폐기하고 revoked 를 반환한다 (line 262-264).
    let now = now();
    let (provider, conns) = install_fresh_conns(Some(mysql_row_for_rotate("F", now + 1000, DbValue::Null, (now - 10).into(), Some("manual"))));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = rot(&store, "old", "new", "F", "e", 1, 2);

    let rotated = last_conn(&conns);
    assert_eq!(result, "revoked");
    assert!(!rotated.has_verb("INSERT"));
    assert!(rotated.has_verb("UPDATE"));
    assert!(rotated.committed());
}

#[test]
fn test_mysql_rotate_exception_rolls_back_and_raises() {
    // rotate 중 예외가 나면 rollback 후 예외를 재전파한다 (line 277-279).
    let (store, boom) = install_init_then_boom();

    let err = store.rotate("old", "new", "F", "e", 1, 2, None, None).unwrap_err();
    assert!(err.to_string().contains("db down"), "{err}");

    assert!(boom.rolled_back() && !boom.committed());
}

#[test]
fn test_mysql_revoke_family_commits() {
    // MySQLRefreshTokenStore.revoke_family 는 UPDATE 후 commit 한다 (line 282-291).
    let (provider, conns) = install_fresh_conns(None);
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    store.revoke_family("F", "logout").unwrap();

    let revoked = last_conn(&conns);
    assert!(revoked.has_verb("UPDATE"));
    assert!(revoked.committed() && !revoked.rolled_back());
}

#[test]
fn test_mysql_revoke_family_exception_rolls_back_and_raises() {
    // revoke_family 중 예외가 나면 rollback 후 예외를 재전파한다 (line 289-291).
    let (store, boom) = install_init_then_boom();

    let err = store.revoke_family("F", "logout").unwrap_err();
    assert!(err.to_string().contains("db down"), "{err}");

    assert!(boom.rolled_back() && !boom.committed());
}

// ========== Phase 0: 관측 전용 읽기 조회 ==========

#[test]
fn test_get_token_observation_reports_rotation_state() {
    // 회전 전에는 replaced_by_present=False, 회전 후에는 True 를 보고한다.
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now, now + 1000);

    let before = store.get_token_observation("token-1").unwrap();
    assert_eq!(before, Some(json!({"replaced_by_present": false, "revoked_at": null, "revoke_reason": null})));

    assert_eq!(rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now + 1, now + 1001), "ok");

    let after = store.get_token_observation("token-1").unwrap();
    assert!(after.is_some());
    let after = after.unwrap();
    assert_eq!(after["replaced_by_present"], json!(true));
    assert_eq!(after["revoke_reason"], json!("rotated"));
}

#[test]
fn test_get_token_observation_returns_none_for_unknown_token() {
    let store = memory_mysql_store();
    assert!(store.get_token_observation("no-such-token").unwrap().is_none());
}

#[test]
fn test_get_token_observation_does_not_mutate_state() {
    // 관측 조회는 읽기 전용이므로 이후 회전 결과에 영향을 주지 않는다.
    let store = memory_mysql_store();
    let now = now();
    issue(&store, "token-1", "family-1", "admin@example.com", now, now + 1000);

    store.get_token_observation("token-1").unwrap();
    store.get_token_observation("token-1").unwrap();

    assert_eq!(rot(&store, "token-1", "token-2", "family-1", "admin@example.com", now + 1, now + 1001), "ok");
}

#[test]
fn test_mysql_get_token_observation_reads_row() {
    let (provider, conns) = install_fresh_conns(Some(row([("replaced_by", "successor".into()), ("revoked_at", 123_i64.into()), ("revoke_reason", "rotated".into())])));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = store.get_token_observation("old").unwrap();

    assert_eq!(result, Some(json!({"replaced_by_present": true, "revoked_at": 123, "revoke_reason": "rotated"})));
    // 읽기 전용이므로 커밋/롤백이 없어야 한다
    let obs = last_conn(&conns);
    assert_eq!(obs.verbs(), vec!["SELECT"]);
    assert!(!obs.committed() && !obs.rolled_back());
}

#[test]
fn test_mysql_get_token_observation_returns_none_for_unknown_token() {
    let (provider, _conns) = install_fresh_conns(None);
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    assert!(store.get_token_observation("missing").unwrap().is_none());
}

// ========== Admin 세션 관리: 집계/폐기 ==========

/// 활성/폐기/만료 family를 각각 하나씩 만든 메모리 MySQL store를 돌려준다.
fn session_fixture() -> (MySQLRefreshTokenStore, i64) {
    let store = memory_mysql_store();
    let now = now();
    // 활성 family: 두 번 회전해 토큰 row 3개
    issue(&store, "a1", &fam('a'), "admin@example.com", now - 100, now + 1000);
    rot(&store, "a1", "a2", &fam('a'), "admin@example.com", now - 50, now + 1500);
    rot(&store, "a2", "a3", &fam('a'), "admin@example.com", now - 10, now + 2000);
    // 로그아웃으로 폐기된 family
    issue(&store, "b1", &fam('b'), "viewer@example.com", now - 200, now + 1000);
    store.revoke_family(&fam('b'), "logout").unwrap();
    // 자연 만료된 family
    issue(&store, "c1", &fam('c'), "viewer@example.com", now - 9999, now - 10);
    (store, now)
}

#[test]
fn test_list_sessions_groups_by_family_and_derives_status() {
    let (store, now) = session_fixture();

    let page = store.list_sessions(&all()).unwrap();

    let by_email_status: BTreeSet<(String, String)> = items_of(&page).iter().map(|i| (i["email"].as_str().unwrap().to_string(), i["status"].as_str().unwrap().to_string())).collect();
    let expected: BTreeSet<(String, String)> =
        [("admin@example.com", "active"), ("viewer@example.com", "revoked"), ("viewer@example.com", "expired")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    assert_eq!(by_email_status, expected);
    assert_eq!(page["summary"], json!({"active": 1, "expired": 1, "revoked": 1, "total": 3}));

    let active = items_of(&page).into_iter().find(|i| i["status"] == "active").unwrap();
    // 회전 이력이 모두 한 family 로 묶이고, 살아 있는 토큰은 최신 하나뿐이다
    assert_eq!(active["token_count"], json!(3));
    assert_eq!(active["valid_token_count"], json!(1));
    assert_eq!(active["created_at"], json!(now - 100));
    assert_eq!(active["last_seen_at"], json!(now - 10));
    assert_eq!(active["expires_at"], json!(now + 2000));
    assert!(active["revoked_at"].is_null());
    assert!(active["revoke_reason"].is_null());
}

#[test]
fn test_list_sessions_reports_revoke_reason_only_for_revoked() {
    let (store, _) = session_fixture();

    let items = items_of(&store.list_sessions(&all()).unwrap());
    let by_status = |s: &str| items.iter().find(|i| i["status"] == s).unwrap().clone();

    assert_eq!(by_status("revoked")["revoke_reason"], json!("logout"));
    assert!(!by_status("revoked")["revoked_at"].is_null());
    // 자연 만료는 폐기 사유가 없다
    assert!(by_status("expired")["revoke_reason"].is_null());
}

#[test]
fn test_list_sessions_active_filter_and_default() {
    let (store, _) = session_fixture();

    let default_page = store.list_sessions(&SessionQuery::default()).unwrap();
    let statuses: Vec<Value> = items_of(&default_page).iter().map(|i| i["status"].clone()).collect();
    assert_eq!(statuses, vec![json!("active")]);
    // 요약은 필터와 무관하게 전체를 센다
    assert_eq!(default_page["summary"]["total"], json!(3));
    assert_eq!(default_page["pagination"]["totalItems"], json!(1));
}

#[test]
fn test_list_sessions_does_not_expose_token_internals() {
    let (store, _) = session_fixture();

    let item = items_of(&store.list_sessions(&all()).unwrap()).remove(0);

    assert!(item.get("jti").is_none());
    assert!(item.get("replaced_by").is_none());
    let keys: BTreeSet<String> = item.as_object().unwrap().keys().cloned().collect();
    let expected: BTreeSet<String> = [
        "session_id",
        "session_label",
        "email",
        "status",
        "created_at",
        "last_seen_at",
        "expires_at",
        "revoked_at",
        "revoke_reason",
        "token_count",
        "valid_token_count",
        "client_ip",
        "user_agent",
        "user_agent_summary",
        "is_current",
        "merged_family_ids",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(keys, expected);
    // 라벨은 family_id 앞부분만 보여준다
    let label = item["session_label"].as_str().unwrap();
    assert!(label.ends_with("..."));
    assert_eq!(label.chars().count(), 11);
}

fn chrome_ua(version: &str) -> String {
    format!("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version}.0.0.0 Safari/537.36")
}

#[test]
fn test_list_sessions_merges_adjacent_browser_version_upgrade() {
    // 같은 계정·IP·OS 에서 브라우저 버전만 1 올라간 두 family 는 한 행으로 합친다.
    let store = memory_mysql_store();
    let now = now();
    issue_from(&store, "old1", &fam('d'), "admin@example.com", now - 500, now + 500, "1.2.3.4", &chrome_ua("130"));
    issue_from(&store, "new1", &fam('e'), "admin@example.com", now - 10, now + 1000, "1.2.3.4", &chrome_ua("131"));

    let items = items_of(&store.list_sessions(&all()).unwrap());

    assert_eq!(items.len(), 1);
    let merged = &items[0];
    assert_eq!(merged["session_id"], json!(fam('e')));
    let merged_ids: HashSet<String> = merged["merged_family_ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert_eq!(merged_ids, HashSet::from([fam('d'), fam('e')]));
    assert_eq!(merged["status"], json!("active"));
    assert_eq!(merged["created_at"], json!(now - 500));
    assert_eq!(merged["last_seen_at"], json!(now - 10));
    assert_eq!(merged["token_count"], json!(2));
    assert_eq!(merged["valid_token_count"], json!(2));
    assert_eq!(merged["user_agent"], json!(chrome_ua("131")));
}

#[test]
fn test_list_sessions_does_not_merge_across_different_ip() {
    let store = memory_mysql_store();
    let now = now();
    issue_from(&store, "f1", &fam('f'), "admin@example.com", now - 500, now + 500, "1.2.3.4", &chrome_ua("130"));
    issue_from(&store, "g1", &fam('0'), "admin@example.com", now - 10, now + 1000, "9.9.9.9", &chrome_ua("131"));

    let items = items_of(&store.list_sessions(&all()).unwrap());

    assert_eq!(items.len(), 2);
}

#[test]
fn test_list_sessions_does_not_merge_when_version_gap_exceeds_tolerance() {
    let store = memory_mysql_store();
    let now = now();
    issue_from(&store, "h1", &fam('1'), "admin@example.com", now - 500, now + 500, "1.2.3.4", &chrome_ua("125"));
    issue_from(&store, "i1", &fam('2'), "admin@example.com", now - 10, now + 1000, "1.2.3.4", &chrome_ua("131"));

    let items = items_of(&store.list_sessions(&all()).unwrap());

    assert_eq!(items.len(), 2);
}

#[test]
fn test_list_sessions_never_merges_revoked_sessions() {
    // 관리자가 명시적으로 폐기한 세션은 최신 활성 세션과 절대 합치지 않는다.
    let store = memory_mysql_store();
    let now = now();
    issue_from(&store, "j1", &fam('3'), "admin@example.com", now - 500, now + 500, "1.2.3.4", &chrome_ua("130"));
    store.revoke_family(&fam('3'), "admin-revoked").unwrap();
    issue_from(&store, "k1", &fam('4'), "admin@example.com", now - 10, now + 1000, "1.2.3.4", &chrome_ua("131"));

    let items = items_of(&store.list_sessions(&all()).unwrap());

    assert_eq!(items.len(), 2);
    let statuses: BTreeSet<String> = items.iter().map(|i| i["status"].as_str().unwrap().to_string()).collect();
    assert_eq!(statuses, BTreeSet::from(["revoked".to_string(), "active".to_string()]));
}

#[test]
fn test_list_sessions_marks_current_session() {
    let (store, _) = session_fixture();

    let page = store.list_sessions(&SessionQuery { current_family_id: Some(fam('b')), ..all() }).unwrap();

    let current: Vec<Value> = items_of(&page).into_iter().filter(|i| i["is_current"] == true).collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["session_id"], json!(fam('b')));
}

#[test]
fn test_list_sessions_paginates_newest_activity_first() {
    let (store, _) = session_fixture();

    let first = store.list_sessions(&SessionQuery { page: 1, page_size: 2, ..all() }).unwrap();
    let second = store.list_sessions(&SessionQuery { page: 2, page_size: 2, ..all() }).unwrap();

    assert_eq!(first["pagination"], json!({"page": 1, "pageSize": 2, "totalItems": 3, "totalPages": 2}));
    assert_eq!(items_of(&first).len(), 2);
    assert_eq!(items_of(&second).len(), 1);
    // last_seen_at 내림차순
    let both: Vec<Value> = items_of(&first).into_iter().chain(items_of(&second)).collect();
    let seen: Vec<i64> = both.iter().map(|i| i["last_seen_at"].as_i64().unwrap()).collect();
    let mut sorted = seen.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, sorted);
    // 페이지 간 중복 없음
    let ids = |page: &Value| -> HashSet<String> { items_of(page).iter().map(|i| i["session_id"].as_str().unwrap().to_string()).collect() };
    assert!(ids(&first).is_disjoint(&ids(&second)));
}

#[test]
fn test_list_sessions_page_beyond_end_is_empty() {
    let (store, _) = session_fixture();
    let page = store.list_sessions(&SessionQuery { page: 99, page_size: 50, ..all() }).unwrap();
    assert!(items_of(&page).is_empty());
}

#[test]
fn test_list_sessions_filters_by_email() {
    let (store, _) = session_fixture();

    let page = store.list_sessions(&SessionQuery { email: Some("viewer@example.com".to_string()), ..all() }).unwrap();

    let emails: BTreeSet<String> = items_of(&page).iter().map(|i| i["email"].as_str().unwrap().to_string()).collect();
    assert_eq!(emails, BTreeSet::from(["viewer@example.com".to_string()]));
    assert_eq!(page["summary"]["total"], json!(2));
}

#[test]
fn test_list_sessions_empty_store() {
    let store = memory_mysql_store();

    let page = store.list_sessions(&all()).unwrap();

    assert!(items_of(&page).is_empty());
    assert_eq!(page["summary"], json!({"active": 0, "expired": 0, "revoked": 0, "total": 0}));
    assert_eq!(page["pagination"]["totalPages"], json!(1));
}

#[test]
fn test_revoke_session_revokes_whole_family() {
    let (store, _) = session_fixture();

    let result = store.revoke_session(&fam('a')).unwrap();

    assert_eq!(result["found"], json!(true));
    assert_eq!(result["revoked"], json!(true));
    assert_eq!(result["session"]["status"], json!("revoked"));
    assert_eq!(result["session"]["revoke_reason"], json!("admin-revoked"));
    // 폐기 후에는 회전이 막힌다
    let now = now();
    assert_eq!(rot(&store, "a3", "a4", &fam('a'), "admin@example.com", now, now + 1000), "revoked");
}

#[test]
fn test_revoke_session_is_idempotent_for_inactive_family() {
    let (store, _) = session_fixture();
    store.revoke_session(&fam('a')).unwrap();

    let again = store.revoke_session(&fam('a')).unwrap();

    // 이미 활성 토큰이 없으므로 revoked=False 지만 성공이고 현재 상태를 돌려준다
    assert_eq!(again["found"], json!(true));
    assert_eq!(again["revoked"], json!(false));
    assert_eq!(again["session"]["status"], json!("revoked"));
}

#[test]
fn test_revoke_session_keeps_existing_terminal_reason() {
    let (store, _) = session_fixture();

    let result = store.revoke_session(&fam('b')).unwrap();

    assert_eq!(result["found"], json!(true));
    assert_eq!(result["revoked"], json!(false));
    // logout 으로 이미 폐기된 family 의 사유를 admin-revoked 로 덮어쓰지 않는다
    assert_eq!(result["session"]["revoke_reason"], json!("logout"));
}

#[test]
fn test_revoke_session_unknown_family() {
    let (store, _) = session_fixture();

    let result = store.revoke_session(&fam('d')).unwrap();

    assert_eq!(result, json!({"found": false, "revoked": false, "session": null}));
}

#[test]
fn test_revoke_session_does_not_touch_other_families() {
    let (store, _) = session_fixture();

    store.revoke_session(&fam('a')).unwrap();

    let page = store.list_sessions(&all()).unwrap();
    let by_id = |id: &str| items_of(&page).into_iter().find(|i| i["session_id"] == id).unwrap();
    assert_eq!(by_id(&fam('b'))["revoke_reason"], json!("logout"));
    assert_eq!(by_id(&fam('c'))["status"], json!("expired"));
}

#[test]
fn test_mysql_list_sessions_uses_same_derivation() {
    // MySQL store 가 aggregate 결과를 안정된 형태로 반환한다.
    let now = now();
    let (provider, _conns) = install_fresh_conns(Some(mysql_session_row(now)));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let page = store.list_sessions(&SessionQuery { current_family_id: Some(fam('a')), ..all() }).unwrap();

    assert_eq!(page["summary"], json!({"active": 1, "expired": 0, "revoked": 0, "total": 1}));
    let item = &items_of(&page)[0];
    assert_eq!(item["status"], json!("active"));
    assert_eq!(item["is_current"], json!(true));
    assert_eq!(item["token_count"], json!(3));
    assert_eq!(item["session_label"], json!("aaaaaaaa..."));
}

#[test]
fn test_mysql_revoke_session_revokes_and_reports_status() {
    let now = now();
    let (provider, conns) = install_fresh_conns(Some(mysql_session_row(now)));
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let result = store.revoke_session(&fam('a')).unwrap();

    assert_eq!(result["found"], json!(true));
    assert_eq!(result["revoked"], json!(true));
    // family revoke UPDATE 가 커밋된 연결이 있어야 한다
    assert!(conns.lock().unwrap().iter().any(|c| c.has_verb("UPDATE") && c.committed()));
}

#[test]
fn test_mysql_revoke_session_unknown_family() {
    let (provider, _conns) = install_fresh_conns(None);
    let store = MySQLRefreshTokenStore::with_connector(provider).unwrap();

    assert_eq!(store.revoke_session(&fam('d')).unwrap(), json!({"found": false, "revoked": false, "session": null}));
}

// ========== 접속 IP / User-Agent 수집 ==========

const CHROME_MAC_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const SAFARI_IOS_UA: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

#[test]
fn test_list_sessions_reports_latest_client_info() {
    // 세션의 IP/UA 는 family 안에서 가장 최근 발급된 토큰 값을 보여준다.
    let store = memory_mysql_store();
    let now = now();
    issue_from(&store, "t1", &fam('a'), "admin@example.com", now - 100, now + 1000, "203.0.113.7", CHROME_MAC_UA);
    // 같은 세션이 다른 회선에서 갱신되면 최신 값으로 덮인다
    store.rotate("t1", "t2", &fam('a'), "admin@example.com", now - 10, now + 2000, Some("198.51.100.42"), Some(SAFARI_IOS_UA)).unwrap();

    let item = items_of(&store.list_sessions(&all()).unwrap()).remove(0);

    assert_eq!(item["client_ip"], json!("198.51.100.42"));
    assert_eq!(item["user_agent"], json!(SAFARI_IOS_UA));
    assert_eq!(item["user_agent_summary"], json!("Safari 17 / iOS 17"));
}

#[test]
fn test_list_sessions_returns_empty_client_info_for_legacy_rows() {
    // 컬럼 추가 이전에 쌓인 세션은 빈 문자열로 나온다(None 이 화면까지 새지 않는다).
    let (store, _) = session_fixture();

    for item in items_of(&store.list_sessions(&all()).unwrap()) {
        assert_eq!(item["client_ip"], json!(""));
        assert_eq!(item["user_agent"], json!(""));
        assert_eq!(item["user_agent_summary"], json!(""));
    }
}

#[test]
fn test_store_truncates_oversized_client_info() {
    // UA 는 상한이 없으므로 컬럼 폭에 맞게 잘라 저장한다.
    let store = memory_mysql_store();
    let now = now();
    issue_from(&store, "t1", &fam('a'), "admin@example.com", now, now + 1000, "203.0.113.7", &"U".repeat(MAX_USER_AGENT_LENGTH * 2));

    let item = items_of(&store.list_sessions(&all()).unwrap()).remove(0);

    assert_eq!(item["user_agent"].as_str().unwrap().chars().count(), MAX_USER_AGENT_LENGTH);
}

#[test]
fn test_mysql_migration_adds_client_columns_when_missing() {
    // MySQL 도 기존 테이블에 컬럼이 없으면 ALTER 를 실행한다.
    let (provider, conns) = install_fresh_conns_with_columns(None, 0);
    MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let first = conns.lock().unwrap()[0].clone();
    assert_eq!(first.verbs().iter().filter(|v| *v == "ALTER").count(), 2);
}

#[test]
fn test_mysql_migration_skips_alter_when_columns_exist() {
    let (provider, conns) = install_fresh_conns(None);
    MySQLRefreshTokenStore::with_connector(provider).unwrap();

    let first = conns.lock().unwrap()[0].clone();
    assert!(!first.has_verb("ALTER"));
}

#[test]
fn test_summarize_user_agent() {
    // pytest.mark.parametrize 8건을 한 함수에서 돈다.
    let cases: Vec<(&str, &str)> = vec![
        (CHROME_MAC_UA, "Chrome 131 / macOS 10.15"),
        (SAFARI_IOS_UA, "Safari 17 / iOS 17"),
        ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0", "Edge 131 / Windows 10+"),
        ("Mozilla/5.0 (X11; Linux x86_64; rv:133.0) Gecko/20100101 Firefox/133.0", "Firefox 133 / Linux"),
        ("Mozilla/5.0 (Linux; Android 14; SM-S918N) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/23.0 Chrome/115.0.0.0 Mobile Safari/537.36", "Samsung Internet 23 / Android 14"),
        // 파생 브라우저는 Chrome 토큰을 함께 달고 다니므로 파생 쪽이 이겨야 한다
        ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Whale/3.28.266.14 Safari/537.36", "Whale 3 / Windows 10+"),
        // 브라우저를 못 알아보면 원문 앞부분으로 봇/스크립트를 식별한다
        ("curl/8.5.0", "curl/8.5.0"),
        ("", ""),
    ];
    for (user_agent, expected) in cases {
        assert_eq!(summarize_user_agent(user_agent), expected, "user_agent={user_agent:?}");
    }
}

// ========== 실제 MySQL 대상 검증 (집계 SQL / 마이그레이션) ==========

/// 실제 MySQL 컨테이너에 붙은 스토어. 테스트마다 테이블을 비운다. 테이블을 공유하므로 락으로 직렬화한다.
fn mysql_store() -> (MutexGuard<'static, ()>, MySQLRefreshTokenStore) {
    mysql_fixture::mysql_container();
    let guard = mysql_fixture::db_lock();
    let store = MySQLRefreshTokenStore::new().unwrap();
    let mut conn = store._connect().unwrap();
    conn.cursor().execute("TRUNCATE TABLE refresh_tokens", &[]).unwrap();
    conn.commit().unwrap();
    (guard, store)
}

#[test]
fn test_mysql_aggregate_sql_runs_and_reports_latest_client_info() {
    // 윈도우 함수 기반 집계가 MySQL 8 에서 실제로 실행되고 최신 IP/UA 를 돌려준다.
    let (_guard, store) = mysql_store();
    let now = now();
    issue_from(&store, "t1", &fam('a'), "admin@example.com", now - 100, now + 1000, "203.0.113.7", CHROME_MAC_UA);
    assert_eq!(store.rotate("t1", "t2", &fam('a'), "admin@example.com", now - 10, now + 2000, Some("198.51.100.42"), Some(SAFARI_IOS_UA)).unwrap(), Rotation::Ok);

    let page = store.list_sessions(&all()).unwrap();

    assert_eq!(items_of(&page).len(), 1);
    let item = &items_of(&page)[0];
    assert_eq!(item["status"], json!("active"));
    assert_eq!(item["token_count"], json!(2));
    assert_eq!(item["client_ip"], json!("198.51.100.42"));
    assert_eq!(item["user_agent"], json!(SAFARI_IOS_UA));
    assert_eq!(item["user_agent_summary"], json!("Safari 17 / iOS 17"));
}

#[test]
fn test_mysql_aggregate_sql_filters_by_email() {
    // {where} 가 서브쿼리로 들어가도 파라미터 순서(now, now, email)가 맞는지 본다.
    let (_guard, store) = mysql_store();
    let now = now();
    issue_from(&store, "t1", &fam('a'), "admin@example.com", now, now + 1000, "203.0.113.7", CHROME_MAC_UA);
    issue_from(&store, "t2", &fam('b'), "viewer@example.com", now, now + 1000, "198.51.100.42", SAFARI_IOS_UA);

    let page = store.list_sessions(&SessionQuery { email: Some("viewer@example.com".to_string()), ..all() }).unwrap();

    let emails: Vec<Value> = items_of(&page).iter().map(|i| i["email"].clone()).collect();
    assert_eq!(emails, vec![json!("viewer@example.com")]);
    assert_eq!(items_of(&page)[0]["client_ip"], json!("198.51.100.42"));
}

#[test]
fn test_mysql_migration_adds_columns_to_legacy_table() {
    // 컬럼 없는 기존 테이블을 열어도 ALTER 로 보정되고 기존 row 는 NULL 로 남는다.
    mysql_fixture::mysql_container();
    let _guard = mysql_fixture::db_lock();
    let bootstrap = MySQLRefreshTokenStore::new().unwrap();
    {
        let mut conn = bootstrap._connect().unwrap();
        let mut cur = conn.cursor();
        cur.execute("DROP TABLE IF EXISTS refresh_tokens", &[]).unwrap();
        cur.execute(
            "CREATE TABLE refresh_tokens (jti VARCHAR(64) PRIMARY KEY, family_id VARCHAR(64) NOT NULL, email VARCHAR(320) NOT NULL, issued_at BIGINT NOT NULL, expires_at BIGINT NOT NULL, replaced_by VARCHAR(64), revoked_at BIGINT, revoke_reason VARCHAR(64), INDEX idx_refresh_tokens_family (family_id)) ENGINE=InnoDB",
            &[],
        )
        .unwrap();
        let now = now();
        cur.execute("INSERT INTO refresh_tokens VALUES ('t1', %s, 'admin@example.com', %s, %s, NULL, NULL, NULL)", &[fam('a').as_str().into(), now.into(), (now + 1000).into()]).unwrap();
        drop(cur);
        conn.commit().unwrap();
    }

    let store = MySQLRefreshTokenStore::new().unwrap(); // _init_db 가 마이그레이션을 수행한다

    let item = items_of(&store.list_sessions(&all()).unwrap()).remove(0);
    assert_eq!(item["client_ip"], json!(""));
    assert_eq!(item["user_agent"], json!(""));
}

// --- UA 요약 / 세션 목록 상한 ---

#[test]
fn test_summarize_user_agent_with_only_browser_or_only_os() {
    // pytest.mark.parametrize 2건을 한 함수에서 돈다.
    let cases = [("Mozilla/5.0 Chrome/120.0 Safari/537.36", "Chrome 120"), ("curl (iPhone OS 17_2 like)", "iOS 17")];
    for (user_agent, expected) in cases {
        assert_eq!(summarize_user_agent(user_agent), expected, "user_agent={user_agent:?}");
    }
}

#[test]
fn test_parse_browser_name_version_returns_none_when_pattern_has_no_version() {
    let patterns = [(regex::Regex::new("Foo").unwrap(), "Foo {0}")];
    assert!(backend::refresh_token_store::_parse_browser_name_version_with("Foo/1", &patterns).is_none());
}

#[test]
fn test_mysql_session_rows_warns_and_limits_in_memory_store() {
    let (store, _) = session_fixture();
    let store = store.with_max_session_groups(1);
    let logs = LogCapture::start(log::Level::Warn);
    let rows = store._session_rows(None, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert!(logs.messages().iter().any(|m| m.contains("상한")), "{:?}", logs.messages());
}

#[test]
fn test_mysql_session_rows_warns_when_limit_is_reached() {
    // MagicMock 커서 대신 FakeConn을 쓴다. fetchall은 [{"family_id": "F"}]를 돌려준다.
    let conn = FakeConn::new(Some(row([("family_id", "F".into())])));
    let store = MySQLRefreshTokenStore::with_connection_provider(provider_of(&conn)).with_max_session_groups(1);
    let logs = LogCapture::start(log::Level::Warn);
    let rows = store._session_rows(Some("e"), Some("F")).unwrap();
    assert_eq!(rows, vec![row([("family_id", "F".into())])]);
    assert!(logs.messages().iter().any(|m| m.contains("상한")), "{:?}", logs.messages());
    // now 두 번 + email + family_id
    assert_eq!(conn.last_params().len(), 4);
}

#[test]
fn test_mysql_session_rows_treats_none_result_as_empty() {
    let conn = FakeConn::fetchall_none();
    let store = MySQLRefreshTokenStore::with_connection_provider(provider_of(&conn));
    assert!(store._session_rows(None, None).unwrap().is_empty());
}
