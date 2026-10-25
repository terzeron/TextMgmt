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
use crate::book::Book;
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

/// 백그라운드 작업 실행기. Starlette `BackgroundTasks`와 `asyncio.to_thread`에 대응한다.
pub trait TaskRunner: Send + Sync {
    /// 기본 구현은 새 스레드에서 실행한다.
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        match std::thread::Builder::new().name(name.to_string()).spawn(job) {
            Ok(handle) => Box::new(ThreadHandle(handle)),
            Err(e) => {
                log::error!("failed to start task {name}: {e}");
                Box::new(DoneHandle)
            }
        }
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

/// 카테고리 매핑 facade. 동기 메서드다. 지금은 viewer 접근 제어에 필요한 조회만 있다.
pub trait CategoryMappingApi: Send + Sync {
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
}

impl CategoryMappingApi for CategoryMapping {
    fn rename_category(&self, old: &str, new: &str, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::rename_category(self, old, new, content_type))
    }
    fn get_all_mappings(&self, content_type: &str) -> Result<Value, MappingError> {
        Ok(serde_json::json!(CategoryMapping::get_all_mappings(self, content_type)))
    }
    fn get_keywords(&self, category: &str, content_type: &str) -> Result<Vec<String>, MappingError> {
        Ok(CategoryMapping::get_keywords(self, category, content_type))
    }
    fn set_keywords(&self, category: &str, keywords: &[String], content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::set_keywords(self, category, &keywords.iter().map(String::as_str).collect::<Vec<_>>(), content_type))
    }
    fn add_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::add_keyword(self, category, keyword, content_type))
    }
    fn remove_keyword(&self, category: &str, keyword: &str, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::remove_keyword(self, category, keyword, content_type))
    }
    fn delete_category(&self, category: &str, content_type: &str, prefix: bool) -> Result<bool, MappingError> {
        Ok(CategoryMapping::delete_category(self, category, content_type, prefix))
    }
    fn update_all_mappings(&self, mappings: &Value, content_type: &str) -> Result<bool, MappingError> {
        let mappings = serde_json::from_value(mappings.clone()).map_err(|e| MappingError::Other(e.to_string()))?;
        Ok(CategoryMapping::update_all_mappings(self, &mappings, content_type))
    }
    fn set_hidden(&self, category: &str, hidden: bool, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::set_hidden(self, category, hidden, content_type))
    }
    fn set_latest_excluded(&self, category: &str, excluded: bool, content_type: &str) -> Result<bool, MappingError> {
        Ok(CategoryMapping::set_latest_excluded(self, category, excluded, content_type))
    }
    fn get_hidden_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
        Ok(CategoryMapping::get_hidden_categories(self, content_type))
    }

    fn get_latest_excluded_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
        Ok(CategoryMapping::get_latest_excluded_categories(self, content_type))
    }
}

// ---------------------------------------------------------------------------
// 책 관리자
// ---------------------------------------------------------------------------

/// `get_book_content`의 결과: 디스크 파일 또는 본문 텍스트.
#[derive(Debug, Clone, PartialEq)]
pub enum BookContent {
    File(PathBuf),
    Text(String),
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

/// 책/만화 관리자 facade. 책과 만화가 같은 trait를 쓴다. 지금은 문서 3.2의 통일 시그니처 일부만 있다.
#[async_trait]
pub trait BookManagerApi: Send + Sync {
    async fn delete_book(&self, _id: i64) -> (String, Option<String>) {
        ("Error".into(), unimplemented_error())
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

    async fn get_book(&self, _id: i64) -> (Option<Book>, Option<String>) {
        (None, unimplemented_error())
    }

    async fn get_categories(&self) -> (Value, Option<String>) {
        (Value::Null, unimplemented_error())
    }

    /// (목록, 전체 수, 다음 커서, 오류)
    async fn get_books_in_category_paged(&self, _category: &str, _size: Option<i64>, _cursor: Option<&str>) -> (Vec<Book>, i64, Option<String>, Option<String>) {
        (Vec::new(), 0, None, unimplemented_error())
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

    async fn delete_pdf_pages(
        &self,
        _id: i64,
        _pages: &[i64],
        _crops: Option<&[Value]>,
        _splits: Option<&[Value]>,
        _inserts: Option<&[Value]>,
        _order: Option<&[i64]>,
    ) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
    }

    async fn preview_insert_image(&self, _url: &str) -> (Option<Vec<u8>>, Option<String>) {
        (None, unimplemented_error())
    }

    async fn preview_insert_upload(&self, _data: &[u8]) -> (Option<Vec<u8>>, Option<String>) {
        (None, unimplemented_error())
    }

    async fn get_html_resource(&self, _book_id: i64, _resource_path: &str) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn get_cover(&self, _book_id: i64, _width: Option<i64>) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn validate_epub(&self, _book_id: i64) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
    }

