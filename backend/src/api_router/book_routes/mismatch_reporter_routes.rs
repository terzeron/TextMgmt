//! 카테고리 불일치 조회 라우트(`book_manager::mismatch_reporter`).

use super::super::ApiError;
use super::super::AppDeps;
use super::super::RequireAdmin;
use super::super::Routes;
use super::manager;
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
        routes =
            routes.get(&format!("{prefix}/category-mismatches"), &["require_admin"], get_mismatches).get(&format!("{prefix}/category-mismatches/{{*category}}"), &["require_admin"], mismatch_details);
    }
    routes
}

async fn get_mismatches(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> Json<Value> {
    let response = match manager(&deps, &uri).and_then(|bm| bm.get_category_mismatches(false).map_err(|_| ApiError::unavailable())) {
        Ok(result) => json!({"status":"success","result":result}),
        Err(_) => json!({"status":"failure","error":"카테고리 불일치 조회 중 오류가 발생했습니다"}),
    };
    Json(response)
}

async fn mismatch_details(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(category): Path<String>) -> Json<Value> {
    let response = match manager(&deps, &uri).and_then(|bm| bm.get_category_mismatch_details(&category).map_err(|_| ApiError::unavailable())) {
        Ok(result) => json!({"status":"success","result":result}),
        Err(_) => json!({"status":"failure","error":"카테고리 불일치 조회 중 오류가 발생했습니다"}),
    };
    Json(response)
}
