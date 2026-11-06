//! 미리보기와 PDF 쪽 조회 라우트(`book_manager::preview_builder`).

use super::super::app_deps::AppDeps;
use super::super::error_responses::ApiResult;
use super::super::request_extractors::ApiQuery;
use super::super::request_extractors::RequireAuth;
use super::super::route_registry::Routes;
use super::book_response;
use super::checked_manager;
use super::parse_book_id;
use super::run_blocking;
use auth_manager::auth_service::Claims;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use std::sync::Arc;

#[allow(unused_imports)]
use super::*;

mod html_previewer_routes;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new().get("/pdf-pages/{book_id}", &["require_auth"], book_pages).get("/comics/pdf-pages/{book_id}", &["require_auth"], comic_pages);
    for prefix in ["", "/comics"] {
        routes = routes.get(&format!("{prefix}/preview/{{book_id}}"), &["require_auth"], preview);
    }
    routes.merge(html_previewer_routes::routes())
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
    book_response(run_blocking(async move { manager.get_book_preview_with_pages(book_id, query.pages, query.chapters, Some(&base)).await }).await?)
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

async fn pages(deps: &AppDeps, payload: &Claims, book_id: &str, query: PagesQuery, comics: bool) -> ApiResult<Response> {
    let book_id = parse_book_id(book_id)?;
    let manager = checked_manager(deps, payload, book_id, comics).await?;
    let (start, end) = (query.start.unwrap_or(1), query.end.unwrap_or(1));
    book_response(run_blocking(async move { manager.get_pdf_pages(book_id, start, end).await }).await?)
}
