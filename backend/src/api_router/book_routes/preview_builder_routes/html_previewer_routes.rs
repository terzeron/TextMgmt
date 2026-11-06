//! HTML 책 미리보기 리소스 라우트(`book_manager::preview_builder::html_previewer`).

use super::super::super::app_deps::AppDeps;
use super::super::super::error_responses::ApiResult;
use super::super::super::request_extractors::ApiQuery;
use super::super::super::request_extractors::RequireAuth;
use super::super::super::route_registry::Routes;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use std::sync::Arc;

#[allow(unused_imports)]
use super::super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes.get(&format!("{prefix}/html-resource/{{book_id}}"), &["require_auth"], html_resource);
    }
    routes
}

#[derive(Deserialize)]
struct ResourceQuery {
    path: String,
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
    book_response(run_blocking(async move { manager.get_html_resource(book_id, &query.path).await }).await?)
}
