//! `backend::main` 단위 테스트.
#![allow(dead_code)]

mod common;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering, Mutex};
use backend::book_manager::{BookContent, BookManagerApi, BookResponse, CoverPage};
use backend::es_manager::{EsError, EsManagerApi};
use backend::main::{ApplyCategoryChanges, CategoryMappingApi, MappingError, ProposeCategoryChanges, ReloadProgress, route_table};
use common::es_fake::FakeEsManager;
use serde_json::{json, Value};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use backend::book_metadata::{Book, BookLookupError, BookPage};
use db::{DbValue, Row};
use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, StoreError};
use crate::common::shared_fixtures::{arr, books, count, cover_page, flag, is_truthy, opt_book, opt_json, opt_string, page, strings, to_response, value_result};

/// 분류 제안 작업 상태를 메모리로 흉내 낸 category_mapping mock. 전이 규칙까지 흉내 낸다.
/// 항목에 `_age_seconds`를 넣으면 '갱신이 그만큼 끊긴 작업'이 된다.
struct MockCat {
    store: Mutex<HashMap<String, Value>>,
    clear_calls: AtomicUsize,
    proposal_items: Mutex<Vec<Value>>,
    proposal_add_batch_sizes: Mutex<Vec<usize>>,
    proposal_write_thread: Mutex<Option<std::thread::ThreadId>>,
    reload_locks: Mutex<HashMap<String, Value>>,
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
        Arc::new(Self {
            store: Mutex::new(store),
            clear_calls: AtomicUsize::new(0),
            proposal_items: Mutex::new(Vec::new()),
            proposal_add_batch_sizes: Mutex::new(Vec::new()),
            proposal_write_thread: Mutex::new(None),
            reload_locks: Mutex::new(HashMap::new()),
        })
    }

    fn effective(&self, content_type: &str, stale_seconds: i64) -> Value {
        let mut current = self.store.lock().unwrap().get(content_type).cloned().unwrap_or(json!({"status": "idle", "content_type": content_type}));
        let obj = current.as_object_mut().unwrap();
        let running = matches!(obj.get("status").and_then(Value::as_str), Some("running" | "applying"));
        if running && obj.remove("_age_seconds").and_then(|v| v.as_f64()).unwrap_or(0.0) > stale_seconds as f64 {
            obj.insert("status".into(), json!("failed"));
            let error = obj.get("error").filter(|e| !e.is_null() && *e != &json!("")).cloned().unwrap_or(json!("응답 없이 중단된 것으로 보입니다."));
            obj.insert("error".into(), error);
        }
        obj.remove("_age_seconds");
        current
    }
}

impl CategoryMappingApi for MockCat {
    fn acquire_reload_lock(&self, content_type: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        let lock_key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let blocker = if category.is_some() {
            locks.get("_bulk").filter(|s| s["status"] == "running").or_else(|| locks.get(lock_key).filter(|s| s["status"] == "running"))
        } else {
            locks.values().find(|s| s["status"] == "running")
        };
        if let Some(status) = blocker {
            return Ok((false, Some("already running".into()), Some(status.clone())));
        }
        let status = json!({"status":"running","category":category,"reload_source":reload_source.unwrap_or("bulk"),"content_type":content_type,"indexed_count":0,"deleted_count":0,"failed_count":0,"before_count":0,"after_count":0,"error":null});
        locks.insert(lock_key.to_string(), status);
        Ok((true, None, None))
    }

    fn heartbeat_reload_lock(&self, _content_type: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let status = locks.get_mut(key).ok_or_else(|| "missing reload lock".to_string())?;
        for (field, value) in counts.as_object().ok_or_else(|| "counts must be an object".to_string())? {
            status[field] = value.clone();
        }
        Ok(())
    }

    fn complete_reload_lock(&self, _content_type: &str, state: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let key = category.unwrap_or("_bulk");
        let mut locks = self.reload_locks.lock().unwrap();
        let status = locks.get_mut(key).ok_or_else(|| "missing reload lock".to_string())?;
        status["status"] = json!(state);
        status["error"] = json!(error);
        for (field, value) in counts.as_object().ok_or_else(|| "counts must be an object".to_string())? {
            status[field] = value.clone();
        }
        Ok(())
    }

    fn get_all_mappings(&self, _content_type: &str) -> Result<Value, backend::main::MappingError> {
        Ok(json!({}))
    }

    fn get_reload_status(&self, _content_type: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        Ok(self.reload_locks.lock().unwrap().get(category.unwrap_or("_bulk")).cloned())
    }

    fn add_classify_proposal_items(&self, items: &[Value], _source_category: &str, _content_type: &str) -> Result<(), MappingError> {
        *self.proposal_write_thread.lock().unwrap() = Some(std::thread::current().id());
        self.proposal_add_batch_sizes.lock().unwrap().push(items.len());
        self.proposal_items.lock().unwrap().extend_from_slice(items);
        Ok(())
    }