    async fn validate_pdf(&self, _book_id: i64) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
    }

    async fn get_books_in_category(&self, _category: &str) -> (Vec<Book>, Option<String>) {
        (Vec::new(), unimplemented_error())
    }

    async fn get_latest_books(&self, _size: i64, _exclude_categories: Option<&[String]>) -> (Vec<Book>, i64, Option<String>) {
        (Vec::new(), 0, unimplemented_error())
    }

    async fn search_by_keyword_paged(&self, _keyword: &str, _size: i64, _offset: i64, _excluded: Option<&[String]>, _category: Option<&str>) -> (Vec<Book>, i64, Option<String>) {
        (Vec::new(), 0, unimplemented_error())
    }

    async fn search_similar_books_paged(&self, _book_id: i64, _size: i64, _offset: i64) -> (Vec<Book>, i64, Option<String>) {
        (Vec::new(), 0, unimplemented_error())
    }

    /// (항목 [(대표 책, 작품 디렉터리 이름 또는 "")], 전체 항목 수, 다음 커서, 오류)
    async fn get_cover_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _exclude_categories: Option<&[String]>) -> (Vec<(Book, String)>, i64, Option<String>, Option<String>) {
        (Vec::new(), 0, None, unimplemented_error())
    }

    async fn get_uncovered_books_paged(
        &self,
        _category: &str,
        _size: i64,
        _cursor: &str,
        _exclude_categories: Option<&[String]>,
    ) -> (Vec<(Book, String)>, Option<i64>, Option<String>, Option<String>) {
        (Vec::new(), None, None, unimplemented_error())
    }

    fn get_category_mismatches(&self, _force_refresh: bool) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 표지를 URL 또는 올린 이미지(base64)로 넣거나 바꾼다: (결과, 오류 메시지).
    async fn set_epub_cover(&self, _book_id: i64, _url: Option<&str>, _image_data: Option<&str>) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
    }

    /// EPUB 쪽 이미지의 크롭·분할·삭제·삽입을 한 번에 저장한다. 항목은 라우트가 검증한 JSON 객체다.
    async fn edit_epub_images(&self, _book_id: i64, _crops: &[Value], _splits: &[Value], _deletes: &[Value], _inserts: &[Value]) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
    }

    /// EPUB 표지를 뺀다.
    async fn remove_epub_cover(&self, _book_id: i64) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
    }

    /// `before_idref` 페이지 앞에 삽화를 넣는다.
    async fn insert_epub_illustration(&self, _book_id: i64, _before_idref: &str, _url: Option<&str>, _image_data: Option<&str>) -> (Option<Value>, Option<String>) {
        (None, unimplemented_error())
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
pub trait BookAccess: Send + Sync {
    fn get_book_and_ensure_viewer_access(&self, _book_id: i64, _payload: &Value, _content_type: &str) -> Result<(Option<BookStub>, Option<String>), HTTPException> {
        Err(HTTPException::new(500, UNIMPLEMENTED))
    }
}

/// 설정이 없을 때 쓰는 접근 확인. 기본 구현(미구현 오류)이다.
pub struct UnavailableBookAccess;

impl BookAccess for UnavailableBookAccess {}
