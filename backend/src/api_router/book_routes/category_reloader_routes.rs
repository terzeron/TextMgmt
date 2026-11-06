//! 카테고리 재적재 라우트(`book_manager::category_reloader`). 진행 상태는 카테고리 매핑 저장소에 둔다.

use super::super::ApiError;
use super::super::ApiJson;
use super::super::ApiQuery;
use super::super::ApiResult;
use super::super::AppDeps;
use super::super::BookManagerApi;
use super::super::CategoryMappingApi;
use super::super::ReloadProgress;
use super::super::RequestValidationError;
use super::super::RequireAdmin;
use super::super::Routes;
use super::manager;
use axum::Json;
use axum::extract::OriginalUri;
use axum::extract::State;
use book_manager::MismatchReload;
use book_manager::ReloadCounts;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;
use std::sync::Mutex;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .post(&format!("{prefix}/category-mismatches/reload"), &["require_admin"], reload_category)
            .post(&format!("{prefix}/category-mismatches/reload-mismatches"), &["require_admin"], reload_mismatches)
            .post(&format!("{prefix}/category-mismatches/reload-all"), &["require_admin"], reload_all_mismatches)
            .get(&format!("{prefix}/category-mismatches/reload-status"), &["require_admin"], reload_status);
    }
    routes
}

#[derive(Deserialize)]
struct Category {
    category: String,
}

