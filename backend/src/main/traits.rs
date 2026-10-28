//! 앱이 주입받는 능력(trait)과 실제·기본 구현. 설계: docs/rust-test-seams.md 3.0, 3.2.
//!
//! 규칙: 모든 trait는 `Send + Sync`이고 모든 메서드에 기본 구현이 있다. 기본값은 `Err(..unimplemented)`/빈 값이다.
//! 테스트 대역은 쓰는 메서드만 구현한다. facade 이름은 `XxxApi`, 좁은 능력은 역할 명사다.
//!
//! 이 파일의 `BookManagerApi`, `CategoryMappingApi`, `ViewHistoryApi`, `BookAccess`는 라우트를 붙이는 쪽이 필요로 하는
//! 메서드만 먼저 정의한 뼈대다. 새 라우트가 메서드를 더 필요로 하면 여기에 기본 구현과 함께 추가한다.
//! (실제 `BookManager`가 생기면 `backend::book_manager`가 이 trait를 `pub use`하거나 구현한다.)

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use async_trait::async_trait;
use serde_json::Value;

use crate::auth::HTTPException;
use crate::book::{Book, BookLookupError, BookPage};
use crate::book_manager::CoverPage;
use crate::category_mapping::CategoryMapping;
use crate::es_manager::EsManagerApi;
use crate::view_history_store::{ViewHistoryError, ViewHistoryStore};

/// 사용 가능한 구현이 없을 때 쓰는 사유 문구.
pub const UNIMPLEMENTED: &str = "unimplemented";

// ---------------------------------------------------------------------------
// Google 로그인
// ---------------------------------------------------------------------------

/// Google ID 토큰 검증. Python `google_id_token.verify_oauth2_token`과 `TM_GOOGLE_CLIENT_ID`에 대응한다.
pub trait GoogleVerifier: Send + Sync {
    /// 설정된 OAuth 클라이언트 ID. 없으면 None.
    fn client_id(&self) -> Option<String> {
        std::env::var("TM_GOOGLE_CLIENT_ID").ok().filter(|v| !v.is_empty())
    }

    /// 검증에 성공하면 토큰의 claims를 돌려준다. 실패 사유는 문자열이다(Python `ValueError`).
    fn verify(&self, _credential: &str, _audience: &str) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }
}

/// 자리표시자. Google 공개 키를 HTTPS로 가져오는 TLS 클라이언트가 아직 없어서 항상 "검증 불가"를 돌려준다.
/// 그래서 이 구현으로는 아무도 로그인할 수 없다(실패로 닫힌다).
pub struct UnavailableGoogleVerifier;

impl GoogleVerifier for UnavailableGoogleVerifier {
    fn verify(&self, _credential: &str, _audience: &str) -> Result<Value, String> {
        Err("Google ID token verification is not available in this build".to_string())
    }
}

// ---------------------------------------------------------------------------
// 백그라운드 작업
// ---------------------------------------------------------------------------

/// `TaskRunner::spawn`이 돌려주는 핸들.
pub trait TaskHandle: Send {
    fn is_finished(&self) -> bool {
        true
    }

    fn join(self: Box<Self>) {}
}

struct ThreadHandle(JoinHandle<()>);

impl TaskHandle for ThreadHandle {
    fn is_finished(&self) -> bool {
        self.0.is_finished()
    }

    fn join(self: Box<Self>) {
        let _ = self.0.join();
    }
}

struct DoneHandle;

impl TaskHandle for DoneHandle {}

fn task_handle_from_spawn(name: &str, result: std::io::Result<JoinHandle<()>>) -> Box<dyn TaskHandle> {
    match result {
        Ok(handle) => Box::new(ThreadHandle(handle)),
        Err(error) => {
            log::error!("failed to start task {name}: {error}");
            Box::new(DoneHandle)
        }
    }
}

/// 백그라운드 작업 실행기. Starlette `BackgroundTasks`와 `asyncio.to_thread`에 대응한다.
pub trait TaskRunner: Send + Sync {
    /// 기본 구현은 새 스레드에서 실행한다.
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        task_handle_from_spawn(name, std::thread::Builder::new().name(name.to_string()).spawn(job))
    }

    /// 블로킹 풀로 넘어간 함수 이름을 알린다(Python `to_thread_spy`). 기본 구현은 아무것도 하지 않는다.
    fn spawn_blocking_observed(&self, _name: &str) {}
}

