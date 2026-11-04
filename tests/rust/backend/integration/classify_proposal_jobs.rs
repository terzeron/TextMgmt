#[path = "../common/mod.rs"]
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use backend::main::{
    AppDeps, AuthMode, BookstoreFactory, CategoryMappingApi, ClientErrorLogModel, TaskHandle, TaskRunner, app_with,
};
use common::TestClient;
use serde_json::{Value, json};
use crate::common::shared_fixtures::{enc};
use crate::common::shared_fixtures::{MockBm, MockCat, mock_bm};

fn admin_payload() -> Value {
    let exp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600;
    json!({"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": exp})
}

fn client(bm: &Arc<MockBm>, cat: &Arc<MockCat>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm.clone()).with_category_mapping(cat.clone())))
}

#[derive(Default)]
struct QueuedTasks(Mutex<Vec<Box<dyn FnOnce() + Send>>>);
struct EmptyTaskHandle;
impl TaskHandle for EmptyTaskHandle {}
impl TaskRunner for QueuedTasks {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        self.0.lock().unwrap().push(job);
        Box::new(EmptyTaskHandle)
    }
}

impl QueuedTasks {
    fn run_all(&self) {
        for job in std::mem::take(&mut *self.0.lock().unwrap()) {
            std::thread::spawn(job).join().unwrap();
        }
    }
}

fn client_with_runner(bm: &Arc<MockBm>, cat: &Arc<MockCat>, runner: Arc<dyn TaskRunner>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm.clone()).with_category_mapping(cat.clone()).with_task_runner(runner)))
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
async fn test_delete_classify_proposal_clears_items_and_resets_idle_status() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"done","total_count":4}))));
    *cat.proposal_items.lock().unwrap() = vec![json!({"file_path":"0_inbox/a.epub"})];

    let response = client(&bm, &cat).delete("/categories/classify-proposal").await;

    assert_eq!(response.json(), json!({"status":"success","result":{"cleared":true}}));
    assert_eq!(cat.clear_calls.load(Ordering::SeqCst), 1);
    assert_eq!(cat.effective("book", 300)["status"], "idle");
    assert!(cat.proposal_items.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_delete_classify_proposal_returns_unavailable_without_mapping_store() {
    let app = app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_category_mapping_factory(|| Err("injected mapping initialization failure".into())));
    let response = TestClient::with_app(app).delete("/categories/classify-proposal").await;
    assert_eq!(response.status, 503, "{}", response.text());
}

#[tokio::test]
async fn classify_proposal_start_atomically_claims_and_queues_work() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let runner = Arc::new(QueuedTasks::default());
    let response = client_with_runner(&bm, &cat, runner.clone()).post_json("/categories/classify-proposal", &json!({"category":"0_inbox","delay":0})).await;
    assert_eq!(response.json()["result"]["started"], true);
    assert_eq!(response.json()["result"]["source_category"], "0_inbox");
    assert_eq!(cat.effective("book", 300)["status"], "running");
    assert_eq!(runner.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn classify_proposal_runtime_creation_failure_marks_job_failed() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let runner = Arc::new(QueuedTasks::default());
    let runtime_builds = Arc::new(AtomicUsize::new(0));
    let observed_builds = runtime_builds.clone();
    let app = app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm).with_category_mapping(cat.clone()).with_task_runner(runner.clone()).with_task_runtime_factory(move || {
        observed_builds.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::other("injected runtime failure"))
    }));

    let response = TestClient::with_app(app).post_json("/categories/classify-proposal", &json!({"category":"0_inbox"})).await;
    assert_eq!(response.json()["result"]["started"], true);
    runner.run_all();

    let status = cat.effective("book", 300);
    assert_eq!(runtime_builds.load(Ordering::SeqCst), 1);
    assert_eq!(status["status"], "failed");
    assert_eq!(status["error"], "분류 제안에 실패했습니다.");
}

