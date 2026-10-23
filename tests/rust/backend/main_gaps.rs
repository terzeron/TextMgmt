//! 원본: tests/test_main_gaps.py
//!
//! main.py 라우터의 실패 응답과 입력 검증 분기를 mock 기반으로 확인한다.
//! 원본이 `tests/test_main_mock.py`에서 가져오는 fixture(client, mock_bm, mock_cat, install_classify_status_store)는
//! 테스트 파일끼리 공유할 수 없어서 이 파일 안에 같은 의미로 다시 만들었다.
//!
//! 가정한 API (아직 구현 전이다. Python과 이름·시그니처가 같다고 본다):
//! - `backend::main::{app_with(deps: AppDeps) -> axum::Router, AppDeps, AuthMode, ClientErrorLogModel}`
//!   - `AppDeps::new(auth: AuthMode) -> AppDeps`, `.with_book_manager(Arc<dyn BookManagerApi>)`, `.with_category_mapping(Arc<dyn CategoryMappingApi>)` (소비형 빌더, 설계 3.2)
//!   - `AuthMode::Fixed(serde_json::Value)`: `require_auth`/`require_admin` 의존성 override에 대응한다.
//!   - `ClientErrorLogModel { error_type: String, message: String, url: String, stack: Option<String>, component_stack: Option<String>, user_agent: Option<String>, timestamp: Option<String> }` (`Deserialize`)
//! - `backend::main::run_classify_apply_job(book_manager: &dyn BookManagerApi, category_mapping: &dyn CategoryMappingApi, content_type: &str, items: Vec<Value>, allowed: HashSet<String>, clean_existing: bool, apply_token: &str)`
//!   (async; `create_item_router` 안의 클로저 `_run_classify_apply_job`을 밖으로 낸 것)
//! - `#[async_trait] backend::book_manager::BookManagerApi` (모든 메서드에 기본 구현이 있다)
//!   - `async fn get_categories(&self) -> (Value, Option<String>)`
//!   - `async fn apply_category_changes(&self, items: &[Value], allowed: &HashSet<String>, clean_existing: bool) -> (Value, Option<String>)`
//!   - `fn path_prefix(&self) -> PathBuf`, `fn es_manager(&self) -> Arc<dyn EsManagerApi>`
//!   - `backend::es_manager::EsManagerApi`는 `book_manager`가 아니라 `es_manager` 모듈에 있다.
//! - `backend::es_manager::{EsManagerApi, EsError}` (설계 3.3. 모든 메서드에 기본 구현이 있다)
//!   - `fn search_categories_pdf_stats(&self, categories: &[String]) -> Result<Value, EsError>`
//!   - `fn search_category_pdf_stats(&self, category: &str) -> Result<Value, EsError>`
//!   - 대역은 `common::es_fake::FakeEsManager`를 쓴다.
//! - `backend::category_mapping::CategoryMappingApi` (동기 메서드, 모든 메서드에 기본 구현이 있다)
//!   - `get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Value`
//!   - `set_classify_proposal_status(&self, status: Value, content_type: &str)`
//!   - `merge_classify_proposal_status(&self, fields: Value, content_type: &str)`
//!   - `try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> (bool, Value)`
//!   - `try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> (bool, Value)`
//!   - `clear_classify_proposal_items(&self, content_type: &str)`

mod common;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use backend::book_manager::BookManagerApi;
use backend::category_mapping::CategoryMappingApi;
use backend::es_manager::EsManagerApi;
use backend::main::{AppDeps, AuthMode, ClientErrorLogModel, app_with};
use common::TestClient;
use common::es_fake::FakeEsManager;
use serde_json::{Value, json};

/// 쿼리 값을 퍼센트 인코딩한다. `params={...}`에 대응한다.
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn admin_payload() -> Value {
    let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
    json!({"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": exp})
}

/// 분류 제안 작업 상태를 메모리로 흉내 낸 category_mapping mock. 전이 규칙까지 흉내 낸다.
/// 항목에 `_age_seconds`를 넣으면 '갱신이 그만큼 끊긴 작업'이 된다.
struct MockCat {
    store: Mutex<HashMap<String, Value>>,
    clear_calls: AtomicUsize,
}

