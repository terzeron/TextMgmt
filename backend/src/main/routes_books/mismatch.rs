//! 카테고리 불일치 화면의 파일·Elasticsearch 문서 작업.

use std::sync::{Arc, Mutex};

use axum::{
    Json,
    extract::{OriginalUri, Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, BookManagerApi, CategoryMappingApi, ReloadProgress, RequestValidationError, RequireAdmin, Routes};
use super::manager;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .get(&format!("{prefix}/category-mismatches"), &["require_admin"], get_mismatches)
            .post(&format!("{prefix}/category-mismatches/index-file"), &["require_admin"], index_file)
            .post(&format!("{prefix}/category-mismatches/delete-file"), &["require_admin"], delete_file)
            .delete(&format!("{prefix}/category-mismatches/es-doc/{{book_id}}"), &["require_admin"], delete_es_doc)
            .post(&format!("{prefix}/category-mismatches/reload"), &["require_admin"], reload_category)
            .post(&format!("{prefix}/category-mismatches/reload-mismatches"), &["require_admin"], reload_mismatches)
            .post(&format!("{prefix}/category-mismatches/reload-all"), &["require_admin"], reload_all_mismatches)
            .get(&format!("{prefix}/category-mismatches/reload-status"), &["require_admin"], reload_status)
            .get(&format!("{prefix}/category-mismatches/{{*category}}"), &["require_admin"], mismatch_details);
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

#[derive(Deserialize)]
struct FilePath {
    #[serde(default)]
    file_path: String,
}

async fn index_file(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<FilePath>) -> ApiResult<Json<Value>> {
    if body.file_path.is_empty() {
        return Err(ApiError::http(400, "file_path is required"));
    }
    let (id, error) = manager(&deps, &uri)?.index_single_file(&body.file_path).await;
    Ok(Json(match (id, error) {
        (Some(book_id), None) => json!({"status":"success","result":{"book_id":book_id}}),
        (_, error) => json!({"status":"failure","error":error}),
    }))
}

async fn delete_file(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<FilePath>) -> ApiResult<Json<Value>> {
    if body.file_path.is_empty() {
        return Err(ApiError::http(400, "file_path is required"));
    }
    let (status, error) = manager(&deps, &uri)?.delete_file(&body.file_path).await;
    Ok(Json(if status == "Ok" { json!({"status":"success","result":status}) } else { json!({"status":"failure","error":error}) }))
}

async fn delete_es_doc(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(raw_id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&raw_id)?;
    let success = manager(&deps, &uri)?.es_manager().delete(id).unwrap_or(false);
    Ok(Json(if success { json!({"status":"success"}) } else { json!({"status":"failure","error":format!("ES 문서 삭제 실패: {id}")}) }))
}

#[derive(Deserialize)]
struct Category {
    category: String,
}

async fn reload_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Category>) -> ApiResult<Json<Value>> {
    let content_type = if uri.path().starts_with("/comics/") { "comic" } else { "book" };
    let (result, error) = manager(&deps, &uri)?.reload_category(&body.category, content_type).await;
    Ok(Json(match error {
        Some(error) => json!({"status":"failure","error":error}),
        None => json!({"status":"success","result":result}),
    }))
}

async fn mismatch_details(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(category): Path<String>) -> Json<Value> {
    let response = match manager(&deps, &uri).and_then(|bm| bm.get_category_mismatch_details(&category).map_err(|_| ApiError::unavailable())) {
        Ok(result) => json!({"status":"success","result":result}),
        Err(_) => json!({"status":"failure","error":"카테고리 불일치 조회 중 오류가 발생했습니다"}),
    };
    Json(response)
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
                    let latest_counts = Arc::new(Mutex::new(json!({})));
                    let progress_counts = latest_counts.clone();
                    let progress: ReloadProgress = Arc::new(move |counts| {
                        if let Ok(mut latest) = progress_counts.lock()
                            && let (Some(target), Some(source)) = (latest.as_object_mut(), counts.as_object())
                        {
                            target.extend(source.clone());
                        }
                    });
                    let heartbeat_mapping = mapping.clone();
                    let heartbeat_type = content_type.clone();
                    let heartbeat_category = category.clone();
                    let heartbeat_counts = latest_counts.clone();
                    let flusher = tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(flush_interval).await;
                            let counts = heartbeat_counts.lock().map(|value| value.clone()).unwrap_or_else(|_| json!({}));
                            if let Err(error) = heartbeat_mapping.heartbeat_reload_lock(&heartbeat_type, heartbeat_category.as_deref(), &counts) {
                                log::warn!("재적재 heartbeat 갱신 실패: {error}");
                            }
                        }
                    });
                    let (result, error) = match category.as_deref() {
                        Some(category) => manager.reload_category_mismatch_files(category, &content_type, Some(progress)).await,
                        None => manager.reload_category_mismatches(&content_type, Some(progress)).await,
                    };
                    flusher.abort();
                    let counts = json!({
                        "indexed_count": result["indexed_count"].as_i64().unwrap_or(0),
                        "deleted_count": result["deleted_count"].as_i64().unwrap_or(0),
                        "failed_count": result["failed_count"].as_i64().unwrap_or(0),
                        "before_count": result["before_count"].as_i64().unwrap_or(0),
                        "after_count": result["after_count"].as_i64().unwrap_or(0),
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