#[tokio::test]
async fn classify_proposal_flushes_new_items_in_ten_item_batches_and_keeps_the_tail() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.path_prefix.lock().unwrap() = tmp.path().to_path_buf();
    let items: Vec<Value> = (0..25).map(|index| json!({"file_path":format!("0_inbox/{index}.epub"),"apply_status":"pending"})).collect();
    *bm.proposal_progress.lock().unwrap() = items.iter().enumerate().map(|(index, item)| json!({"total_count":25,"processed_count":index+1,"new_items":[item]})).collect();
    *bm.proposal_result.lock().unwrap() = Some(json!({"source_category":"0_inbox","total_count":25,"processed_count":25,"failures":[]}));
    let runner = Arc::new(QueuedTasks::default());

    let response = client_with_runner(&bm, &cat, runner.clone()).post_json("/categories/classify-proposal", &json!({"category":"0_inbox","delay":0})).await;
    assert_eq!(response.status, 200, "{}", response.text());
    let job = runner.0.lock().unwrap().pop().expect("proposal task queued");
    tokio::task::spawn_blocking(job).await.unwrap();

    assert_eq!(*cat.proposal_add_batch_sizes.lock().unwrap(), [10, 10, 5]);
    assert_eq!(*cat.proposal_items.lock().unwrap(), items);
    assert_eq!(cat.effective("book", 300)["status"], "ready");
}

#[tokio::test]
async fn classify_proposal_start_refuses_an_existing_job_without_queueing_another() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"running"}))));
    let runner = Arc::new(QueuedTasks::default());
    let response = client_with_runner(&bm, &cat, runner.clone()).post_json("/categories/classify-proposal", &json!({"category":"0_inbox"})).await;
    assert_eq!(response.json()["result"]["already_running"], true);
    assert!(runner.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn queued_proposal_job_persists_placeholders_updates_progress_and_finishes_ready() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let request_thread = std::thread::current().id();
    let runner = Arc::new(QueuedTasks::default());
    let response = client_with_runner(&bm, &cat, runner.clone()).post_json("/categories/classify-proposal", &json!({"category":"0_inbox","delay":0})).await;
    assert_eq!(response.json()["result"]["started"], true);
    let job = runner.0.lock().unwrap().pop().unwrap();
    tokio::task::spawn_blocking(job).await.unwrap();
    assert_ne!(*cat.proposal_write_thread.lock().unwrap(), Some(request_thread), "DB proposal writes must run outside the request executor");
    assert_eq!(cat.effective("book", 300)["status"], "ready");
    assert_eq!(cat.effective("book", 300)["processed_count"], 1);
    let items = cat.proposal_items.lock().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["target_category"], "3_SF");
}

#[tokio::test]
async fn classify_proposal_apply_claims_and_queues_selected_items() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"ready"}))));
    *cat.proposal_items.lock().unwrap() = vec![json!({"file_path":"0_inbox/a.epub","apply_status":"pending"}), json!({"file_path":"0_inbox/moved.epub","apply_status":"moved"})];
    *bm.apply_result.lock().unwrap() = (json!({"applied_count":1,"failed_count":0}), None);
    let runner = Arc::new(QueuedTasks::default());
    let response = client_with_runner(&bm, &cat, runner.clone()).post_json("/categories/classify-proposal/apply", &json!({"items":[{"file_path":"0_inbox/a.epub","target_category":"3_SF"}]})).await;
    assert_eq!(response.json()["result"], json!({"started":true,"total_count":1}));
    assert_eq!(cat.effective("book", 300)["status"], "applying");
    let job = runner.0.lock().unwrap().pop().unwrap();
    tokio::task::spawn_blocking(job).await.unwrap();
    assert_eq!(cat.effective("book", 300)["status"], "done");
    assert_eq!(cat.effective("book", 300)["applied_count"], 1);
}

