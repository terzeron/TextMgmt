//! PDF 페이지 조회. Python `create_book_router`의 책·만화 경로를 함께 등록한다.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;

use super::deps::AppDeps;
use super::errors::ApiResult;
use super::extract::{ApiQuery, RequireAuth};
use super::routes::Routes;
use super::routes_books::{book_response, checked_manager, parse_book_id};

mod edit;

pub(super) fn routes() -> Routes {
    Routes::new().get("/pdf-pages/{book_id}", &["require_auth"], book_pages).get("/comics/pdf-pages/{book_id}", &["require_auth"], comic_pages).merge(edit::routes())
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
    let book_id = parse_book_id(book_id)?;
    let manager = checked_manager(deps, payload, book_id, comics).await?;
    book_response(manager.get_pdf_pages(book_id, query.start.unwrap_or(1), query.end.unwrap_or(1)).await)
}