async fn reload_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Category>) -> ApiResult<Json<Value>> {
    let content_type = if uri.path().starts_with("/comics/") { "comic" } else { "book" };
    Ok(Json(match manager(&deps, &uri)?.reload_category(&body.category, content_type).await {
        Ok(result) => json!({"status":"success","result":result}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}

#[derive(Deserialize)]
struct ReloadStatusQuery {
    category: Option<String>,
}

async fn reload_status(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<ReloadStatusQuery>) -> ApiResult<Json<Value>> {
    let content_type = if uri.path().starts_with("/comics/") { "comic" } else { "book" };
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let result = mapping.get_reload_status(content_type, query.category.as_deref()).map_err(|error| ApiError::Internal(error.to_string()))?.unwrap_or_else(|| json!({"status":"idle"}));
    Ok(Json(json!({"status":"success","result":result})))
}

async fn reload_mismatches(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Category>) -> ApiResult<Json<Value>> {
    let content_type = if uri.path().starts_with("/comics/") { "comic" } else { "book" };
    let manager = manager(&deps, &uri)?;
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let category = body.category;
    let lock_mapping = mapping.clone();
    let lock_category = category.clone();
    let lock_type = content_type.to_string();
    let (started, error, blocking_status) = tokio::task::spawn_blocking(move || lock_mapping.acquire_reload_lock(&lock_type, Some(&lock_category), Some("mismatch")))
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if !started {
        let mut status = blocking_status.unwrap_or_else(|| json!({}));
        if !status.is_object() {
            status = json!({});
        }
        status["already_running"] = json!(true);
        if status.get("status").is_none() {
            status["error"] = json!(error);
        }
        return Ok(Json(json!({"status":"success","result":status})));
    }
    spawn_reload_job(&deps, manager, mapping, content_type, Some(category.clone()));
    Ok(Json(json!({"status":"success","result":{"started":true,"content_type":content_type,"category":category,"reload_source":"mismatch"}})))
}

#[derive(Deserialize)]
struct ReloadAllBody {
    #[serde(default = "default_reload_source")]
    reload_source: String,
}

fn default_reload_source() -> String {
    "bulk".into()
}

async fn reload_all_mismatches(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, body: Option<Json<ReloadAllBody>>) -> ApiResult<Json<Value>> {
    let content_type = if uri.path().starts_with("/comics/") { "comic" } else { "book" };
    let reload_source = body.map(|Json(body)| body.reload_source).unwrap_or_else(default_reload_source);
    if !matches!(reload_source.as_str(), "bulk" | "mismatch") {
        return Err(RequestValidationError::single(&["body", "reload_source"], "Input should be 'bulk' or 'mismatch'", json!(reload_source)).into());
    }
    let manager = manager(&deps, &uri)?;
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let lock_mapping = mapping.clone();
    let lock_type = content_type.to_string();
    let lock_source = reload_source.clone();
    let (started, error, blocking_status) = tokio::task::spawn_blocking(move || lock_mapping.acquire_reload_lock(&lock_type, None, Some(&lock_source)))
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    if !started {
        let mut status = blocking_status.unwrap_or_else(|| json!({}));
        if !status.is_object() {
            status = json!({});
        }
        status["already_running"] = json!(true);
        if status.get("status").is_none() {
            status["error"] = json!(error);
        }
        return Ok(Json(json!({"status":"success","result":status})));
    }
    spawn_reload_job(&deps, manager, mapping, content_type, None);
    Ok(Json(json!({"status":"success","result":{"started":true,"content_type":content_type,"reload_source":reload_source}})))
}

fn spawn_reload_job(deps: &AppDeps, manager: Arc<dyn BookManagerApi>, mapping: Arc<dyn CategoryMappingApi>, content_type: &str, category: Option<String>) {
    let content_type = content_type.to_string();
    let flush_interval = deps.reload_progress_flush_interval();
    deps.task_runner().spawn(
        "category-mismatch-reload",
        Box::new(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = mapping.complete_reload_lock(&content_type, "failed", Some("재적재 실행기를 시작하지 못했습니다"), category.as_deref(), &json!({}));
                    log::error!("불일치 재적재 runtime 생성 실패: {error}");
                    return;
                }
            };
            let failure_mapping = mapping.clone();
            let failure_type = content_type.clone();
            let failure_category = category.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                runtime.block_on(async move {
                    let latest_counts = Arc::new(Mutex::new(ReloadCounts::default()));
                    let progress_counts = latest_counts.clone();
                    let progress: ReloadProgress = Arc::new(move |counts| {
                        if let Ok(mut latest) = progress_counts.lock() {
                            latest.merge(counts);
                        }
                    });
                    let heartbeat_mapping = mapping.clone();
                    let heartbeat_type = content_type.clone();
                    let heartbeat_category = category.clone();
                    let heartbeat_counts = latest_counts.clone();
                    let flusher = tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(flush_interval).await;
                            let counts = heartbeat_counts.lock().map(|value| *value).unwrap_or_default();
                            if let Err(error) = heartbeat_mapping.heartbeat_reload_lock(&heartbeat_type, heartbeat_category.as_deref(), &json!(counts)) {
                                log::warn!("재적재 heartbeat 갱신 실패: {error}");
                            }
                        }
                    });
                    let reloaded = match category.as_deref() {
                        Some(category) => manager.reload_category_mismatch_files(category, &content_type, Some(progress)).await,
                        None => manager.reload_category_mismatches(&content_type, Some(progress)).await,
                    };
                    flusher.abort();
                    // 실패하면 건수는 0이다. 구현은 실패 때 부분 건수를 내지 않는다.
                    let (result, error) = match reloaded {
                        Ok(result) => (result, None),
                        Err(error) => (MismatchReload::default(), Some(error)),
                    };
                    let counts = json!({
                        "indexed_count": result.indexed_count,
                        "deleted_count": result.deleted_count,
                        "failed_count": result.failed_count,
                        "before_count": result.before_count,
                        "after_count": result.after_count,
                    });
                    let status = if error.is_some() { "failed" } else { "done" };
                    if let Err(store_error) = mapping.complete_reload_lock(&content_type, status, error.as_deref(), category.as_deref(), &counts) {
                        log::error!("재적재 완료 상태 저장 실패: {store_error}");
                    }
                })
            }));
            if outcome.is_err() {
                log::error!("불일치 재적재 작업에서 panic 발생");
                if let Err(error) = failure_mapping.complete_reload_lock(&failure_type, "failed", Some("카테고리 불일치 조회 중 오류가 발생했습니다"), failure_category.as_deref(), &json!({}))
                {
                    log::error!("불일치 재적재 실패 상태 저장 실패: {error}");
                }
            }
        }),
    );
}
