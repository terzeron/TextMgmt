//! 도서·만화 수정과 카테고리 관리. 실제 파일 작업은 BookManager에 위임한다.
use super::{kind, manager};
use crate::book::Book;
use crate::book_manager::BookDeletion;
use crate::main::extract::python_bool;
use crate::main::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, ApplyCategoryChanges, BookManagerApi, CategoryMappingApi, MappingError, ProposeCategoryChanges, RequireAdmin, Routes};
use axum::{
    Json,
    extract::{OriginalUri, Path, State},
};
use fancy_regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path as FsPath, PathBuf},
    sync::{Arc, Mutex},
};

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .put(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], update_book)
            .delete(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], delete_book)
            .put(&format!("{prefix}/categories/rename"), &["require_admin"], rename_category)
            .post(&format!("{prefix}/categories/bulk-rename-files"), &["require_admin"], bulk_rename_files)
            .post(&format!("{prefix}/categories/delete"), &["require_admin"], delete_category)
            .get(&format!("{prefix}/category-delete-preview"), &["require_admin"], delete_preview)
            .delete(&format!("{prefix}/categories/classify-proposal"), &["require_admin"], delete_classify_proposal)
            .get(&format!("{prefix}/categories/classify-proposal"), &["require_admin"], get_classify_proposal)
            .post(&format!("{prefix}/categories/classify-proposal"), &["require_admin"], start_classify_proposal)
            .post(&format!("{prefix}/categories/classify-proposal/apply"), &["require_admin"], apply_classify_proposal)
            .delete(&format!("{prefix}/categories/classify-proposal/item"), &["require_admin"], delete_classify_proposal_item)
            .delete(&format!("{prefix}/categories/classify-proposal/applied"), &["require_admin"], delete_applied_classify_proposal_items)
            .get(&format!("{prefix}/similar-names"), &["require_admin"], similar_names)
            .get(&format!("{prefix}/category-pdf-stats"), &["require_admin"], pdf_stats);
    }
    routes
}

#[derive(Deserialize)]
struct ClassifyProposalBody {
    category: String,
    #[serde(default = "default_true")]
    use_bookstore: bool,
    #[serde(default = "default_true")]
    use_content_meta: bool,
    #[serde(default = "default_delay")]
    delay: f64,
}

#[derive(Deserialize)]
struct ClassifyApplyItem {
    file_path: String,
    target_category: String,
}

#[derive(Deserialize)]
struct ClassifyApplyBody {
    items: Vec<ClassifyApplyItem>,
    #[serde(default)]
    clean_existing: bool,
}

fn default_true() -> bool {
    true
}
fn default_delay() -> f64 {
    1.2
}

async fn start_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<ClassifyProposalBody>) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri).to_string();
    let manager = manager(&deps, &uri)?;
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let initial = json!({"status":"running","source_category":body.category,"total_count":0,"processed_count":0,"failures":[]});
    let start_mapping = mapping.clone();
    let start_type = content_type.clone();
    let (started, current) = tokio::task::spawn_blocking(move || start_mapping.try_start_classify_proposal(initial, &start_type, 300))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !started {
        return Ok(Json(json!({"status":"success","result":{"already_running":true,"status":current["status"],"content_type":content_type}})));
    }

    let job_type = content_type.clone();
    let runtime_factory = deps.task_runtime_factory();
    deps.task_runner().spawn(
        "classify-proposal",
        Box::new(move || {
            let failure_mapping = mapping.clone();
            let failure_type = job_type.clone();
            let runtime = match runtime_factory() {
                Ok(runtime) => runtime,
                Err(error) => {
                    log::error!("분류 제안 runtime 생성 실패: {error}");
                    set_proposal_failed(failure_mapping.as_ref(), &failure_type, "분류 제안에 실패했습니다.");
                    return;
                }
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                runtime.block_on(run_classify_proposal_job(manager, mapping, job_type, body.category, body.use_bookstore, body.use_content_meta, body.delay));
            }));
            if outcome.is_err() {
                log::error!("분류 제안 작업에서 panic 발생");
                set_proposal_failed(failure_mapping.as_ref(), &failure_type, "분류 제안에 실패했습니다.");
            }
        }),
    );
    Ok(Json(
        json!({"status":"success","result":{"started":true,"status":current["status"],"content_type":current["content_type"],"source_category":current["source_category"],"total_count":current["total_count"],"processed_count":current["processed_count"],"failures":current["failures"]}}),
    ))
}

async fn apply_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<ClassifyApplyBody>) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri).to_string();
    let manager = manager(&deps, &uri)?;
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let token = crate::auth::uuid_hex();
    let claim_mapping = mapping.clone();
    let claim_type = content_type.clone();
    let claim_token = token.clone();
    let (started, _) = tokio::task::spawn_blocking(move || claim_mapping.try_begin_classify_apply(&claim_token, &claim_type, 300))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if !started {
        return Ok(Json(json!({"status":"failure","error":"적용할 제안이 없습니다."})));
    }
    let proposal_items = tokio::task::spawn_blocking({
        let mapping = mapping.clone();
        let content_type = content_type.clone();
        move || mapping.get_classify_proposal_items(&content_type)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let allowed: HashSet<String> = proposal_items
        .iter()
        .filter_map(|item| {
            let status = item["apply_status"].as_str().unwrap_or("pending");
            matches!(status, "pending" | "failed" | "moving").then(|| item["file_path"].as_str().map(str::to_owned)).flatten()
        })
        .collect();
    let items: Vec<Value> = body.items.into_iter().map(|item| json!({"file_path":item.file_path,"target_category":item.target_category})).collect();
    let total_count = items.len();
    let job_type = content_type.clone();
    let runtime_factory = deps.task_runtime_factory();
    deps.task_runner().spawn(
        "classify-proposal-apply",
        Box::new(move || {
            let failure_mapping = mapping.clone();
            let failure_type = job_type.clone();
            let failure_token = token.clone();
            let runtime = match runtime_factory() {
                Ok(runtime) => runtime,
                Err(error) => {
                    log::error!("분류 적용 runtime 생성 실패: {error}");
                    if let Ok(status) = failure_mapping.get_classify_proposal_status(&failure_type, 300)
                        && status["apply_token"].as_str() == Some(failure_token.as_str())
                    {
                        set_proposal_failed(failure_mapping.as_ref(), &failure_type, "분류 적용에 실패했습니다.");
                    }
                    return;
                }
            };
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                runtime.block_on(run_classify_apply_job(manager, mapping, job_type, items, allowed, body.clean_existing, token));
            }));
            if outcome.is_err() {
                log::error!("분류 적용 작업에서 panic 발생");
                if let Ok(status) = failure_mapping.get_classify_proposal_status(&failure_type, 300)
                    && status["apply_token"].as_str() == Some(failure_token.as_str())
                {
                    set_proposal_failed(failure_mapping.as_ref(), &failure_type, "분류 적용에 실패했습니다.");
                }
            }
        }),
    );
    Ok(Json(json!({"status":"success","result":{"started":true,"total_count":total_count}})))
}