#[tokio::test]
async fn classify_proposal_apply_runtime_failure_marks_matching_job_failed() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"ready"}))));
    let runner = Arc::new(QueuedTasks::default());
    let runtime_builds = Arc::new(AtomicUsize::new(0));
    let observed_builds = runtime_builds.clone();
    let app = app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm).with_category_mapping(cat.clone()).with_task_runner(runner.clone()).with_task_runtime_factory(move || {
        observed_builds.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::other("injected runtime failure"))
    }));

    let response = TestClient::with_app(app).post_json("/categories/classify-proposal/apply", &json!({"items":[{"file_path":"0_inbox/a.epub","target_category":"3_SF"}]})).await;
    assert_eq!(response.json()["result"]["started"], true);
    runner.run_all();

    let status = cat.effective("book", 300);
    assert_eq!(runtime_builds.load(Ordering::SeqCst), 1);
    assert_eq!(status["status"], "failed");
    assert_eq!(status["error"], "분류 적용에 실패했습니다.");
}

#[tokio::test]
async fn classify_proposal_apply_refuses_running_or_empty_proposals() {
    for status in ["running", "applying", "idle"] {
        let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":status}))));
        let runner = Arc::new(QueuedTasks::default());
        let response = client_with_runner(&bm, &cat, runner.clone()).post_json("/categories/classify-proposal/apply", &json!({"items":[]})).await;
        assert_eq!(response.json(), json!({"status":"failure","error":"적용할 제안이 없습니다."}));
        assert!(runner.0.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn classify_proposal_get_returns_saved_status_and_items() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"ready","total_count":1}))));
    *cat.proposal_items.lock().unwrap() = vec![json!({"file_path":"0_inbox/a.epub","apply_status":"pending"})];
    let response = client(&bm, &cat).get("/categories/classify-proposal").await;
    assert_eq!(
        response.json(),
        json!({"status":"success","result":{"status":"ready","total_count":1,"content_type":"book","items":[{"file_path":"0_inbox/a.epub","apply_status":"pending","book_id":null}]}})
    );
}

#[tokio::test]
async fn classify_proposal_get_fills_missing_book_id_from_existing_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("0_inbox/a.epub");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"epub").unwrap();
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"ready"}))));
    *bm.path_prefix.lock().unwrap() = tmp.path().to_path_buf();
    *cat.proposal_items.lock().unwrap() = vec![json!({"file_path":"0_inbox/a.epub"})];

    let response = client(&bm, &cat).get("/categories/classify-proposal").await;

    let item = &response.json()["result"]["items"][0];
    assert!(item["book_id"].as_i64().is_some_and(|id| id > 0));
}

#[tokio::test]
async fn classify_proposal_item_delete_refuses_running_jobs_and_deletes_one_path() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"running"}))));
    *cat.proposal_items.lock().unwrap() = vec![json!({"file_path":"0_inbox/a.epub"})];
    let refused = client(&bm, &cat).delete(&format!("/categories/classify-proposal/item?file_path={}", enc("0_inbox/a.epub"))).await;
    assert_eq!(refused.json(), json!({"status":"failure","error":"작업이 진행 중입니다."}));
    cat.set_classify_proposal_status(json!({"status":"ready"}), "book").unwrap();
    let deleted = client(&bm, &cat).delete(&format!("/categories/classify-proposal/item?file_path={}", enc("0_inbox/a.epub"))).await;
    assert_eq!(deleted.json(), json!({"status":"success","result":{"deleted_count":1}}));
}

#[tokio::test]
async fn classify_proposal_applied_delete_removes_only_moved_items() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(Some(json!({"status":"done"}))));
    *cat.proposal_items.lock().unwrap() = vec![json!({"file_path":"a","apply_status":"moved"}), json!({"file_path":"b","apply_status":"failed"})];
    let response = client(&bm, &cat).delete("/categories/classify-proposal/applied").await;
    assert_eq!(response.json(), json!({"status":"success","result":{"deleted_count":1}}));
    assert_eq!(cat.proposal_items.lock().unwrap().len(), 1);
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
async fn similar_names_returns_failure_when_blocking_es_task_panics() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (json!({"Novel": 1}), None);
    bm.es.fail_with("search_categories_pdf_stats", || panic!("injected ES panic"));

    let response = client(&bm, &cat).get("/similar-names?name=Novel").await;

    assert_eq!(response.json(), json!({"status":"failure","error":"유사 이름 검색에 실패했습니다."}));
}

