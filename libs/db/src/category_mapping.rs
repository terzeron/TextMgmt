//! 카테고리별 키워드 매핑, 숨김 카테고리, 재적재 락, 분류 제안 항목·상태를 MySQL에 보관한다.
//! 연결은 `db::ConnectionProvider`가 연산마다 새로 열어 준다.
//!
//! 원본과 다른 점:
//! - `content_type` 기본값("book")이 없다. 호출자가 매번 넘긴다.
//! - DB 오류를 예외로 올리지 않는다. 반환 타입이 허용하는 만큼 기본값(`false`, 빈 목록)이나 `Err`로 돌려주고 `log::error!`로 남긴다.
//!   `try_*`와 `acquire_reload_lock`은 오류 때 "시작·획득 안 됨"으로 답한다.
//! - 마이그레이션의 `information_schema` 조회는 `table_schema = DATABASE()`를 쓴다. 제공자가 DB 이름을 모르기 때문이다.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use chrono::{Local, NaiveDateTime};
use serde_json::{Map, Value};

use crate::{ConnectionProvider, DbConnection, DbError, DbValue, MysqlConfig, Row, mysql_provider};
use common::UNIMPLEMENTED;

/// 재적재 락의 heartbeat(updated_at)가 이 시간 이상 갱신되지 않으면 죽은 작업으로 본다.
/// 총 소요 시간이 아니라 "최근에 살아있다는 신호"로 판정해야, 오래 걸리는 정상 재적재와 안 풀리는 죽은 락을 구분한다.
pub const RELOAD_LOCK_HEARTBEAT_STALE_SECONDS: i64 = 5 * 60;
/// 일괄/전체 재적재가 쓰는 락 키. 카테고리별 재적재는 카테고리명을 그대로 쓴다.
pub const BULK_LOCK_KEY: &str = "__all__";
pub const RELOAD_SOURCE_BULK: &str = "bulk";
pub const RELOAD_SOURCE_MISMATCH: &str = "mismatch";

const PROGRESS_COUNT_FIELDS: [&str; 5] = ["indexed_count", "deleted_count", "failed_count", "before_count", "after_count"];
const STALE_ERROR_MESSAGE: &str = "응답 없이 중단된 것으로 보입니다.";
const STALE_CLASSIFY_PROPOSAL_ERROR_MESSAGE: &str = "백엔드가 다시 시작되어 진행 중이던 작업이 중단되었습니다. 남은 항목을 확인한 뒤 진행하세요.";

/// 카테고리 매핑 저장소 오류. 알 수 없는 진행 카운트 필드나, 락 갱신 중 DB 오류를 담는다.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryMappingError(pub String);

impl fmt::Display for CategoryMappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CategoryMappingError {}

/// `get_connection`이 돌려주는 연결. 커서를 직접 다루지 않고 문장 단위로 실행한다. 버릴 때 닫는다.
pub struct MappingConnection {
    inner: Box<dyn DbConnection>,
}

impl MappingConnection {
    /// 문장을 실행하고 영향받은 행 수를 돌려준다.
    pub fn execute(&mut self, sql: &str, params: &[DbValue]) -> Result<u64, DbError> {
        self.inner.execute(sql, params)
    }

    pub fn fetch_all(&mut self, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, DbError> {
        self.inner.fetch_all(sql, params)
    }

    pub fn fetch_one(&mut self, sql: &str, params: &[DbValue]) -> Result<Option<Row>, DbError> {
        self.inner.fetch_one(sql, params)
    }

    pub fn commit(&mut self) -> Result<(), DbError> {
        self.inner.commit()
    }

    pub fn rollback(&mut self) -> Result<(), DbError> {
        self.inner.rollback()
    }
}

impl Drop for MappingConnection {
    fn drop(&mut self) {
        self.inner.close();
    }
}

pub struct CategoryMapping {
    provider: ConnectionProvider,
}

fn text_of<'a>(row: &'a Row, key: &str) -> Option<&'a str> {
    row.get(key).and_then(DbValue::as_str)
}

fn is_empty_value(v: Option<&DbValue>) -> bool {
    match v {
        None | Some(DbValue::Null) => true,
        Some(DbValue::Text(s)) => s.is_empty(),
        _ => false,
    }
}

/// ISO 8601 문자열. 마이크로초가 0이면 소수부를 생략한다.
fn isoformat(dt: &NaiveDateTime) -> String {
    let micros = dt.and_utc().timestamp_subsec_micros();
    if micros == 0 { dt.format("%Y-%m-%dT%H:%M:%S").to_string() } else { format!("{}.{micros:06}", dt.format("%Y-%m-%dT%H:%M:%S")) }
}

fn iso_or_null(v: Option<&DbValue>) -> DbValue {
    match v {
        Some(DbValue::DateTime(dt)) => DbValue::Text(isoformat(dt)),
        _ => DbValue::Null,
    }
}

/// `updated_at`이 지금(앱 지역 시간)보다 `RELOAD_LOCK_HEARTBEAT_STALE_SECONDS`를 넘게 과거인가.
fn heartbeat_is_stale(updated_at: Option<&DbValue>) -> bool {
    match updated_at {
        Some(DbValue::DateTime(dt)) => (Local::now().naive_local() - *dt).num_milliseconds() > RELOAD_LOCK_HEARTBEAT_STALE_SECONDS * 1000,
        _ => false,
    }
}