async fn run_classify_proposal_job(
    manager: Arc<dyn BookManagerApi>,
    mapping: Arc<dyn CategoryMappingApi>,
    content_type: String,
    category: String,
    use_bookstore: bool,
    use_content_meta: bool,
    delay: f64,
) {
    if let Err(error) = mapping.clear_classify_proposal_items(&content_type) {
        set_proposal_failed(mapping.as_ref(), &content_type, "분류 제안 초기화에 실패했습니다.");
        log::error!("분류 제안 항목 초기화 실패: {error}");
        return;
    }
    let mappings = match mapping.get_all_mappings(&content_type) {
        Ok(value) => value,
        Err(error) => {
            set_proposal_failed(mapping.as_ref(), &content_type, "분류 제안에 실패했습니다.");
            log::error!("분류 제안 매핑 조회 실패: {error}");
            return;
        }
    };
    let progress_mapping = mapping.clone();
    let progress_type = content_type.clone();
    let source_category = category.clone();
    let pending_new = Arc::new(Mutex::new(Vec::<Value>::new()));
    let pending_updated = Arc::new(Mutex::new(Vec::<Value>::new()));
    let flush_error = Arc::new(Mutex::new(None::<String>));
    let callback_new = pending_new.clone();
    let callback_updated = pending_updated.clone();
    let callback_flush_error = flush_error.clone();
    let on_progress = move |mut progress: Value| {
        if let Some(items) = progress.get("new_items").and_then(Value::as_array).cloned() {
            let mut pending = callback_new.lock().expect("proposal new-item buffer poisoned");
            pending.extend(items);
            if pending.len() >= 10 {
                let batch = std::mem::take(&mut *pending);
                if let Err(error) = progress_mapping.add_classify_proposal_items(&batch, &source_category, &progress_type) {
                    log::error!("분류 제안 항목 저장 실패: {error}");
                    callback_flush_error.lock().expect("proposal flush error poisoned").get_or_insert_with(|| error.to_string());
                }
            }
        }
        if let Some(items) = progress.get("updated_items").and_then(Value::as_array).cloned() {
            let mut pending = callback_updated.lock().expect("proposal updated-item buffer poisoned");
            pending.extend(items);
            if pending.len() >= 10 {
                let batch = std::mem::take(&mut *pending);
                if let Err(error) = progress_mapping.update_classify_proposal_item_payloads(&batch, &progress_type) {
                    log::error!("분류 제안 항목 payload 저장 실패: {error}");
                    callback_flush_error.lock().expect("proposal flush error poisoned").get_or_insert_with(|| error.to_string());
                }
            }
        }
        if let Some(object) = progress.as_object_mut() {
            object.remove("new_items");
            object.remove("updated_items");
        }
        if let Err(error) = progress_mapping.merge_classify_proposal_status(progress, &progress_type) {
            log::error!("분류 제안 진행 상태 저장 실패: {error}");
            callback_flush_error.lock().expect("proposal flush error poisoned").get_or_insert_with(|| error.to_string());
        }
    };
    let (result, error) = manager
        .propose_category_changes(ProposeCategoryChanges {
            category: &category,
            mappings: &mappings,
            content_type: &content_type,
            use_bookstore,
            use_content_meta,
            delay,
            on_progress: Some(&on_progress),
        })
        .await;
    flush_proposal_item_buffer(&pending_new, |items| {
        if let Err(error) = mapping.add_classify_proposal_items(items, &category, &content_type) {
            log::error!("분류 제안 항목 저장 실패: {error}");
            flush_error.lock().expect("proposal flush error poisoned").get_or_insert_with(|| error.to_string());
        }
    });
    flush_proposal_item_buffer(&pending_updated, |items| {
        if let Err(error) = mapping.update_classify_proposal_item_payloads(items, &content_type) {
            log::error!("분류 제안 항목 payload 저장 실패: {error}");
            flush_error.lock().expect("proposal flush error poisoned").get_or_insert_with(|| error.to_string());
        }
    });
    let final_mapping = mapping.clone();
    let final_type = content_type.clone();
    if flush_error.lock().expect("proposal flush error poisoned").is_some() {
        set_proposal_failed(final_mapping.as_ref(), &final_type, "분류 제안에 실패했습니다.");
        return;
    }
    if let Some(error) = error {
        set_proposal_failed(mapping.as_ref(), &content_type, &error);
        return;
    }
    let mut state = match mapping.get_classify_proposal_status(&content_type, 300) {
        Ok(state) => state,
        Err(error) => {
            log::error!("분류 제안 상태 조회 실패: {error}");
            set_proposal_failed(mapping.as_ref(), &content_type, "분류 제안에 실패했습니다.");
            return;
        }
    };
    if let (Some(state), Some(result)) = (state.as_object_mut(), result.as_object()) {
        for (key, value) in result {
            if key != "items" {
                state.insert(key.clone(), value.clone());
            }
        }
        state.insert("status".into(), json!("ready"));
    }
    if let Err(error) = mapping.set_classify_proposal_status(state, &content_type) {
        log::error!("분류 제안 완료 상태 저장 실패: {error}");
        set_proposal_failed(mapping.as_ref(), &content_type, "분류 제안에 실패했습니다.");
    }
}