#[tokio::test]
async fn test_similar_names_scores_directories_against_reference_stats() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (json!({"_root": 1, "Source": 1, "Novel": 1, "Novelty": 1}), None);
    bm.es.set_categories_pdf_stats(json!({
        "Source": {"file_count": 10, "page_count": 1000, "total_file_size": 100000000},
        "Novel": {"file_count": 10, "page_count": 1000, "total_file_size": 100000000},
        "Novelty": {"file_count": 100, "page_count": 10000, "total_file_size": 1000000000}
    }));

    let response = client(&bm, &cat).get("/similar-names?name=Novel&exclude_category=Source").await;
    let results = response.json()["result"].as_array().unwrap().clone();
    assert_eq!(results.iter().map(|item| item["kind"].as_str().unwrap()).collect::<Vec<_>>(), ["directory", "directory"]);
    assert_eq!(results[0]["category"], "Novel");
    assert_eq!(results[0]["score"], 100.0);
    assert!((results[1]["score"].as_f64().unwrap() - (70.0 * 5.0 / 6.0 + 3.0)).abs() < 1e-9);
}

#[tokio::test]
async fn test_similar_names_uses_python_full_casefold_for_matching_and_order() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (json!(["Straße", "zza", "ßa"]), None);

    let response = client(&bm, &cat).get("/similar-names?name=STRASSE").await;
    let results = response.json()["result"].as_array().unwrap().clone();
    assert_eq!(results[0]["category"], "Straße");
    assert_eq!(results[0]["score"], 70.0);

    let response = client(&bm, &cat).get("/similar-names?name=a").await;
    let results = response.json()["result"].as_array().unwrap().clone();
    assert_eq!(results.iter().map(|item| item["category"].as_str().unwrap()).collect::<Vec<_>>(), ["ßa", "zza"]);
}

#[tokio::test]
async fn similar_names_accepts_mapping_objects_and_excludes_nested_categories() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (json!({"_root": {}, "Novel": {}, "Novel/Child": {}, "Archive": {}}), None);

    let response = client(&bm, &cat).get("/similar-names?name=Novel&exclude_category=Novel").await;

    assert_eq!(response.json(), json!({"status":"success","result":[]}));
}

#[tokio::test]
async fn similar_names_treats_an_unexpected_category_shape_as_empty() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (Value::Null, None);

    let response = client(&bm, &cat).get("/similar-names?name=Novel").await;

    assert_eq!(response.json(), json!({"status":"success","result":[]}));
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
async fn test_category_mismatch_file_actions_are_available_to_admin() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let client = client(&bm, &cat);

    let indexed = client.post_json("/category-mismatches/index-file", &json!({"file_path": "a.txt"})).await;
    assert_eq!(indexed.json(), json!({"status": "success", "result": {"book_id": 17}}));

    let deleted = client.post_json("/category-mismatches/delete-file", &json!({"file_path": "a.txt"})).await;
    assert_eq!(deleted.json(), json!({"status": "success", "result": "Ok"}));

    let es_deleted = client.delete("/category-mismatches/es-doc/17").await;
    assert_eq!(es_deleted.json(), json!({"status": "success"}));
    assert_eq!(bm.es.deleted(), vec![17]);

    let missing_path = client.post_json("/category-mismatches/index-file", &json!({})).await;
    assert_eq!(missing_path.status, 400);

    bm.es.set_delete_result(false);
    let delete_failed = client.delete("/category-mismatches/es-doc/17").await;
    assert_eq!(delete_failed.json(), json!({"status":"failure","error":"ES 문서 삭제 실패: 17"}));
}