fn json_string(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

fn lock_key_of(category: Option<&str>) -> &str {
    category.filter(|c| !c.is_empty()).unwrap_or(BULK_LOCK_KEY)
}

fn validate_counts(counts: &[(&str, i64)]) -> Result<(), CategoryMappingError> {
    let unknown: Vec<&str> = counts.iter().map(|(k, _)| *k).filter(|k| !PROGRESS_COUNT_FIELDS.contains(k)).collect();
    if unknown.is_empty() { Ok(()) } else { Err(CategoryMappingError(format!("알 수 없는 진행 카운트 필드: {unknown:?}"))) }
}

fn count_set_clause(counts: &[(&str, i64)]) -> String {
    counts.iter().map(|(field, _)| format!(", {field} = ?")).collect()
}

fn db_error_to_value_error(op: &str, e: DbError) -> CategoryMappingError {
    log::error!("{op} failed: {e}");
    CategoryMappingError(e.to_string())
}

impl CategoryMapping {
    /// 초기화(`init_db`)를 건너뛰고 연결 제공자만 갈아끼운다.
    pub fn with_connection_provider(provider: ConnectionProvider) -> Self {
        Self { provider }
    }

    /// 테이블을 만들고 마이그레이션까지 마친 인스턴스를 돌려준다.
    pub fn with_connector(provider: ConnectionProvider) -> Result<Self, DbError> {
        let mapping = Self { provider };
        mapping.init_db()?;
        Ok(mapping)
    }

    /// `TM_MYSQL_*` 환경 변수로 접속한다(사용자 기본값 `tmuser`).
    pub fn from_env(env: &dyn Fn(&str) -> Option<String>) -> Result<Self, DbError> {
        let config = MysqlConfig::from_env(env, "tmuser");
        log::info!("CategoryMapping initialized with MySQL host: {}:{}, database: {}", config.host, config.port, config.database);
        Self::with_connector(mysql_provider(config))
    }

    /// 프로세스 환경 변수로 `from_env`를 부른다.
    pub fn new() -> Result<Self, DbError> {
        Self::from_env(&|key| std::env::var(key).ok())
    }

    /// 새 연결을 하나 연다. 테스트와 운영 점검용이다.
    pub fn get_connection(&self) -> Result<MappingConnection, DbError> {
        Ok(MappingConnection { inner: (self.provider)()? })
    }

    // ---- 내부 도우미 ----

    /// 새 연결로 `f`를 실행하고 연결을 닫는다. 커밋은 `f`가 한다.
    fn run<T>(&self, f: impl FnOnce(&mut dyn DbConnection) -> Result<T, DbError>) -> Result<T, DbError> {
        let mut conn = (self.provider)()?;
        let result = f(conn.as_mut());
        conn.close();
        result
    }

    /// `f`를 실행하고 커밋한다. 어느 단계든 실패하면 롤백한다.
    fn tx<T>(&self, f: impl FnOnce(&mut dyn DbConnection) -> Result<T, DbError>) -> Result<T, DbError> {
        self.run(|conn| {
            let result = f(&mut *conn).and_then(|value| conn.commit().map(|()| value));
            if result.is_err() {
                let _ = conn.rollback();
            }
            result
        })
    }

    /// 읽기 전용 SELECT 한 번.
    fn select(&self, sql: &str, params: &[DbValue]) -> Result<Vec<Row>, DbError> {
        self.run(|conn| conn.fetch_all(sql, params))
    }

    fn select_strings(&self, op: &str, sql: &str, params: &[DbValue], column: &str) -> Result<Vec<String>, DbError> {
        let rows = self.select(sql, params)?;
        let values: Vec<String> = rows.iter().filter_map(|r| text_of(r, column).map(str::to_string)).collect();
        log::debug!("{op}: {} rows", values.len());
        Ok(values)
    }

    // ---- 초기화와 마이그레이션 ----

    fn init_db(&self) -> Result<(), DbError> {
        self.run(|conn| {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS category_keywords (id INT AUTO_INCREMENT PRIMARY KEY, category VARCHAR(255) NOT NULL, keyword VARCHAR(255) NOT NULL, content_type VARCHAR(10) NOT NULL DEFAULT 'book', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, UNIQUE KEY unique_category_keyword (category, keyword, content_type), INDEX idx_category (category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
                &[],
            )?;
            conn.execute(
                "CREATE TABLE IF NOT EXISTS hidden_categories (id INT AUTO_INCREMENT PRIMARY KEY, category VARCHAR(255) NOT NULL, content_type VARCHAR(10) NOT NULL DEFAULT 'book', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, UNIQUE KEY unique_category_content_type (category, content_type), INDEX idx_category (category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
                &[],
            )?;
            conn.execute(
                "CREATE TABLE IF NOT EXISTS latest_excluded_categories (id INT AUTO_INCREMENT PRIMARY KEY, category VARCHAR(255) NOT NULL, content_type VARCHAR(10) NOT NULL DEFAULT 'book', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, UNIQUE KEY unique_latest_excluded_category_content_type (category, content_type), INDEX idx_category (category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
                &[],
            )?;
            conn.execute("CREATE TABLE IF NOT EXISTS reload_locks (content_type VARCHAR(10) NOT NULL PRIMARY KEY, started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci", &[])?;
            // 분류 제안 항목은 행 단위로 쌓는다. JSON 상태 파일 하나에 담으면 한 권 기록할 때마다 전체를 다시 직렬화해 비용이 책 수의 제곱으로 늘었다.
            // file_path는 1024자라 UNIQUE를 걸면 인덱스 길이 제한에 걸린다. 중복은 clear로 관리한다.
            // 항목 필드가 늘 수 있어 payload 하나에 JSON으로 담는다. apply_status/apply_error는 인덱스를 타도록 컬럼으로 뺐다.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS classify_proposal_items (id INT AUTO_INCREMENT PRIMARY KEY, content_type VARCHAR(10) NOT NULL DEFAULT 'book', source_category VARCHAR(255) NOT NULL, file_path VARCHAR(1024) NOT NULL, payload JSON NOT NULL, apply_status VARCHAR(20) NOT NULL DEFAULT 'pending', apply_error TEXT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, INDEX idx_content_source (content_type, source_category), INDEX idx_content_file (content_type, file_path(255)), INDEX idx_apply_status (content_type, apply_status)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
                &[],
            )?;
            Self::migrate_add_content_type(conn)?;
            Self::migrate_add_apply_status(conn)?;
            // 분류 제안 작업의 상태. 여러 프로세스가 잠금 없이 JSON 파일을 같이 쓰던 것을 DB 행 + FOR UPDATE로 옮겼다.
            // status는 FOR UPDATE로 판단해야 해서 컬럼으로 뺐고, updated_at은 하트비트다. 갱신이 끊기면 죽은 작업으로 본다.
            conn.execute(
                "CREATE TABLE IF NOT EXISTS classify_proposal_status (content_type VARCHAR(10) NOT NULL PRIMARY KEY, status VARCHAR(20) NOT NULL DEFAULT 'idle', payload JSON NOT NULL, updated_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci",
                &[],
            )?;
            Self::migrate_classify_proposal_item_indexes(conn)?;
            Self::migrate_reload_locks(conn)?;
            Self::migrate_reload_locks_lock_key(conn)?;
            conn.commit()
        })?;
        log::debug!("Database initialized");
        Ok(())
    }

    /// `SELECT COUNT(*) AS cnt ...`의 결과가 0인가. 행이 없으면 거짓이다.
    fn count_is_zero(conn: &mut dyn DbConnection, sql: &str, params: &[DbValue]) -> Result<bool, DbError> {
        Ok(conn.fetch_one(sql, params)?.is_some_and(|row| row.get("cnt").and_then(DbValue::as_i64) == Some(0)))
    }

    fn count_is_positive(conn: &mut dyn DbConnection, sql: &str, params: &[DbValue]) -> Result<bool, DbError> {
        Ok(conn.fetch_one(sql, params)?.is_some_and(|row| row.get("cnt").and_then(DbValue::as_i64).is_some_and(|n| n > 0)))
    }

    fn migrate_add_content_type(conn: &mut dyn DbConnection) -> Result<(), DbError> {
        let table_unique_indexes =
            [("category_keywords", "unique_category_keyword"), ("hidden_categories", "unique_category_content_type"), ("latest_excluded_categories", "unique_latest_excluded_category_content_type")];
        for (table, unique_index) in table_unique_indexes {
            if Self::count_is_zero(conn, "SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? AND column_name = 'content_type'", &[table.into()])?
            {
                log::info!("Migrating table {table}: adding content_type column");
                conn.execute(&format!("ALTER TABLE {table} ADD COLUMN content_type VARCHAR(10) NOT NULL DEFAULT 'book'"), &[])?;
                if table == "category_keywords" {
                    if let Err(e) = conn.execute(&format!("ALTER TABLE {table} DROP INDEX unique_category_keyword"), &[]) {
                        log::debug!("Index unique_category_keyword not found, skipping: {e}");
                    }
                    conn.execute(&format!("ALTER TABLE {table} ADD UNIQUE KEY unique_category_keyword (category, keyword, content_type)"), &[])?;
                } else {
                    if let Err(e) = conn.execute(&format!("ALTER TABLE {table} DROP INDEX category"), &[]) {
                        log::debug!("Index category not found, skipping: {e}");
                    }
                    conn.execute(&format!("ALTER TABLE {table} ADD UNIQUE KEY {unique_index} (category, content_type)"), &[])?;
                }
            }
        }
        Ok(())
    }

    /// 기존 `classify_proposal_items`에 apply_status/apply_error 컬럼과 인덱스를 더한다. 컬럼이 없을 때만 ALTER한다.
    fn migrate_add_apply_status(conn: &mut dyn DbConnection) -> Result<(), DbError> {
        if Self::count_is_zero(
            conn,
            "SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'classify_proposal_items' AND column_name = 'apply_status'",
            &[],
        )? {
            log::info!("Migrating table classify_proposal_items: adding apply_status/apply_error columns");
            conn.execute("ALTER TABLE classify_proposal_items ADD COLUMN apply_status VARCHAR(20) NOT NULL DEFAULT 'pending'", &[])?;
            conn.execute("ALTER TABLE classify_proposal_items ADD COLUMN apply_error TEXT NULL", &[])?;
            conn.execute("ALTER TABLE classify_proposal_items ADD INDEX idx_apply_status (content_type, apply_status)", &[])?;
        }
        Ok(())
    }

    /// `classify_proposal_items`의 인덱스를 (content_type, file_path) 기준으로 맞춘다.
    /// 승인 작업이 책 한 권마다 `WHERE content_type = ? AND file_path = ?`로 갱신하므로 이 인덱스가 없으면 매번 전수 훑기가 된다.
    /// file_path는 1024자라 앞 255자만 인덱스에 쓴다. `idx_content_type`은 `idx_content_source`의 왼쪽 접두사와 같아 함께 지운다.
    fn migrate_classify_proposal_item_indexes(conn: &mut dyn DbConnection) -> Result<(), DbError> {
        if Self::count_is_zero(
            conn,
            "SELECT COUNT(*) AS cnt FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'classify_proposal_items' AND index_name = 'idx_content_file'",
            &[],
        )? {
            log::info!("Migrating table classify_proposal_items: adding idx_content_file");
            conn.execute("ALTER TABLE classify_proposal_items ADD INDEX idx_content_file (content_type, file_path(255))", &[])?;
        }

        if Self::count_is_positive(
            conn,
            "SELECT COUNT(*) AS cnt FROM information_schema.statistics WHERE table_schema = DATABASE() AND table_name = 'classify_proposal_items' AND index_name = 'idx_content_type'",
            &[],
        )? {
            log::info!("Migrating table classify_proposal_items: dropping redundant idx_content_type");
            conn.execute("ALTER TABLE classify_proposal_items DROP INDEX idx_content_type", &[])?;
        }
        Ok(())
    }

    /// `reload_locks`를 진행 상황(heartbeat·카운트)까지 담는 공유 작업 상태 테이블로 넓힌다.
    fn migrate_reload_locks(conn: &mut dyn DbConnection) -> Result<(), DbError> {
        let new_columns = [
            ("category", "ALTER TABLE reload_locks ADD COLUMN category VARCHAR(255) NULL"),
            ("status", "ALTER TABLE reload_locks ADD COLUMN status VARCHAR(10) NOT NULL DEFAULT 'running'"),
            ("updated_at", "ALTER TABLE reload_locks ADD COLUMN updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP"),
            ("indexed_count", "ALTER TABLE reload_locks ADD COLUMN indexed_count INT NOT NULL DEFAULT 0"),
            ("deleted_count", "ALTER TABLE reload_locks ADD COLUMN deleted_count INT NOT NULL DEFAULT 0"),
            ("failed_count", "ALTER TABLE reload_locks ADD COLUMN failed_count INT NOT NULL DEFAULT 0"),
            ("before_count", "ALTER TABLE reload_locks ADD COLUMN before_count INT NOT NULL DEFAULT 0"),
            ("after_count", "ALTER TABLE reload_locks ADD COLUMN after_count INT NOT NULL DEFAULT 0"),
            ("error", "ALTER TABLE reload_locks ADD COLUMN error TEXT NULL"),
            ("reload_source", "ALTER TABLE reload_locks ADD COLUMN reload_source VARCHAR(20) NOT NULL DEFAULT 'bulk'"),
        ];
        for (column, alter_sql) in new_columns {
            if Self::count_is_zero(
                conn,
                "SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'reload_locks' AND column_name = ?",
                &[column.into()],
            )? {
                log::info!("Migrating table reload_locks: adding {column} column");
                conn.execute(alter_sql, &[])?;
                if column == "reload_source" {
                    conn.execute("UPDATE reload_locks SET reload_source = ? WHERE category IS NOT NULL", &[RELOAD_SOURCE_MISMATCH.into()])?;
                }
            }
        }
        Ok(())
    }

    /// `reload_locks`를 content_type 단일 락에서 (content_type, lock_key) 복합 락으로 넓힌다.
    /// 기존 행은 category가 있으면 그 카테고리를, 없으면 `BULK_LOCK_KEY`를 lock_key로 채워 보존한다.
    fn migrate_reload_locks_lock_key(conn: &mut dyn DbConnection) -> Result<(), DbError> {
        if Self::count_is_positive(conn, "SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = 'reload_locks' AND column_name = 'lock_key'", &[])? {
            return Ok(());
        }
        log::info!("Migrating table reload_locks: adding lock_key column and switching primary key to (content_type, lock_key)");
        // DDL의 DEFAULT 절은 바인드 파라미터를 지원하지 않아 상수 BULK_LOCK_KEY를 직접 넣는다.
        conn.execute(&format!("ALTER TABLE reload_locks ADD COLUMN lock_key VARCHAR(255) NOT NULL DEFAULT '{BULK_LOCK_KEY}'"), &[])?;
        conn.execute("UPDATE reload_locks SET lock_key = COALESCE(category, ?)", &[BULK_LOCK_KEY.into()])?;
        conn.execute("ALTER TABLE reload_locks DROP PRIMARY KEY, ADD PRIMARY KEY (content_type, lock_key)", &[])?;
        Ok(())
    }

    // ---- 키워드 ----

    /// `{category: [keyword, ...]}`. 카테고리 순서는 결정적이다. 키워드 동률 판정과 API 응답 순서가 이 순서를 따른다.
    pub fn get_all_mappings(&self, content_type: &str) -> Result<BTreeMap<String, Vec<String>>, DbError> {
        let rows = self.select("SELECT category, keyword FROM category_keywords WHERE content_type = ? ORDER BY category, keyword", &[content_type.into()])?;
        let mut mappings: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in &rows {
            if let (Some(category), Some(keyword)) = (text_of(row, "category"), text_of(row, "keyword")) {
                mappings.entry(category.to_string()).or_default().push(keyword.to_string());
            }
        }
        log::debug!("get_all_mappings({content_type}): {} categories", mappings.len());
        Ok(mappings)
    }

    pub fn get_keywords(&self, category: &str, content_type: &str) -> Result<Vec<String>, DbError> {
        let rows = self.select("SELECT keyword FROM category_keywords WHERE category = ? AND content_type = ? ORDER BY keyword", &[category.into(), content_type.into()])?;
        let keywords: Vec<String> = rows.iter().filter_map(|row| text_of(row, "keyword").map(str::to_string)).collect();
        log::debug!("get_keywords({category}, {content_type}): {} rows", keywords.len());
        Ok(keywords)
    }

    /// 이미 있는 키워드나 빈 키워드면 거짓이다.
    pub fn add_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, DbError> {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            log::warn!("add_keyword: empty keyword");
            return Ok(false);
        }
        match self.tx(|conn| conn.execute("INSERT INTO category_keywords (category, keyword, content_type) VALUES (?, ?, ?)", &[category.into(), keyword.into(), content_type.into()])) {
            Ok(_) => {
                log::info!("add_keyword({category}, {keyword}, {content_type}): success");
                Ok(true)
            }
            Err(DbError::Constraint(_)) => {
                log::warn!("add_keyword({category}, {keyword}, {content_type}): already exists");
                Ok(false)
            }
            Err(e) => {
                log::error!("add_keyword({category}, {keyword}, {content_type}) failed: {e}");
                Err(e)
            }
        }
    }

    pub fn remove_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, DbError> {
        let deleted =
            self.tx(|conn| Ok(conn.execute("DELETE FROM category_keywords WHERE category = ? AND keyword = ? AND content_type = ?", &[category.into(), keyword.into(), content_type.into()])? > 0))?;
        log::info!("remove_keyword({category}, {keyword}, {content_type}): {}", if deleted { "success" } else { "not found" });
        Ok(deleted)
    }

    /// 카테고리의 키워드를 통째로 바꾼다. 공백을 걷어내고 중복을 없앤다.
    pub fn set_keywords(&self, category: &str, keywords: &[&str], content_type: &str) -> bool {
        let mut seen = HashSet::new();
        let keywords: Vec<&str> = keywords.iter().map(|k| k.trim()).filter(|k| !k.is_empty() && seen.insert(*k)).collect();
        let result = self.tx(|conn| {
            conn.execute("DELETE FROM category_keywords WHERE category = ? AND content_type = ?", &[category.into(), content_type.into()])?;
            if !keywords.is_empty() {
                let rows: Vec<Vec<DbValue>> = keywords.iter().map(|kw| vec![category.into(), (*kw).into(), content_type.into()]).collect();
                conn.execute_batch("INSERT INTO category_keywords (category, keyword, content_type) VALUES (?, ?, ?)", &rows)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                log::info!("set_keywords({category}, {content_type}): {} keywords set", keywords.len());
                true
            }
            Err(e) => {
                log::error!("set_keywords({category}, {content_type}) failed: {e}");
                false
            }
        }
    }

    /// 해당 content_type의 매핑을 전부 지우고 새로 넣는다.
    pub fn update_all_mappings(&self, mappings: &HashMap<String, Vec<String>>, content_type: &str) -> bool {
        let result = self.tx(|conn| {
            conn.execute("DELETE FROM category_keywords WHERE content_type = ?", &[content_type.into()])?;
            let mut rows: Vec<Vec<DbValue>> = Vec::new();
            for (category, keywords) in mappings {
                for keyword in keywords {
                    let keyword = keyword.trim();
                    if !keyword.is_empty() {
                        rows.push(vec![category.as_str().into(), keyword.into(), content_type.into()]);
                    }
                }
            }
            if !rows.is_empty() {
                conn.execute_batch("INSERT IGNORE INTO category_keywords (category, keyword, content_type) VALUES (?, ?, ?)", &rows)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                log::info!("update_all_mappings({content_type}): {} categories updated", mappings.len());
                true
            }
            Err(e) => {
                log::error!("update_all_mappings({content_type}) failed: {e}");
                false
            }
        }
    }

    /// `prefix`가 참이면 하위 카테고리(`category/*`)도 지운다.
    pub fn delete_category(&self, category: &str, content_type: &str, prefix: bool) -> Result<bool, DbError> {
        let deleted = self.tx(|conn| {
            let deleted = if prefix {
                conn.execute("DELETE FROM category_keywords WHERE (category = ? OR category LIKE ?) AND content_type = ?", &[category.into(), format!("{category}/%").into(), content_type.into()])?
            } else {
                conn.execute("DELETE FROM category_keywords WHERE category = ? AND content_type = ?", &[category.into(), content_type.into()])?
            };
            Ok(deleted > 0)
        })?;
        log::info!("delete_category({category}, {content_type}, prefix={prefix}): {}", if deleted { "success" } else { "not found" });
        Ok(deleted)
    }

    /// 키워드 부분 일치로 카테고리를 찾는다.
    pub fn search_by_keyword(&self, keyword: &str, content_type: &str) -> Result<Vec<String>, DbError> {
        self.select_strings(
            &format!("search_by_keyword({keyword}, {content_type})"),
            "SELECT DISTINCT category FROM category_keywords WHERE keyword LIKE ? AND content_type = ? ORDER BY category",
            &[format!("%{keyword}%").into(), content_type.into()],
            "category",
        )
    }

    // ---- 숨김·최신 제외 카테고리 ----

    pub fn get_hidden_categories(&self, content_type: &str) -> Result<Vec<String>, DbError> {
        self.select_strings(&format!("get_hidden_categories({content_type})"), "SELECT category FROM hidden_categories WHERE content_type = ? ORDER BY category", &[content_type.into()], "category")
    }

    pub fn set_hidden(&self, category: &str, hidden: bool, content_type: &str) -> Result<bool, DbError> {
        self.set_category_flag("set_hidden", "hidden_categories", category, hidden, content_type)
    }

    /// 카테고리 목록 테이블(`hidden_categories`, `latest_excluded_categories`)에 행을 넣거나 뺀다.
    /// 넣기 실패는 `Ok(false)`, 빼기 실패는 오류다.
    fn set_category_flag(&self, label: &str, table: &str, category: &str, on: bool, content_type: &str) -> Result<bool, DbError> {
        let result = self.tx(|conn| {
            if on {
                conn.execute(&format!("INSERT IGNORE INTO {table} (category, content_type) VALUES (?, ?)"), &[category.into(), content_type.into()])
            } else {
                conn.execute(&format!("DELETE FROM {table} WHERE category = ? AND content_type = ?"), &[category.into(), content_type.into()])
            }
        });
        match (on, result) {
            (_, Ok(_)) => {
                log::info!("{label}({category}, {on}, {content_type}): success");
                Ok(true)
            }
            (true, Err(e)) => {
                log::error!("{label}({category}, {on}, {content_type}) failed: {e}");
                Ok(false)
            }
            (false, Err(e)) => {
                log::error!("{label}({category}, {on}, {content_type}) failed: {e}");
                Err(e)
            }
        }
    }

    /// 최신 자료 검색에서 제외할 카테고리.
    pub fn get_latest_excluded_categories(&self, content_type: &str) -> Result<Vec<String>, DbError> {
        self.select_strings(
            &format!("get_latest_excluded_categories({content_type})"),
            "SELECT category FROM latest_excluded_categories WHERE content_type = ? ORDER BY category",
            &[content_type.into()],
            "category",
        )
    }

    pub fn set_latest_excluded(&self, category: &str, excluded: bool, content_type: &str) -> Result<bool, DbError> {
        self.set_category_flag("set_latest_excluded", "latest_excluded_categories", category, excluded, content_type)
    }

    /// 카테고리명을 바꾼다. 키워드·숨김·최신 제외 테이블을 한 트랜잭션으로 갱신한다.
    pub fn rename_category(&self, old_category: &str, new_category: &str, content_type: &str) -> bool {
        let result = self.tx(|conn| {
            for table in ["category_keywords", "hidden_categories", "latest_excluded_categories"] {
                conn.execute(&format!("UPDATE {table} SET category = ? WHERE category = ? AND content_type = ?"), &[new_category.into(), old_category.into(), content_type.into()])?;
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                log::info!("rename_category({old_category} -> {new_category}, {content_type}): success");
                true
            }
            Err(e) => {
                log::error!("rename_category({old_category} -> {new_category}, {content_type}) failed: {e}");
                false
            }
        }
    }

    // ---- 재적재 락 ----

    fn normalize_reload_source(reload_source: Option<&str>, category: Option<&str>) -> &'static str {
        if category.is_some_and(|c| !c.is_empty()) {
            return RELOAD_SOURCE_MISMATCH;
        }
        match reload_source {
            Some(RELOAD_SOURCE_MISMATCH) => RELOAD_SOURCE_MISMATCH,
            _ => RELOAD_SOURCE_BULK,
        }
    }

    /// 재적재 락을 얻는다. `(얻었는가, 안내 메시지, 막고 있는 작업의 상태)`.
    ///
    /// 카테고리가 있으면 그 카테고리 락과 전체 락만, 없으면 이 content_type의 모든 락을 본다. 다른 카테고리끼리는 걸리지 않는다.
    /// 완료된 행은 마지막 결과 조회를 위해 남기므로 INSERT 실패가 아니라 status/heartbeat를 직접 보고 판단한다.
    /// heartbeat가 `RELOAD_LOCK_HEARTBEAT_STALE_SECONDS` 넘게 멈춘 'running' 행은 죽은 작업으로 보고 무시한다.
    /// DB 오류는 호출자에게 전달한다.
    pub fn acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Row>), DbError> {
        let category = category.filter(|c| !c.is_empty());
        let lock_key = lock_key_of(category);
        let normalized_reload_source = Self::normalize_reload_source(reload_source, category);
        let outcome = self.run(|conn| {
            let rows = if lock_key == BULK_LOCK_KEY {
                conn.fetch_all("SELECT lock_key, status, updated_at FROM reload_locks WHERE content_type = ? FOR UPDATE", &[content_type.into()])?
            } else {
                conn.fetch_all("SELECT lock_key, status, updated_at FROM reload_locks WHERE content_type = ? AND lock_key IN (?, ?) FOR UPDATE", &[content_type.into(), lock_key.into(), BULK_LOCK_KEY.into()])?
            };

            let mut blocking_key: Option<String> = None;
            for row in &rows {
                if text_of(row, "status") != Some("running") {
                    continue;
                }
                if !heartbeat_is_stale(row.get("updated_at")) {
                    blocking_key = text_of(row, "lock_key").map(str::to_string);
                    break;
                }
                log::warn!("acquire_reload_lock({content_type}/{lock_key}): heartbeat({:?}) 정지된 죽은 락({:?}) 감지, 무시하고 진행", row.get("updated_at"), row.get("lock_key"));
            }

            if let Some(blocking_key) = blocking_key {
                conn.commit()?;
                return Ok(Some(blocking_key));
            }

            conn.execute(
                "INSERT INTO reload_locks (content_type, lock_key, category, reload_source, status, started_at, updated_at, indexed_count, deleted_count, failed_count, before_count, after_count, error) \
                 VALUES (?, ?, ?, ?, 'running', NOW(), NOW(), 0, 0, 0, 0, 0, NULL) \
                 ON DUPLICATE KEY UPDATE category = VALUES(category), status = 'running', started_at = NOW(), updated_at = NOW(), \
                 reload_source = VALUES(reload_source), indexed_count = 0, deleted_count = 0, failed_count = 0, before_count = 0, after_count = 0, error = NULL",
                &[content_type.into(), lock_key.into(), category.into(), normalized_reload_source.into()],
            )?;
            conn.commit()?;
            Ok(None)
        });
        match outcome? {
            None => Ok((true, None, None)),
            Some(blocking_key) => {
                let blocking_category = (blocking_key != BULK_LOCK_KEY).then_some(blocking_key.as_str());
                let blocking_status = self.get_reload_status(content_type, blocking_category)?;
                let message = if blocking_key == lock_key {
                    "이미 재적재 작업이 진행 중입니다. 완료 후 다시 시도하세요."
                } else {
                    "다른 재적재 작업이 진행 중이라 지금은 실행할 수 없습니다. 완료 후 다시 시도하세요."
                };
                Ok((false, Some(message.to_string()), blocking_status))
            }
        }
    }

    /// 진행 중 heartbeat(updated_at)와 진행 카운트를 갱신한다. 알 수 없는 카운트 필드나 DB 오류면 `Err`다.
    pub fn heartbeat_reload_lock(&self, content_type: &str, category: Option<&str>, counts: &[(&str, i64)]) -> Result<(), CategoryMappingError> {
        validate_counts(counts)?;
        let lock_key = lock_key_of(category);
        let sql = format!("UPDATE reload_locks SET updated_at = NOW(){} WHERE content_type = ? AND lock_key = ?", count_set_clause(counts));
        let mut params: Vec<DbValue> = counts.iter().map(|(_, n)| (*n).into()).collect();
        params.extend([content_type.into(), lock_key.into()]);
        self.tx(|conn| conn.execute(&sql, &params).map(drop)).map_err(|e| db_error_to_value_error("heartbeat_reload_lock", e))
    }

    /// 재적재 완료(성공/실패)를 기록한다. 행은 지우지 않아 새로고침 뒤에도 마지막 결과를 볼 수 있다. 다음 `acquire_reload_lock`이 덮어쓴다.
    pub fn complete_reload_lock(&self, content_type: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &[(&str, i64)]) -> Result<(), CategoryMappingError> {
        validate_counts(counts)?;
        let lock_key = lock_key_of(category);
        let sql = format!("UPDATE reload_locks SET status = ?, updated_at = NOW(), error = ?{} WHERE content_type = ? AND lock_key = ?", count_set_clause(counts));
        let mut params: Vec<DbValue> = vec![status.into(), error.into()];
        params.extend(counts.iter().map(|(_, n)| DbValue::from(*n)));
        params.extend([content_type.into(), lock_key.into()]);
        self.tx(|conn| conn.execute(&sql, &params).map(drop)).map_err(|e| db_error_to_value_error("complete_reload_lock", e))
    }

    /// 재적재 작업 상태. 카테고리가 있으면 그 락, 없으면 전체 락을 본다. 이력이 없으면 `None`이다.
    ///
    /// 'running'인데 heartbeat가 끊겼으면 행은 두고 조회 결과에서만 'failed'로 보여준다. 정리는 다음 `acquire_reload_lock`이 한다.
    pub fn get_reload_status(&self, content_type: &str, category: Option<&str>) -> Result<Option<Row>, DbError> {
        let lock_key = lock_key_of(category);
        let fetched = self.run(|conn| {
            conn.fetch_one(
                "SELECT category, reload_source, status, started_at, updated_at, indexed_count, deleted_count, failed_count, before_count, after_count, error FROM reload_locks WHERE content_type = ? AND lock_key = ?",
                &[content_type.into(), lock_key.into()],
            )
        });
        let row = match fetched? {
            Some(row) => row,
            None => return Ok(None),
        };
        let mut status = text_of(&row, "status").unwrap_or_default().to_string();
        let mut error = row.get("error").cloned().unwrap_or(DbValue::Null);
        if status == "running" && heartbeat_is_stale(row.get("updated_at")) {
            status = "failed".to_string();
            if is_empty_value(Some(&error)) {
                error = STALE_ERROR_MESSAGE.into();
            }
        }
        let row_category = text_of(&row, "category");
        let mut out = Row::new();
        out.insert("category".into(), row.get("category").cloned().unwrap_or(DbValue::Null));
        out.insert("reload_source".into(), Self::normalize_reload_source(text_of(&row, "reload_source"), row_category).into());
        out.insert("status".into(), status.into());
        out.insert("started_at".into(), iso_or_null(row.get("started_at")));
        out.insert("updated_at".into(), iso_or_null(row.get("updated_at")));
        for field in PROGRESS_COUNT_FIELDS {
            out.insert(field.into(), row.get(field).cloned().unwrap_or(DbValue::Null));
        }
        out.insert("error".into(), error);
        Ok(Some(out))
    }

    /// 락 행을 완전히 지운다. 일반 완료 경로는 `complete_reload_lock`이다.
    pub fn release_reload_lock(&self, content_type: &str, category: Option<&str>) -> Result<(), DbError> {
        let lock_key = lock_key_of(category);
        self.tx(|conn| conn.execute("DELETE FROM reload_locks WHERE content_type = ? AND lock_key = ?", &[content_type.into(), lock_key.into()]).map(drop))
    }

    // ---- 분류 제안 항목 ----

    /// 새 분류 제안을 시작할 때 이 content_type의 기존 항목을 모두 지운다. 이전 제안의 행이 섞여 승인되는 일을 막는다.
    pub fn clear_classify_proposal_items(&self, content_type: &str) -> Result<(), DbError> {
        self.tx(|conn| conn.execute("DELETE FROM classify_proposal_items WHERE content_type = ?", &[content_type.into()]).map(drop))
    }

    /// 분류된 항목을 `execute_batch` 한 번과 커밋 한 번으로 적재한다. 비어 있으면 DB에 가지 않는다.
    pub fn add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str) -> Result<(), DbError> {
        if items.is_empty() {
            return Ok(());
        }
        let rows: Vec<Vec<DbValue>> =
            items.iter().map(|item| vec![content_type.into(), source_category.into(), item.get("file_path").and_then(Value::as_str).into(), json_string(item).into()]).collect();
        self.tx(|conn| conn.execute_batch("INSERT INTO classify_proposal_items (content_type, source_category, file_path, payload) VALUES (?, ?, ?, ?)", &rows).map(drop))
    }

    /// 항목을 분류가 끝난 순서(id 오름차순)로 돌려준다. `apply_status`/`apply_error`는 컬럼 값을 payload 위에 덮어 싣는다.
    pub fn get_classify_proposal_items(&self, content_type: &str) -> Result<Vec<Value>, DbError> {
        let rows = self.select("SELECT payload, apply_status, apply_error FROM classify_proposal_items WHERE content_type = ? ORDER BY id ASC", &[content_type.into()])?;
        let mut items = Vec::with_capacity(rows.len());
        for row in &rows {
            let payload = text_of(row, "payload").ok_or_else(|| DbError::Other("classify proposal payload is missing".into()))?;
            let mut item: Value = serde_json::from_str(payload).map_err(|error| DbError::Other(format!("invalid classify proposal payload: {error}")))?;
            let object = item.as_object_mut().ok_or_else(|| DbError::Other("classify proposal payload must be a JSON object".into()))?;
            object.insert("apply_status".into(), text_of(row, "apply_status").map_or(Value::Null, Value::from));
            object.insert("apply_error".into(), text_of(row, "apply_error").map_or(Value::Null, Value::from));
            items.push(item);
        }
        Ok(items)
    }

    /// 분류가 끝난 항목의 payload를 덮어쓴다. `apply_status`는 승인 작업이 쓰는 컬럼이라 건드리지 않는다.
    pub fn update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str) -> Result<(), DbError> {
        if items.is_empty() {
            return Ok(());
        }
        let rows: Vec<Vec<DbValue>> = items.iter().map(|item| vec![json_string(item).into(), content_type.into(), item.get("file_path").and_then(Value::as_str).into()]).collect();
        self.tx(|conn| conn.execute_batch("UPDATE classify_proposal_items SET payload = ? WHERE content_type = ? AND file_path = ?", &rows).map(drop))
    }

    /// 파일 하나를 옮긴 직후 그 행의 이동 상태만 바로 기록한다. 중단 후 재개하려면 건별 기록이 필요하다.
    pub fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, content_type: &str) -> Result<(), DbError> {
        self.tx(|conn| {
            conn.execute(
                "UPDATE classify_proposal_items SET apply_status = ?, apply_error = ? WHERE content_type = ? AND file_path = ?",
                &[apply_status.into(), apply_error.into(), content_type.into(), file_path.into()],
            )
        })
        .map(drop)
    }

    /// 이동이 끝난(`apply_status = 'moved'`) 행만 지우고 지운 개수를 돌려준다. 대기·실패 행은 재시도를 위해 남긴다.
    pub fn delete_applied_classify_proposal_items(&self, content_type: &str) -> Result<u64, DbError> {
        self.tx(|conn| conn.execute("DELETE FROM classify_proposal_items WHERE content_type = ? AND apply_status = 'moved'", &[content_type.into()]))
    }

    /// 제안 항목 한 건을 지우고 지운 개수를 돌려준다. 표에서 책을 지우면 그 행도 없애야 이미 없는 파일의 행이 되살아나지 않는다.
    pub fn delete_classify_proposal_item(&self, file_path: &str, content_type: &str) -> Result<u64, DbError> {
        self.tx(|conn| conn.execute("DELETE FROM classify_proposal_items WHERE content_type = ? AND file_path = ?", &[content_type.into(), file_path.into()]))
    }

    // ---- 분류 제안 작업 상태 ----
    //
    // 여러 프로세스(웹 서버 worker x replica)가 같이 쓰므로 DB 행 + SELECT ... FOR UPDATE로 판단과 쓰기를 묶는다. reload_locks와 같은 방식이다.

    /// 저장된 상태에 '갱신이 끊겼는가'를 반영해 돌려준다. 행은 고치지 않는다(조회가 쓰기가 되면 폴링끼리 경합한다).
    /// 낡음은 앱이 아니라 SQL이 센다(`TIMESTAMPDIFF ... NOW(3)`). 앱(KST)과 DB(UTC)의 시간대 차이를 피하려는 것이다.
    fn classify_proposal_effective_status(status: &str, payload: Value, age_seconds: i64, stale_seconds: i64) -> Value {
        let mut payload = match payload {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        payload.insert("status".into(), status.into());
        if matches!(status, "running" | "applying") && age_seconds > stale_seconds {
            payload.insert("status".into(), "failed".into());
            payload.insert("error".into(), STALE_CLASSIFY_PROPOSAL_ERROR_MESSAGE.into());
        }
        Value::Object(payload)
    }

    fn read_classify_proposal_status_row(conn: &mut dyn DbConnection, sql: &str, content_type: &str) -> Result<Option<(String, Value, i64)>, DbError> {
        match conn.fetch_one(sql, &[content_type.into()])? {
            Some(row) => {
                let status = text_of(&row, "status").unwrap_or("idle").to_string();
                let payload = match text_of(&row, "payload") {
                    Some(payload) => serde_json::from_str::<Value>(payload).map_err(|error| DbError::Other(format!("invalid classify proposal status payload: {error}")))?,
                    None => Value::Null,
                };
                // updated_at이 없거나 깨졌으면 오래된 것으로 본다. SQL TIMESTAMPDIFF는 없는 시각에 NULL을 돌려준다.
                let age = row.get("age_seconds").and_then(DbValue::as_i64).unwrap_or(i64::MAX);
                Ok(Some((status, payload, age)))
            }
            None => Ok(None),
        }
    }

    /// 분류 제안 작업의 현재 상태. 작업이 없으면 `{"status": "idle", "content_type": ...}`다.
    pub fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, DbError> {
        let fetched = self.run(|conn| {
            Self::read_classify_proposal_status_row(
                conn,
                "SELECT status, payload, TIMESTAMPDIFF(SECOND, updated_at, NOW(3)) AS age_seconds FROM classify_proposal_status WHERE content_type = ?",
                content_type,
            )
        });
        match fetched? {
            Some((status, payload, age)) => Ok(Self::classify_proposal_effective_status(&status, payload, age, stale_seconds)),
            None => Ok(serde_json::json!({"status": "idle", "content_type": content_type})),
        }
    }

    /// 상태를 통째로 바꾼다. status 컬럼과 payload 안의 status를 함께 맞춘다.
    pub fn set_classify_proposal_status(&self, status: &Value, content_type: &str) -> Result<(), DbError> {
        let mut payload = status.as_object().cloned().unwrap_or_default();
        payload.insert("content_type".into(), content_type.into());
        let column_status = payload.get("status").and_then(Value::as_str).unwrap_or("idle").to_string();
        let payload_json = json_string(&Value::Object(payload));
        self.tx(|conn| {
            conn.execute(
                "INSERT INTO classify_proposal_status (content_type, status, payload, updated_at) VALUES (?, ?, ?, NOW(3)) ON DUPLICATE KEY UPDATE status = VALUES(status), payload = VALUES(payload), updated_at = NOW(3)",
                &[content_type.into(), column_status.as_str().into(), payload_json.as_str().into()],
            )
        }).map(drop)
    }

    /// 진행률처럼 일부 필드만 갱신한다. `status`는 절대 바꾸지 않는다. 서버가 `JSON_MERGE_PATCH`로 합치므로 읽기-쓰기 창이 없다.
    pub fn merge_classify_proposal_status(&self, fields: &Value, content_type: &str) -> Result<(), DbError> {
        let patch: Map<String, Value> = fields.as_object().map(|m| m.iter().filter(|(k, _)| k.as_str() != "status").map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
        if patch.is_empty() {
            return Ok(());
        }
        let patch_json = json_string(&Value::Object(patch));
        self.tx(|conn| {
            conn.execute("UPDATE classify_proposal_status SET payload = JSON_MERGE_PATCH(payload, ?), updated_at = NOW(3) WHERE content_type = ?", &[patch_json.as_str().into(), content_type.into()])
        })
        .map(drop)
    }

    /// 이미 도는 작업이 없을 때만 새 제안 작업을 시작 상태로 만든다. `(시작했는가, 현재 상태)`.
    /// 실패하면 현재 상태는 이미 도는 작업의 것이다. 판단과 쓰기가 `FOR UPDATE` 한 트랜잭션이라 프로세스가 몇 개든 둘이 동시에 시작하지 못한다.
    pub fn try_start_classify_proposal(&self, status: &Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), DbError> {
        self.try_transition_classify_proposal(
            |current| {
                if matches!(current, "running" | "applying") {
                    return None;
                }
                let mut next = status.as_object().cloned().unwrap_or_default();
                next.insert("content_type".into(), content_type.into());
                Some(Value::Object(next))
            },
            content_type,
            stale_seconds,
            false,
        )
    }

    /// 승인할 제안이 있을 때만 적용 중 상태를 선점하고 토큰을 발급한다.
    /// ready뿐 아니라 done과 failed에서도 받는다. 승인 도중 중단돼 failed로 굳어도 남은 pending 행을 이어서 승인할 수 있어야 한다.
    pub fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), DbError> {
        self.try_transition_classify_proposal(
            |current| {
                if !matches!(current, "ready" | "done" | "failed") {
                    return None;
                }
                Some(serde_json::json!({"status": "applying", "apply_token": apply_token}))
            },
            content_type,
            stale_seconds,
            true,
        )
    }

    /// 잠글 행을 먼저 만든다. 반드시 별도 트랜잭션이어야 한다.
    /// 같은 트랜잭션에서 `INSERT IGNORE` 직후 `SELECT ... FOR UPDATE`를 하면 두 프로세스가 공유 잠금을 쥔 채 배타 잠금으로 올리려다 InnoDB 데드락이 난다.
    fn ensure_classify_proposal_status_row(&self, content_type: &str) -> Result<(), DbError> {
        let initial = json_string(&serde_json::json!({"status": "idle", "content_type": content_type}));
        self.tx(|conn| conn.execute("INSERT IGNORE INTO classify_proposal_status (content_type, status, payload) VALUES (?, 'idle', ?)", &[content_type.into(), initial.as_str().into()])).map(drop)
    }

    fn try_transition_classify_proposal(&self, next_status: impl FnOnce(&str) -> Option<Value>, content_type: &str, stale_seconds: i64, merge_into_current: bool) -> Result<(bool, Value), DbError> {
        self.ensure_classify_proposal_status_row(content_type)?;
        self.run(|conn| {
            let row = Self::read_classify_proposal_status_row(
                conn,
                "SELECT status, payload, TIMESTAMPDIFF(SECOND, updated_at, NOW(3)) AS age_seconds FROM classify_proposal_status WHERE content_type = ? FOR UPDATE",
                content_type,
            )?;
            let (status, payload, age) = row.unwrap_or_else(|| ("idle".to_string(), Value::Null, 0));
            let current = Self::classify_proposal_effective_status(&status, payload, age, stale_seconds);
            let current_status = current.get("status").and_then(Value::as_str).unwrap_or("idle").to_string();

            let Some(decided) = next_status(&current_status) else {
                conn.commit()?;
                return Ok((false, current));
            };

            let mut next = match (merge_into_current, current, decided) {
                (true, Value::Object(mut base), Value::Object(over)) => {
                    base.extend(over);
                    base
                }
                (_, _, Value::Object(over)) => over,
                _ => Map::new(),
            };
            next.insert("content_type".into(), content_type.into());
            let next_status_column = next.get("status").and_then(Value::as_str).unwrap_or("idle").to_string();
            let next_json = json_string(&Value::Object(next.clone()));
            conn.execute(
                "UPDATE classify_proposal_status SET status = ?, payload = ?, updated_at = NOW(3) WHERE content_type = ?",
                &[next_status_column.as_str().into(), next_json.as_str().into(), content_type.into()],
            )?;
            conn.commit()?;
            Ok((true, Value::Object(next)))
        })
    }
}