    fn update_classify_proposal_item_payloads(&self, items: &[Value], _content_type: &str) -> Result<(), MappingError> {
        let mut current = self.proposal_items.lock().unwrap();
        for item in items {
            let path = item["file_path"].as_str();
            if let Some(existing) = current.iter_mut().find(|old| old["file_path"].as_str() == path) {
                *existing = item.clone();
            }
        }
        Ok(())
    }

    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, _content_type: &str) -> Result<(), MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let item = items.iter_mut().find(|item| item["file_path"] == file_path).ok_or_else(|| MappingError::Other("proposal item not found".into()))?;
        item["apply_status"] = json!(apply_status);
        item["apply_error"] = apply_error.map_or(Value::Null, |error| json!(error));
        Ok(())
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        Ok(self.effective(content_type, stale_seconds))
    }

    fn set_classify_proposal_status(&self, mut status: Value, content_type: &str) -> Result<(), MappingError> {
        status["content_type"] = json!(content_type);
        self.store.lock().unwrap().insert(content_type.to_string(), status);
        Ok(())
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        let mut store = self.store.lock().unwrap();
        let entry = store.entry(content_type.to_string()).or_insert(json!({"status": "idle", "content_type": content_type}));
        for (key, value) in fields.as_object().unwrap() {
            if key != "status" {
                entry[key] = value.clone();
            }
        }
        Ok(())
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let current = self.effective(content_type, stale_seconds);
        if matches!(current["status"].as_str(), Some("running" | "applying")) {
            return Ok((false, current));
        }
        self.set_classify_proposal_status(status, content_type).unwrap();
        Ok((true, self.effective(content_type, stale_seconds)))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let current = self.effective(content_type, stale_seconds);
        if !matches!(current["status"].as_str(), Some("ready" | "done" | "failed")) {
            return Ok((false, current));
        }
        let mut next = current;
        next["status"] = json!("applying");
        next["apply_token"] = json!(apply_token);
        self.set_classify_proposal_status(next, content_type).unwrap();
        Ok((true, self.effective(content_type, stale_seconds)))
    }

    fn clear_classify_proposal_items(&self, _content_type: &str) -> Result<(), MappingError> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        self.proposal_items.lock().unwrap().clear();
        Ok(())
    }

    fn get_classify_proposal_items(&self, _content_type: &str) -> Result<Vec<Value>, MappingError> {
        Ok(self.proposal_items.lock().unwrap().clone())
    }

    fn delete_classify_proposal_item(&self, file_path: &str, _content_type: &str) -> Result<u64, MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let before = items.len();
        items.retain(|item| item["file_path"] != file_path);
        Ok((before - items.len()) as u64)
    }

    fn delete_applied_classify_proposal_items(&self, _content_type: &str) -> Result<u64, MappingError> {
        let mut items = self.proposal_items.lock().unwrap();
        let before = items.len();
        items.retain(|item| item["apply_status"] != "moved");
        Ok((before - items.len()) as u64)
    }
}