impl MockCat {
    /// `install_classify_status_store(mock_cat, initial)`에 대응한다.
    fn with_store(initial: Option<Value>) -> Arc<Self> {
        let mut store = HashMap::from([("book".to_string(), json!({"status": "idle", "content_type": "book"})), ("comic".to_string(), json!({"status": "idle", "content_type": "comic"}))]);
        if let Some(initial) = initial {
            let content_type = initial.get("content_type").cloned().unwrap_or(json!("book"));
            let mut state = initial.clone();
            state["content_type"] = content_type;
            store.insert("book".to_string(), state);
        }
        Arc::new(Self { store: Mutex::new(store), clear_calls: AtomicUsize::new(0) })
    }

    fn effective(&self, content_type: &str, stale_seconds: i64) -> Value {
        let mut current = self.store.lock().unwrap().get(content_type).cloned().unwrap_or(json!({"status": "idle", "content_type": content_type}));
        let obj = current.as_object_mut().unwrap();
        let running = matches!(obj.get("status").and_then(Value::as_str), Some("running" | "applying"));
        if running && obj.remove("_age_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0) > stale_seconds as f64 {
            obj.insert("status".into(), json!("failed"));
            let error = obj.get("error").filter(|e| !e.is_null() && e != &json!("")).cloned().unwrap_or(json!("응답 없이 중단된 것으로 보입니다."));
            obj.insert("error".into(), error);
        }
        obj.remove("_age_seconds");
        current
    }
}

impl CategoryMappingApi for MockCat {
    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Value {
        self.effective(content_type, stale_seconds)
    }

    fn set_classify_proposal_status(&self, mut status: Value, content_type: &str) {
        status["content_type"] = json!(content_type);
        self.store.lock().unwrap().insert(content_type.to_string(), status);
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(content_type.to_string()).or_insert(json!({"status": "idle", "content_type": content_type}));
        for (key, value) in fields.as_object().unwrap() {
            if key != "status" {
                entry[key] = value.clone();
            }
        }
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> (bool, Value) {
        let current = self.effective(content_type, stale_seconds);
        if matches!(current["status"].as_str(), Some("running" | "applying")) {
            return (false, current);
        }
        self.set_classify_proposal_status(status, content_type);
        (true, self.effective(content_type, stale_seconds))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> (bool, Value) {
        let current = self.effective(content_type, stale_seconds);
        if !matches!(current["status"].as_str(), Some("ready" | "done" | "failed")) {
            return (false, current);
        }
        let mut next = current;
        next["status"] = json!("applying");
        next["apply_token"] = json!(apply_token);
        self.set_classify_proposal_status(next, content_type);
        (true, self.effective(content_type, stale_seconds))
    }

    fn clear_classify_proposal_items(&self, _content_type: &str) {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// `AsyncMock()` book manager.
struct MockBm {
    /// `m.es_manager = MagicMock()`. 결과는 `set_*_pdf_stats`/`fail`로 테스트가 정한다.
    es: Arc<FakeEsManager>,
    categories: Mutex<(Value, Option<String>)>,
    apply_result: Mutex<(Value, Option<String>)>,
    path_prefix: Mutex<PathBuf>,
}

#[async_trait::async_trait]
impl BookManagerApi for MockBm {
    async fn get_categories(&self) -> (Value, Option<String>) {
        self.categories.lock().unwrap().clone()
    }

    async fn apply_category_changes(&self, _items: &[Value], _allowed: &HashSet<String>, _clean_existing: bool) -> (Value, Option<String>) {
        self.apply_result.lock().unwrap().clone()
    }

    fn path_prefix(&self) -> PathBuf {
        self.path_prefix.lock().unwrap().clone()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        self.es.clone()
    }
}

fn mock_bm() -> Arc<MockBm> {
    let es = FakeEsManager::new();
    Arc::new(MockBm { es, categories: Mutex::new((json!({}), None)), apply_result: Mutex::new((json!({}), None)), path_prefix: Mutex::new(PathBuf::new()) })
}

fn client(bm: &Arc<MockBm>, cat: &Arc<MockCat>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm.clone()).with_category_mapping(cat.clone())))
}

#[ignore = "closure introspection: create_item_router 안의 _run_classify_apply_job 클로저를 __closure__로 꺼낸다; DI 설계 후 활성화"]
#[tokio::test]
async fn test_apply_job_marks_state_failed_when_book_manager_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bm = mock_bm();
    let cat = MockCat::with_store(Some(json!({"status": "applying", "source_category": "0_inbox", "apply_token": "tok"})));
    *bm.apply_result.lock().unwrap() = (json!({}), Some("적용 오류".into()));
    *bm.path_prefix.lock().unwrap() = tmp.path().to_path_buf();

