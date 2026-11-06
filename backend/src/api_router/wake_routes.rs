//! Storage wake-up endpoint.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use axum::http::{HeaderMap, header::HOST, uri::Authority};
use axum::{
    Json,
    extract::State,
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
};
use serde_json::json;

use super::{ApiError, ApiResult, AppDeps, Routes};

pub(super) fn routes() -> Routes {
    Routes::new().get("/wake", &[], wake_storage)
}

async fn wake_storage(State(deps): State<Arc<AppDeps>>, headers: HeaderMap) -> ApiResult<Response> {
    let configured_host = std::env::var("TM_FRONTEND_URL").ok().and_then(|value| Uri::try_from(value.as_str()).ok()).and_then(|uri| uri.host().map(str::to_ascii_lowercase));
    let request_host = headers.get(HOST).and_then(|value| value.to_str().ok()).and_then(|value| Authority::from_str(value).ok()).map(|authority| authority.host().to_ascii_lowercase());
    if configured_host.is_none() || configured_host != request_host {
        return Err(ApiError::http(404, "Not found"));
    }

    let book_dir = PathBuf::from(std::env::var("TM_BOOK_DIR").unwrap_or_else(|_| "/books".into()));
    let fs = deps.fs();
    let result = tokio::task::spawn_blocking(move || fs.read_dir(&book_dir)).await;
    match result {
        Ok(Ok(_)) => Ok(Json(json!({"status":"success"})).into_response()),
        Ok(Err(error)) => {
            log::warn!("wake_storage failed: {error}");
            Ok((StatusCode::SERVICE_UNAVAILABLE, Json(json!({"status":"failure"}))).into_response())
        }
        Err(error) => Err(ApiError::Internal(error.to_string())),
    }
}