/// 실제 실행기: 작업마다 스레드를 띄운다.
pub struct ThreadRunner;

impl TaskRunner for ThreadRunner {}

/// 응답을 돌려주기 전에 같은 스레드에서 작업을 끝낸다(Starlette `TestClient` 가정). 스레드 로컬 로그 수집에 필요하다.
pub struct InlineRunner;

impl TaskRunner for InlineRunner {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        job();
        Box::new(DoneHandle)
    }
}

// ---------------------------------------------------------------------------
// 조회 이력
// ---------------------------------------------------------------------------

/// 사용자별 조회 이력 저장소 facade. `ViewHistoryStore`가 구현한다.
pub trait ViewHistoryApi: Send + Sync {
    fn record_view(&self, _email: &str, _content_type: &str, _book_id: i64, _title: &str, _category: &str) -> Result<(), ViewHistoryError> {
        Err(ViewHistoryError::RuntimeError(UNIMPLEMENTED.to_string()))
    }

    fn list_recent_views(&self, _limit: usize) -> Result<Value, ViewHistoryError> {
        Err(ViewHistoryError::RuntimeError(UNIMPLEMENTED.to_string()))
    }
}

impl ViewHistoryApi for ViewHistoryStore {
    fn record_view(&self, email: &str, content_type: &str, book_id: i64, title: &str, category: &str) -> Result<(), ViewHistoryError> {
        ViewHistoryStore::record_view(self, email, content_type, book_id, title, category)
    }

    fn list_recent_views(&self, limit: usize) -> Result<Value, ViewHistoryError> {
        ViewHistoryStore::list_recent_views(self, limit)
    }
}

// ---------------------------------------------------------------------------
// 카테고리 매핑
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

fn db_row_to_json(row: crate::db::Row) -> Value {
    let object = row
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                crate::db::DbValue::Null => Value::Null,
                crate::db::DbValue::Int(number) => serde_json::json!(number),
                crate::db::DbValue::Text(text) => serde_json::json!(text),
                crate::db::DbValue::DateTime(datetime) => serde_json::json!(datetime.to_string()),
            };
            (key, value)
        })
        .collect();
    Value::Object(object)
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
    fn get_all_mappings(&self, _content_type: &str) -> Result<Value, MappingError> {
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
    fn get_all_mappings(&self, content_type: &str) -> Result<Value, MappingError> {
        CategoryMapping::get_all_mappings(self, content_type).map(|value| serde_json::json!(value)).map_err(|e| MappingError::Other(e.to_string()))
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

// ---------------------------------------------------------------------------
// 책 관리자
// ---------------------------------------------------------------------------

/// `get_book_content`의 결과: 디스크 파일, 본문 텍스트 또는 변환 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum BookContent {
    File(PathBuf),
    Text(String),
    Error(String),
}

/// 미리보기·PDF 쪽·표지 같은 응답 하나. 구 `PreviewResponse`, `PdfPagesResponse`, `CoverResponse`를 합친 것이다.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookResponse {
    pub status_code: u16,
    pub media_type: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

struct NoEs;

impl EsManagerApi for NoEs {}

fn unimplemented_error() -> Option<String> {
    Some(UNIMPLEMENTED.to_string())
}

pub struct ProposeCategoryChanges<'a> {
    pub category: &'a str,
    pub mappings: &'a Value,
    pub content_type: &'a str,
    pub use_bookstore: bool,
    pub use_content_meta: bool,
    pub delay: f64,
    pub on_progress: Option<&'a (dyn Fn(Value) + Send + Sync)>,
}

pub struct ApplyCategoryChanges<'a> {
    pub items: &'a [Value],
    pub allowed_file_paths: &'a std::collections::HashSet<String>,
    pub content_type: &'a str,
    pub clean_existing: bool,
    pub on_progress: Option<&'a (dyn Fn(Value) + Send + Sync)>,
    pub on_item_done: Option<&'a (dyn Fn(Value) + Send + Sync)>,
    pub should_continue: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
}

pub type ReloadProgress = Arc<dyn Fn(Value) + Send + Sync>;