/// `AsyncMock()` book manager.
struct MockBm {
    /// `m.es_manager = MagicMock()`. 결과는 `set_*_pdf_stats`/`fail`로 테스트가 정한다.
    es: Arc<FakeEsManager>,
    categories: Mutex<(Value, Option<String>)>,
    proposal_progress: Mutex<Vec<Value>>,
    proposal_result: Mutex<Option<Value>>,
    apply_result: Mutex<(Value, Option<String>)>,
    path_prefix: Mutex<PathBuf>,
    mismatch_details: Mutex<Value>,
    mismatch_categories: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl BookManagerApi for MockBm {
    fn get_category_mismatch_details(&self, category: &str) -> Result<Value, String> {
        self.mismatch_categories.lock().unwrap().push(category.to_string());
        Ok(self.mismatch_details.lock().unwrap().clone())
    }

    async fn reload_category(&self, category: &str, content_type: &str) -> Result<Value, String> {
        Ok(json!({"category":category,"content_type":content_type,"processed":3}))
    }

    async fn reload_category_mismatch_files(&self, category: &str, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<Value, String> {
        if let Some(report) = on_progress {
            report(json!({"before_count":2,"indexed_count":1}));
        }
        Ok(json!({"content_type":content_type,"category":category,"indexed_count":1,"deleted_count":0,"failed_count":0,"before_count":2,"after_count":1}))
    }

    async fn reload_category_mismatches(&self, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<Value, String> {
        if let Some(report) = on_progress {
            report(json!({"before_count":2,"indexed_count":1}));
        }
        Ok(json!({"content_type":content_type,"indexed_count":1,"deleted_count":0,"failed_count":0,"before_count":2,"after_count":1}))
    }

    async fn index_single_file(&self, _file_path: &str) -> Result<i64, String> {
        Ok(17)
    }

    async fn delete_file(&self, _file_path: &str) -> Result<(), String> {
        Ok(())
    }

    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> Result<Value, String> {
        if let Some(progress) = request.on_progress {
            let events = self.proposal_progress.lock().unwrap().clone();
            if events.is_empty() {
                progress(json!({"total_count":1,"processed_count":0,"new_items":[{"file_path":"0_inbox/a.epub","apply_status":"pending"}]}));
                progress(json!({"total_count":1,"processed_count":1,"updated_items":[{"file_path":"0_inbox/a.epub","target_category":"3_SF","apply_status":"pending"}]}));
            } else {
                for event in events {
                    progress(event);
                }
            }
        }
        let result = self
            .proposal_result
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| json!({"source_category":"0_inbox","total_count":1,"processed_count":1,"items":[{"file_path":"0_inbox/a.epub","target_category":"3_SF"}],"failures":[]}));
        Ok(result)
    }

    async fn get_categories(&self) -> Result<Value, String> {
        let (value, error) = self.categories.lock().unwrap().clone();
        error.map_or(Ok(value), Err)
    }

    async fn apply_category_changes(&self, _items: &[Value], _allowed: &HashSet<String>, _clean_existing: bool) -> Result<Value, String> {
        let (value, error) = self.apply_result.lock().unwrap().clone();
        error.map_or(Ok(value), Err)
    }

    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<Value, String> {
        if let Some(progress) = request.on_progress {
            progress(json!({"processed_count":request.items.len()}));
        }
        if let Some(done) = request.on_item_done {
            for item in request.items {
                if request.allowed_file_paths.contains(item["file_path"].as_str().unwrap_or_default()) {
                    done(json!({"file_path":item["file_path"],"apply_status":"moved","apply_error":null}));
                }
            }
        }
        if request.should_continue.is_some_and(|check| !check()) {
            return Err("stopped".into());
        }
        self.apply_category_changes(request.items, request.allowed_file_paths, request.clean_existing).await
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
    Arc::new(MockBm {
        es,
        categories: Mutex::new((json!({}), None)),
        proposal_progress: Mutex::new(Vec::new()),
        proposal_result: Mutex::new(None),
        apply_result: Mutex::new((json!({}), None)),
        path_prefix: Mutex::new(PathBuf::new()),
        mismatch_details: Mutex::new(json!({"fs_only":[{"file_path":"A/a.txt"}]})),
        mismatch_categories: Mutex::new(Vec::new()),
    })
}

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

fn normalize_path(method: &str, path: &str) -> String {
    let path = path.replace("{*", "{");
    if method != "GET" {
        return path;
    }
    ["/categories/rename", "/categories/delete"].into_iter().find_map(|reserved| path.strip_suffix(reserved).map(|prefix| format!("{prefix}/categories/{{category}}"))).unwrap_or(path)
}

#[test]
fn rust_registers_every_python_openapi_route_and_method() {
    let expected: Vec<(String, String, Vec<String>)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/python_openapi_routes.json"))).unwrap();
    let actual: HashSet<(String, String, Vec<String>)> = route_table()
        .into_iter()
        .flat_map(|route| {
            route.methods.into_iter().map(move |method| {
                let dependencies = route.dependencies.clone();
                (method.clone(), normalize_path(&method, &route.path), dependencies)
            })
        })
        .collect();
    let expected: HashSet<_> = expected.into_iter().collect();
    let missing: Vec<_> = expected.difference(&actual).collect();
    let extra: Vec<_> = actual.difference(&expected).collect();
    assert!(missing.is_empty() && extra.is_empty(), "Rust missing routes: {missing:?}; Rust-only routes: {extra:?}");
}

#[test]
fn real_mapping_facade_propagates_mapping_read_database_failures() {
    use db::{DbError, category_mapping::CategoryMapping};
    let mapping = CategoryMapping::with_connection_provider(Arc::new(|| Err(DbError::Other("offline".into()))));
    assert!(CategoryMappingApi::get_all_mappings(&mapping, "book").is_err());
    assert!(CategoryMappingApi::get_keywords(&mapping, "fiction", "book").is_err());
    assert!(!CategoryMappingApi::set_keywords(&mapping, "fiction", &["one".into()], "book").unwrap());
    assert!(CategoryMappingApi::add_keyword(&mapping, "fiction", "one", "book").is_err());
    assert!(CategoryMappingApi::remove_keyword(&mapping, "fiction", "one", "book").is_err());
    assert!(CategoryMappingApi::delete_category(&mapping, "fiction", "book", false).is_err());
    assert!(!CategoryMappingApi::update_all_mappings(&mapping, &json!({"fiction":["one"]}), "book").unwrap());
    assert!(CategoryMappingApi::get_hidden_categories(&mapping, "book").is_err());
    assert!(CategoryMappingApi::get_latest_excluded_categories(&mapping, "book").is_err());
    assert!(!CategoryMappingApi::set_hidden(&mapping, "fiction", true, "book").unwrap());
    assert!(!CategoryMappingApi::set_latest_excluded(&mapping, "fiction", true, "book").unwrap());
}

// ── 목 매니저: AsyncMock/MagicMock 대응 ──────────────────────────────────────
//
// 설계 3.2: 앱에는 `AppDeps`로 `Arc<dyn BookManagerApi>`/`Arc<dyn CategoryMappingApi>`를 넣는다. `Mock`은 그 trait를 구현하는
// 테스트 로컬 어댑터다. trait 메서드마다 호출을 `Mock::call(이름, CallArgs)`로 넘기고, 테스트가 정한 `returns`/`side_effect`의
// JSON 결과를 trait의 반환형으로 바꿔 준다. 호출 기록 모양(위치 인자/키워드 인자)은 원본 Python 호출을 따른다.
// 반환형이 `Result`가 아닌 메서드에서 `Err(String)`(= Python 예외)이 나오면 패닉으로 올린다.

/// 비동기 결과. 콜백과 side effect가 쓴다.
type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// `on_progress`/`on_item_done`/`should_continue`를 "JSON 하나를 받아 JSON 하나를 돌려주는 awaitable"로 통일한 것.
type Callback<'a> = Arc<dyn Fn(Value) -> Fut<'a, Value> + Send + Sync + 'a>;

/// Python 호출 하나의 기록. 위치 인자, 키워드 인자(JSON 객체), 콜백.
#[derive(Clone)]
struct CallArgs<'a> {
    args: Vec<Value>,
    kwargs: Value,
    callbacks: HashMap<String, Callback<'a>>,
}

#[derive(Clone)]
struct RecordedArgs {
    args: Vec<Value>,
    kwargs: Value,
    callback_names: HashSet<String>,
    thread_id: std::thread::ThreadId,
}

impl<'a> CallArgs<'a> {
    fn new(args: Vec<Value>, kwargs: Value) -> Self {
        Self { args, kwargs, callbacks: HashMap::new() }
    }

    fn with_callback(mut self, name: &str, callback: Option<Callback<'a>>) -> Self {
        if let Some(callback) = callback {
            self.callbacks.insert(name.to_string(), callback);
        }
        self
    }
}

/// `call_args!([위치 인자...], {"키워드": 값, ...})`. 값은 `json!`으로 바뀐다.
macro_rules! call_args {
    ([$($a:expr),* $(,)?], {$($k:literal : $v:expr),* $(,)?}) => {
        CallArgs::new(vec![$(json!($a)),*], json!({$($k: $v),*}))
    };
}

type SideEffect = Arc<dyn for<'a> Fn(CallArgs<'a>) -> Fut<'a, Result<Value, String>> + Send + Sync>;

#[derive(Default)]
struct MockState {
    returns: HashMap<String, Value>,
    side_effects: HashMap<String, SideEffect>,
    calls: Vec<(String, RecordedArgs)>,
    path_prefix: Option<PathBuf>,
}

/// 메서드별 `return_value`/`side_effect`를 정하고 호출을 기록한다. 설정이 없는 메서드는 null을 돌려준다.
#[derive(Clone, Default)]
struct Mock(Arc<Mutex<MockState>>);

impl Mock {
    fn new() -> Self {
        Self::default()
    }