#[tokio::test]
async fn test_category_mismatch_detail_and_reload_routes() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let client = client(&bm, &cat);

    let details = client.get("/category-mismatches/A/B").await;
    assert_eq!(details.json(), json!({"status":"success","result":{"fs_only":[{"file_path":"A/a.txt"}]}}));
    assert_eq!(bm.mismatch_categories.lock().unwrap().as_slice(), &["A/B"]);

    let reloaded = client.post_json("/category-mismatches/reload", &json!({"category":"A"})).await;
    assert_eq!(reloaded.json(), json!({"status":"success","result":{"category":"A","content_type":"book","processed":3}}));

    let status = client.get("/category-mismatches/reload-status").await;
    assert_eq!(status.json(), json!({"status":"success","result":{"status":"idle"}}));
}

#[tokio::test]
async fn test_category_mismatch_reload_starts_a_background_job() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let runner = Arc::new(QueuedTasks::default());
    let client = client_with_runner(&bm, &cat, runner.clone());

    let started = client.post_json("/category-mismatches/reload-mismatches", &json!({"category":"A"})).await;

    assert_eq!(started.json(), json!({"status":"success","result":{"started":true,"content_type":"book","category":"A","reload_source":"mismatch"}}));
    assert_eq!(runner.0.lock().unwrap().len(), 1);

    let attached = client.post_json("/category-mismatches/reload-mismatches", &json!({"category":"A"})).await;
    assert_eq!(attached.json()["result"]["already_running"], true);

    runner.run_all();
    let status = client.get("/category-mismatches/reload-status?category=A").await;
    assert_eq!(status.json()["result"]["status"], "done");
    assert_eq!(status.json()["result"]["indexed_count"], 1);

    let bulk = client.request(axum::http::Method::POST, "/category-mismatches/reload-all", None, Vec::new()).await;
    assert_eq!(bulk.json(), json!({"status":"success","result":{"started":true,"content_type":"book","reload_source":"bulk"}}));
    runner.run_all();
}

#[tokio::test]
async fn test_category_pdf_stats_hides_internal_errors() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    bm.es.fail("search_category_pdf_stats", "ES 내부 오류 상세");

    let response = client(&bm, &cat).get(&format!("/category-pdf-stats?category={}", enc("2_science"))).await;

    assert_eq!(response.json(), json!({"status": "failure", "error": "PDF 통계를 불러오지 못했습니다."}));
}

#[tokio::test]
async fn bulk_rename_treats_unexpected_categories_shape_as_empty() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    *bm.categories.lock().unwrap() = (Value::Null, None);

    let response = client(&bm, &cat).post_json("/categories/bulk-rename-files", &json!({"category":"0_inbox","pattern":"a","replacement":"b"})).await;

    assert_eq!(response.json(), json!({"status":"success","result":{"changed_count":0,"failed_count":0,"failures":[]}}));
}

#[tokio::test]
async fn bulk_rename_returns_unavailable_when_runtime_book_manager_cannot_initialize() {
    let deps = AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager_factory(|| Err("injected BookManager initialization failure".into())).with_category_mapping(MockCat::with_store(None));
    let app = app_with(deps);
    let token = backend::auth_service::create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);

    let response = TestClient::with_app(app).cookie("tm_access_token", &token).post_json("/categories/bulk-rename-files", &json!({"category":"0_inbox","pattern":"a","replacement":"b"})).await;

    assert_eq!(response.status, 503, "{}", response.text());
    assert_eq!(response.json(), json!({"detail":"서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요."}));
}

#[test]
fn test_client_error_model_accepts_explicit_null_optional_fields() {
    let body: ClientErrorLogModel =
        serde_json::from_value(json!({"error_type": "CUSTOM_ERROR", "message": "m", "url": "u", "stack": null, "component_stack": null, "user_agent": null, "timestamp": null}))
            .expect("명시적 null을 받아야 한다");
    assert!(body.stack.is_none());
    assert!(body.timestamp.is_none());
}

