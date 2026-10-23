//! MySQL 접근 추상화. pymysql `DictCursor`의 의미(`%s` 자리표시자, 열 이름 맵 행, 수동 커밋, IntegrityError)를 Diesel 위에서 재현한다.
//! 서비스 코드는 `DbConnection`/`ConnectionProvider`만 알고, 테스트는 가짜 연결을 주입한다. 설계: docs/rust-test-seams.md 3.4
//!
//! 주의: Diesel은 연결을 열 때 `time_zone`을 `+00:00`으로 맞춘다. pymysql은 서버 기본값을 쓴다.
//! SQL의 `NOW()`나 `TIMESTAMP` 열을 읽는 코드를 옮길 때 이 차이를 확인해야 한다.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use diesel::connection::SimpleConnection;
use diesel::deserialize::{self, FromSql, QueryableByName};
use diesel::mysql::{Mysql, MysqlConnection, MysqlType};
use diesel::result::{DatabaseErrorKind, Error as DieselError};
use diesel::row::{Field, NamedRow};
use diesel::sql_types::{BigInt, Date, Datetime, Double, Nullable, Text, Time, Timestamp};
use diesel::{sql_query, Connection, RunQueryDsl};

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

/// 열 이름 → 값. `DictCursor`가 돌려주는 dict에 대응한다.
pub type Row = BTreeMap<String, DbValue>;

#[derive(Debug, Clone, PartialEq)]
pub enum DbError {
    /// `pymysql.IntegrityError`: 키 중복, 외래 키, NOT NULL 위반.
    IntegrityError(String),
    Other(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::IntegrityError(msg) => write!(f, "무결성 오류: {msg}"),
            DbError::Other(msg) => write!(f, "DB 오류: {msg}"),
        }
    }
}

impl std::error::Error for DbError {}

// ---------------------------------------------------------------------------
// 추상화
// ---------------------------------------------------------------------------

pub trait DbCursor {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError>;
    fn executemany(&mut self, sql: &str, seq: &[Vec<DbValue>]) -> Result<(), DbError>;
    /// 남은 행을 모두 돌려준다. 결과 집합이 없는 문장(INSERT 등) 뒤에는 `None`이다.
    fn fetchall(&mut self) -> Option<Vec<Row>>;
    fn fetchone(&mut self) -> Option<Row>;
    /// SELECT는 읽은 행 수, 그 밖의 문장은 영향받은 행 수다.
    fn rowcount(&self) -> u64;
}

pub trait DbConnection: Send {
    fn cursor(&mut self) -> Box<dyn DbCursor>;
    fn commit(&mut self) -> Result<(), DbError>;
    fn rollback(&mut self) -> Result<(), DbError>;
    fn close(&mut self);
}

/// 호출할 때마다 새 연결을 연다. Python 코드가 연산마다 `pymysql.connect`를 부르는 것과 같다.
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

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn mysql_url(c: &MysqlConfig) -> String {
    // libmysqlclient는 "localhost"를 유닉스 소켓으로 해석한다. pymysql처럼 항상 TCP로 붙게 루프백 주소로 바꾼다.
    let host = if c.host == "localhost" { "127.0.0.1" } else { c.host.as_str() };
    format!("mysql://{}:{}@{}:{}/{}", percent_encode(&c.user), percent_encode(&c.password), host, c.port, c.database)
}

/// pymysql의 `%s`를 MySQL 준비된 문장의 `?`로 바꾼다. `%%`는 `%` 한 글자다.
pub fn to_mysql_placeholders(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('s') => {
                    chars.next();
                    out.push('?');
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                }
                _ => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Diesel 구현
// ---------------------------------------------------------------------------

fn map_error(e: DieselError) -> DbError {
    match e {
        DieselError::DatabaseError(DatabaseErrorKind::UniqueViolation | DatabaseErrorKind::ForeignKeyViolation | DatabaseErrorKind::NotNullViolation | DatabaseErrorKind::CheckViolation, info) => {
            DbError::IntegrityError(info.message().to_string())
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

struct MysqlCursor {
    conn: SharedConn,
    rows: std::vec::IntoIter<Row>,
    has_result: bool,
    rowcount: u64,
}

impl MysqlCursor {
    fn run(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError> {
        let sql = to_mysql_placeholders(sql);
        let is_query = returns_rows(&sql);
        let mut q = sql_query(sql).into_boxed::<Mysql>();
        for p in params {
            q = match p {
                DbValue::Null => q.bind::<Nullable<Text>, _>(None::<String>),
                DbValue::Int(n) => q.bind::<BigInt, _>(*n),
                DbValue::Text(s) => q.bind::<Text, _>(s.clone()),
                DbValue::DateTime(d) => q.bind::<Timestamp, _>(*d),
            };
        }
        let mut guard = lock(&self.conn);
        if is_query {
            let rows: Vec<DynRow> = q.load(&mut *guard).map_err(map_error)?;
            self.rowcount = rows.len() as u64;
            self.rows = rows.into_iter().map(|r| r.0).collect::<Vec<_>>().into_iter();
            self.has_result = true;
        } else {
            let n = q.execute(&mut *guard).map_err(map_error)?;
            self.rowcount = n as u64;
            self.rows = Vec::new().into_iter();
            self.has_result = false;
        }
        Ok(())
    }
}

impl DbCursor for MysqlCursor {
    fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<(), DbError> {
        self.run(sql, params)
    }

    fn executemany(&mut self, sql: &str, seq: &[Vec<DbValue>]) -> Result<(), DbError> {
        let mut total = 0;
        for params in seq {
            self.run(sql, params)?;
            total += self.rowcount;
        }
        self.rowcount = total;
        Ok(())
    }

    fn fetchall(&mut self) -> Option<Vec<Row>> {
        self.has_result.then(|| self.rows.by_ref().collect())
    }

    fn fetchone(&mut self) -> Option<Row> {
        self.rows.next()
    }

    fn rowcount(&self) -> u64 {
        self.rowcount
    }
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
}

impl DbConnection for MysqlDbConnection {
    fn cursor(&mut self) -> Box<dyn DbCursor> {
        Box::new(MysqlCursor { conn: self.conn.clone(), rows: Vec::new().into_iter(), has_result: false, rowcount: 0 })
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

/// 새 연결을 연다. pymysql 기본값처럼 자동 커밋을 끄고(`autocommit=0`), 커밋은 `commit()`으로 명시한다.
pub fn connect(config: &MysqlConfig) -> Result<Box<dyn DbConnection>, DbError> {
    let mut conn = MysqlConnection::establish(&mysql_url(config)).map_err(|e| DbError::Other(format!("MySQL 연결 실패 ({}:{}): {e}", config.host, config.port)))?;
    conn.batch_execute("SET autocommit=0").map_err(map_error)?;
    Ok(Box::new(MysqlDbConnection { conn: Arc::new(Mutex::new(conn)), closed: false }))
}

pub fn connect_from_env(env: &dyn Fn(&str) -> Option<String>, default_user: &str) -> Result<Box<dyn DbConnection>, DbError> {
    connect(&MysqlConfig::from_env(env, default_user))
}

/// 설정을 고정한 제공자. 서비스에 주입해서 연산마다 새 연결을 얻게 한다.
pub fn mysql_provider(config: MysqlConfig) -> ConnectionProvider {
    Arc::new(move || connect(&config))
}