    fn returns(&self, method: &str, value: Value) {
        let mut s = self.0.lock().unwrap();
        s.side_effects.remove(method);
        s.returns.insert(method.to_string(), value);
    }

    fn side_effect(&self, method: &str, f: impl for<'a> Fn(CallArgs<'a>) -> Fut<'a, Result<Value, String>> + Send + Sync + 'static) {
        self.0.lock().unwrap().side_effects.insert(method.to_string(), Arc::new(f));
    }

    /// `side_effect=RuntimeError(msg)`에 대응한다.
    fn raises(&self, method: &str, msg: &str) {
        let msg = msg.to_string();
        self.side_effect(method, move |_| {
            let m = msg.clone();
            Box::pin(async move { Err(m) })
        });
    }

    fn set_path_prefix(&self, p: &Path) {
        self.0.lock().unwrap().path_prefix = Some(p.to_path_buf());
    }

    fn calls(&self, method: &str) -> Vec<RecordedArgs> {
        self.0.lock().unwrap().calls.iter().filter(|(m, _)| m == method).map(|(_, c)| c.clone()).collect()
    }

    fn call_count(&self, method: &str) -> usize {
        self.calls(method).len()
    }

    fn assert_not_called(&self, method: &str) {
        assert_eq!(self.call_count(method), 0, "{method}는 호출되지 않아야 한다");
    }

    fn assert_called_once_with(&self, method: &str, args: Value, kwargs: Value) {
        let calls = self.calls(method);
        assert_eq!(calls.len(), 1, "{method} 호출 횟수");
        assert_eq!((Value::Array(calls[0].args.clone()), calls[0].kwargs.clone()), (args, kwargs), "{method} 인자");
    }

    /// 마지막 호출 기준 `assert_called_with`.
    fn assert_called_with(&self, method: &str, args: Value, kwargs: Value) {
        let calls = self.calls(method);
        let last = calls.last().unwrap_or_else(|| panic!("{method}가 호출되지 않았다"));
        assert_eq!((Value::Array(last.args.clone()), last.kwargs.clone()), (args, kwargs), "{method} 마지막 인자");
    }

    fn assert_any_call(&self, method: &str, args: Value, kwargs: Value) {
        let found = self.calls(method).iter().any(|c| Value::Array(c.args.clone()) == args && c.kwargs == kwargs);
        assert!(found, "{method}를 이 인자로 부른 적이 없다: {args} {kwargs}");
    }