#[tokio::test]
async fn wake_only_reads_storage_for_the_configured_frontend_host() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let denied = client(&bm, &cat).header("host", "not-frontend.example").get("/wake").await;
    assert_eq!(denied.status, 404);

    let accepted = client(&bm, &cat).header("host", "localhost").get("/wake").await;
    assert_eq!(accepted.json(), json!({"status":"success"}));

    let failing_fs = common::faulty_fs::FaultyFs::new();
    failing_fs.set_read_dir(|_| Some(std::io::Error::other("storage unavailable")));
    let failing_app = app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_fs(failing_fs));
    let failed = TestClient::with_app(failing_app).header("host", "localhost").get("/wake").await;
    assert_eq!(failed.status, 503);
    assert_eq!(failed.json(), json!({"status":"failure"}));
}

struct MockOnlineBookstore;

impl bookstore::store_clients::BookstoreSearch for MockOnlineBookstore {
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<bookstore::store_clients::SearchRow>, String, String) {
        if isbn.is_empty() && title.is_empty() && author.is_empty() {
            return (Vec::new(), String::new(), "unknown".into());
        }
        if title == "none" {
            return (Vec::new(), "none".into(), "title".into());
        }
        assert_eq!((isbn, title, author), ("9781", "Book", "Author"));
        let rows = (0..6)
            .map(|index| (format!("Title {index}"), "Author".into(), "Category".into(), format!("https://book/{index}"), String::new(), if index == 0 { "9781".into() } else { String::new() }))
            .collect();
        (rows, "Book Author".into(), "title_author".into())
    }

    fn api_search_url(&self, keyword: &str) -> String {
        format!("https://example.test/search?q={keyword}")
    }
}

#[tokio::test]
async fn bookstore_search_validates_inputs_and_limits_results() {
    let (bm, cat) = (mock_bm(), MockCat::with_store(None));
    let factory: BookstoreFactory = Arc::new(|name| (name == "yes24").then(|| Box::new(MockOnlineBookstore) as Box<dyn bookstore::store_clients::BookstoreSearch>));
    let app = app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm).with_category_mapping(cat).with_bookstore_factory(factory));
    let client = TestClient::with_app(app);

    assert_eq!(client.get("/search/bookstore/unknown?title=t").await.status, 404);
    let missing = client.get("/search/bookstore/yes24").await;
    assert_eq!(missing.status, 400);
    assert_eq!(missing.json()["detail"], "제목 또는 저자를 입력해주세요");
    assert_eq!(client.get("/search/bookstore/yes24?title=%20%20").await.json()["status"], "not_found");

    let result = client.get("/search/bookstore/yes24?isbn=%209781%20&title=%20Book%20&author=%20Author%20").await;
    assert_eq!(result.json()["status"], "success");
    assert_eq!(result.json()["search_url"], "https://example.test/search?q=Book Author");
    assert_eq!(result.json()["result"].as_array().unwrap().len(), 4);
    assert_eq!(result.json()["result"][0]["isbn"], "9781");

    let not_found = client.get("/search/bookstore/yes24?title=none").await;
    assert_eq!(not_found.json()["status"], "not_found");
    assert_eq!(not_found.json()["result"], json!([]));
}

#[test]
fn bookstore_search_factory_covers_every_supported_store() {
    for store in ["yes24", "aladin", "kyobo", "ridi", "naver", "naverseries", "munpia", "joara", "naverwebtoon", "kakaowebtoon", "bomtoon", "lezhin", "toptoon"] {
        assert!(bookstore::store_clients::create_bookstore_search(store).is_some(), "store={store}");
    }
    assert!(bookstore::store_clients::create_bookstore_search("unknown").is_none());
}
