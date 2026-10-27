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
    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> (Value, Option<String>) {
        if let Some(callback) = request.on_progress {
            callback(self.progress.clone());
        }
        (json!({"new_count": 2, "items": [{"internal": true}]}), self.error.clone())
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
