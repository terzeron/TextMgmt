//! 사용자별 최근 조회 이력 저장소. Python `backend/view_history_store.py`의 Rust 대응이다.
//!
//! 운영은 k8s 멀티 replica라 pod 로컬 저장은 이력이 파편화되므로 MySQL 전용이다.
//! 제목·카테고리는 조회 시점의 스냅샷으로 저장한다. 설계: docs/rust-test-seams.md 3.4

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde_json::{Value, json};
use sysdeps::{Clock, EnvFn, SystemClock};

use crate::db::{ConnectionProvider, DbConnection, DbError, DbValue, MysqlConfig, mysql_provider};

/// 사용자·콘텐츠 유형별로 보관하는 최근 조회 건수. 초과분은 기록 시점에 정리하므로
/// 테이블 크기가 (사용자 수 × 유형 2 × 이 값)으로 고정되고 별도 purge가 필요 없다.
pub const MAX_RECENT_VIEWS: usize = 50;

pub const CONTENT_TYPES: [&str; 2] = ["book", "comic"];

/// 조회 목록 전체를 읽을 때의 방어적 상한. `record_view`가 사용자별로 정리하므로 정상 상태에서는 도달하지 않는다.
pub const MAX_HISTORY_ROWS: usize = 20000;

const CREATE_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS view_history (
        id BIGINT AUTO_INCREMENT PRIMARY KEY,
        email VARCHAR(320) NOT NULL,
        content_type VARCHAR(10) NOT NULL,
        book_id INT NOT NULL,
        title VARCHAR(512) NOT NULL,
        category VARCHAR(255) NOT NULL DEFAULT '',
        viewed_at BIGINT NOT NULL,
        UNIQUE KEY uniq_view_history (email, content_type, book_id),
        INDEX idx_view_history_recent (email, content_type, viewed_at)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
";

/// 같은 책을 다시 열면 새 row를 만들지 않고 조회 시각과 스냅샷만 갱신한다.
const UPSERT_SQL: &str = "
    INSERT INTO view_history (email, content_type, book_id, title, category, viewed_at)
    VALUES (%s, %s, %s, %s, %s, %s)
    ON DUPLICATE KEY UPDATE
        title = VALUES(title),
        category = VALUES(category),
        viewed_at = VALUES(viewed_at)
";

/// 최신 N건만 남기고 나머지를 지운다. MySQL은 IN 서브쿼리에 LIMIT을 직접 쓸 수 없어 파생 테이블로 한 번 감싼다.
const TRIM_SQL: &str = "
    DELETE FROM view_history
    WHERE email = %s AND content_type = %s AND id NOT IN (
        SELECT id FROM (
            SELECT id FROM view_history
            WHERE email = %s AND content_type = %s
            ORDER BY viewed_at DESC, id DESC
            LIMIT %s
        ) AS keep
    )
";

const SELECT_ALL_SQL: &str = "
    SELECT email, content_type, book_id, title, category, viewed_at
    FROM view_history
    ORDER BY viewed_at DESC, id DESC
    LIMIT %s
";

/// Python의 `ValueError`와 DB 예외(`RuntimeError`)에 대응한다.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewHistoryError {
    ValueError(String),
    RuntimeError(String),
}

