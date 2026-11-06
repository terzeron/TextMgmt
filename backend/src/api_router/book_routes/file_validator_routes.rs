//! EPUB·PDF 검증 라우트(`book_manager::file_validator`).

use super::super::app_deps::AppDeps;
use super::super::error_responses::ApiResult;
use super::super::request_extractors::RequireAuth;
use super::super::route_registry::Routes;
use axum::Json;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes.get(&format!("{prefix}/validate/{{book_id}}"), &["require_auth"], validate);
    }
    routes
}

async fn validate(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, Path(raw_id): Path<String>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    let Ok(book) = manager.get_book(book_id).await else { return Ok(Json(json!({"status":"failure", "error":format!("Book not found: {book_id}")}))) };
    let validated = match book.file_type.as_str() {
        "epub" => manager.validate_epub(book_id).await.map(|result| json!(result)),
        "pdf" => manager.validate_pdf(book_id).await.map(|result| json!(result)),
        _ => return Ok(Json(json!({"status":"failure", "error":format!("Validation not supported for type: {}", book.file_type)}))),
    };
    Ok(Json(match validated {
        Ok(result) => json!({"status":"success", "result":result}),
        Err(error) => json!({"status":"failure", "error":error}),
    }))
}
