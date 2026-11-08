//! MySQL 접근 추상화. Diesel 위에서 `?` 자리표시자, 열 이름 맵 행, 수동 커밋, 제약 위반 오류를 제공한다.
//! 서비스 코드는 `DbConnection`/`ConnectionProvider`만 알고, 테스트는 가짜 연결을 주입한다. 설계: docs/rust-test-seams.md 3.4
//!
//! 시간대: Diesel은 연결을 열 때 `time_zone`을 `+00:00`으로 바꾸므로, `connect`가 서버 전역 설정으로 되돌린다.

pub mod bookstore_cache_store;
pub mod category_mapping;
pub mod view_history_store;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use common::url_parser::quote_component;
use diesel::connection::SimpleConnection;
use diesel::deserialize::{self, FromSql, QueryableByName};
use diesel::mysql::{Mysql, MysqlConnection, MysqlType};
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel::row::{Field, NamedRow};
use diesel::sql_types::{BigInt, Date, Datetime, Double, Nullable, Text, Time, Timestamp};
use diesel::{Connection, RunQueryDsl, sql_query};

// ---------------------------------------------------------------------------
// 값과 오류
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Int(i64),
    Text(String),
    DateTime(NaiveDateTime),
}

/// JSON 응답용 변환. 날짜는 `NaiveDateTime`의 `Display` 문자열이다.
impl From<&DbValue> for serde_json::Value {
    fn from(value: &DbValue) -> Self {
        match value {
            DbValue::Null => Self::Null,
            DbValue::Int(n) => Self::from(*n),
            DbValue::Text(s) => Self::from(s.as_str()),
            DbValue::DateTime(d) => Self::from(d.to_string()),
        }
    }
}

impl DbValue {
    pub fn is_null(&self) -> bool {
        matches!(self, DbValue::Null)
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            DbValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            DbValue::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl From<&str> for DbValue {
    fn from(v: &str) -> Self {
        DbValue::Text(v.to_string())
    }
}

impl From<String> for DbValue {
    fn from(v: String) -> Self {
        DbValue::Text(v)
    }
}

impl From<i64> for DbValue {
    fn from(v: i64) -> Self {
        DbValue::Int(v)
    }
}

impl From<i32> for DbValue {
    fn from(v: i32) -> Self {
        DbValue::Int(v.into())
    }
}

impl From<NaiveDateTime> for DbValue {
    fn from(v: NaiveDateTime) -> Self {
        DbValue::DateTime(v)
    }
}

impl<T: Into<DbValue>> From<Option<T>> for DbValue {
    fn from(v: Option<T>) -> Self {
        v.map_or(DbValue::Null, Into::into)
    }
}

/// JSON 값을 옮긴다. 정수·문자열·null은 그대로, 불리언은 0/1, 나머지는 JSON 문자열이다.
impl From<serde_json::Value> for DbValue {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => DbValue::Null,
            serde_json::Value::Bool(b) => DbValue::Int(b.into()),
            serde_json::Value::Number(n) => n.as_i64().map(DbValue::Int).unwrap_or_else(|| DbValue::Text(n.to_string())),
            serde_json::Value::String(s) => DbValue::Text(s),
            other => DbValue::Text(other.to_string()),
        }
    }
}

/// 열 구조를 미리 정하지 않은 SELECT 결과 한 행: 열 이름 → 값.
pub type Row = BTreeMap<String, DbValue>;

#[derive(Debug, Clone, PartialEq)]
pub enum DbError {
    /// 제약 위반: 키 중복, 외래 키, NOT NULL 위반.
    Constraint(String),
    Other(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Constraint(msg) => write!(f, "무결성 오류: {msg}"),
            DbError::Other(msg) => write!(f, "DB 오류: {msg}"),
        }
    }
}

impl std::error::Error for DbError {}

// ---------------------------------------------------------------------------
// 추상화
// ---------------------------------------------------------------------------