    /// 호출을 기록하고 side effect 또는 `returns` 값을 돌려준다. `Err`는 Python 예외다.
    fn call<'a>(&self, method: &str, args: CallArgs<'a>) -> Fut<'a, Result<Value, String>> {
        let (effect, ret) = {
            let mut s = self.0.lock().unwrap();
            s.calls.push((
                method.to_string(),
                RecordedArgs { args: args.args.clone(), kwargs: args.kwargs.clone(), callback_names: args.callbacks.keys().cloned().collect(), thread_id: std::thread::current().id() },
            ));
            (s.side_effects.get(method).cloned(), s.returns.get(method).cloned())
        };
        match effect {
            Some(f) => f(args),
            None => Box::pin(async move { Ok(ret.unwrap_or(Value::Null)) }),
        }
    }

    /// 반환형이 `Result`가 아닌 비동기 메서드용. 예외는 패닉이다.
    async fn invoke<'a>(&self, method: &str, args: CallArgs<'a>) -> Value {
        match self.call(method, args).await {
            Ok(v) => v,
            Err(e) => panic!("{method}가 예외를 던졌다: {e}"),
        }
    }

    /// 동기 trait 메서드용. side effect가 바로 끝나는 future여야 한다.
    fn invoke_sync_try(&self, method: &str, args: CallArgs<'_>) -> Result<Value, String> {
        poll_ready(self.call(method, args))
    }

    fn invoke_sync(&self, method: &str, args: CallArgs<'_>) -> Value {
        match self.invoke_sync_try(method, args) {
            Ok(v) => v,
            Err(e) => panic!("{method}가 예외를 던졌다: {e}"),
        }
    }
}

/// 이미 끝난 future를 한 번 폴링해서 값을 꺼낸다. 동기 trait 메서드(`CategoryMappingApi` 등)가 쓴다.
fn poll_ready<T>(mut fut: Fut<'_, T>) -> T {
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("동기 trait 메서드의 side effect는 바로 끝나는 future여야 한다"),
    }
}

fn progress_callback<'a>(f: Option<&'a (dyn Fn(Value) + Send + Sync)>) -> Option<Callback<'a>> {
    f.map(|f| -> Callback {
        Arc::new(move |event: Value| -> Fut<'a, Value> {
            f(event);
            Box::pin(async { Value::Null })
        })
    })
}

fn continue_callback<'a>(f: Option<&'a (dyn Fn() -> bool + Send + Sync)>) -> Option<Callback<'a>> {
    f.map(|f| -> Callback {
        Arc::new(move |_: Value| -> Fut<'a, Value> {
            let value = f();
            Box::pin(async move { json!(value) })
        })
    })
}

#[async_trait::async_trait]
impl BookManagerApi for Mock {
    fn path_prefix(&self) -> PathBuf {
        self.0.lock().unwrap().path_prefix.clone().unwrap_or_default()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        Arc::new(EsProxy(self.clone()))
    }

    /// `[책, null]`은 찾음, `[null, 오류]`는 조회 실패, `[null, null]`은 없음이다.
    async fn get_book(&self, id: i64) -> Result<Book, BookLookupError> {
        let v = self.invoke("get_book", call_args!([id], {})).await;
        match (opt_book(arr(&v, 0)), opt_string(arr(&v, 1))) {
            (Some(book), _) => Ok(book),
            (None, Some(error)) => Err(BookLookupError::Failed(error)),
            (None, None) => Err(BookLookupError::NotFound(id)),
        }
    }

    async fn get_book_content(&self, id: i64) -> BookContent {
        let v = self.invoke("get_book_content", call_args!([id], {})).await;
        match v.get("__error__").and_then(Value::as_str) {
            Some(message) => BookContent::Error(message.to_string()),
            None => BookContent::Text(v.as_str().unwrap_or("").to_string()),
        }
    }

    async fn get_book_preview(&self, id: i64, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        to_response(self.invoke("get_book_preview", call_args!([], {"book_id": id, "chapters": chapters, "resource_base_url": resource_base_url})).await)
    }

    async fn get_book_preview_with_pages(&self, id: i64, pages: Option<i64>, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        to_response(
            self.invoke("get_book_preview", call_args!([], {"book_id": id, "pages": pages.unwrap_or(5), "chapters": chapters.unwrap_or(10), "resource_base_url": resource_base_url.unwrap_or("")}))
                .await,
        )
    }

    async fn get_pdf_pages(&self, id: i64, start: i64, end: i64) -> BookResponse {
        to_response(self.invoke("get_pdf_pages", call_args!([], {"book_id": id, "start": start, "end": end})).await)
    }

    async fn get_html_resource(&self, book_id: i64, resource_path: &str) -> BookResponse {
        to_response(self.invoke("get_html_resource", call_args!([], {"book_id": book_id, "resource_path": resource_path})).await)
    }

    async fn validate_epub(&self, book_id: i64) -> Result<Value, String> {
        value_result(&self.invoke("validate_epub", call_args!([book_id], {})).await)
    }

    async fn validate_pdf(&self, book_id: i64) -> Result<Value, String> {
        value_result(&self.invoke("validate_pdf", call_args!([book_id], {})).await)
    }