/// 책/만화 관리자 facade. 책과 만화가 같은 trait를 쓴다. 지금은 문서 3.2의 통일 시그니처 일부만 있다.
#[async_trait]
pub trait BookManagerApi: Send + Sync {
    async fn propose_category_changes(&self, _request: ProposeCategoryChanges<'_>) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn apply_category_changes(&self, _items: &[Value], _allowed_file_paths: &std::collections::HashSet<String>, _clean_existing: bool) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> (Value, Option<String>) {
        self.apply_category_changes(request.items, request.allowed_file_paths, request.clean_existing).await
    }
    async fn delete_book(&self, _id: i64) -> (String, Option<String>) {
        ("Error".into(), unimplemented_error())
    }
    async fn index_single_file(&self, _file_path: &str) -> (Option<i64>, Option<String>) {
        (None, unimplemented_error())
    }
    async fn delete_file(&self, _file_path: &str) -> (String, Option<String>) {
        ("Error".into(), unimplemented_error())
    }
    async fn reload_category(&self, _category: &str, _content_type: &str) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn reload_category_mismatch_files(&self, _category: &str, _content_type: &str, _on_progress: Option<ReloadProgress>) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn reload_category_mismatches(&self, _content_type: &str, _on_progress: Option<ReloadProgress>) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn rename_category(&self, _old: &str, _new: &str) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn delete_category(&self, _category: &str, _delete_files: bool) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    async fn preview_delete_category(&self, _category: &str) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
    fn path_prefix(&self) -> PathBuf {
        PathBuf::new()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        Arc::new(NoEs)
    }

    async fn get_book(&self, _id: i64) -> Result<Book, BookLookupError> {
        Err(BookLookupError::Failed(UNIMPLEMENTED.to_string()))
    }

    async fn get_categories(&self) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_books_in_category_paged(&self, _category: &str, _size: Option<i64>, _cursor: Option<&str>) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_book_content(&self, _id: i64) -> BookContent {
        BookContent::Text(String::new())
    }

    async fn get_book_preview(&self, _id: i64, _chapters: Option<i64>, _resource_base_url: Option<&str>) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    /// PDF 페이지 수를 받는 확장. 기존 facade 대역은 챕터 기반 메서드를 계속 쓸 수 있다.
    async fn get_book_preview_with_pages(&self, id: i64, _pages: Option<i64>, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        self.get_book_preview(id, chapters, resource_base_url).await
    }

    async fn get_pdf_pages(&self, _id: i64, _start: i64, _end: i64) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn delete_pdf_pages(&self, _id: i64, _pages: &[i64], _crops: Option<&[Value]>, _splits: Option<&[Value]>, _inserts: Option<&[Value]>, _order: Option<&[i64]>) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn preview_insert_image(&self, _url: &str) -> Result<Vec<u8>, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn preview_insert_upload(&self, _data: &[u8]) -> Result<Vec<u8>, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_html_resource(&self, _book_id: i64, _resource_path: &str) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn get_cover(&self, _book_id: i64, _width: Option<i64>) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn validate_epub(&self, _book_id: i64) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn validate_pdf(&self, _book_id: i64) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_books_in_category(&self, _category: &str) -> Result<Vec<Book>, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_latest_books(&self, _size: i64, _exclude_categories: Option<&[String]>) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn search_by_keyword_paged(&self, _keyword: &str, _size: i64, _offset: i64, _excluded: Option<&[String]>, _category: Option<&str>) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn search_similar_books_paged(&self, _book_id: i64, _size: i64, _offset: i64) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_cover_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_uncovered_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    fn get_category_mismatches(&self, _force_refresh: bool) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    fn get_category_mismatch_details(&self, _category: &str) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 표지를 URL 또는 올린 이미지(base64)로 넣거나 바꾼다.
    async fn set_epub_cover(&self, _book_id: i64, _url: Option<&str>, _image_data: Option<&str>) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 쪽 이미지의 크롭·분할·삭제·삽입을 한 번에 저장한다. 항목은 라우트가 검증한 JSON 객체다.
    async fn edit_epub_images(&self, _book_id: i64, _crops: &[Value], _splits: &[Value], _deletes: &[Value], _inserts: &[Value]) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 표지를 뺀다.
    async fn remove_epub_cover(&self, _book_id: i64) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// `before_idref` 페이지 앞에 삽화를 넣는다.
    async fn insert_epub_illustration(&self, _book_id: i64, _before_idref: &str, _url: Option<&str>, _image_data: Option<&str>) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, _id: i64, _category: &str, _title: &str, _author: &str, _path: &std::path::Path, _file_type: &str, _force: bool) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }
}

/// 설정이 없을 때 쓰는 관리자. 모든 메서드가 기본 구현(미구현 오류)이다.
pub struct UnavailableBookManager;