/// 연결 하나. SQL 자리표시자는 `?`다.
pub trait DbConnection: Send {
    /// 문장을 실행하고 영향받은 행 수를 돌려준다. SELECT면 읽은 행 수다.
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<u64, DbError>;
    /// SELECT를 실행하고 모든 행을 돌려준다.
    fn fetch_all(&mut self, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, DbError>;
    fn commit(&mut self) -> Result<(), DbError>;
    fn rollback(&mut self) -> Result<(), DbError>;
    fn close(&mut self);

    /// SELECT를 실행하고 첫 행을 돌려준다. 행이 없으면 `None`이다.
    fn fetch_one(&mut self, sql: &str, params: &[DbValue]) -> Result<Option<Row>, DbError> {
        Ok(self.fetch_all(sql, params)?.into_iter().next())
    }

    /// 같은 문장을 인자 묶음마다 실행하고 영향받은 행 수의 합을 돌려준다.
    fn execute_batch(&mut self, sql: &str, rows: &[Vec<DbValue>]) -> Result<u64, DbError> {
        rows.iter().try_fold(0, |total, params| Ok(total + self.execute(sql, params)?))
    }
}

/// 호출할 때마다 새 연결을 연다.
pub type ConnectionProvider = Arc<dyn Fn() -> Result<Box<dyn DbConnection>, DbError> + Send + Sync>;

// ---------------------------------------------------------------------------
// 접속 설정
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub struct MysqlConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
}

impl MysqlConfig {
    /// `TM_MYSQL_*` 환경 변수에서 읽는다. 사용자 기본값은 모듈마다 달라서(`tmuser`, `textmanager`) 인자로 받는다.
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>, default_user: &str) -> Self {
        Self {
            host: env("TM_MYSQL_HOST").unwrap_or_else(|| "localhost".into()),
            port: env("TM_MYSQL_PORT").and_then(|p| p.parse().ok()).unwrap_or(3306),
            database: env("TM_MYSQL_DATABASE").unwrap_or_else(|| "textmanager".into()),
            user: env("TM_MYSQL_USER").unwrap_or_else(|| default_user.into()),
            password: env("TM_MYSQL_PASSWORD").unwrap_or_default(),
        }
    }
}

impl fmt::Debug for MysqlConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MysqlConfig").field("host", &self.host).field("port", &self.port).field("database", &self.database).field("user", &self.user).field("password", &"***").finish()
    }
}

pub fn mysql_url(c: &MysqlConfig) -> String {
    // libmysqlclient는 "localhost"를 유닉스 소켓으로 해석한다. 항상 TCP로 붙게 루프백 주소로 바꾼다.
    let host = if c.host == "localhost" { "127.0.0.1" } else { c.host.as_str() };
    format!("mysql://{}:{}@{}:{}/{}", quote_component(&c.user), quote_component(&c.password), host, c.port, c.database)
}

// ---------------------------------------------------------------------------
// Diesel 구현
// ---------------------------------------------------------------------------

fn map_error(e: DieselError) -> DbError {
    match e {
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation | DatabaseErrorKind::ForeignKeyViolation | DatabaseErrorKind::NotNullViolation | DatabaseErrorKind::CheckViolation, info) => {
            DbError::Constraint(info.message().to_string())
        }
        DieselError::DatabaseError(_, info) => DbError::Other(info.message().to_string()),
        other => DbError::Other(other.to_string()),
    }
}

/// 열 구조를 미리 모르는 SELECT 결과 한 행.
struct DynRow(Row);

impl QueryableByName<Mysql> for DynRow {
    fn build<'a>(row: &impl NamedRow<'a, Mysql>) -> deserialize::Result<Self> {
        let mut out = Row::new();
        for i in 0..row.field_count() {
            let Some(field) = diesel::row::Row::get(row, i) else { continue };
            let name = field.field_name().unwrap_or_default().to_string();
            let value = match field.value() {
                None => DbValue::Null,
                Some(raw) => match raw.value_type() {
                    MysqlType::Tiny
                    | MysqlType::UnsignedTiny
                    | MysqlType::Short
                    | MysqlType::UnsignedShort
                    | MysqlType::Long
                    | MysqlType::UnsignedLong
                    | MysqlType::LongLong
                    | MysqlType::UnsignedLongLong => DbValue::Int(<i64 as FromSql<BigInt, Mysql>>::from_sql(raw)?),
                    MysqlType::Float | MysqlType::Double => DbValue::Text(<f64 as FromSql<Double, Mysql>>::from_sql(raw)?.to_string()),
                    MysqlType::DateTime => DbValue::DateTime(<NaiveDateTime as FromSql<Datetime, Mysql>>::from_sql(raw)?),
                    MysqlType::Timestamp => DbValue::DateTime(<NaiveDateTime as FromSql<Timestamp, Mysql>>::from_sql(raw)?),
                    MysqlType::Date => DbValue::DateTime(<NaiveDate as FromSql<Date, Mysql>>::from_sql(raw)?.and_time(NaiveTime::MIN)),
                    MysqlType::Time => DbValue::Text(<NaiveTime as FromSql<Time, Mysql>>::from_sql(raw)?.to_string()),
                    // 문자열, DECIMAL, 바이너리, 열거형은 UTF-8 문자열로 읽는다. 잘못된 바이트는 대체 문자가 된다.
                    _ => DbValue::Text(String::from_utf8_lossy(raw.as_bytes()).into_owned()),
                },
            };
            out.insert(name, value);
        }
        Ok(DynRow(out))
    }
}