    async fn get_books_in_category(&self, category: &str) -> Result<Vec<Book>, String> {
        let v = self.invoke("get_books_in_category", call_args!([category], {})).await;
        opt_string(arr(&v, 1)).map_or_else(|| Ok(books(arr(&v, 0))), Err)
    }

    async fn get_categories(&self) -> Result<Value, String> {
        let v = self.invoke("get_categories", call_args!([], {})).await;
        opt_string(arr(&v, 1)).map_or_else(|| Ok(arr(&v, 0)), Err)
    }

    async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[String]>, category: Option<&str>) -> Result<BookPage, String> {
        let mut kwargs = json!({"size": size, "offset": offset, "exclude_categories": exclude_categories});
        if let Some(category) = category {
            kwargs["category"] = json!(category);
        }
        let v = self.invoke("search_by_keyword_paged", CallArgs::new(vec![json!(keyword)], kwargs)).await;
        page(&v)
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> Result<BookPage, String> {
        let v = self.invoke("get_latest_books", call_args!([], {"size": size, "exclude_categories": exclude_categories})).await;
        page(&v)
    }

    async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> Result<BookPage, String> {
        let v = self.invoke("search_similar_books_paged", call_args!([book_id], {"size": size, "offset": offset})).await;
        page(&v)
    }

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        let v = self.invoke("get_cover_books_paged", call_args!([], {"category": category, "size": size, "cursor": cursor, "exclude_categories": exclude_categories})).await;
        cover_page(&v)
    }

    async fn get_uncovered_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        let v = self.invoke("get_uncovered_books_paged", call_args!([], {"category": category, "size": size, "cursor": cursor, "exclude_categories": exclude_categories})).await;
        cover_page(&v)
    }

    #[allow(clippy::too_many_arguments)]
    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> Result<Value, String> {
        let args = call_args!([request.category, request.mappings], {"content_type": request.content_type, "use_bookstore": request.use_bookstore, "use_content_meta": request.use_content_meta, "delay": request.delay})
            .with_callback("on_progress", progress_callback(request.on_progress));
        let v = self.invoke("propose_category_changes", args).await;
        value_result(&v)
    }

    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<Value, String> {
        let mut allowed: Vec<&String> = request.allowed_file_paths.iter().collect();
        allowed.sort();
        let args = call_args!([request.items, allowed], {"content_type": request.content_type, "clean_existing": request.clean_existing})
            .with_callback("on_progress", progress_callback(request.on_progress))
            .with_callback("on_item_done", progress_callback(request.on_item_done))
            .with_callback("should_continue", continue_callback(request.should_continue));
        let v = self.invoke("apply_category_changes", args).await;
        value_result(&v)
    }

    async fn index_single_file(&self, file_path: &str) -> Result<i64, String> {
        let v = self.invoke("index_single_file", call_args!([file_path], {})).await;
        match (arr(&v, 0).as_i64(), opt_string(arr(&v, 1))) {
            (Some(book_id), None) => Ok(book_id),
            (_, error) => Err(error.unwrap_or_default()),
        }
    }

    /// `["Ok", null]`만 성공이다.
    async fn delete_file(&self, file_path: &str) -> Result<(), String> {
        let v = self.invoke("delete_file", call_args!([file_path], {})).await;
        if arr(&v, 0).as_str() == Some("Ok") { Ok(()) } else { Err(opt_string(arr(&v, 1)).unwrap_or_default()) }
    }

    async fn reload_category(&self, category: &str, content_type: &str) -> Result<Value, String> {
        match self.call("reload_category", call_args!([category, content_type], {})).await {
            Ok(v) => value_result(&v),
            Err(e) => Err(e),
        }
    }

    async fn reload_category_mismatches(&self, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<Value, String> {
        let args = call_args!([content_type], {}).with_callback(
            "on_progress",
            on_progress.map(|f| {
                Arc::new(move |event| -> Fut<'_, Value> {
                    let f = f.clone();
                    Box::pin(async move {
                        f(event);
                        Value::Null
                    })
                }) as Callback<'_>
            }),
        );
        match self.call("reload_category_mismatches", args).await {
            Ok(v) => value_result(&v),
            Err(e) => Err(e),
        }
    }

    async fn reload_category_mismatch_files(&self, category: &str, content_type: &str, on_progress: Option<ReloadProgress>) -> Result<Value, String> {
        let args = call_args!([category, content_type], {}).with_callback(
            "on_progress",
            on_progress.map(|f| {
                Arc::new(move |event| -> Fut<'_, Value> {
                    let f = f.clone();
                    Box::pin(async move {
                        f(event);
                        Value::Null
                    })
                }) as Callback<'_>
            }),
        );
        match self.call("reload_category_mismatch_files", args).await {
            Ok(v) => value_result(&v),
            Err(e) => Err(e),
        }
    }

    fn get_category_mismatch_details(&self, category: &str) -> Result<Value, String> {
        self.invoke_sync_try("get_category_mismatch_details", call_args!([category], {}))
    }
}

/// `book_manager.es_manager.<메서드>` 호출은 점 표기 이름(`es_manager.delete`)으로 같은 `Mock`에 기록한다.
struct EsProxy(Mock);