fn flush_proposal_item_buffer(buffer: &Mutex<Vec<Value>>, flush: impl FnOnce(&[Value])) {
    let items = std::mem::take(&mut *buffer.lock().expect("proposal item buffer poisoned"));
    if !items.is_empty() {
        flush(&items);
    }
}

fn set_proposal_failed(mapping: &dyn CategoryMappingApi, content_type: &str, error: &str) {
    let mut state = match mapping.get_classify_proposal_status(content_type, 300) {
        Ok(state) => state,
        Err(error) => {
            log::error!("분류 제안 실패 상태를 기록하기 위한 조회 실패: {error}");
            return;
        }
    };
    state["status"] = json!("failed");
    state["error"] = json!(error);
    if let Err(save_error) = mapping.set_classify_proposal_status(state, content_type) {
        log::error!("분류 제안 실패 상태 저장 실패: {save_error}");
    }
}

async fn run_classify_apply_job(
    manager: Arc<dyn BookManagerApi>,
    mapping: Arc<dyn CategoryMappingApi>,
    content_type: String,
    items: Vec<Value>,
    allowed: HashSet<String>,
    clean_existing: bool,
    token: String,
) {
    let callback_error = Arc::new(Mutex::new(None::<String>));
    let progress_mapping = mapping.clone();
    let progress_type = content_type.clone();
    let progress_error = callback_error.clone();
    let on_progress = move |progress: Value| {
        if let Err(error) = progress_mapping.merge_classify_proposal_status(progress, &progress_type) {
            log::error!("분류 적용 진행 상태 저장 실패: {error}");
            progress_error.lock().expect("proposal apply callback error poisoned").get_or_insert_with(|| error.to_string());
        }
    };
    let status_mapping = mapping.clone();
    let status_type = content_type.clone();
    let status_allowed = allowed.clone();
    let item_status_error = callback_error.clone();
    let on_item_done = move |entry: Value| {
        let path = entry["file_path"].as_str().unwrap_or_default();
        if status_allowed.contains(path)
            && let Err(error) = status_mapping.update_classify_proposal_item_status(path, entry["apply_status"].as_str().unwrap_or("failed"), entry["apply_error"].as_str(), &status_type)
        {
            log::error!("분류 적용 항목 상태 저장 실패({path}): {error}");
            item_status_error.lock().expect("proposal apply callback error poisoned").get_or_insert_with(|| error.to_string());
        }
    };
    let continue_mapping = mapping.clone();
    let continue_type = content_type.clone();
    let continue_token = token.clone();
    let should_continue = move || match continue_mapping.get_classify_proposal_status(&continue_type, 300) {
        Ok(status) => status["apply_token"].as_str() == Some(continue_token.as_str()),
        Err(error) => {
            log::error!("분류 적용 상태 조회 실패, 작업을 중단한다: {error}");
            false
        }
    };
    let (result, error) = manager
        .apply_category_changes_with(ApplyCategoryChanges {
            items: &items,
            allowed_file_paths: &allowed,
            content_type: &content_type,
            clean_existing,
            on_progress: Some(&on_progress),
            on_item_done: Some(&on_item_done),
            should_continue: Some(&should_continue),
        })
        .await;
    if !should_continue() {
        log::warn!("분류 적용 작업 중단: apply token이 바뀌었다");
        return;
    }
    if callback_error.lock().expect("proposal apply callback error poisoned").is_some() {
        set_proposal_failed(mapping.as_ref(), &content_type, "분류 적용에 실패했습니다.");
        return;
    }
    if let Some(error) = error {
        set_proposal_failed(mapping.as_ref(), &content_type, &error);
        return;
    }
    let mut state = match mapping.get_classify_proposal_status(&content_type, 300) {
        Ok(state) => state,
        Err(error) => {
            log::error!("분류 적용 완료 상태 조회 실패: {error}");
            set_proposal_failed(mapping.as_ref(), &content_type, "분류 적용에 실패했습니다.");
            return;
        }
    };
    state["status"] = json!("done");
    state["applied_count"] = result["applied_count"].clone();
    state["failed_count"] = result["failed_count"].clone();
    if let Err(error) = mapping.set_classify_proposal_status(state, &content_type) {
        log::error!("분류 적용 완료 상태 저장 실패: {error}");
        set_proposal_failed(mapping.as_ref(), &content_type, "분류 적용에 실패했습니다.");
    }
}

