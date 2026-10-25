//! 책·만화의 미리보기, 다운로드, 리소스 및 검증 라우트.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{OriginalUri, Path, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::deps::AppDeps;
use super::errors::{ApiError, ApiResult, RequestValidationError};
use super::extract::{ApiQuery, RequireAuth};
use super::routes::Routes;
use super::traits::{BookContent, BookManagerApi, BookResponse, MappingError};
use crate::book_manager::BookManager;

mod browse;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .get(&format!("{prefix}/preview/{{book_id}}"), &["require_auth"], preview)
            .get(&format!("{prefix}/download/{{book_id}}"), &["require_auth"], download)
            .get(&format!("{prefix}/html-resource/{{book_id}}"), &["require_auth"], html_resource)
            .get(&format!("{prefix}/cover/{{book_id}}"), &["require_auth"], cover)
            .get(&format!("{prefix}/validate/{{book_id}}"), &["require_auth"], validate);
    }
    routes.merge(browse::routes())
}

pub(super) fn parse_book_id(raw: &str) -> ApiResult<i64> {
    raw.trim().parse().map_err(|_| RequestValidationError::single(&["path", "book_id"], "Input should be a valid integer", json!(raw)).into())
}

/// viewer는 카테고리 정책을 확인한 뒤 파일을 읽는다. admin은 조회 제한을 받지 않는다.
pub(super) async fn checked_manager(deps: &AppDeps, payload: &Value, book_id: i64, comics: bool) -> ApiResult<Arc<dyn BookManagerApi>> {
    let manager = if comics { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("도서 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })?;
    if payload.get("role").and_then(Value::as_str) == Some("viewer") {
        let (book, _) = manager.get_book(book_id).await;
        if let Some(book) = book {
            let mapping = deps.category_mapping().map_err(|e| {
                log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
                ApiError::unavailable()
            })?;
            let hidden = mapping.get_hidden_categories(if comics { "comic" } else { "book" }).map_err(|e| match e {
                MappingError::Http(status, detail) => ApiError::http(status, &detail),
                MappingError::Other(message) => ApiError::Internal(message),
            })?;
            if !book.category.is_empty() && hidden.iter().any(|category| book.category == *category || book.category.starts_with(&format!("{category}/"))) {
                return Err(ApiError::http(403, "접근 권한이 없는 카테고리입니다."));
            }
        }
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

#[derive(Deserialize)]
struct PreviewQuery {
    pages: Option<i64>,
    chapters: Option<i64>,
}

async fn preview(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<PreviewQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let comics = uri.path().starts_with("/comics/");
    let manager = checked_manager(&deps, &payload, book_id, comics).await?;
    let base = format!("{}/html-resource/{book_id}", if comics { "/comics" } else { "" });
    book_response(manager.get_book_preview_with_pages(book_id, query.pages, query.chapters, Some(&base)).await)
}

async fn download(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(raw_id): Path<String>, OriginalUri(uri): OriginalUri) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    match manager.get_book_content(book_id).await {
        BookContent::Text(text) => Ok(Json(text).into_response()),
        BookContent::File(path) => {
            let (media_type, headers) = BookManager::file_response_meta(&path);
            let body = std::fs::read(path).map_err(|e| ApiError::Internal(e.to_string()))?;
            book_response(BookResponse { status_code: 200, media_type: Some(media_type), headers, body })
        }
    }
}

#[derive(Deserialize)]
struct ResourceQuery {
    path: String,
}

#[derive(Deserialize)]
struct CoverQuery {
    w: Option<i64>,
}

async fn cover(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<CoverQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    book_response(manager.get_cover(book_id, query.w).await)
}

async fn html_resource(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<ResourceQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    book_response(manager.get_html_resource(book_id, &query.path).await)
}

async fn validate(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(raw_id): Path<String>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    let (book, _) = manager.get_book(book_id).await;
    let Some(book) = book else { return Ok(Json(json!({"status":"failure", "error":format!("Book not found: {book_id}")}))) };
    let (result, error) = match book.file_type.as_str() {
        "epub" => manager.validate_epub(book_id).await,
        "pdf" => manager.validate_pdf(book_id).await,
        _ => return Ok(Json(json!({"status":"failure", "error":format!("Validation not supported for type: {}", book.file_type)}))),
    };
    Ok(Json(match (result, error) {
        (Some(result), None) => json!({"status":"success", "result":result}),
        (_, error) => json!({"status":"failure", "error":error}),
    }))
}