impl BookManagerApi for UnavailableBookManager {}

// ---------------------------------------------------------------------------
// viewer 접근 확인
// ---------------------------------------------------------------------------

/// `BookAccess`가 돌려주는 책 요약.
#[derive(Debug, Clone, PartialEq)]
pub struct BookStub {
    pub title: String,
    pub category: Option<String>,
}

/// Python `_get_book_and_ensure_viewer_access`: 책을 찾고, viewer가 볼 수 없는 카테고리면 403 오류를 돌려준다.
#[async_trait]
pub trait BookAccess: Send + Sync {
    async fn get_book_and_ensure_viewer_access(&self, _book_id: i64, _payload: &Value, _content_type: &str) -> Result<(Option<BookStub>, Option<String>), HTTPException> {
        Err(HTTPException::new(500, UNIMPLEMENTED))
    }
}

/// 설정이 없을 때 쓰는 접근 확인. 기본 구현(미구현 오류)이다.
pub struct UnavailableBookAccess;

impl BookAccess for UnavailableBookAccess {}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;

    struct EmptyMapping;
    struct EmptyGoogleVerifier;
    struct EmptyBookAccess;
    struct EmptyTaskHandle;
    struct EmptyTaskRunner;
    struct EmptyViewHistory;

    impl CategoryMappingApi for EmptyMapping {}
    impl GoogleVerifier for EmptyGoogleVerifier {}
    impl BookAccess for EmptyBookAccess {}
    impl TaskHandle for EmptyTaskHandle {}
    impl TaskRunner for EmptyTaskRunner {}
    impl ViewHistoryApi for EmptyViewHistory {}

    #[test]
    fn google_verifier_and_view_history_defaults_are_explicit() {
        let expected_client_id = std::env::var("TM_GOOGLE_CLIENT_ID").ok().filter(|value| !value.is_empty());
        assert_eq!(EmptyGoogleVerifier.client_id(), expected_client_id);
        assert_eq!(EmptyGoogleVerifier.verify("token", "audience").unwrap_err(), UNIMPLEMENTED);

        assert!(EmptyViewHistory.record_view("a@b", "book", 1, "title", "category").is_err());
        assert!(EmptyViewHistory.list_recent_views(5).is_err());
    }

    #[test]
    fn task_handle_and_runners_cover_inline_and_thread_execution() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let handle: Box<dyn TaskHandle> = Box::new(EmptyTaskHandle);
        assert!(handle.is_finished());
        handle.join();

        let completed = Arc::new(AtomicBool::new(false));
        let thread_completed = completed.clone();
        let handle = ThreadRunner.spawn("traits-test", Box::new(move || thread_completed.store(true, Ordering::SeqCst)));
        handle.join();
        assert!(completed.load(Ordering::SeqCst));

        let inline_completed = completed.clone();
        InlineRunner.spawn("inline-test", Box::new(move || inline_completed.store(true, Ordering::SeqCst))).join();
        assert!(completed.load(Ordering::SeqCst));
        EmptyTaskRunner.spawn_blocking_observed("blocking-test");
        let failed = task_handle_from_spawn("failed-task", Err(std::io::Error::other("thread limit")));
        assert!(failed.is_finished());
        failed.join();
        assert!(UnavailableGoogleVerifier.verify("token", "audience").is_err());
    }

    #[tokio::test]
    async fn unavailable_book_access_fails_closed() {
        assert!(EmptyBookAccess.get_book_and_ensure_viewer_access(1, &json!({}), "book").await.is_err());
        assert!(UnavailableBookAccess.get_book_and_ensure_viewer_access(1, &json!({}), "book").await.is_err());
    }

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
        let mut row = crate::db::Row::new();
        row.insert("null".into(), crate::db::DbValue::Null);
        row.insert("number".into(), crate::db::DbValue::Int(7));
        row.insert("text".into(), crate::db::DbValue::Text("value".into()));
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap().and_hms_opt(1, 2, 3).unwrap();
        row.insert("date".into(), crate::db::DbValue::DateTime(date));
        assert_eq!(db_row_to_json(row), json!({"null": null, "number": 7, "text": "value", "date": "2026-10-27 01:02:03"}));

        assert_eq!(reload_counts(&json!({"done": 2, "failed": 0})).unwrap(), vec![("done", 2), ("failed", 0)]);
        assert!(reload_counts(&json!([])).unwrap_err().contains("객체가 아닙니다"));
        assert!(reload_counts(&json!({"done": "2"})).unwrap_err().contains("정수가 아닙니다: done"));
    }

    #[test]
    fn concrete_category_mapping_adapter_forwards_database_errors() {
        let mapping = CategoryMapping::with_connection_provider(Arc::new(|| Err(crate::db::DbError::Other("offline".into()))));
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

    #[tokio::test]
    async fn unavailable_book_manager_returns_documented_defaults() {
        let manager = UnavailableBookManager;
        let allowed = HashSet::new();
        let items = [];
        let request = ApplyCategoryChanges { items: &items, allowed_file_paths: &allowed, content_type: "book", clean_existing: false, on_progress: None, on_item_done: None, should_continue: None };

        assert_eq!(
            manager
                .propose_category_changes(ProposeCategoryChanges {
                    category: "fiction",
                    mappings: &json!({}),
                    content_type: "book",
                    use_bookstore: false,
                    use_content_meta: false,
                    delay: 0.0,
                    on_progress: None,
                })
                .await
                .1
                .as_deref(),
            Some(UNIMPLEMENTED)
        );
        assert_eq!(manager.apply_category_changes(&items, &allowed, false).await.1.as_deref(), Some(UNIMPLEMENTED));
        assert_eq!(manager.apply_category_changes_with(request).await.1.as_deref(), Some(UNIMPLEMENTED));
        assert!(manager.delete_book(1).await.1.is_some());
        assert!(manager.index_single_file("a.epub").await.1.is_some());
        assert!(manager.delete_file("a.epub").await.1.is_some());
        assert!(manager.reload_category("fiction", "book").await.1.is_some());
        assert!(manager.reload_category_mismatch_files("fiction", "book", None).await.1.is_some());
        assert!(manager.reload_category_mismatches("book", None).await.1.is_some());
        assert!(manager.rename_category("old", "new").await.1.is_some());
        assert!(manager.delete_category("fiction", false).await.1.is_some());
        assert!(manager.preview_delete_category("fiction").await.1.is_some());
        assert!(manager.path_prefix().as_os_str().is_empty());
        assert!(matches!(manager.es_manager().create_index(), Err(crate::es_manager::EsError::Unsupported)));
        assert!(manager.get_book(1).await.is_err());
        assert!(manager.get_categories().await.is_err());
        assert!(manager.get_books_in_category_paged("fiction", None, None).await.is_err());
        assert_eq!(manager.get_book_content(1).await, BookContent::Text(String::new()));
        assert_eq!(manager.get_book_preview(1, None, None).await.status_code, 501);
        assert_eq!(manager.get_book_preview_with_pages(1, Some(2), None, None).await.status_code, 501);
        assert_eq!(manager.get_pdf_pages(1, 0, 1).await.status_code, 501);
        assert!(manager.delete_pdf_pages(1, &[], None, None, None, None).await.is_err());
        assert!(manager.preview_insert_image("https://example.test/a.png").await.is_err());
        assert!(manager.preview_insert_upload(&[]).await.is_err());
        assert_eq!(manager.get_html_resource(1, "a.html").await.status_code, 501);
        assert_eq!(manager.get_cover(1, None).await.status_code, 501);
        assert!(manager.validate_epub(1).await.is_err());
        assert!(manager.validate_pdf(1).await.is_err());
        assert!(manager.get_books_in_category("fiction").await.is_err());
        assert!(manager.get_latest_books(10, None).await.is_err());
        assert!(manager.search_by_keyword_paged("query", 10, 0, None, None).await.is_err());
        assert!(manager.search_similar_books_paged(1, 10, 0).await.is_err());
        assert!(manager.get_cover_books_paged("fiction", 10, "", None).await.is_err());
        assert!(manager.get_uncovered_books_paged("fiction", 10, "", None).await.is_err());
        assert!(manager.get_category_mismatches(false).is_err());
        assert!(manager.get_category_mismatch_details("fiction").is_err());
        assert!(manager.set_epub_cover(1, None, None).await.is_err());
        assert!(manager.edit_epub_images(1, &[], &[], &[], &[]).await.is_err());
        assert!(manager.remove_epub_cover(1).await.is_err());
        assert!(manager.insert_epub_illustration(1, "page-1", None, None).await.is_err());
        assert!(manager.update_book(1, "fiction", "title", "author", std::path::Path::new("a.epub"), "epub", false).await.1.is_some());
    }
}