async fn get_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri).to_string();
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let status = tokio::task::spawn_blocking({
        let mapping = mapping.clone();
        let content_type = content_type.clone();
        move || mapping.get_classify_proposal_status(&content_type, 300)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    let items =
        tokio::task::spawn_blocking(move || mapping.get_classify_proposal_items(&content_type)).await.map_err(|e| ApiError::Internal(e.to_string()))?.map_err(|e| ApiError::Internal(e.to_string()))?;
    let manager = manager(&deps, &uri)?;
    let path_prefix = manager.path_prefix();
    let items = items
        .into_iter()
        .map(|mut item| {
            if item.get("book_id").is_none_or(Value::is_null)
                && let Some(file_path) = item.get("file_path").and_then(Value::as_str)
            {
                item["book_id"] = crate::book_manager::BookManager::_book_id_for_path(&path_prefix.join(file_path)).map_or(Value::Null, |id| json!(id));
            }
            item
        })
        .collect::<Vec<_>>();
    let mut result = status.as_object().cloned().unwrap_or_default();
    result.insert("items".into(), json!(items));
    Ok(Json(json!({"status":"success","result":result})))
}

#[derive(Deserialize)]
struct ProposalItemQuery {
    file_path: String,
}

async fn delete_classify_proposal_item(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<ProposalItemQuery>) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri).to_string();
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let status = tokio::task::spawn_blocking({
        let mapping = mapping.clone();
        let content_type = content_type.clone();
        move || mapping.get_classify_proposal_status(&content_type, 300)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if matches!(status["status"].as_str(), Some("running" | "applying")) {
        return Ok(Json(json!({"status":"failure","error":"작업이 진행 중입니다."})));
    }
    let deleted = tokio::task::spawn_blocking(move || mapping.delete_classify_proposal_item(&query.file_path, &content_type))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(json!({"status":"success","result":{"deleted_count":deleted}})))
}

async fn delete_applied_classify_proposal_items(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri).to_string();
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let status = tokio::task::spawn_blocking({
        let mapping = mapping.clone();
        let content_type = content_type.clone();
        move || mapping.get_classify_proposal_status(&content_type, 300)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if matches!(status["status"].as_str(), Some("running" | "applying")) {
        return Ok(Json(json!({"status":"failure","error":"작업이 진행 중입니다."})));
    }
    let deleted = tokio::task::spawn_blocking(move || mapping.delete_applied_classify_proposal_items(&content_type))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(Json(json!({"status":"success","result":{"deleted_count":deleted}})))
}

async fn delete_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri);
    let mapping = deps.category_mapping().map_err(|e| {
        log::warn!("분류 제안 상태 저장소를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })?;
    let current = tokio::task::spawn_blocking({
        let mapping = mapping.clone();
        let content_type = content_type.to_string();
        move || mapping.get_classify_proposal_status(&content_type, 300)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    if matches!(current["status"].as_str(), Some("running" | "applying")) {
        return Ok(Json(json!({"status":"failure","error":"작업이 진행 중입니다."})));
    }
    tokio::task::spawn_blocking(move || {
        mapping.clear_classify_proposal_items(content_type).map_err(|error| ApiError::Internal(error.to_string()))?;
        mapping.set_classify_proposal_status(json!({"status":"idle","content_type":content_type}), content_type).map_err(|error| ApiError::Internal(error.to_string()))?;
        Ok::<(), ApiError>(())
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))??;
    Ok(Json(json!({"status":"success","result":{"cleared":true}})))
}

#[derive(Deserialize)]
struct SimilarNamesQuery {
    name: String,
    #[serde(default, rename = "author")]
    _author: String,
    #[serde(default)]
    exclude_category: String,
}

fn normalized_category_name(value: &str) -> Vec<char> {
    pyfmt::py_casefold(value).replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().collect()
}

fn sequence_ratio(a: &[char], b: &[char]) -> f64 {
    let (mut stack, mut matches) = (vec![(0, a.len(), 0, b.len())], 0usize);
    while let Some((alo, ahi, blo, bhi)) = stack.pop() {
        let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0usize);
        let mut previous = vec![0usize; bhi.saturating_sub(blo) + 1];
        for (i, a_char) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut current = vec![0usize; previous.len()];
            for j in blo..bhi {
                if *a_char == b[j] {
                    current[j - blo + 1] = previous[j - blo] + 1;
                    if current[j - blo + 1] > best_size {
                        (best_i, best_j, best_size) = (i + 1 - current[j - blo + 1], j + 1 - current[j - blo + 1], current[j - blo + 1]);
                    }
                }
            }
            previous = current;
        }
        if best_size > 0 {
            matches += best_size;
            if alo < best_i && blo < best_j {
                stack.push((alo, best_i, blo, best_j));
            }
            if best_i + best_size < ahi && best_j + best_size < bhi {
                stack.push((best_i + best_size, ahi, best_j + best_size, bhi));
            }
        }
    }
    if a.is_empty() && b.is_empty() { 1.0 } else { 2.0 * matches as f64 / (a.len() + b.len()) as f64 }
}

async fn similar_names(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<SimilarNamesQuery>) -> ApiResult<Json<Value>> {
    let failure = |error: &str| Json(json!({"status":"failure","error":error}));
    let name = query.name.trim();
    if name.is_empty() {
        return Ok(failure("name is required"));
    }
    let manager = manager(&deps, &uri)?;
    let categories = match manager.get_categories().await {
        Ok(categories) => categories,
        Err(error) => return Ok(failure(&error)),
    };
    let categories: Vec<String> = match categories {
        Value::Array(values) => values.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Value::Object(values) => values.into_iter().map(|(key, _)| key).collect(),
        _ => Vec::new(),
    };
    let normalized = normalized_category_name(name);
    let excluded_prefix = (!query.exclude_category.is_empty()).then(|| format!("{}/", query.exclude_category));
    let mut matches: Vec<(String, f64)> = categories
        .into_iter()
        .filter(|category| *category != "_root" && *category != query.exclude_category && excluded_prefix.as_ref().is_none_or(|prefix| !category.starts_with(prefix)))
        .filter_map(|category| {
            let leaf = category.rsplit('/').next().unwrap_or(&category);
            let score = sequence_ratio(&normalized, &normalized_category_name(leaf));
            (score >= 0.35).then_some((category, score))
        })
        .collect();
    let mut stats_categories: Vec<String> = matches.iter().map(|(category, _)| category.clone()).collect();
    if !query.exclude_category.is_empty() && query.exclude_category != "_root" {
        stats_categories.push(query.exclude_category.clone());
    }
    let es = manager.es_manager();
    let stats = tokio::task::spawn_blocking(move || es.search_categories_pdf_stats(&stats_categories)).await;
    let stats = match stats {
        Ok(Ok(stats)) => stats,
        Err(error) => {
            log::error!("similar names join failed: {error}");
            return Ok(failure("유사 이름 검색에 실패했습니다."));
        }
        Ok(Err(error)) => {
            log::error!("similar names ES lookup failed: {error}");
            return Ok(failure("유사 이름 검색에 실패했습니다."));
        }
    };
    let reference = stats.get(&query.exclude_category).cloned().unwrap_or(Value::Null);
    let mut result: Vec<Value> = matches.drain(..).map(|(category, name_score)| {
        let value = stats.get(&category).cloned().unwrap_or(Value::Null);
        let mut score = name_score * 70.0;
        for key in ["file_count", "total_file_size", "page_count"] {
            let v = value[key].as_i64().unwrap_or(0);
            let r = reference[key].as_i64().unwrap_or(0);
            if !reference.is_null() { score += if v == r { 10.0 } else if v.max(r) > 0 { 10.0 * v.min(r) as f64 / v.max(r) as f64 } else { 0.0 }; }
        }
        json!({"kind":"directory","id":category,"category":category,"label":category,"file_count":value["file_count"].as_i64().unwrap_or(0),"page_count":value["page_count"].as_i64().unwrap_or(0),"total_file_size":value["total_file_size"].as_i64().unwrap_or(0),"score":score})
    }).collect();
    result.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap_or(0.0)
            .total_cmp(&a["score"].as_f64().unwrap_or(0.0))
            .then_with(|| pyfmt::py_casefold(a["label"].as_str().unwrap_or("")).cmp(&pyfmt::py_casefold(b["label"].as_str().unwrap_or(""))))
    });
    result.truncate(10);
    Ok(Json(json!({"status":"success","result":result})))
}