impl fmt::Display for ViewHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewHistoryError::ValueError(msg) | ViewHistoryError::RuntimeError(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ViewHistoryError {}

impl From<DbError> for ViewHistoryError {
    fn from(e: DbError) -> Self {
        match e {
            DbError::IntegrityError(msg) | DbError::Other(msg) => ViewHistoryError::RuntimeError(msg),
        }
    }
}

pub struct ViewHistoryStore {
    provider: ConnectionProvider,
    max_history_rows: usize,
    clock: Arc<dyn Clock>,
}

impl ViewHistoryStore {
    /// `TM_MYSQL_*` 환경 변수로 접속하고 테이블을 만든다.
    pub fn new() -> Result<Self, ViewHistoryError> {
        Self::from_env(&|key| std::env::var(key).ok())
    }

    pub fn from_env(env: EnvFn) -> Result<Self, ViewHistoryError> {
        let store = Self::with_connection_provider(mysql_provider(MysqlConfig::from_env(env, "textmanager")));
        store.init_db()?;
        Ok(store)
    }

    /// 주입한 연결 제공자를 쓴다. `init_db`는 부르지 않는다.
    pub fn with_connection_provider(provider: ConnectionProvider) -> Self {
        Self { provider, max_history_rows: MAX_HISTORY_ROWS, clock: Arc::new(SystemClock) }
    }

    pub fn with_max_history_rows(mut self, max_history_rows: usize) -> Self {
        self.max_history_rows = max_history_rows;
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn get_connection(&self) -> Result<Box<dyn DbConnection>, DbError> {
        (self.provider)()
    }

    fn init_db(&self) -> Result<(), ViewHistoryError> {
        let mut conn = self.get_connection()?;
        let result = (|| {
            conn.cursor().execute(CREATE_TABLE_SQL, &[])?;
            conn.commit()
        })();
        conn.close();
        Ok(result?)
    }

    /// 조회 1건을 기록하고 사용자·유형별 보관 상한을 유지한다. 실패하면 롤백하고 오류를 돌려준다.
    pub fn record_view(&self, email: &str, content_type: &str, book_id: i64, title: &str, category: &str) -> Result<(), ViewHistoryError> {
        if !CONTENT_TYPES.contains(&content_type) {
            return Err(ViewHistoryError::ValueError(format!("unknown content_type: {content_type}")));
        }
        let now = self.clock.now() as i64;
        let mut conn = self.get_connection()?;
        let result = (|| {
            let mut cur = conn.cursor();
            cur.execute(UPSERT_SQL, &[email.into(), content_type.into(), book_id.into(), title.into(), category.into(), now.into()])?;
            cur.execute(TRIM_SQL, &[email.into(), content_type.into(), email.into(), content_type.into(), (MAX_RECENT_VIEWS as i64).into()])?;
            drop(cur);
            conn.commit()
        })();
        if result.is_err() {
            // 롤백 실패는 원래 오류를 가리지 않도록 버린다.
            let _ = conn.rollback();
        }
        conn.close();
        Ok(result?)
    }

    /// 사용자별로 책/만화 최근 조회 목록을 돌려준다. 사용자는 마지막 조회가 최근인 순서다.
    pub fn list_recent_views(&self, limit: usize) -> Result<Value, ViewHistoryError> {
        let mut conn = self.get_connection()?;
        let result = (|| {
            let mut cur = conn.cursor();
            cur.execute(SELECT_ALL_SQL, &[(self.max_history_rows as i64).into()])?;
            Ok::<_, DbError>(cur.fetchall().unwrap_or_default())
        })();
        conn.close();
        let rows = result?;
        if rows.len() >= self.max_history_rows {
            log::warn!("조회 이력이 상한({})에 도달해 잘렸습니다.", self.max_history_rows);
        }

        struct User {
            email: String,
            last_viewed_at: i64,
            book: Vec<Value>,
            comic: Vec<Value>,
        }
        let mut users: Vec<User> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for row in &rows {
            let text = |key: &str| row.get(key).and_then(DbValue::as_str).unwrap_or("").to_string();
            let number = |key: &str| row.get(key).and_then(DbValue::as_i64).unwrap_or(0);
            let email = text("email");
            let slot = *index.entry(email.clone()).or_insert_with(|| {
                users.push(User { email, last_viewed_at: 0, book: Vec::new(), comic: Vec::new() });
                users.len() - 1
            });
            let user = &mut users[slot];
            let content_type = text("content_type");
            let is_book = match content_type.as_str() {
                "book" => true,
                "comic" => false,
                // 알 수 없는 content_type은 조용히 버리지 않고 남긴다.
                _ => {
                    log::warn!("알 수 없는 content_type 이력을 건너뜁니다: {content_type}");
                    continue;
                }
            };
            let viewed_at = number("viewed_at");
            user.last_viewed_at = user.last_viewed_at.max(viewed_at);
            let bucket = if is_book { &mut user.book } else { &mut user.comic };
            if bucket.len() < limit {
                bucket.push(json!({"book_id": number("book_id"), "title": text("title"), "category": text("category"), "viewed_at": viewed_at}));
            }
        }

        // 안정 정렬이라 마지막 조회 시각이 같으면 SQL 순서를 유지한다.
        users.sort_by(|a, b| b.last_viewed_at.cmp(&a.last_viewed_at));
        let users: Vec<Value> = users.into_iter().map(|u| json!({"email": u.email, "last_viewed_at": u.last_viewed_at, "book": u.book, "comic": u.comic})).collect();
        Ok(json!({"limit": limit, "users": users}))
    }
}

pub fn create_view_history_store() -> Result<ViewHistoryStore, ViewHistoryError> {
    ViewHistoryStore::new()
}