type SharedConn = Arc<Mutex<MysqlConnection>>;

fn lock(conn: &SharedConn) -> std::sync::MutexGuard<'_, MysqlConnection> {
    conn.lock().unwrap_or_else(|e| e.into_inner())
}

/// 결과 집합을 돌려주는 문장인지. 그렇지 않으면 영향받은 행 수를 돌려주는 경로로 실행한다.
fn returns_rows(sql: &str) -> bool {
    let first = sql.trim_start().split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("").to_ascii_uppercase();
    matches!(first.as_str(), "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" | "WITH")
}

struct MysqlDbConnection {
    conn: SharedConn,
    closed: bool,
}

impl MysqlDbConnection {
    fn batch(&mut self, sql: &str) -> Result<(), DbError> {
        if self.closed {
            return Err(DbError::Other("닫힌 연결이다".into()));
        }
        lock(&self.conn).batch_execute(sql).map_err(map_error)
    }

    fn query(sql: &str, params: &[DbValue]) -> diesel::query_builder::BoxedSqlQuery<'static, Mysql, diesel::query_builder::SqlQuery> {
        let mut q = sql_query(sql.to_string()).into_boxed::<Mysql>();
        for p in params {
            q = match p {
                DbValue::Null => q.bind::<Nullable<Text>, _>(None::<String>),
                DbValue::Int(n) => q.bind::<BigInt, _>(*n),
                DbValue::Text(s) => q.bind::<Text, _>(s.clone()),
                DbValue::DateTime(d) => q.bind::<Timestamp, _>(*d),
            };
        }
        q
    }
}

impl DbConnection for MysqlDbConnection {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<u64, DbError> {
        if returns_rows(sql) {
            return Ok(self.fetch_all(sql, params)?.len() as u64);
        }
        let n = Self::query(sql, params).execute(&mut *lock(&self.conn)).map_err(map_error)?;
        Ok(n as u64)
    }

    fn fetch_all(&mut self, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, DbError> {
        let rows: Vec<DynRow> = Self::query(sql, params).load(&mut *lock(&self.conn)).map_err(map_error)?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    fn commit(&mut self) -> Result<(), DbError> {
        self.batch("COMMIT")
    }

    fn rollback(&mut self) -> Result<(), DbError> {
        self.batch("ROLLBACK")
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

/// 새 연결을 연다. 자동 커밋을 끄고(`autocommit=0`), 커밋은 `commit()`으로 명시한다.
pub fn connect(config: &MysqlConfig) -> Result<Box<dyn DbConnection>, DbError> {
    let mut conn = MysqlConnection::establish(&mysql_url(config)).map_err(|e| DbError::Other(format!("MySQL 연결 실패 ({}:{}): {e}", config.host, config.port)))?;
    // Diesel은 연결할 때 time_zone을 +00:00으로 바꾼다. 서버 설정을 따르게 되돌린다.
    conn.batch_execute("SET autocommit=0, time_zone=@@global.time_zone").map_err(map_error)?;
    Ok(Box::new(MysqlDbConnection { conn: Arc::new(Mutex::new(conn)), closed: false }))
}

pub fn connect_from_env(env: &dyn Fn(&str) -> Option<String>, default_user: &str) -> Result<Box<dyn DbConnection>, DbError> {
    connect(&MysqlConfig::from_env(env, default_user))
}

/// 설정을 고정한 제공자. 서비스에 주입해서 연산마다 새 연결을 얻게 한다.
pub fn mysql_provider(config: MysqlConfig) -> ConnectionProvider {
    Arc::new(move || connect(&config))
}

#[cfg(test)]
mod tests {
    use super::{DbError, DieselError, map_error};

    #[test]
    fn non_database_diesel_errors_become_other_db_errors() {
        assert!(matches!(map_error(DieselError::NotFound), DbError::Other(message) if !message.is_empty()));
    }
}