impl EsManagerApi for EsProxy {
    fn delete(&self, doc_id: i64) -> Result<bool, EsError> {
        self.0.invoke_sync_try("es_manager.delete", call_args!([doc_id], {})).map(|v| flag(&v)).map_err(EsError::Other)
    }
}

/// category_mapping 대역. 키워드 인자 모양은 라우트의 Python 호출(`content_type=...`)을 따른다.
impl CategoryMappingApi for Mock {
    fn get_all_mappings(&self, ct: &str) -> Result<Value, MappingError> {
        self.invoke_sync_try("get_all_mappings", call_args!([], {"content_type": ct})).map_err(MappingError::Other)
    }

    fn get_keywords(&self, category: &str, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_keywords", call_args!([category], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_keywords(&self, category: &str, keywords: &[String], ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_keywords", call_args!([category, keywords], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn add_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("add_keyword", call_args!([category, keyword], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn remove_keyword(&self, category: &str, keyword: &str, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("remove_keyword", call_args!([category, keyword], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn delete_category(&self, category: &str, ct: &str, prefix: bool) -> Result<bool, MappingError> {
        self.invoke_sync_try("delete_category", call_args!([category], {"content_type": ct, "prefix": prefix})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn update_all_mappings(&self, mappings: &Value, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("update_all_mappings", call_args!([mappings], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn get_hidden_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_hidden_categories", call_args!([], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_hidden(&self, category: &str, hidden: bool, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_hidden", call_args!([category, hidden], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn get_latest_excluded_categories(&self, ct: &str) -> Result<Vec<String>, MappingError> {
        self.invoke_sync_try("get_latest_excluded_categories", call_args!([], {"content_type": ct})).map(strings).map_err(MappingError::Other)
    }

    fn set_latest_excluded(&self, category: &str, excluded: bool, ct: &str) -> Result<bool, MappingError> {
        self.invoke_sync_try("set_latest_excluded", call_args!([category, excluded], {"content_type": ct})).map(|v| flag(&v)).map_err(MappingError::Other)
    }

    fn acquire_reload_lock(&self, ct: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), MappingError> {
        let v = self.invoke_sync("acquire_reload_lock", call_args!([ct, category, reload_source], {}));
        Ok((flag(&arr(&v, 0)), opt_string(arr(&v, 1)), opt_json(arr(&v, 2))))
    }

    fn heartbeat_reload_lock(&self, ct: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        self.invoke_sync_try("heartbeat_reload_lock", CallArgs::new(vec![json!(ct), json!(category)], counts.clone())).map(|_| ())
    }

    fn complete_reload_lock(&self, ct: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let mut kwargs = counts.clone();
        if let Some(category) = category {
            kwargs["category"] = json!(category);
        }
        self.invoke_sync_try("complete_reload_lock", CallArgs::new(vec![json!(ct), json!(status), json!(error)], kwargs)).map(|_| ())
    }

    fn get_reload_status(&self, ct: &str, category: Option<&str>) -> Result<Option<Value>, MappingError> {
        Ok(opt_json(self.invoke_sync("get_reload_status", call_args!([ct, category], {}))))
    }

    fn get_classify_proposal_status(&self, content_type: &str, stale_seconds: i64) -> Result<Value, MappingError> {
        self.invoke_sync_try("get_classify_proposal_status", call_args!([content_type, stale_seconds], {})).map_err(MappingError::Other)
    }

    fn set_classify_proposal_status(&self, status: Value, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("set_classify_proposal_status", call_args!([status, content_type], {})).map(|_| ()).map_err(MappingError::Other)
    }

    fn merge_classify_proposal_status(&self, fields: Value, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("merge_classify_proposal_status", call_args!([fields, content_type], {})).map(|_| ()).map_err(MappingError::Other)
    }

    fn try_start_classify_proposal(&self, status: Value, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let v = self.invoke_sync("try_start_classify_proposal", call_args!([status, content_type, stale_seconds], {}));
        Ok((flag(&arr(&v, 0)), arr(&v, 1)))
    }

    fn try_begin_classify_apply(&self, apply_token: &str, content_type: &str, stale_seconds: i64) -> Result<(bool, Value), MappingError> {
        let v = self.invoke_sync("try_begin_classify_apply", call_args!([apply_token, content_type, stale_seconds], {}));
        Ok((flag(&arr(&v, 0)), arr(&v, 1)))
    }

    fn clear_classify_proposal_items(&self, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync("clear_classify_proposal_items", call_args!([], {"content_type": content_type}));
        Ok(())
    }

    fn get_classify_proposal_items(&self, content_type: &str) -> Result<Vec<Value>, MappingError> {
        Ok(self.invoke_sync("get_classify_proposal_items", call_args!([], {"content_type": content_type})).as_array().cloned().unwrap_or_default())
    }

    fn add_classify_proposal_items(&self, items: &[Value], source_category: &str, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("add_classify_proposal_items", call_args!([items, source_category], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn update_classify_proposal_item_payloads(&self, items: &[Value], content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("update_classify_proposal_item_payloads", call_args!([items], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn update_classify_proposal_item_status(&self, file_path: &str, apply_status: &str, apply_error: Option<&str>, content_type: &str) -> Result<(), MappingError> {
        self.invoke_sync_try("update_classify_proposal_item_status", call_args!([file_path, apply_status, apply_error], {"content_type": content_type})).map(|_| ()).map_err(MappingError::Other)
    }

    fn delete_applied_classify_proposal_items(&self, content_type: &str) -> Result<u64, MappingError> {
        Ok(count(&self.invoke_sync("delete_applied_classify_proposal_items", call_args!([], {"content_type": content_type}))).max(0) as u64)
    }

    fn delete_classify_proposal_item(&self, file_path: &str, content_type: &str) -> Result<u64, MappingError> {
        Ok(count(&self.invoke_sync("delete_classify_proposal_item", call_args!([file_path], {"content_type": content_type}))).max(0) as u64)
    }
}

/// refresh token 저장소 대역. `rotate`/`revoke_family`만 쓴다. Python 키워드 인자(`*`) 모양으로 기록한다.
/// TODO(common): `RefreshTokenStoreApi`의 시그니처는 설계 문서에 없다. 파이썬 `refresh_token_store.py`를 따랐다.
impl RefreshTokenStoreApi for Mock {
    fn store_issued(&self, token_id: &str, family_id: &str, email: &str, issued_at: i64, expires_at: i64, client_ip: Option<&str>, user_agent: Option<&str>) -> Result<(), StoreError> {
        let kwargs = json!({"token_id":token_id,"family_id":family_id,"email":email,"issued_at":issued_at,"expires_at":expires_at,"client_ip":client_ip,"user_agent":user_agent});
        self.invoke_sync_try("store_issued", CallArgs::new(vec![], kwargs)).map(|_| ()).map_err(StoreError::Unsupported)
    }
    #[allow(clippy::too_many_arguments)]
    fn rotate(
        &self,
        current_token_id: &str,
        new_token_id: &str,
        family_id: &str,
        email: &str,
        issued_at: i64,
        expires_at: i64,
        client_ip: Option<&str>,
        user_agent: Option<&str>,
    ) -> Result<Rotation, StoreError> {
        let kwargs = json!({"current_token_id": current_token_id, "new_token_id": new_token_id, "family_id": family_id, "email": email, "issued_at": issued_at, "expires_at": expires_at, "client_ip": client_ip, "user_agent": user_agent});
        self.invoke_sync_try("rotate", CallArgs::new(vec![], kwargs)).and_then(|v| v.as_str().unwrap_or("").parse()).map_err(StoreError::Unsupported)
    }

    fn revoke_family(&self, family_id: &str, reason: &str) -> Result<(), StoreError> {
        self.invoke_sync_try("revoke_family", call_args!([family_id, reason], {})).map(|_| ()).map_err(StoreError::Unsupported)
    }

    fn get_token_observation(&self, token_id: &str) -> Result<Option<Value>, StoreError> {
        self.invoke_sync_try("get_token_observation", call_args!([token_id], {})).map(opt_json).map_err(StoreError::Unsupported)
    }

    fn _session_rows(&self, email: Option<&str>, family_id: Option<&str>) -> Result<Vec<Row>, StoreError> {
        let value = self.invoke_sync_try("_session_rows", call_args!([email, family_id], {})).map_err(StoreError::Unsupported)?;
        let rows = value
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                row.as_object()
                    .into_iter()
                    .flat_map(|obj| obj.iter())
                    .map(|(key, value)| {
                        (
                            key.clone(),
                            match value {
                                Value::Null => DbValue::Null,
                                Value::Number(n) => DbValue::Int(n.as_i64().unwrap_or_default()),
                                Value::String(s) => DbValue::Text(s.clone()),
                                _ => DbValue::Text(value.to_string()),
                            },
                        )
                    })
                    .collect()
            })
            .collect();
        Ok(rows)
    }
}

// ── 분류 제안 작업 상태 저장소 흉내 ──────────────────────────────────────────

type Store = Arc<Mutex<HashMap<String, Value>>>;

fn effective(store: &Store, content_type: &str, stale_seconds: f64) -> Value {
    let mut current = store.lock().unwrap().get(content_type).cloned().unwrap_or_else(|| json!({"status": "idle", "content_type": content_type}));
    let obj = current.as_object_mut().unwrap();
    let age = obj.remove("_age_seconds").and_then(|a| a.as_f64()).unwrap_or(0.0);
    let status = obj.get("status").and_then(Value::as_str).unwrap_or("").to_string();
    if (status == "running" || status == "applying") && age > stale_seconds {
        obj.insert("status".into(), json!("failed"));
        let error = obj.get("error").filter(|e| is_truthy(e)).cloned().unwrap_or_else(|| json!("응답 없이 중단된 것으로 보입니다."));
        obj.insert("error".into(), error);
    }
    current
}

/// 카테고리가 빈 문자열이면 False (main.py:252).
#[cfg(any())] // hidden category 필터는 공개 라우트 통합 테스트에서 검증한다.
#[test]
fn test_category_matches_hidden_empty_category_returns_false() {
    assert!(!backend::main::category_matches_hidden("", &["cat1", "cat2"]));
}
