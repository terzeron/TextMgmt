//! 자동 분류 제안 작업의 시작·조회·적용·삭제 라우트.

use super::*;

#[derive(Deserialize)]
pub(super) struct ClassifyProposalBody {
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
pub(super) struct ClassifyApplyBody {
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

pub(super) async fn start_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<ClassifyProposalBody>) -> ApiResult<Json<Value>> {
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

pub(super) async fn apply_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<ClassifyApplyBody>) -> ApiResult<Json<Value>> {
    let content_type = kind(&uri).to_string();
    let manager = manager(&deps, &uri)?;
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let token = crate::auth_service::uuid_hex();
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
    let proposed = manager
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
    let result = match proposed {
        Ok(result) => result,
        Err(error) => {
            set_proposal_failed(mapping.as_ref(), &content_type, &error);
            return;
        }
    };
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
    let applied = manager
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
    let result = match applied {
        Ok(result) => result,
        Err(error) => {
            set_proposal_failed(mapping.as_ref(), &content_type, &error);
            return;
        }
    };
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

pub(super) async fn get_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
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
pub(super) struct ProposalItemQuery {
    file_path: String,
}

pub(super) async fn delete_classify_proposal_item(
    _: RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<ProposalItemQuery>,
) -> ApiResult<Json<Value>> {
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

pub(super) async fn delete_applied_classify_proposal_items(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
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

pub(super) async fn delete_classify_proposal(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
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

#[cfg(test)]
mod proposal_tests {
    use super::run_classify_proposal_job;
    use crate::main::{BookManagerApi, CategoryMappingApi, MappingError, ProposeCategoryChanges};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    struct ProposalSetupMapping {
        clear_error: bool,
        mappings_error: bool,
        state: Mutex<Value>,
    }

    impl CategoryMappingApi for ProposalSetupMapping {
        fn clear_classify_proposal_items(&self, _: &str) -> Result<(), MappingError> {
            if self.clear_error { Err(MappingError::Other("clear failed".into())) } else { Ok(()) }
        }

        fn get_all_mappings(&self, _: &str) -> Result<Value, MappingError> {
            if self.mappings_error { Err(MappingError::Other("mappings failed".into())) } else { Ok(json!({})) }
        }

        fn get_classify_proposal_status(&self, _: &str, _: i64) -> Result<Value, MappingError> {
            Ok(self.state.lock().unwrap().clone())
        }

        fn set_classify_proposal_status(&self, state: Value, _: &str) -> Result<(), MappingError> {
            *self.state.lock().unwrap() = state;
            Ok(())
        }
    }

    struct NoProposalManager;

    impl BookManagerApi for NoProposalManager {}

    struct ProposalFlowMapping {
        state: Mutex<Value>,
        events: Mutex<Vec<String>>,
        fail_add: bool,
        fail_update: bool,
        fail_merge: bool,
    }

    impl CategoryMappingApi for ProposalFlowMapping {
        fn clear_classify_proposal_items(&self, _: &str) -> Result<(), MappingError> {
            Ok(())
        }

        fn get_all_mappings(&self, _: &str) -> Result<Value, MappingError> {
            Ok(json!({"fiction": ["keyword"]}))
        }

        fn get_classify_proposal_status(&self, _: &str, _: i64) -> Result<Value, MappingError> {
            Ok(self.state.lock().unwrap().clone())
        }

        fn set_classify_proposal_status(&self, state: Value, _: &str) -> Result<(), MappingError> {
            *self.state.lock().unwrap() = state;
            Ok(())
        }

        fn merge_classify_proposal_status(&self, fields: Value, _: &str) -> Result<(), MappingError> {
            if self.fail_merge {
                return Err(MappingError::Other("progress merge failed".into()));
            }
            if let (Some(state), Some(fields)) = (self.state.lock().unwrap().as_object_mut(), fields.as_object()) {
                state.extend(fields.clone());
            }
            Ok(())
        }

        fn add_classify_proposal_items(&self, items: &[Value], _: &str, _: &str) -> Result<(), MappingError> {
            self.events.lock().unwrap().push(format!("add:{}", items.len()));
            if self.fail_add { Err(MappingError::Other("item add failed".into())) } else { Ok(()) }
        }

        fn update_classify_proposal_item_payloads(&self, items: &[Value], _: &str) -> Result<(), MappingError> {
            self.events.lock().unwrap().push(format!("update:{}", items.len()));
            if self.fail_update { Err(MappingError::Other("item update failed".into())) } else { Ok(()) }
        }
    }

    struct ProgressProposalManager {
        progress: Value,
        error: Option<String>,
    }

    #[async_trait::async_trait]
    impl BookManagerApi for ProgressProposalManager {
        async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> Result<Value, String> {
            if let Some(callback) = request.on_progress {
                callback(self.progress.clone());
            }
            self.error.clone().map_or_else(|| Ok(json!({"new_count": 2, "items": [{"internal": true}]})), Err)
        }
    }

    async fn run_setup(clear_error: bool, mappings_error: bool) -> ProposalSetupMapping {
        let mapping = Arc::new(ProposalSetupMapping { clear_error, mappings_error, state: Mutex::new(json!({"status":"running"})) });
        run_classify_proposal_job(Arc::new(NoProposalManager), mapping.clone(), "book".into(), "inbox".into(), true, true, 0.0).await;
        Arc::try_unwrap(mapping).ok().unwrap()
    }

    async fn run_flow(progress: Value, error: Option<String>, fail_add: bool, fail_update: bool, fail_merge: bool) -> ProposalFlowMapping {
        let mapping = Arc::new(ProposalFlowMapping { state: Mutex::new(json!({"status":"running"})), events: Mutex::new(Vec::new()), fail_add, fail_update, fail_merge });
        let manager = Arc::new(ProgressProposalManager { progress, error });
        run_classify_proposal_job(manager, mapping.clone(), "book".into(), "inbox".into(), true, true, 0.0).await;
        Arc::try_unwrap(mapping).ok().unwrap()
    }

    #[tokio::test]
    async fn proposal_job_marks_initialization_and_mapping_lookup_failures() {
        let clear_failure = run_setup(true, false).await;
        assert_eq!(clear_failure.state.lock().unwrap()["status"], "failed");
        assert_eq!(clear_failure.state.lock().unwrap()["error"], "분류 제안 초기화에 실패했습니다.");

        let mappings_failure = run_setup(false, true).await;
        assert_eq!(mappings_failure.state.lock().unwrap()["status"], "failed");
        assert_eq!(mappings_failure.state.lock().unwrap()["error"], "분류 제안에 실패했습니다.");
    }

    #[tokio::test]
    async fn proposal_job_flushes_pending_items_and_completes_status() {
        let mapping = run_flow(json!({"processed_count": 1, "new_items": [{"id": 1}], "updated_items": [{"id": 2}]}), None, false, false, false).await;

        assert_eq!(mapping.events.lock().unwrap().as_slice(), ["add:1", "update:1"]);
        assert_eq!(*mapping.state.lock().unwrap(), json!({"status":"ready","processed_count":1,"new_count":2}));
    }

    #[tokio::test]
    async fn proposal_job_marks_callback_flush_failures() {
        let new_items = (0..10).map(|id| json!({"id": id})).collect::<Vec<_>>();
        let updated_items = (10..20).map(|id| json!({"id": id})).collect::<Vec<_>>();
        let mapping = run_flow(json!({"new_items": new_items, "updated_items": updated_items}), None, true, true, false).await;

        assert_eq!(mapping.events.lock().unwrap().as_slice(), ["add:10", "update:10"]);
        assert_eq!(mapping.state.lock().unwrap()["status"], "failed");
        assert_eq!(mapping.state.lock().unwrap()["error"], "분류 제안에 실패했습니다.");
    }
}

#[cfg(test)]
mod tests {
    use super::{ClassifyProposalBody, flush_proposal_item_buffer, run_classify_apply_job, set_proposal_failed};
    use crate::main::{ApplyCategoryChanges, BookManagerApi, CategoryMappingApi, MappingError};
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
        async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<serde_json::Value, String> {
            if let Some(on_progress) = request.on_progress {
                on_progress(json!({"processed_count":1}));
            }
            if let Some(on_item_done) = request.on_item_done {
                on_item_done(json!({"file_path":"a.epub","apply_status":"moved"}));
            }
            if let Some(should_continue) = request.should_continue {
                let _ = should_continue();
            }
            self.error.map_or_else(|| Ok(json!({"applied_count":1,"failed_count":0})), |error| Err(error.to_string()))
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
    fn classify_proposal_body_uses_defaults() {
        let body: ClassifyProposalBody = serde_json::from_value(json!({"category":"inbox"})).unwrap();
        assert!(body.use_bookstore);
        assert!(body.use_content_meta);
        assert_eq!(body.delay, 1.2);
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
}