    // content_type="book"인 라우터 안의 작업 함수. 가정한 seam이다.
    backend::main::run_classify_apply_job(
        bm.as_ref(),
        cat.as_ref(),
        "book",
        vec![json!({"file_path": "0_inbox/a.epub", "target_category": "3_SF"})],
        HashSet::from(["0_inbox/a.epub".to_string()]),
        false,
        "tok",
    )
    .await;

    let state = cat.store.lock().unwrap().get("book").cloned().unwrap();
    assert_eq!(state["status"], "failed");
    assert_eq!(state["error"], "적용 오류");
}

#[tokio::test]
async fn test_delete_classify_proposal_is_refused_while_job_is_running() {
    for running_status in ["running", "applying"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let bm = mock_bm();
        let cat = MockCat::with_store(Some(json!({"status": running_status})));
        *bm.path_prefix.lock().unwrap() = tmp.path().to_path_buf();

        let response = client(&bm, &cat).delete("/categories/classify-proposal").await;

        assert_eq!(response.json(), json!({"status": "failure", "error": "작업이 진행 중입니다."}), "running_status={running_status}");
        assert_eq!(cat.clear_calls.load(Ordering::SeqCst), 0, "running_status={running_status}: clear_classify_proposal_items가 불리면 안 된다");
    }
}

#[tokio::test]
async fn test_similar_names_requires_a_name() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let response = client(&bm, &cat).get(&format!("/similar-names?name={}", enc("   "))).await;
    assert_eq!(response.json(), json!({"status": "failure", "error": "name is required"}));
}

#[tokio::test]
async fn test_similar_names_reports_category_lookup_error() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (json!({}), Some("카테고리 조회 실패".into()));

    let response = client(&bm, &cat).get(&format!("/similar-names?name={}", enc("소설"))).await;

    assert_eq!(response.json(), json!({"status": "failure", "error": "카테고리 조회 실패"}));
}

#[tokio::test]
async fn test_similar_names_hides_internal_errors() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (json!({"소설": 1}), None);
    bm.es.fail("search_categories_pdf_stats", "ES 내부 오류 상세");

    let response = client(&bm, &cat).get(&format!("/similar-names?name={}", enc("소설"))).await;

    assert_eq!(response.json(), json!({"status": "failure", "error": "유사 이름 검색에 실패했습니다."}));
}

#[tokio::test]
async fn test_category_pdf_stats_rejects_root_and_empty_category() {
    for category in ["_root", ""] {
        let (bm, cat) = (mock_bm(), MockCat::with_store(None));
        let response = client(&bm, &cat).get(&format!("/category-pdf-stats?category={}", enc(category))).await;
        assert_eq!(response.status, 400, "category={category:?}");
    }
}

#[tokio::test]
async fn test_category_pdf_stats_returns_aggregated_stats() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let stats = json!({"file_count": 2, "page_count": 10, "total_file_size": 99});
    bm.es.set_category_pdf_stats(stats.clone());

    let response = client(&bm, &cat).get(&format!("/category-pdf-stats?category={}", enc("2_science"))).await;

    assert_eq!(response.json(), json!({"status": "success", "result": stats}));
}

#[tokio::test]
async fn test_category_pdf_stats_hides_internal_errors() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    bm.es.fail("search_category_pdf_stats", "ES 내부 오류 상세");

    let response = client(&bm, &cat).get(&format!("/category-pdf-stats?category={}", enc("2_science"))).await;

    assert_eq!(response.json(), json!({"status": "failure", "error": "PDF 통계를 불러오지 못했습니다."}));
}

#[test]
fn test_client_error_model_accepts_explicit_null_optional_fields() {
    let body: ClientErrorLogModel =
        serde_json::from_value(json!({"error_type": "CUSTOM_ERROR", "message": "m", "url": "u", "stack": null, "component_stack": null, "user_agent": null, "timestamp": null}))
            .expect("명시적 null을 받아야 한다");
    assert!(body.stack.is_none());
    assert!(body.timestamp.is_none());
}
