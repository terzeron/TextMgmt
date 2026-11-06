//! 책·만화 라우트. 하위 모듈은 그 라우트가 부르는 `book_manager` 모듈 이름을 따른다(예: `catalog_query_routes` → `book_manager::catalog_query`).
//! 책 라우트가 함께 쓰는 helper(책 ID 해석, 숨김 카테고리 검사, 매니저 고르기, 응답 변환)는 이 파일에 둔다.

use super::app_capabilities::CategoryMappingApi;
use super::app_capabilities::MappingError;
use super::app_deps::AppDeps;
use super::error_responses::ApiError;
use super::error_responses::ApiResult;
use super::error_responses::RequestValidationError;
use super::route_registry::Routes;
use auth_manager::auth_service::Claims;
use axum::Json;
use axum::body::Body;
use axum::http::Uri;
use axum::response::Response;
use book_manager::BookManagerApi;
use book_manager::BookResponse;
use book_manager::book_metadata::BookLookupError;
use book_manager::book_metadata::{Book, BookView};
use db::category_mapping::is_hidden_category;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

mod book_updater_routes;
mod catalog_query_routes;
mod category_editor_routes;
mod category_proposer_routes;
mod category_reloader_routes;
mod cover_finder_routes;
mod cover_list_routes;
mod file_validator_routes;
mod html_previewer_routes;
mod mismatch_reporter_routes;
mod pdf_edit_routes;
mod preview_builder_routes;

pub(super) fn routes() -> Routes {
    Routes::new()
        .merge(book_updater_routes::routes())
        .merge(catalog_query_routes::routes())
        .merge(category_editor_routes::routes())
        .merge(category_proposer_routes::routes())
        .merge(category_reloader_routes::routes())
        .merge(cover_finder_routes::routes())
        .merge(cover_list_routes::routes())
        .merge(file_validator_routes::routes())
        .merge(html_previewer_routes::routes())
        .merge(mismatch_reporter_routes::routes())
        .merge(pdf_edit_routes::routes())
        .merge(preview_builder_routes::routes())
}

pub(super) fn parse_book_id(raw: &str) -> ApiResult<i64> {
    raw.trim().parse().map_err(|_| RequestValidationError::single(&["path", "book_id"], "Input should be a valid integer, unable to parse string as an integer", json!(raw)).into())
}

/// 미리보기·쪽 추출은 LibreOffice 변환, EPUB 재압축, PDF 로드처럼 오래 막는 일을 한다.
/// tokio worker를 묶으면 다른 API가 멈추므로 blocking 스레드에서 돌린다.
pub(super) async fn run_blocking<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static) -> ApiResult<T> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(work)).await.map_err(|e| ApiError::Internal(format!("blocking task failed: {e}")))
}

/// 경로로 도서와 만화를 가른다. 카테고리 정책과 ES가 쓰는 종류 이름이다.
fn kind(uri: &Uri) -> &'static str {
    if uri.path().starts_with("/comics/") { "comic" } else { "book" }
}

fn manager(deps: &AppDeps, uri: &Uri) -> ApiResult<Arc<dyn BookManagerApi>> {
    manager_for(deps, kind(uri) == "comic")
}

