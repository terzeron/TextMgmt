//! PDF 페이지 조회. Python `create_book_router`의 책·만화 경로를 함께 등록한다.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use super::deps::AppDeps;
use super::errors::{ApiError, ApiResult, RequestValidationError};
use super::extract::{ApiQuery, RequireAuth};
use super::routes::Routes;
use super::traits::MappingError;

pub(super) fn routes() -> Routes {
    Routes::new().get("/pdf-pages/{book_id}", &["require_auth"], book_pages).get("/comics/pdf-pages/{book_id}", &["require_auth"], comic_pages)
}

#[derive(Deserialize)]
struct PagesQuery {
    start: Option<i64>,
    end: Option<i64>,
}

async fn book_pages(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>, ApiQuery(query): ApiQuery<PagesQuery>) -> ApiResult<Response> {
    pages(&deps, &payload, &book_id, query, false).await
}

async fn comic_pages(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(book_id): Path<String>, ApiQuery(query): ApiQuery<PagesQuery>) -> ApiResult<Response> {
    pages(&deps, &payload, &book_id, query, true).await
}

async fn pages(deps: &AppDeps, payload: &Value, book_id: &str, query: PagesQuery, comics: bool) -> ApiResult<Response> {
    let book_id = book_id.trim().parse::<i64>().map_err(|_| RequestValidationError::single(&["path", "book_id"], "Input should be a valid integer", json!(book_id)))?;
    let manager = if comics { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("PDF 관리자를 쓸 수 없다: {e}");
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
    let result = manager.get_pdf_pages(book_id, query.start.unwrap_or(1), query.end.unwrap_or(1)).await;
    let mut response = Response::builder().status(result.status_code);
    if let Some(media_type) = result.media_type {
        response = response.header("content-type", media_type);
    }
    for (name, value) in result.headers {
        response = response.header(name, value);
    }
    response.body(Body::from(result.body)).map_err(|e| ApiError::Internal(e.to_string()))
}