#[derive(Deserialize)]
struct BulkRenameFiles {
    category: String,
    pattern: String,
    replacement: String,
}

async fn bulk_rename_files(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<BulkRenameFiles>) -> ApiResult<Json<Value>> {
    if body.category.is_empty() || body.category == "_root" {
        return Err(ApiError::http(400, "A directory category is required"));
    }
    if body.pattern.is_empty() || body.pattern.chars().count() > 256 {
        return Ok(Json(json!({"status":"failure","error":"정규표현식은 1~256자여야 합니다.","changed_count":0})));
    }
    let regex = match Regex::new(&body.pattern) {
        Ok(regex) => regex,
        Err(error) => return Ok(Json(json!({"status":"failure","error":format!("정규표현식 오류: {error}"),"changed_count":0}))),
    };
    let manager = manager(&deps, &uri)?;
    let categories = match manager.get_categories().await {
        Ok(categories) => categories,
        Err(error) => return Ok(Json(json!({"status":"failure","error":error,"changed_count":0}))),
    };
    let mut books = Vec::<Book>::new();
    if let Some(categories) = categories.as_array() {
        for category in categories.iter().filter_map(Value::as_str).filter(|category| *category == body.category || category.starts_with(&format!("{}/", body.category))) {
            let mut cursor: Option<String> = None;
            loop {
                let page = match manager.get_books_in_category_paged(category, None, cursor.as_deref()).await {
                    Ok(page) => page,
                    Err(error) => return Ok(Json(json!({"status":"failure","error":error,"changed_count":0}))),
                };
                books.extend(page.books);
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
            }
        }
    }
    books.sort_by(|a, b| pyfmt::py_casefold(&a.file_path.to_string_lossy()).cmp(&pyfmt::py_casefold(&b.file_path.to_string_lossy())));
    let mut plans = Vec::new();
    for book in books {
        let name = book.file_path.file_name().unwrap_or_default().to_string_lossy();
        let changed = match regex.try_replacen(&name, 0, body.replacement.as_str()) {
            Ok(value) => value.into_owned(),
            Err(error) => return Ok(Json(json!({"status":"failure","error":format!("정규표현식 오류: {error}"),"changed_count":0}))),
        };
        if changed.is_empty() || changed == "." || changed == ".." || changed.contains('/') || changed.contains('\\') {
            return Ok(Json(json!({"status":"failure","error":format!("변경할 수 없는 파일 이름입니다: {changed}"),"changed_count":0})));
        }
        if changed != name {
            plans.push((book, changed));
        }
    }

    let fs = deps.fs();
    let mut reserved = HashSet::<PathBuf>::new();
    let mut changed_count = 0usize;
    let mut failures = Vec::new();
    for (book, desired_name) in plans {
        let source = &book.file_path;
        let mut target = source.with_file_name(&desired_name);
        let exists = fs.is_file(&target).unwrap_or(false);
        let same = exists && fs.same_file(source, &target).unwrap_or(false);
        if (exists && !same) || reserved.contains(&target) {
            let extension = FsPath::new(&desired_name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let stem = desired_name.strip_suffix(&extension).unwrap_or(&desired_name);
            let serial_re = Regex::new(&format!(r"^{} \((\d+)\){}$", fancy_regex::escape(stem), fancy_regex::escape(&extension))).expect("escaped serial regex");
            let mut largest = 0u64;
            if exists && !same {
                match fs.read_dir(source.parent().unwrap_or_else(|| FsPath::new("."))) {
                    Ok(entries) => {
                        for entry in entries {
                            if let Some(file_name) = entry.file_name().map(|n| n.to_string_lossy().into_owned())
                                && let Ok(Some(captures)) = serial_re.captures(&file_name)
                            {
                                largest = largest.max(captures.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
                            }
                        }
                    }
                    Err(error) => {
                        failures.push(json!({"file":source.file_name().unwrap_or_default().to_string_lossy(),"error":error.to_string()}));
                        continue;
                    }
                }
            } else {
                for path in &reserved {
                    if let Some(file_name) = path.file_name().map(|n| n.to_string_lossy())
                        && let Ok(Some(captures)) = serial_re.captures(&file_name)
                    {
                        largest = largest.max(captures.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
                    }
                }
            }
            let mut serial = largest + 1;
            loop {
                let candidate = format!("{stem} ({serial}){extension}");
                target = source.with_file_name(&candidate);
                if !fs.is_file(&target).unwrap_or(false) && !reserved.contains(&target) {
                    break;
                }
                serial += 1;
            }
        }
        let extension = FsPath::new(&target).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let title = target.file_name().unwrap_or_default().to_string_lossy();
        let title = if extension.is_empty() { title.to_string() } else { title.strip_suffix(&format!(".{extension}")).unwrap_or(&title).to_string() };
        match manager.update_book(book.book_id, &book.category, &title, &book.author, &target, &book.file_type, false).await {
            Ok(()) => {
                changed_count += 1;
                reserved.insert(target);
            }
            Err(error) => failures.push(json!({"file":source.file_name().unwrap_or_default().to_string_lossy(),"error":error})),
        }
    }
    if let Err(error) = manager.es_manager().refresh() {
        log::warn!("bulk rename 후 색인 갱신 실패: {error}");
    }
    let failed_count = failures.len();
    failures.truncate(20);
    Ok(Json(json!({"status":"success","result":{"changed_count":changed_count,"failed_count":failed_count,"failures":failures}})))
}
fn result(value: Value, error: Option<String>) -> Json<Value> {
    Json(match error {
        Some(error) => json!({"status":"failure","error":error}),
        None => json!({"status":"success","result":value}),
    })
}
async fn mapping<T: Send + 'static>(deps: Arc<AppDeps>, action: impl FnOnce(Arc<dyn CategoryMappingApi>) -> Result<T, MappingError> + Send + 'static) -> ApiResult<T> {
    let result = tokio::task::spawn_blocking(move || deps.category_mapping().map_err(MappingError::Other).and_then(action)).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    result.map_err(|e| match e {
        MappingError::Http(code, message) => ApiError::http(code, &message),
        MappingError::Other(message) => ApiError::Internal(message),
    })
}

#[derive(Deserialize)]
struct ForceQuery {
    #[serde(default, deserialize_with = "python_bool")]
    force: bool,
}
// 사용하지 않는 BookModel 필드도 HTTP 입력 계약에 따라 검증한다.
#[derive(Deserialize)]
struct BookModel {
    #[serde(rename = "book_id")]
    _book_id: i64,
    category: String,
    title: String,
    author: String,
    file_path: String,
    file_type: String,
    #[serde(rename = "file_size")]
    _file_size: i64,
    #[serde(rename = "updated_time")]
    _updated_time: String,
    #[serde(default, rename = "line_count")]
    _line_count: i64,
    #[serde(default, rename = "page_count")]
    _page_count: i64,
    #[serde(default, rename = "isbn")]
    _isbn: String,
    #[serde(default, rename = "created_time")]
    _created_time: String,
    #[serde(default, rename = "score")]
    _score: f64,
}
async fn update_book(
    _: RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<ForceQuery>,
    ApiJson(body): ApiJson<BookModel>,
) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = manager(&deps, &uri)?;
    let path = manager.path_prefix().join(&body.file_path);
    Ok(Json(match manager.update_book(id, &body.category, &body.title, &body.author, &path, &body.file_type, query.force).await {
        Ok(()) => json!({"status":"success","result":"Ok"}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}
async fn delete_book(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    Ok(Json(match manager(&deps, &uri)?.delete_book(id).await {
        Ok(BookDeletion::Done) => json!({"status":"success","result":"Ok"}),
        Ok(BookDeletion::DoneWithWarning(warning)) => json!({"status":"success","result":"Warning","warning":warning}),
        Err(error) => json!({"status":"failure","error":error}),
    }))
}
#[derive(Deserialize)]
struct Rename {
    old_category: String,
    new_category: String,
}
async fn rename_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Rename>) -> ApiResult<Json<Value>> {
    let (mut value, error) = manager(&deps, &uri)?.rename_category(&body.old_category, &body.new_category).await;
    if error.is_none() {
        let ct = kind(&uri);
        let updated = mapping(deps, move |m| m.rename_category(&body.old_category, &body.new_category, ct)).await?;
        if !updated {
            log::warn!("rename_category: MySQL 매핑 갱신 실패");
        }
        value["mapping_updated"] = json!(updated);
    }
    Ok(result(value, error))
}
#[derive(Deserialize)]
struct Delete {
    category: String,
    #[serde(default, deserialize_with = "python_bool")]
    delete_files: bool,
}
async fn delete_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Delete>) -> ApiResult<Json<Value>> {
    let (mut value, error) = manager(&deps, &uri)?.delete_category(&body.category, body.delete_files).await;
    if error.is_none() {
        let ct = kind(&uri);
        let deleted = mapping(deps, move |m| {
            let deleted = m.delete_category(&body.category, ct, true)?;
            let prefix = format!("{}/", body.category);
            m.set_hidden(&body.category, false, ct)?;
            for hidden in m.get_hidden_categories(ct)? {
                if hidden.starts_with(&prefix) {
                    m.set_hidden(&hidden, false, ct)?;
                }
            }
            m.set_latest_excluded(&body.category, false, ct)?;
            for excluded in m.get_latest_excluded_categories(ct)? {
                if excluded.starts_with(&prefix) {
                    m.set_latest_excluded(&excluded, false, ct)?;
                }
            }
            Ok(deleted)
        })
        .await?;
        if !deleted {
            log::warn!("delete_category: MySQL 키워드 매핑 삭제 대상 없음");
        }
        value["mapping_deleted"] = json!(deleted);
    }
    Ok(result(value, error))
}
#[derive(Deserialize)]
struct CategoryQuery {
    category: String,
}
async fn delete_preview(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    let (value, error) = manager(&deps, &uri)?.preview_delete_category(&query.category).await;
    Ok(result(value, error))
}
async fn pdf_stats(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    if query.category.is_empty() || query.category == "_root" {
        return Err(ApiError::http(400, "A directory category is required"));
    }
    let es = manager(&deps, &uri)?.es_manager();
    match tokio::task::spawn_blocking(move || es.search_category_pdf_stats(&query.category)).await {
        Ok(Ok(value)) => Ok(result(value, None)),
        error => {
            log::error!("get_category_pdf_stats failed: {error:?}");
            Ok(result(Value::Null, Some("PDF 통계를 불러오지 못했습니다.".into())))
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/rust/backend/admin_proposal_unit.rs"]
mod proposal_tests;

#[cfg(test)]
mod tests {
    use super::{ClassifyProposalBody, flush_proposal_item_buffer, kind, manager, mapping, normalized_category_name, result, run_classify_apply_job, sequence_ratio, set_proposal_failed};
    use crate::main::{ApiError, AppDeps, ApplyCategoryChanges, AuthMode, BookManagerApi, CategoryMappingApi, MappingError};
    use axum::http::Uri;
    use serde_json::json;
    use std::{collections::HashSet, sync::Mutex};

    struct StatusMapping {
        read_error: bool,
        write_error: bool,
        state: Mutex<serde_json::Value>,
        writes: Mutex<usize>,
    }

    impl CategoryMappingApi for StatusMapping {
        fn get_classify_proposal_status(&self, _: &str, _: i64) -> Result<serde_json::Value, MappingError> {
            if self.read_error { Err(MappingError::Other("read failed".into())) } else { Ok(self.state.lock().unwrap().clone()) }
        }

        fn set_classify_proposal_status(&self, state: serde_json::Value, _: &str) -> Result<(), MappingError> {
            *self.writes.lock().unwrap() += 1;
            if self.write_error {
                Err(MappingError::Other("write failed".into()))
            } else {
                *self.state.lock().unwrap() = state;
                Ok(())
            }
        }
    }

    fn status_mapping(read_error: bool, write_error: bool) -> StatusMapping {
        StatusMapping { read_error, write_error, state: Mutex::new(json!({"status":"running"})), writes: Mutex::new(0) }
    }

    struct ApplyMapping {
        state: Mutex<serde_json::Value>,
        events: Mutex<Vec<String>>,
        merge_error: bool,
        read_error: bool,
    }

    impl CategoryMappingApi for ApplyMapping {
        fn get_classify_proposal_status(&self, _: &str, _: i64) -> Result<serde_json::Value, MappingError> {
            if self.read_error { Err(MappingError::Other("status read failed".into())) } else { Ok(self.state.lock().unwrap().clone()) }
        }

        fn set_classify_proposal_status(&self, state: serde_json::Value, _: &str) -> Result<(), MappingError> {
            *self.state.lock().unwrap() = state;
            Ok(())
        }

        fn merge_classify_proposal_status(&self, fields: serde_json::Value, _: &str) -> Result<(), MappingError> {
            if self.merge_error {
                return Err(MappingError::Other("progress write failed".into()));
            }
            if let (Some(state), Some(fields)) = (self.state.lock().unwrap().as_object_mut(), fields.as_object()) {
                state.extend(fields.clone());
            }
            Ok(())
        }

        fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, _: Option<&str>, _: &str) -> Result<(), MappingError> {
            self.events.lock().unwrap().push(format!("{file_path}:{apply_status}"));
            Ok(())
        }
    }

    struct ApplyManager {
        error: Option<&'static str>,
    }

    #[async_trait::async_trait]
    impl BookManagerApi for ApplyManager {
        async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> (serde_json::Value, Option<String>) {
            if let Some(on_progress) = request.on_progress {
                on_progress(json!({"processed_count":1}));
            }
            if let Some(on_item_done) = request.on_item_done {
                on_item_done(json!({"file_path":"a.epub","apply_status":"moved"}));
            }
            if let Some(should_continue) = request.should_continue {
                let _ = should_continue();
            }
            (json!({"applied_count":1,"failed_count":0}), self.error.map(str::to_string))
        }
    }

    fn apply_mapping(token: &str, merge_error: bool) -> ApplyMapping {
        ApplyMapping { state: Mutex::new(json!({"status":"applying","apply_token":token})), events: Mutex::new(Vec::new()), merge_error, read_error: false }
    }

    async fn run_apply(mapping: ApplyMapping, manager_error: Option<&'static str>, token: &str) -> ApplyMapping {
        let mapping = std::sync::Arc::new(mapping);
        let manager = std::sync::Arc::new(ApplyManager { error: manager_error });
        let mut allowed = HashSet::new();
        allowed.insert("a.epub".to_string());
        run_classify_apply_job(manager, mapping.clone(), "book".into(), vec![json!({"file_path":"a.epub","target_category":"fiction"})], allowed, false, token.into()).await;
        std::sync::Arc::try_unwrap(mapping).ok().unwrap()
    }

    #[tokio::test]
    async fn mapping_runs_action_and_converts_mapping_errors() {
        let deps = std::sync::Arc::new(AppDeps::new(AuthMode::Cookies).with_category_mapping(std::sync::Arc::new(status_mapping(false, false))));

        let value = mapping(deps.clone(), |_| Ok::<_, MappingError>(42)).await.unwrap();
        assert_eq!(value, 42);

        let http_error = mapping(deps.clone(), |_| Err::<(), _>(MappingError::Http(409, "conflict".into()))).await.unwrap_err();
        assert!(matches!(http_error, ApiError::Http(error) if error.status_code == 409 && error.detail == "conflict"));

        let other_error = mapping(deps, |_| Err::<(), _>(MappingError::Other("storage failed".into()))).await.unwrap_err();
        assert!(matches!(other_error, ApiError::Internal(message) if message == "storage failed"));
    }

    #[tokio::test]
    async fn apply_job_persists_progress_item_and_success_state() {
        let mapping = run_apply(apply_mapping("token", false), None, "token").await;

        assert_eq!(mapping.events.lock().unwrap().as_slice(), ["a.epub:moved"]);
        assert_eq!(*mapping.state.lock().unwrap(), json!({"status":"done","apply_token":"token","processed_count":1,"applied_count":1,"failed_count":0}));
    }

    #[tokio::test]
    async fn apply_job_marks_failure_after_callback_or_manager_errors() {
        let callback_failure = run_apply(apply_mapping("token", true), None, "token").await;
        assert_eq!(callback_failure.state.lock().unwrap()["status"], "failed");
        assert_eq!(callback_failure.state.lock().unwrap()["error"], "분류 적용에 실패했습니다.");

        let manager_failure = run_apply(apply_mapping("token", false), Some("manager failed"), "token").await;
        assert_eq!(manager_failure.state.lock().unwrap()["status"], "failed");
        assert_eq!(manager_failure.state.lock().unwrap()["error"], "manager failed");
    }

    #[tokio::test]
    async fn apply_job_does_not_finalize_after_its_token_is_replaced() {
        let mapping = run_apply(apply_mapping("new-token", false), None, "old-token").await;

        assert_eq!(mapping.state.lock().unwrap()["status"], "applying");
        assert_eq!(mapping.state.lock().unwrap()["apply_token"], "new-token");
    }

    #[tokio::test]
    async fn apply_job_stops_when_status_cannot_be_read() {
        let mut mapping = apply_mapping("token", false);
        mapping.read_error = true;
        let mapping = run_apply(mapping, None, "token").await;

        assert_eq!(mapping.state.lock().unwrap()["status"], "applying");
        assert_eq!(mapping.state.lock().unwrap()["apply_token"], "token");
    }

    #[test]
    fn classify_proposal_body_deserializes_python_defaults() {
        let body: ClassifyProposalBody = serde_json::from_value(json!({"category":"inbox"})).unwrap();
        assert!(body.use_bookstore);
        assert!(body.use_content_meta);
        assert_eq!(body.delay, 1.2);
    }

    #[test]
    fn category_similarity_handles_empty_equal_and_disjoint_names() {
        assert_eq!(normalized_category_name(" A__B--C "), "a b c".chars().collect::<Vec<_>>());
        assert_eq!(sequence_ratio(&[], &[]), 1.0);
        assert_eq!(sequence_ratio(&"same".chars().collect::<Vec<_>>(), &"same".chars().collect::<Vec<_>>()), 1.0);
        assert_eq!(sequence_ratio(&"abc".chars().collect::<Vec<_>>(), &"xyz".chars().collect::<Vec<_>>()), 0.0);
    }

    #[test]
    fn content_type_and_manager_follow_the_original_uri() {
        let deps = AppDeps::new(AuthMode::Cookies);
        let book_uri = Uri::from_static("/books/1");
        let comic_uri = Uri::from_static("/comics/books/1");

        assert_eq!(kind(&book_uri), "book");
        assert_eq!(kind(&comic_uri), "comic");
        let book_manager = manager(&deps, &book_uri).unwrap();
        let comic_manager = manager(&deps, &comic_uri).unwrap();
        assert!(!std::sync::Arc::ptr_eq(&book_manager, &comic_manager));
    }

    #[test]
    fn proposal_item_buffer_flushes_once_and_becomes_empty() {
        let buffer = Mutex::new(vec![json!({"file_path":"a.txt"}), json!({"file_path":"b.txt"})]);
        let mut flushed = Vec::new();

        flush_proposal_item_buffer(&buffer, |items| flushed.extend_from_slice(items));

        assert_eq!(flushed, vec![json!({"file_path":"a.txt"}), json!({"file_path":"b.txt"})]);
        assert!(buffer.lock().unwrap().is_empty());
    }

    #[test]
    fn proposal_item_buffer_skips_empty_flushes() {
        let buffer = Mutex::new(Vec::new());
        let mut called = false;

        flush_proposal_item_buffer(&buffer, |_| called = true);

        assert!(!called);
    }

    #[test]
    fn failed_proposal_status_updates_existing_state() {
        let mapping = status_mapping(false, false);
        set_proposal_failed(&mapping, "book", "proposal failed");
        assert_eq!(*mapping.state.lock().unwrap(), json!({"status":"failed","error":"proposal failed"}));
        assert_eq!(*mapping.writes.lock().unwrap(), 1);
    }

    #[test]
    fn failed_proposal_status_handles_read_and_write_errors() {
        let read_error = status_mapping(true, false);
        set_proposal_failed(&read_error, "book", "proposal failed");
        assert_eq!(*read_error.writes.lock().unwrap(), 0);

        let write_error = status_mapping(false, true);
        set_proposal_failed(&write_error, "book", "proposal failed");
        assert_eq!(*write_error.writes.lock().unwrap(), 1);
        assert_eq!(write_error.state.lock().unwrap()["status"], "running");
    }

    #[test]
    fn result_serializes_success_and_failure_shapes() {
        let success = result(json!({"count":2}), None);
        assert_eq!(success.0, json!({"status":"success","result":{"count":2}}));

        let failure = result(serde_json::Value::Null, Some("failed".into()));
        assert_eq!(failure.0, json!({"status":"failure","error":"failed"}));
    }
}
