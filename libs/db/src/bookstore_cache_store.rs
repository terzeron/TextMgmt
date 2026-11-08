//! 서점 검색 결과 캐시. 같은 책에 대해 서점을 다시 검색하지 않으려고 둔다.
//!
//! 키는 inode와 ISBN-13 두 가지이고 서점마다 따로 둔다. 결과가 있으면 만료 없이, "결과 없음"은 30일 뒤 만료한다.
//! 차단·오류는 호출하는 쪽이 적지 않는다. 만료 판정은 읽은 뒤 여기서 한다.

use sysdeps::EnvFn;

use crate::{ConnectionProvider, DbConnection, DbError, DbValue, MysqlConfig, Row, mysql_provider};

/// "결과 없음"을 다시 검색하기까지의 시간.
pub const NOT_FOUND_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;

const CREATE_INODE_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS bookstore_inode_cache (
        inode BIGINT NOT NULL,
        store VARCHAR(16) NOT NULL,
        method VARCHAR(16) NOT NULL,
        query VARCHAR(1024) NOT NULL,
        isbn13 VARCHAR(13) NULL,
        status VARCHAR(16) NOT NULL,
        rows_json MEDIUMTEXT NOT NULL,
        fetched_at BIGINT NOT NULL,
        expires_at BIGINT NULL,
        PRIMARY KEY (inode, store)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
";

const CREATE_ISBN_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS bookstore_isbn_cache (
        isbn13 VARCHAR(13) NOT NULL,
        store VARCHAR(16) NOT NULL,
        status VARCHAR(16) NOT NULL,
        rows_json MEDIUMTEXT NOT NULL,
        fetched_at BIGINT NOT NULL,
        expires_at BIGINT NULL,
        PRIMARY KEY (isbn13, store)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
";

const UPSERT_INODE_SQL: &str = "
    INSERT INTO bookstore_inode_cache (inode, store, method, query, isbn13, status, rows_json, fetched_at, expires_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON DUPLICATE KEY UPDATE
        method = VALUES(method),
        query = VALUES(query),
        isbn13 = VALUES(isbn13),
        status = VALUES(status),
        rows_json = VALUES(rows_json),
        fetched_at = VALUES(fetched_at),
        expires_at = VALUES(expires_at)
";

const UPSERT_ISBN_SQL: &str = "
    INSERT INTO bookstore_isbn_cache (isbn13, store, status, rows_json, fetched_at, expires_at)
    VALUES (?, ?, ?, ?, ?, ?)
    ON DUPLICATE KEY UPDATE
        status = VALUES(status),
        rows_json = VALUES(rows_json),
        fetched_at = VALUES(fetched_at),
        expires_at = VALUES(expires_at)
";

const SELECT_INODE_SQL: &str = "
    SELECT status, rows_json, method, query, isbn13, fetched_at, expires_at
    FROM bookstore_inode_cache WHERE inode = ? AND store = ?
";

/// ISBN 행은 ISBN으로 검색한 결과다. 방식과 검색어를 ISBN 행 모양에 맞춰 채운다.
const SELECT_ISBN_SQL: &str = "
    SELECT status, rows_json, 'isbn' AS method, isbn13 AS query, NULL AS isbn13, fetched_at, expires_at
    FROM bookstore_isbn_cache WHERE isbn13 = ? AND store = ?
";

/// 캐시 행 하나. `status`는 `found`/`not_found`, `method`는 `isbn`/`title_author`/`title`이다.
/// `query`는 그때 쓴 검색어(ISBN이면 ISBN-13, 이름이면 `BookQuery::title_key`), `isbn13`은 inode 행에 남기는 그때의 ISBN이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedLookup {
    pub status: String,
    pub rows_json: String,
    pub method: String,
    pub query: String,
    pub isbn13: Option<String>,
    pub fetched_at: i64,
}

impl CachedLookup {
    fn expires_at(&self) -> DbValue {
        if self.status == "not_found" { DbValue::Int(self.fetched_at + NOT_FOUND_TTL_SECONDS) } else { DbValue::Null }
    }
}

fn text(row: &Row, key: &str) -> String {
    match row.get(key) {
        Some(DbValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}

fn int(row: &Row, key: &str) -> Option<i64> {
    match row.get(key) {
        Some(DbValue::Int(n)) => Some(*n),
        _ => None,
    }
}

/// 만료 전이면 행을 돌려준다.
fn fresh(row: Option<Row>, now: i64) -> Option<CachedLookup> {
    let row = row?;
    if int(&row, "expires_at").is_some_and(|expires| now >= expires) {
        return None;
    }
    let isbn13 = Some(text(&row, "isbn13")).filter(|s| !s.is_empty());
    Some(CachedLookup {
        status: text(&row, "status"),
        rows_json: text(&row, "rows_json"),
        method: text(&row, "method"),
        query: text(&row, "query"),
        isbn13,
        fetched_at: int(&row, "fetched_at").unwrap_or(0),
    })
}

pub struct BookstoreCacheStore {
    provider: ConnectionProvider,
}

impl BookstoreCacheStore {
    /// `TM_MYSQL_*` 환경 변수로 접속하고 테이블을 만든다.
    pub fn new() -> Result<Self, DbError> {
        Self::from_env(&|key| std::env::var(key).ok())
    }

    pub fn from_env(env: EnvFn) -> Result<Self, DbError> {
        let store = Self::with_connection_provider(mysql_provider(MysqlConfig::from_env(env, "textmanager")));
        store.init_db()?;
        Ok(store)
    }

    /// 주입한 연결 제공자를 쓴다. 테이블은 만들지 않는다.
    pub fn with_connection_provider(provider: ConnectionProvider) -> Self {
        Self { provider }
    }

    fn with_conn<T>(&self, f: impl FnOnce(&mut dyn DbConnection) -> Result<T, DbError>) -> Result<T, DbError> {
        let mut conn = (self.provider)()?;
        let result = f(conn.as_mut());
        conn.close();
        result
    }

    fn init_db(&self) -> Result<(), DbError> {
        self.with_conn(|conn| {
            conn.execute(CREATE_INODE_TABLE_SQL, &[])?;
            conn.execute(CREATE_ISBN_TABLE_SQL, &[])?;
            conn.commit()
        })
    }

    pub fn get_by_inode(&self, inode: i64, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError> {
        self.with_conn(|conn| Ok(fresh(conn.fetch_one(SELECT_INODE_SQL, &[DbValue::Int(inode), DbValue::Text(store.to_string())])?, now)))
    }

    pub fn get_by_isbn(&self, isbn13: &str, store: &str, now: i64) -> Result<Option<CachedLookup>, DbError> {
        self.with_conn(|conn| Ok(fresh(conn.fetch_one(SELECT_ISBN_SQL, &[DbValue::Text(isbn13.to_string()), DbValue::Text(store.to_string())])?, now)))
    }

    pub fn put_inode(&self, inode: i64, store: &str, lookup: &CachedLookup) -> Result<(), DbError> {
        let params = [
            DbValue::Int(inode),
            DbValue::Text(store.to_string()),
            DbValue::Text(lookup.method.clone()),
            DbValue::Text(lookup.query.clone()),
            lookup.isbn13.clone().map_or(DbValue::Null, DbValue::Text),
            DbValue::Text(lookup.status.clone()),
            DbValue::Text(lookup.rows_json.clone()),
            DbValue::Int(lookup.fetched_at),
            lookup.expires_at(),
        ];
        self.with_conn(|conn| {
            conn.execute(UPSERT_INODE_SQL, &params)?;
            conn.commit()
        })
    }

    pub fn put_isbn(&self, isbn13: &str, store: &str, lookup: &CachedLookup) -> Result<(), DbError> {
        let params = [
            DbValue::Text(isbn13.to_string()),
            DbValue::Text(store.to_string()),
            DbValue::Text(lookup.status.clone()),
            DbValue::Text(lookup.rows_json.clone()),
            DbValue::Int(lookup.fetched_at),
            lookup.expires_at(),
        ];
        self.with_conn(|conn| {
            conn.execute(UPSERT_ISBN_SQL, &params)?;
            conn.commit()
        })
    }
}