fn manager_for(deps: &AppDeps, comics: bool) -> ApiResult<Arc<dyn BookManagerApi>> {
    if comics { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("도서 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })
}

impl From<MappingError> for ApiError {
    fn from(error: MappingError) -> Self {
        match error {
            MappingError::Http(status, detail) => ApiError::http(status, &detail),
            MappingError::Other(message) => ApiError::Internal(message),
        }
    }
}

/// viewer에게 숨긴 카테고리. viewer가 아니면 비어 있다.
fn hidden_categories(deps: &AppDeps, payload: &Claims, comics: bool) -> ApiResult<Vec<String>> {
    if !payload.is_viewer() {
        return Ok(Vec::new());
    }
    let mapping = deps.category_mapping().map_err(|e| {
        log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
        ApiError::unavailable()
    })?;
    Ok(mapping.get_hidden_categories(if comics { "comic" } else { "book" })?)
}

fn ensure_allowed(category: &str, hidden: &[String]) -> ApiResult<()> {
    if is_hidden_category(category, hidden) { Err(ApiError::http(403, "접근 권한이 없는 카테고리입니다.")) } else { Ok(()) }
}

/// viewer는 카테고리 정책을 확인한 뒤 파일을 읽는다. admin은 조회 제한을 받지 않는다.
pub(super) async fn checked_manager(deps: &AppDeps, payload: &Claims, book_id: i64, comics: bool) -> ApiResult<Arc<dyn BookManagerApi>> {
    let manager = manager_for(deps, comics)?;
    if payload.is_viewer() {
        // 카테고리를 모르면 숨김 여부를 판정할 수 없다. 통과시키지 않고 거절한다(fail-closed).
        let book = match manager.get_book(book_id).await {
            Ok(book) => book,
            Err(BookLookupError::NotFound(_)) => return Err(ApiError::http(404, &format!("Book not found: {book_id}"))),
            Err(error) => {
                log::warn!("viewer 접근 확인 중 도서를 조회하지 못했다: book_id={book_id}, {error}");
                return Err(ApiError::unavailable());
            }
        };
        ensure_allowed(&book.category, &hidden_categories(deps, payload, comics)?)?;
    }
    Ok(manager)
}

pub(super) fn book_response(result: BookResponse) -> ApiResult<Response> {
    let mut response = Response::builder().status(result.status_code);
    if let Some(media_type) = result.media_type {
        response = response.header("content-type", media_type);
    }
    for (name, value) in result.headers {
        response = response.header(name, value);
    }
    response.body(Body::from(result.body)).map_err(|e| ApiError::Internal(e.to_string()))
}

fn result(outcome: Result<impl Serialize, String>) -> Json<Value> {
    Json(match outcome {
        Ok(value) => json!({"status":"success","result":value}),
        Err(error) => json!({"status":"failure","error":error}),
    })
}

async fn with_mapping<T: Send + 'static>(deps: Arc<AppDeps>, action: impl FnOnce(Arc<dyn CategoryMappingApi>) -> Result<T, MappingError> + Send + 'static) -> ApiResult<T> {
    let result = tokio::task::spawn_blocking(move || deps.category_mapping().map_err(MappingError::Other).and_then(action)).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    result.map_err(|e| match e {
        MappingError::Http(code, message) => ApiError::http(code, &message),
        MappingError::Other(message) => ApiError::Internal(message),
    })
}

#[derive(Deserialize)]
struct CategoryNameQuery {
    category: String,
}

/// manager의 책 루트를 기준으로 경로를 상대화한 응답 모양.
pub(super) fn book_view(book: &Book, manager: &dyn BookManagerApi) -> BookView {
    let manager_prefix = manager.path_prefix();
    let prefix = if !manager_prefix.as_os_str().is_empty() && book.file_path.starts_with(&manager_prefix) { manager_prefix } else { Book::path_prefix() };
    book.view_with_prefix(&prefix)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::run_blocking;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn run_blocking_keeps_the_worker_free() {
        // worker가 하나뿐이어도 느린 미리보기가 다른 요청을 막지 않아야 한다.
        let (started, wait_started) = tokio::sync::oneshot::channel();
        let slow = tokio::spawn(run_blocking(async move {
            let _ = started.send(());
            std::thread::sleep(Duration::from_millis(500));
            7
        }));
        wait_started.await.unwrap();
        let start = Instant::now();
        tokio::spawn(async { 1 }).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(200), "{:?}", start.elapsed());
        assert_eq!(slow.await.unwrap().unwrap(), 7);
    }

    use super::{kind, manager, result, with_mapping};
    use crate::api_router::{ApiError, AppDeps, AuthMode, CategoryMappingApi, MappingError};
    use axum::http::Uri;
    use serde_json::json;

    /// `mapping`은 매핑 저장소를 action에 넘기기만 한다. 이 테스트의 action은 저장소를 쓰지 않는다.
    struct NoMapping;

    impl CategoryMappingApi for NoMapping {}

    #[tokio::test]
    async fn mapping_runs_action_and_converts_mapping_errors() {
        let deps = std::sync::Arc::new(AppDeps::new(AuthMode::Cookies).with_category_mapping(std::sync::Arc::new(NoMapping)));

        let value = with_mapping(deps.clone(), |_| Ok::<_, MappingError>(42)).await.unwrap();
        assert_eq!(value, 42);

        let http_error = with_mapping(deps.clone(), |_| Err::<(), _>(MappingError::Http(409, "conflict".into()))).await.unwrap_err();
        assert!(matches!(http_error, ApiError::Http(error) if error.status_code == 409 && error.detail == "conflict"));

        let other_error = with_mapping(deps, |_| Err::<(), _>(MappingError::Other("storage failed".into()))).await.unwrap_err();
        assert!(matches!(other_error, ApiError::Internal(message) if message == "storage failed"));
    }
    #[test]
    fn content_type_and_manager_follow_the_original_uri() {
        let deps = AppDeps::new(AuthMode::Cookies);
        let book_uri = Uri::from_static("/books/1");
        let comic_uri = Uri::from_static("/comics/books/1");

        assert_eq!(kind(&book_uri), "book");
        assert_eq!(kind(&comic_uri), "comic");
        let book_manager = manager(&deps, &book_uri).unwrap();
        let comic_manager = manager(&deps, &comic_uri).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&book_manager, &comic_manager));
    }
    #[test]
    fn result_serializes_success_and_failure_shapes() {
        let success = result(Ok(json!({"count":2})));
        assert_eq!(success.0, json!({"status":"success","result":{"count":2}}));

        let failure = result(Err::<serde_json::Value, _>("failed".into()));
        assert_eq!(failure.0, json!({"status":"failure","error":"failed"}));
    }
}