/// 카테고리가 숨긴 카테고리이거나 그 하위면 참이다. 모든 viewer 접근 검사가 이 판정 하나를 쓴다.
pub fn is_hidden_category(category: &str, hidden: &[String]) -> bool {
    !category.is_empty() && hidden.iter().any(|h| category == h || category.strip_prefix(h.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

// ---------------------------------------------------------------------------
// 라우트용 facade: JSON 값으로 주고받는 `CategoryMappingApi`와 그 구현
// ---------------------------------------------------------------------------

/// 카테고리 매핑 facade의 오류. `Http`는 라우트가 그대로 응답으로 바꾼다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingError {
    Http(u16, String),
    Other(String),
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MappingError::Http(status, detail) => write!(f, "{status}: {detail}"),
            MappingError::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for MappingError {}

fn unimplemented_mapping<T>() -> Result<T, MappingError> {
    Err(MappingError::Other(UNIMPLEMENTED.to_string()))
}

fn db_row_to_json(row: Row) -> Value {
    Value::Object(row.iter().map(|(key, value)| (key.clone(), Value::from(value))).collect())
}

fn reload_counts(value: &Value) -> Result<Vec<(&str, i64)>, String> {
    value
        .as_object()
        .ok_or_else(|| "재적재 진행 수치가 객체가 아닙니다".to_string())?
        .iter()
        .map(|(key, value)| value.as_i64().map(|count| (key.as_str(), count)).ok_or_else(|| format!("재적재 진행 수치가 정수가 아닙니다: {key}")))
        .collect()
}

/// 카테고리 매핑 facade. 동기 메서드다. 지금은 viewer 접근 제어에 필요한 조회만 있다.
pub trait CategoryMappingApi: Send + Sync {
    fn acquire_reload_lock(&self, _content_type: &str, _category: Option<&str>, _reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        unimplemented_mapping()
    }
    fn heartbeat_reload_lock(&self, _content_type: &str, _category: Option<&str>, _counts: &Value) -> Result<(), String> {
        Err(UNIMPLEMENTED.into())
    }
    fn complete_reload_lock(&self, _content_type: &str, _status: &str, _error: Option<&str>, _category: Option<&str>, _counts: &Value) -> Result<(), String> {
        Err(UNIMPLEMENTED.into())
    }
    fn get_reload_status(&self, _content_type: &str, _category: Option<&str>) -> Result<Option<Value>, MappingError> {
        unimplemented_mapping()
    }
    fn rename_category(&self, _old: &str, _new: &str, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    /// 카테고리 → 키워드 목록.
    fn get_all_mappings(&self, _content_type: &str) -> Result<BTreeMap<String, Vec<String>>, MappingError> {
        unimplemented_mapping()
    }
    fn get_keywords(&self, _category: &str, _content_type: &str) -> Result<Vec<String>, MappingError> {
        unimplemented_mapping()
    }
    fn set_keywords(&self, _category: &str, _keywords: &[String], _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn add_keyword(&self, _category: &str, _keyword: &str, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn remove_keyword(&self, _category: &str, _keyword: &str, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn delete_category(&self, _category: &str, _content_type: &str, _prefix: bool) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn update_all_mappings(&self, _mappings: &Value, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn set_hidden(&self, _category: &str, _hidden: bool, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn set_latest_excluded(&self, _category: &str, _excluded: bool, _content_type: &str) -> Result<bool, MappingError> {
        unimplemented_mapping()
    }
    fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        unimplemented_mapping()
    }

    fn get_latest_excluded_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        unimplemented_mapping()
    }

    fn get_classify_proposal_status(&self, _content_type: &str, _stale_seconds: i64) -> Result<Value, MappingError> {
        unimplemented_mapping()
    }
    fn set_classify_proposal_status(&self, _status: Value, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn merge_classify_proposal_status(&self, _fields: Value, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn try_start_classify_proposal(&self, _status: Value, content_type: &str, _stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        Ok((false, self.get_classify_proposal_status(content_type, 900)?))
    }
    fn try_begin_classify_apply(&self, _apply_token: &str, content_type: &str, _stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        Ok((false, self.get_classify_proposal_status(content_type, 900)?))
    }
    fn clear_classify_proposal_items(&self, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn get_classify_proposal_items(&self, _content_type: &str) -> Result<Vec<Value>, MappingError> {
        unimplemented_mapping()
    }
    fn delete_classify_proposal_item(&self, _file_path: &str, _content_type: &str) -> Result<u64, MappingError> {
        unimplemented_mapping()
    }
    fn delete_applied_classify_proposal_items(&self, _content_type: &str) -> Result<u64, MappingError> {
        unimplemented_mapping()
    }
    fn add_classify_proposal_items(&self, _items: &[Value], _source_category: &str, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn update_classify_proposal_item_payloads(&self, _items: &[Value], _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
    fn update_classify_proposal_item_status(&self, _file_path: &str, _apply_status: &str, _apply_error: Option<&str>, _content_type: &str) -> Result<(), MappingError> {
        unimplemented_mapping()
    }
}

impl CategoryMappingApi for CategoryMapping {
    fn acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        CategoryMapping::acquire_reload_lock(self, content_type, category, reload_source)
            .map(|(acquired, error, status)| (acquired, error, status.map(db_row_to_json)))
            .map_err(|e| MappingError::Other(e.to_string()))
    }

    fn heartbeat_reload_lock(&self, content_type: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let counts = reload_counts(counts)?;
        CategoryMapping::heartbeat_reload_lock(self, content_type, category, &counts).map_err(|error| error.to_string())
    }

    fn complete_reload_lock(&self, content_type: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let counts = reload_counts(counts)?;
        CategoryMapping::complete_reload_lock(self, content_type, status, error, category, &counts).map_err(|error| error.to_string())
    }

    fn get_reload_status(&self, content_type: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        CategoryMapping::get_reload_status(self, content_type, category).map(|row| row.map(db_row_to_json)).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn rename_category(&self, old: &str, new: &str, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::rename_category(self, old, new, content_type))
    }
    fn get_all_mappings(&self, content_type: &str) -> Result<BTreeMap<String, Vec<String>>, MappingError> {
        CategoryMapping::get_all_mappings(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn get_keywords(&self, category: &str, content_type: &str) -> Result<Vec<String>, MappingError> {
        CategoryMapping::get_keywords(self, category, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn set_keywords(&self, category: &str, keywords: &[String], content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::set_keywords(self, category, &keywords.iter().map(String::as_str).collect::<Vec<_>>(), content_type))
    }
    fn add_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::add_keyword(self, category, keyword, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn remove_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::remove_keyword(self, category, keyword, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn delete_category(&self, category: &str, content_type: &str, prefix: bool) -> Result<bool, MappingError> {
        CategoryMapping::delete_category(self, category, content_type, prefix).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn update_all_mappings(&self, mappings: &Value, content_type: &str) -> Result<bool, MappingError> {
        let mappings = serde_json::from_value(mappings.clone()).map_err(|e| MappingError::Other(e.to_string()))?;
        Ok(CategoryMapping::update_all_mappings(self, &mappings, content_type))
    }
    fn set_hidden(&self, category: &str, hidden: bool, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::set_hidden(self, category, hidden, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn set_latest_excluded(&self, category: &str, excluded: bool, content_type: &str) -> Result<bool, MappingError> {
        CategoryMapping::set_latest_excluded(self, category, excluded, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn get_hidden_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
        CategoryMapping::get_hidden_categories(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }

    fn get_latest_excluded_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
        CategoryMapping::get_latest_excluded_categories(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        CategoryMapping::get_classify_proposal_status(self, content_type, stale_seconds).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn set_classify_proposal_status(&self, status: Value, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::set_classify_proposal_status(self, &status, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::merge_classify_proposal_status(self, &fields, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        CategoryMapping::try_start_classify_proposal(self, &status, content_type, stale_seconds).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        CategoryMapping::try_begin_classify_apply(self, apply_token, content_type, stale_seconds).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn clear_classify_proposal_items(&self, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::clear_classify_proposal_items(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn get_classify_proposal_items(&self, content_type: &str) -> Result<Vec<Value>, MappingError> {
        CategoryMapping::get_classify_proposal_items(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn delete_classify_proposal_item(&self, file_path: &str, content_type: &str) -> Result<u64, MappingError> {
        CategoryMapping::delete_classify_proposal_item(self, file_path, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn delete_applied_classify_proposal_items(&self, content_type: &str) -> Result<u64, MappingError> {
        CategoryMapping::delete_applied_classify_proposal_items(self, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::add_classify_proposal_items(self, items, source_category, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::update_classify_proposal_item_payloads(self, items, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, content_type: &str) -> Result<(), MappingError> {
        CategoryMapping::update_classify_proposal_item_status(self, file_path, apply_status, apply_error, content_type).map_err(|e| MappingError::Other(e.to_string()))
    }
}

#[cfg(test)]
mod facade_tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    struct EmptyMapping;

    impl CategoryMappingApi for EmptyMapping {}

    #[test]
    fn category_mapping_defaults_report_unimplemented() {
        let mapping = EmptyMapping;
        let counts = json!({"indexed_count": 1});
        let item = json!({"path": "/books/a.epub"});

        assert!(mapping.acquire_reload_lock("book", None, None).is_err());
        assert_eq!(mapping.heartbeat_reload_lock("book", None, &counts).unwrap_err(), UNIMPLEMENTED);
        assert_eq!(mapping.complete_reload_lock("book", "done", None, None, &counts).unwrap_err(), UNIMPLEMENTED);
        assert!(mapping.get_reload_status("book", None).is_err());
        assert!(mapping.rename_category("old", "new", "book").is_err());
        assert!(mapping.get_all_mappings("book").is_err());
        assert!(mapping.get_keywords("fiction", "book").is_err());
        assert!(mapping.set_keywords("fiction", &[], "book").is_err());
        assert!(mapping.add_keyword("fiction", "word", "book").is_err());
        assert!(mapping.remove_keyword("fiction", "word", "book").is_err());
        assert!(mapping.delete_category("fiction", "book", false).is_err());
        assert!(mapping.update_all_mappings(&json!({}), "book").is_err());
        assert!(mapping.set_hidden("fiction", true, "book").is_err());
        assert!(mapping.set_latest_excluded("fiction", true, "book").is_err());
        assert!(mapping.get_hidden_categories("book").is_err());
        assert!(mapping.get_latest_excluded_categories("book").is_err());
        assert!(mapping.get_classify_proposal_status("book", 60).is_err());
        assert!(mapping.set_classify_proposal_status(json!({}), "book").is_err());
        assert!(mapping.merge_classify_proposal_status(json!({}), "book").is_err());
        assert!(mapping.try_start_classify_proposal(json!({}), "book", 60).is_err());
        assert!(mapping.try_begin_classify_apply("token", "book", 60).is_err());
        assert!(mapping.clear_classify_proposal_items("book").is_err());
        assert!(mapping.get_classify_proposal_items("book").is_err());
        assert!(mapping.delete_classify_proposal_item("/a", "book").is_err());
        assert!(mapping.delete_applied_classify_proposal_items("book").is_err());
        assert!(mapping.add_classify_proposal_items(std::slice::from_ref(&item), "old", "book").is_err());
        assert!(mapping.update_classify_proposal_item_payloads(&[item], "book").is_err());
        assert!(mapping.update_classify_proposal_item_status("/a", "failed", None, "book").is_err());
    }

    #[test]
    fn mapping_errors_format_http_and_other_messages() {
        assert_eq!(MappingError::Http(409, "busy".into()).to_string(), "409: busy");
        assert_eq!(MappingError::Other("broken".into()).to_string(), "broken");
    }

    #[test]
    fn row_and_reload_progress_converters_cover_all_value_types_and_errors() {
        let mut row = Row::new();
        row.insert("null".into(), DbValue::Null);
        row.insert("number".into(), DbValue::Int(7));
        row.insert("text".into(), DbValue::Text("value".into()));
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap().and_hms_opt(1, 2, 3).unwrap();
        row.insert("date".into(), DbValue::DateTime(date));
        assert_eq!(db_row_to_json(row), json!({"null": null, "number": 7, "text": "value", "date": "2026-10-27 01:02:03"}));

        assert_eq!(reload_counts(&json!({"done": 2, "failed": 0})).unwrap(), vec![("done", 2), ("failed", 0)]);
        assert!(reload_counts(&json!([])).unwrap_err().contains("객체가 아닙니다"));
        assert!(reload_counts(&json!({"done": "2"})).unwrap_err().contains("정수가 아닙니다: done"));
    }

    #[test]
    fn concrete_category_mapping_adapter_forwards_database_errors() {
        let mapping = CategoryMapping::with_connection_provider(Arc::new(|| Err(DbError::Other("offline".into()))));
        let counts = json!({"indexed_count": 1});
        let item = json!({"file_path": "/books/a.epub"});

        assert!(CategoryMappingApi::acquire_reload_lock(&mapping, "book", None, None).is_err());
        assert!(CategoryMappingApi::heartbeat_reload_lock(&mapping, "book", None, &counts).unwrap_err().contains("offline"));
        assert!(CategoryMappingApi::complete_reload_lock(&mapping, "book", "done", None, None, &counts).unwrap_err().contains("offline"));
        assert!(CategoryMappingApi::get_reload_status(&mapping, "book", None).is_err());
        assert!(!CategoryMappingApi::rename_category(&mapping, "old", "new", "book").unwrap());
        assert!(CategoryMappingApi::get_all_mappings(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_keywords(&mapping, "fiction", "book").is_err());
        assert!(!CategoryMappingApi::set_keywords(&mapping, "fiction", &[], "book").unwrap());
        assert!(CategoryMappingApi::add_keyword(&mapping, "fiction", "word", "book").is_err());
        assert!(CategoryMappingApi::remove_keyword(&mapping, "fiction", "word", "book").is_err());
        assert!(CategoryMappingApi::delete_category(&mapping, "fiction", "book", false).is_err());
        assert!(!CategoryMappingApi::update_all_mappings(&mapping, &json!({}), "book").unwrap());
        assert!(!CategoryMappingApi::set_hidden(&mapping, "fiction", true, "book").unwrap());
        assert!(!CategoryMappingApi::set_latest_excluded(&mapping, "fiction", true, "book").unwrap());
        assert!(CategoryMappingApi::get_hidden_categories(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_latest_excluded_categories(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_classify_proposal_status(&mapping, "book", 60).is_err());
        assert!(CategoryMappingApi::set_classify_proposal_status(&mapping, json!({}), "book").is_err());
        assert!(CategoryMappingApi::merge_classify_proposal_status(&mapping, json!({"progress": 1}), "book").is_err());
        assert!(CategoryMappingApi::try_start_classify_proposal(&mapping, json!({}), "book", 60).is_err());
        assert!(CategoryMappingApi::try_begin_classify_apply(&mapping, "token", "book", 60).is_err());
        assert!(CategoryMappingApi::clear_classify_proposal_items(&mapping, "book").is_err());
        assert!(CategoryMappingApi::get_classify_proposal_items(&mapping, "book").is_err());
        assert!(CategoryMappingApi::delete_classify_proposal_item(&mapping, "/a", "book").is_err());
        assert!(CategoryMappingApi::delete_applied_classify_proposal_items(&mapping, "book").is_err());
        assert!(CategoryMappingApi::add_classify_proposal_items(&mapping, std::slice::from_ref(&item), "old", "book").is_err());
        assert!(CategoryMappingApi::update_classify_proposal_item_payloads(&mapping, &[item], "book").is_err());
        assert!(CategoryMappingApi::update_classify_proposal_item_status(&mapping, "/a", "failed", None, "book").is_err());
    }
}
