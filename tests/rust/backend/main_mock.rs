//! 원본: tests/test_main_mock.py
#![allow(dead_code)]
//!
//! backend/main.py의 라우트 단위 목 테스트다. ES와 MySQL 없이 매니저를 가짜로 끼운다.
//! Python은 `_LazyProxy._instance`를 AsyncMock/MagicMock으로 바꿔 끼우고 `dependency_overrides`로 인증을 우회한다.
//! Rust는 `backend::main::app_with(AppDeps)`로 같은 효과를 낸다. 매니저 호출은 메서드 이름과 인자(JSON)로 기록한다(설계 3.2의 테스트 로컬 어댑터 `Mock`).
//!
//! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.2, 3.5, 3.13):
//! - `backend::main::app_with(deps: AppDeps) -> axum::Router`, `AppDeps::new(auth: AuthMode)`와 소비형 빌더 `with_*`.
//!   `Fx::build()` 한 곳에서만 `AppDeps`를 만든다.
//!   - `with_book_manager`, `with_comics_manager`: `Arc<dyn BookManagerApi>`, `with_category_mapping`: `Arc<dyn CategoryMappingApi>`
//!   - `with_fs(Arc<dyn sysdeps::FsOps>)`(구 `list_dir`), `with_google_verifier(Arc<dyn GoogleVerifier>)`(구 `google_client_id`+`google_verify`),
//!     `with_role_resolver(Arc<dyn RoleResolver>)`(구 `determine_role`), `with_refresh_token_store(Arc<dyn RefreshTokenStoreApi>)`,
//!     `with_task_runner(Arc<dyn TaskRunner>)`(구 `to_thread_spy`), `with_bookstore_factory(Arc<dyn Fn() -> Box<dyn BookstoreSearch> + Send + Sync>)`(구 `bookstores`),
//!     `with_reload_progress_flush_interval(Duration)`(구 `reload_progress_flush_interval_seconds`)
//! - `backend::main::{AuthMode, AuthSlots}`: `AuthMode::Shared(Arc<AuthSlots>)`, `AuthSlots { auth: Mutex<Value>, admin: Mutex<Value> }`(공개 필드). 테스트 중간에 바꿀 수 있다(구 `require_auth`/`require_admin` 필드).
//! - `backend::main::{GoogleVerifier, RoleResolver, TaskRunner, BookstoreSearch}`
//!   - `GoogleVerifier { fn client_id(&self) -> Option<String>; fn verify(&self, credential: &str, audience: &str) -> Result<Value, String> }`
//!   - `RoleResolver { fn determine_role(&self, email: &str) -> Option<String> }`
//!   - `TaskRunner`: 모든 메서드에 기본 구현이 있다. `fn spawn_blocking_observed(&self, name: &str)`만 `SpyRunner`가 재정의한다.
//!   - `BookstoreSearch`: 문서는 이름만 정했다. `fn search(&self, isbn, title, author) -> (Vec<(String x6)>, String, String)`, `fn build_search_url(&self, keyword) -> String`을 가정한다.
//! - `backend::book_manager::{BookManagerApi, BookContent, BookResponse, ProgressFn, ContinueFn}` (모든 메서드에 기본 구현이 있다)
//!   - 문서 3.2의 통일 시그니처: `get_book(id)`, `get_book_content(id) -> BookContent`, `get_book_preview(id, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse`,
//!     `get_pdf_pages(id, start, end) -> BookResponse`, `path_prefix()`, `es_manager() -> Arc<dyn EsManagerApi>`
//!   - 문서에 없어 가정한 메서드는 아래 `impl BookManagerApi for Mock`을 본다. 파이썬 `book_manager.py` 시그니처를 따랐고, 콜백 인자는 다음 모양이다.
//!     `ProgressFn = Arc<dyn Fn(Value) -> BoxFuture<'static, ()> + Send + Sync>`, `ContinueFn = Arc<dyn Fn() -> BoxFuture<'static, bool> + Send + Sync>`
//!     호출 예: `propose_category_changes(category, mappings, ...)`, `apply_category_changes(items, allowed, ...)`.
//!   - `Err(String)`(Python 예외)은 반환형이 `Result`인 메서드(`reload_*`, `get_category_mismatch_details`)에서만 값으로 전달된다. 나머지는 `Mock`이 패닉으로 올린다.
//! - `backend::es_manager::{EsManagerApi, EsError}`: `delete(&self, i64) -> Result<bool, EsError>`
//! - `backend::category_mapping::{CategoryMappingApi, MappingError}`: 동기 메서드이고 모든 메서드에 기본 구현이 있다. 메서드 모양은 아래 `impl CategoryMappingApi for Mock`을 본다.
//! - `backend::refresh_token_store::RefreshTokenStoreApi`: `rotate(...) -> Result<String, String>`, `revoke_family(&self, family_id, reason)` (모양은 `impl RefreshTokenStoreApi for Mock`)
//! - `backend::main::create_item_router(manager: Arc<dyn BookManagerApi>, category_mapping: Arc<dyn CategoryMappingApi>, content_type: &str) -> ItemRouter`
//!   - `ItemRouter::run_classify_proposal_job(&self, category: &str, use_bookstore: bool, use_content_meta: bool, delay: f64)` (async)
//!   - `ItemRouter::run_classify_apply_job(&self, items: Vec<Value>, allowed: HashSet<String>, flag: bool, token: Option<&str>)` (async)
//!   - `ItemRouter::classify_apply_endpoint(&self, body: ClassifyApplyModel) -> Value` (async, BackgroundTasks는 만들기만 하고 실행하지 않는다)
//! - `backend::main::{ClassifyApplyModel, ClassifyApplyItemModel, GENERIC_MISMATCH_ERROR}`
//! - `backend::main::{summarize_request_body, is_local_frontend_origin, is_request_from_frontend_host, category_matches_hidden}`
//! - `backend::auth::create_refresh_token(email, role, name: Option<&str>, picture: Option<&str>) -> String` (설계 3.2의 4인자)
//! - `backend::book::Book`: `Default + Clone + Serialize + Deserialize`
//!
//! HTTP 라우트가 백그라운드 작업(제안/적용/reload)을 시작하면, Starlette TestClient처럼 응답을 돌려주기 전에 작업이 끝난다고 가정한다.

mod common;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use axum::http::Method;
use backend::book::{Book, BookLookupError};
use backend::book_manager::{BookContent, BookManagerApi, BookResponse};
use backend::db::{DbValue, Row};
use backend::es_manager::{EsError, EsManagerApi};
use backend::main::{
    AppDeps, ApplyCategoryChanges, AuthMode, AuthSlots, BookstoreFactory, BookstoreSearch, CategoryMappingApi, GoogleVerifier, MappingError, ProposeCategoryChanges, ReloadProgress, RoleResolver,
    TaskHandle, TaskRunner,
};
use backend::refresh_token_store::{RefreshTokenStoreApi, Rotation, StoreError};
use common::env::{EnvGuard, EnvReadGuard};
use common::{Response, TestClient};
use serde_json::{Map, Value, json};
use sysdeps::FsOps;
use tempfile::TempDir;

// ── 공용 상수와 헬퍼 ─────────────────────────────────────────────────────────

fn now_plus_3600() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 3600
}

fn admin_payload() -> Value {
    json!({"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": now_plus_3600()})
}

fn viewer_payload() -> Value {
    json!({"email": "viewer@test.com", "role": "viewer", "name": "Viewer", "picture": "", "exp": now_plus_3600()})
}

fn book_dict() -> Value {
    json!({
        "book_id": 1, "category": "_epub", "title": "Test Book", "author": "Test Author", "file_path": "_epub/test.epub",
        "file_type": "epub", "file_size": 1024, "line_count": 0, "page_count": 0, "isbn": "",
        "updated_time": "2024-01-01T00:00:00.000000", "score": 0.0
    })
}

/// `_make_book(overrides)`에 대응한다. MagicMock 책 대신 `.dict()` 결과를 돌려준다.
fn make_book(overrides: Value) -> Value {
    let mut d = book_dict();
    for (k, v) in overrides.as_object().cloned().unwrap_or_default() {
        d[k] = v;
    }
    d
}

/// fastapi `Response(content=..., media_type=...)`에 대응한다.
fn raw_response(content: &str, media_type: Option<&str>) -> Value {
    json!({"__response__": {"content": content, "media_type": media_type}})
}

/// URL 경로와 쿼리에 들어가는 한글을 퍼센트 인코딩한다. `requests`가 하는 일과 같다.
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
    }
}

fn set_cookies(r: &Response) -> Vec<String> {
    r.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.clone()).collect()
}

fn path_set(v: &Value) -> HashSet<String> {
    v.as_array().expect("배열").iter().map(|x| x.as_str().expect("문자열").to_string()).collect()
}

fn set_of(paths: &[&str]) -> HashSet<String> {
    paths.iter().map(|s| s.to_string()).collect()
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

// JSON -> trait 반환형 변환.

fn arr(v: &Value, i: usize) -> Value {
    v.get(i).cloned().unwrap_or(Value::Null)
}

fn opt_string(v: Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn opt_json(v: Value) -> Option<Value> {
    if v.is_null() { None } else { Some(v) }
}

fn strings(v: Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or(false)
}

fn count(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

fn opt_book(v: Value) -> Option<Book> {
    if v.is_null() { None } else { Some(Book::new(v["book_id"].as_i64().unwrap_or_default(), &v, v["score"].as_f64().unwrap_or_default())) }
}

fn books(v: Value) -> Vec<Book> {
    v.as_array().map(|a| a.iter().cloned().map(|b| opt_book(b).expect("Book JSON")).collect()).unwrap_or_default()
}

/// `[[book, display_title], ...]`
fn titled_books(v: Value) -> Vec<(Book, String)> {
    v.as_array().map(|a| a.iter().map(|row| (opt_book(arr(row, 0)).expect("Book JSON"), arr(row, 1).as_str().unwrap_or("").to_string())).collect()).unwrap_or_default()
}

/// `{"__response__": {"content": .., "media_type": ..}}` -> 200 응답.
fn to_response(v: Value) -> BookResponse {
    let r = v.get("__response__").cloned().unwrap_or(Value::Null);
    BookResponse { status_code: 200, media_type: r["media_type"].as_str().map(str::to_string), headers: Vec::new(), body: r["content"].as_str().unwrap_or("").as_bytes().to_vec() }
}

/// `(Option<Value>, Option<String>)` 쌍. `[결과, 오류]`
fn pair(v: &Value) -> (Option<Value>, Option<String>) {
    (opt_json(arr(v, 0)), opt_string(arr(v, 1)))
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

    async fn validate_epub(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        pair(&self.invoke("validate_epub", call_args!([book_id], {})).await)
    }

    async fn validate_pdf(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        pair(&self.invoke("validate_pdf", call_args!([book_id], {})).await)
    }

    async fn get_books_in_category(&self, category: &str) -> (Vec<Book>, Option<String>) {
        let v = self.invoke("get_books_in_category", call_args!([category], {})).await;
        (books(arr(&v, 0)), opt_string(arr(&v, 1)))
    }

    async fn get_categories(&self) -> (Value, Option<String>) {
        let v = self.invoke("get_categories", call_args!([], {})).await;
        (arr(&v, 0), opt_string(arr(&v, 1)))
    }

    async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[String]>, category: Option<&str>) -> (Vec<Book>, i64, Option<String>) {
        let mut kwargs = json!({"size": size, "offset": offset, "exclude_categories": exclude_categories});
        if let Some(category) = category {
            kwargs["category"] = json!(category);
        }
        let v = self.invoke("search_by_keyword_paged", CallArgs::new(vec![json!(keyword)], kwargs)).await;
        (books(arr(&v, 0)), count(&arr(&v, 1)), opt_string(arr(&v, 2)))
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> (Vec<Book>, i64, Option<String>) {
        let v = self.invoke("get_latest_books", call_args!([], {"size": size, "exclude_categories": exclude_categories})).await;
        (books(arr(&v, 0)), count(&arr(&v, 1)), opt_string(arr(&v, 2)))
    }

    async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> (Vec<Book>, i64, Option<String>) {
        let v = self.invoke("search_similar_books_paged", call_args!([book_id], {"size": size, "offset": offset})).await;
        (books(arr(&v, 0)), count(&arr(&v, 1)), opt_string(arr(&v, 2)))
    }

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> (Vec<(Book, String)>, i64, Option<String>, Option<String>) {
        let v = self.invoke("get_cover_books_paged", call_args!([], {"category": category, "size": size, "cursor": cursor, "exclude_categories": exclude_categories})).await;
        (titled_books(arr(&v, 0)), count(&arr(&v, 1)), opt_string(arr(&v, 2)), opt_string(arr(&v, 3)))
    }

    async fn get_uncovered_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> (Vec<(Book, String)>, Option<i64>, Option<String>, Option<String>) {
        let v = self.invoke("get_uncovered_books_paged", call_args!([], {"category": category, "size": size, "cursor": cursor, "exclude_categories": exclude_categories})).await;
        (titled_books(arr(&v, 0)), arr(&v, 1).as_i64(), opt_string(arr(&v, 2)), opt_string(arr(&v, 3)))
    }

    #[allow(clippy::too_many_arguments)]
    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> (Value, Option<String>) {
        let args = call_args!([request.category, request.mappings], {"content_type": request.content_type, "use_bookstore": request.use_bookstore, "use_content_meta": request.use_content_meta, "delay": request.delay})
            .with_callback("on_progress", progress_callback(request.on_progress));
        let v = self.invoke("propose_category_changes", args).await;
        (arr(&v, 0), opt_string(arr(&v, 1)))
    }

    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> (Value, Option<String>) {
        let mut allowed: Vec<&String> = request.allowed_file_paths.iter().collect();
        allowed.sort();
        let args = call_args!([request.items, allowed], {"content_type": request.content_type, "clean_existing": request.clean_existing})
            .with_callback("on_progress", progress_callback(request.on_progress))
            .with_callback("on_item_done", progress_callback(request.on_item_done))
            .with_callback("should_continue", continue_callback(request.should_continue));
        let v = self.invoke("apply_category_changes", args).await;
        (arr(&v, 0), opt_string(arr(&v, 1)))
    }

    async fn index_single_file(&self, file_path: &str) -> (Option<i64>, Option<String>) {
        let v = self.invoke("index_single_file", call_args!([file_path], {})).await;
        (arr(&v, 0).as_i64(), opt_string(arr(&v, 1)))
    }

    async fn delete_file(&self, file_path: &str) -> (String, Option<String>) {
        let v = self.invoke("delete_file", call_args!([file_path], {})).await;
        (arr(&v, 0).as_str().unwrap_or("").to_string(), opt_string(arr(&v, 1)))
    }

    async fn reload_category(&self, category: &str, content_type: &str) -> (Value, Option<String>) {
        match self.call("reload_category", call_args!([category, content_type], {})).await {
            Ok(v) => (arr(&v, 0), opt_string(arr(&v, 1))),
            Err(e) => (Value::Null, Some(e)),
        }
    }

    async fn reload_category_mismatches(&self, content_type: &str, on_progress: Option<ReloadProgress>) -> (Value, Option<String>) {
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
            Ok(v) => (arr(&v, 0), opt_string(arr(&v, 1))),
            Err(e) => (Value::Null, Some(e)),
        }
    }

    async fn reload_category_mismatch_files(&self, category: &str, content_type: &str, on_progress: Option<ReloadProgress>) -> (Value, Option<String>) {
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
            Ok(v) => (arr(&v, 0), opt_string(arr(&v, 1))),
            Err(e) => (Value::Null, Some(e)),
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

type BookRow = (String, String, String, String, String, String);

/// 서점 클래스 대역. `search`/`build_search_url`의 결과를 `Mock`이 정한다.
/// TODO(common): `BookstoreSearch`의 메서드 모양은 설계 문서에 없다. Python `search(isbn, title, author)`와 `build_search_url(keyword)`를 가정했다.
struct StoreProxy(Mock);

impl BookstoreSearch for StoreProxy {
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<BookRow>, String, String) {
        let v = self.0.invoke_sync("search", call_args!([isbn, title, author], {}));
        let cell = |row: &Value, i: usize| row.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let rows = arr(&v, 0).as_array().cloned().unwrap_or_default().iter().map(|r| (cell(r, 0), cell(r, 1), cell(r, 2), cell(r, 3), cell(r, 4), cell(r, 5))).collect();
        (rows, arr(&v, 1).as_str().unwrap_or("").to_string(), arr(&v, 2).as_str().unwrap_or("").to_string())
    }

    fn api_search_url(&self, keyword: &str) -> String {
        self.0.invoke_sync("api_search_url", call_args!([keyword], {})).as_str().unwrap_or("").to_string()
    }
}

/// `with_bookstore_factory`에 넘길 factory. 서점 이름과 상관없이 같은 대역을 돌려준다(설계 3.2의 factory는 인자가 없다).
fn store_factory(store: Mock) -> BookstoreFactory {
    Arc::new(move |_| Some(Box::new(StoreProxy(store.clone())) as Box<dyn BookstoreSearch>))
}

/// `determine_role` 대역. 항상 같은 역할(또는 `None`)을 돌려준다.
/// TODO(common): main.rs에도 같은 모양의 대역이 있다(`ScriptedRole`). binary가 달라 공유하지 못해 따로 둔다.
struct FixedRole(Option<String>);

impl RoleResolver for FixedRole {
    fn determine_role(&self, _email: &str) -> Option<String> {
        self.0.clone()
    }
}

/// `TM_GOOGLE_CLIENT_ID`와 `google_id_token.verify_oauth2_token` 대역.
struct FixedVerifier {
    client_id: Option<String>,
    result: Result<Value, String>,
}

impl GoogleVerifier for FixedVerifier {
    fn client_id(&self) -> Option<String> {
        self.client_id.clone()
    }

    fn verify(&self, _credential: &str, _audience: &str) -> Result<Value, String> {
        self.result.clone()
    }
}

/// `os.listdir` 대역. 열거 결과(또는 오류)를 테스트가 정한다.
/// TODO(common): `FaultyFs`에는 읽기 결과를 바꾸는 훅이 없어 로컬로 정의했다(main.rs의 `ListDirFs`와 같다). 나머지 `FsOps` 메서드는 trait 기본 구현(설계 3.0 규칙 4)에 맡긴다.
struct ListDirFs(Result<Vec<PathBuf>, String>);

impl FsOps for ListDirFs {
    fn read_dir(&self, _p: &Path) -> std::io::Result<Vec<PathBuf>> {
        self.0.clone().map_err(std::io::Error::other)
    }
}

/// `asyncio.to_thread` spy. 블로킹 풀로 넘어간 함수 이름을 기록한다.
struct SpyRunner(Arc<Mutex<Vec<String>>>);

impl TaskRunner for SpyRunner {
    fn spawn_blocking_observed(&self, name: &str) {
        self.0.lock().unwrap().push(name.to_string());
    }
}

struct JoinedHandle;
impl TaskHandle for JoinedHandle {}

/// 백그라운드 동작은 실제 작업 스레드에서 실행하고 응답 전에 끝낸다.
struct JoinedRunner;
impl TaskRunner for JoinedRunner {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        let _ = std::thread::spawn(job).join();
        Box::new(JoinedHandle)
    }
}

/// 작업을 큐에 보관해 HTTP 응답 뒤까지 실행을 미룬다.
type DeferredJobs = Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>;
struct DeferredRunner(DeferredJobs);
impl TaskRunner for DeferredRunner {
    fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        self.0.lock().unwrap().push(job);
        Box::new(JoinedHandle)
    }
}

fn ready(v: Result<Value, String>) -> Fut<'static, Result<Value, String>> {
    Box::pin(async move { v })
}

/// 위치 인자 `idx` 또는 키워드 인자 `name`에서 값을 꺼낸다.
fn arg(c: &CallArgs, idx: usize, name: &str) -> Value {
    c.args.get(idx).cloned().or_else(|| c.kwargs.get(name).cloned()).unwrap_or(Value::Null)
}

fn arg_or(c: &CallArgs, idx: usize, name: &str, default: Value) -> Value {
    c.args.get(idx).cloned().or_else(|| c.kwargs.get(name).cloned()).unwrap_or(default)
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

fn store_set(store: &Store, status: &Value, content_type: &str) {
    let mut m: Map<String, Value> = status.as_object().cloned().unwrap_or_default();
    m.insert("content_type".into(), json!(content_type));
    store.lock().unwrap().insert(content_type.to_string(), Value::Object(m));
}

/// `install_classify_status_store`에 대응한다. 단순히 반환값을 박지 않고 전이 규칙을 흉내 낸다.
/// 항목에 `_age_seconds`를 넣으면 '갱신이 그만큼 끊긴 작업'이 된다.
fn install_classify_status_store(mock_cat: &Mock, initial: Option<Value>) -> Store {
    let store: Store = Arc::new(Mutex::new(HashMap::new()));
    {
        let mut s = store.lock().unwrap();
        s.insert("book".into(), json!({"status": "idle", "content_type": "book"}));
        s.insert("comic".into(), json!({"status": "idle", "content_type": "comic"}));
        if let Some(init) = initial {
            let mut m = init.as_object().cloned().unwrap_or_default();
            let ct = m.get("content_type").cloned().unwrap_or(json!("book"));
            m.insert("content_type".into(), ct);
            s.insert("book".into(), Value::Object(m));
        }
    }

    let ct_of = |c: &CallArgs, idx: usize| arg_or(c, idx, "content_type", json!("book")).as_str().unwrap().to_string();
    let stale_of = |c: &CallArgs, idx: usize| arg_or(c, idx, "stale_seconds", json!(300)).as_f64().unwrap();

    let st = store.clone();
    mock_cat.side_effect("get_classify_proposal_status", move |c| ready(Ok(effective(&st, &ct_of(&c, 0), stale_of(&c, 1)))));

    let st = store.clone();
    mock_cat.side_effect("set_classify_proposal_status", move |c| {
        store_set(&st, &arg(&c, 0, "status"), &ct_of(&c, 1));
        ready(Ok(Value::Null))
    });

    let st = store.clone();
    mock_cat.side_effect("merge_classify_proposal_status", move |c| {
        let ct = ct_of(&c, 1);
        let mut s = st.lock().unwrap();
        let entry = s.entry(ct.clone()).or_insert_with(|| json!({"status": "idle", "content_type": ct}));
        for (k, v) in arg(&c, 0, "fields").as_object().cloned().unwrap_or_default() {
            if k != "status" {
                entry[k] = v;
            }
        }
        ready(Ok(Value::Null))
    });

    let st = store.clone();
    mock_cat.side_effect("try_start_classify_proposal", move |c| {
        let (ct, stale) = (ct_of(&c, 1), stale_of(&c, 2));
        let current = effective(&st, &ct, stale);
        if matches!(current["status"].as_str(), Some("running" | "applying")) {
            return ready(Ok(json!([false, current])));
        }
        store_set(&st, &arg(&c, 0, "status"), &ct);
        ready(Ok(json!([true, effective(&st, &ct, stale)])))
    });

    let st = store.clone();
    mock_cat.side_effect("try_begin_classify_apply", move |c| {
        let (ct, stale) = (ct_of(&c, 1), stale_of(&c, 2));
        let current = effective(&st, &ct, stale);
        if !matches!(current["status"].as_str(), Some("ready" | "done" | "failed")) {
            return ready(Ok(json!([false, current])));
        }
        let mut next = current.clone();
        next["status"] = json!("applying");
        next["apply_token"] = arg(&c, 0, "apply_token");
        store_set(&st, &next, &ct);
        ready(Ok(json!([true, effective(&st, &ct, stale)])))
    });

    store
}

fn status_of(store: &Store, content_type: &str) -> Value {
    store.lock().unwrap().get(content_type).cloned().unwrap_or(Value::Null)
}

// ── 환경 변수 가드 ───────────────────────────────────────────────────────────

/// 환경 변수를 바꾸는 테스트는 `common::env::EnvGuard`의 쓰기 잠금을, 일반 테스트는 읽기 잠금(`EnvGuard::read`)을 쥔다.
#[allow(dead_code)]
enum Guard {
    Read(EnvReadGuard),
    Env(EnvGuard),
}

// ── fixture: client / mock_bm / mock_cm / mock_cat / override_auth ───────────

/// 원본의 `client`, `mock_bm`, `mock_cm`, `mock_cat`, `override_auth` fixture를 한데 묶는다.
/// 인증 우회는 기본이 관리자이고, 테스트가 `set_auth`로 viewer로 바꾼다.
/// 원본에서 이 fixture들을 요청하지 않은 테스트도 같은 가짜 매니저를 쓴다. 그 테스트들은 매니저를 호출하지 않는다.
struct Fx {
    app: axum::Router,
    slots: Arc<AuthSlots>,
    bm: Mock,
    cm: Mock,
    cat: Mock,
    store: Store,
    _guard: Guard,
}

impl Fx {
    fn new() -> Self {
        common::setup_env();
        Self::build(Guard::Read(EnvGuard::read()), |deps| deps)
    }

    /// `monkeypatch.setenv`를 쓰는 테스트용.
    fn with_env(vars: &[(&str, Option<&str>)]) -> Self {
        Self::build(Guard::Env(EnvGuard::apply(vars)), |deps| deps)
    }

    /// `monkeypatch.setenv`와 `mock.patch`를 함께 쓰는 테스트용.
    fn with_env_tweaked(vars: &[(&str, Option<&str>)], tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        Self::build(Guard::Env(EnvGuard::apply(vars)), tweak)
    }

    /// `mock.patch`로 내부 함수를 바꾸던 테스트용. `AppDeps`의 `with_*` 빌더를 `tweak`에서 이어 붙인다.
    fn tweaked(tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        common::setup_env();
        Self::build(Guard::Read(EnvGuard::read()), tweak)
    }

    /// `AppDeps`를 만드는 유일한 곳이다. 인증은 `AuthMode::Shared`라 테스트가 중간에 바꿀 수 있다.
    fn build(guard: Guard, tweak: impl FnOnce(AppDeps) -> AppDeps) -> Self {
        let (bm, cm, cat) = (Mock::new(), Mock::new(), Mock::new());
        let store = install_classify_status_store(&cat, None);
        let slots = Arc::new(AuthSlots { auth: Mutex::new(admin_payload()), admin: Mutex::new(admin_payload()) });
        let deps = AppDeps::new(AuthMode::Shared(slots.clone()))
            .with_book_manager(Arc::new(bm.clone()))
            .with_comics_manager(Arc::new(cm.clone()))
            .with_category_mapping(Arc::new(cat.clone()))
            .with_refresh_token_store(Arc::new(Mock::new()))
            .with_task_runner(Arc::new(JoinedRunner));
        Self { app: backend::main::app_with(tweak(deps)), slots, bm, cm, cat, store, _guard: guard }
    }

    /// `dependency_overrides[require_auth] = lambda: payload`에 대응한다.
    fn set_auth(&self, payload: Value) {
        *self.slots.auth.lock().unwrap() = payload;
    }

    fn client(&self) -> TestClient {
        TestClient::with_app(self.app.clone()).header("host", "testserver")
    }

    async fn get(&self, path: &str) -> Response {
        self.client().get(path).await
    }

    async fn delete(&self, path: &str) -> Response {
        self.client().delete(path).await
    }

    async fn post_json(&self, path: &str, body: &Value) -> Response {
        self.client().post_json(path, body).await
    }

    async fn put_json(&self, path: &str, body: &Value) -> Response {
        self.client().put_json(path, body).await
    }

    /// 본문 없는 POST.
    async fn post_empty(&self, path: &str) -> Response {
        self.client().request(Method::POST, path, None, Vec::new()).await
    }

    async fn post_with_cookie(&self, path: &str, name: &str, value: &str) -> Response {
        self.client().cookie(name, value).request(Method::POST, path, None, Vec::new()).await
    }
}

// ── /wake ────────────────────────────────────────────────────────────────────

mod test_wake {
    use super::*;

    #[tokio::test]
    async fn test_success() {
        // 열거 가능한 저장소 흉내. `with_fs`가 `os.listdir` 패치를 대신한다.
        let fx = Fx::with_env_tweaked(&[("TM_FRONTEND_URL", Some("http://testserver"))], |d| d.with_fs(Arc::new(ListDirFs(Ok(vec!["a".into(), "b".into(), "c".into()])))));
        let r = fx.get("/wake").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success"}));
    }

    #[tokio::test]
    async fn test_failure() {
        let fx = Fx::with_env_tweaked(&[("TM_FRONTEND_URL", Some("http://testserver"))], |d| d.with_fs(Arc::new(ListDirFs(Err("not mounted".into())))));
        let r = fx.get("/wake").await;
        assert_eq!(r.status, 503);
        let data = r.json();
        assert_eq!(data, json!({"status": "failure"}));
    }

    #[tokio::test]
    async fn test_non_frontend_host_is_hidden() {
        let fx = Fx::with_env(&[("TM_FRONTEND_URL", Some("https://tm.terzeron.com"))]);
        let r = fx.get("/wake").await;
        assert_eq!(r.status, 404);
    }
}

// ── /validate/{book_id} ──────────────────────────────────────────────────────

mod test_validate_book {
    use super::*;

    #[tokio::test]
    async fn test_epub_success() {
        let fx = Fx::new();
        fx.bm.returns("get_book", json!([make_book(json!({"file_type": "epub"})), null]));
        fx.bm.returns("validate_epub", json!([{"valid": true}, null]));

        let r = fx.get("/validate/1").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": {"valid": true}}));
    }

    #[tokio::test]
    async fn test_pdf_success() {
        let fx = Fx::new();
        fx.bm.returns("get_book", json!([make_book(json!({"file_type": "pdf"})), null]));
        fx.bm.returns("validate_pdf", json!([{"pages": 5}, null]));

        let r = fx.get("/validate/1").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
        assert_eq!(r.json()["result"], json!({"pages": 5}));
    }

    #[tokio::test]
    async fn test_unsupported_type() {
        let fx = Fx::new();
        fx.bm.returns("get_book", json!([make_book(json!({"file_type": "txt"})), null]));

        let r = fx.get("/validate/1").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains("txt"));
    }

    #[tokio::test]
    async fn test_book_not_found() {
        let fx = Fx::new();
        fx.bm.returns("get_book", json!([null, "Book not found: 99"]));

        let r = fx.get("/validate/99").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains("99"));
    }

    #[tokio::test]
    async fn test_epub_validation_error() {
        let fx = Fx::new();
        fx.bm.returns("get_book", json!([make_book(json!({"file_type": "epub"})), null]));
        fx.bm.returns("validate_epub", json!([null, "invalid epub structure"]));

        let r = fx.get("/validate/1").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
        assert!(r.json()["error"].as_str().unwrap().contains("invalid epub structure"));
    }
}

// ── /preview/{book_id} ───────────────────────────────────────────────────────

mod test_preview {
    use super::*;

    #[tokio::test]
    async fn test_returns_response() {
        let fx = Fx::new();
        fx.bm.returns("get_book_preview", raw_response("<html/>", Some("text/html")));

        let r = fx.get("/preview/1").await;
        assert_eq!(r.status, 200);
    }

    #[tokio::test]
    async fn test_default_params() {
        let fx = Fx::new();
        fx.bm.returns("get_book_preview", raw_response("preview", None));
        fx.get("/preview/1").await;
        fx.bm.assert_called_once_with("get_book_preview", json!([]), json!({"book_id": 1, "pages": 5, "chapters": 10, "resource_base_url": "/html-resource/1"}));
    }

    #[tokio::test]
    async fn test_custom_params() {
        let fx = Fx::new();
        fx.bm.returns("get_book_preview", raw_response("preview", None));
        fx.get("/preview/1?pages=10&chapters=5").await;
        fx.bm.assert_called_once_with("get_book_preview", json!([]), json!({"book_id": 1, "pages": 10, "chapters": 5, "resource_base_url": "/html-resource/1"}));
    }

    #[tokio::test]
    async fn test_html_resource_proxy() {
        let fx = Fx::new();
        fx.bm.returns("get_html_resource", raw_response("body{}", Some("text/css")));
        let r = fx.get("/html-resource/1?path=style.css").await;
        assert_eq!(r.status, 200);
        fx.bm.assert_called_once_with("get_html_resource", json!([]), json!({"book_id": 1, "resource_path": "style.css"}));
    }
}

mod test_viewer_hidden_access {
    use super::*;

    #[tokio::test]
    async fn test_viewer_hidden_book_download_forbidden() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret"]));
        fx.bm.returns("get_book", json!([make_book(json!({"category": "secret"})), null]));

        let r = fx.get("/download/1").await;

        assert_eq!(r.status, 403);
        assert_eq!(r.json()["detail"], "접근 권한이 없는 카테고리입니다.");
        fx.bm.assert_not_called("get_book_content");
    }

    #[tokio::test]
    async fn test_viewer_download_fails_closed_when_book_lookup_fails() {
        // 조회 오류로 카테고리를 모르면 숨김 검사를 건너뛰지 않고 거절한다.
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret"]));
        fx.bm.returns("get_book", json!([null, "ES 연결 실패"]));

        let r = fx.get("/download/1").await;

        assert_eq!(r.status, 503, "조회 실패는 없음(404)과 구분한다");
        fx.bm.assert_not_called("get_book_content");
    }

    #[tokio::test]
    async fn test_viewer_download_of_a_missing_book_is_not_found() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret"]));
        fx.bm.returns("get_book", json!([null, null]));

        let r = fx.get("/download/1").await;

        assert_eq!(r.status, 404);
        fx.bm.assert_not_called("get_book_content");
    }

    #[tokio::test]
    async fn test_download_maps_book_conversion_error_to_internal_error() {
        let fx = Fx::new();
        fx.bm.returns("get_book_content", json!({"__error__": "malformed ES book"}));

        let response = fx.get("/download/1").await;

        assert_eq!(response.status, 500);
    }

    #[tokio::test]
    async fn test_viewer_hidden_category_listing_forbidden() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret"]));

        let r = fx.get("/categories/secret/sub").await;

        assert_eq!(r.status, 403);
        assert_eq!(r.json()["detail"], "접근 권한이 없는 카테고리입니다.");
        fx.bm.assert_not_called("get_books_in_category");
    }

    #[tokio::test]
    async fn test_viewer_categories_filter_hidden_entries() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret"]));
        fx.bm.returns("get_categories", json!([{"public": 1, "secret": 2, "secret/sub": 3}, null]));

        let r = fx.get("/categories").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": {"public": 1}}));
    }

    #[tokio::test]
    async fn test_viewer_search_merges_hidden_categories() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret", "secret/sub"]));
        fx.bm.returns("search_by_keyword_paged", json!([[], 0, null]));

        let r = fx.get("/search/test?exclude_categories=public,secret").await;

        assert_eq!(r.status, 200);
        fx.bm.assert_called_once_with("search_by_keyword_paged", json!(["test"]), json!({"size": 10, "offset": 0, "exclude_categories": ["public", "secret", "secret/sub"]}));
    }

    #[tokio::test]
    async fn test_search_forwards_category_and_pagination() {
        let fx = Fx::new();
        fx.bm.returns("search_by_keyword_paged", json!([[], 0, null]));

        let response = fx.get(&format!("/search/test?category={}&offset=10&limit=20", enc("과학"))).await;

        assert_eq!(response.status, 200);
        fx.bm.assert_called_once_with("search_by_keyword_paged", json!(["test"]), json!({"size": 20, "offset": 10, "exclude_categories": null, "category": "과학"}));
    }

    #[tokio::test]
    async fn test_latest_uses_latest_excluded_categories() {
        let fx = Fx::new();
        fx.cat.returns("get_latest_excluded_categories", json!(["no_latest"]));
        fx.bm.returns("get_latest_books", json!([[], 0, null]));

        let r = fx.get("/latest?limit=5").await;

        assert_eq!(r.status, 200);
        fx.bm.assert_called_once_with("get_latest_books", json!([]), json!({"size": 5, "exclude_categories": ["no_latest"]}));
    }

    #[tokio::test]
    async fn test_viewer_latest_merges_hidden_and_latest_excluded_categories() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret", "shared"]));
        fx.cat.returns("get_latest_excluded_categories", json!(["no_latest", "shared"]));
        fx.bm.returns("get_latest_books", json!([[], 0, null]));

        let r = fx.get("/latest").await;

        assert_eq!(r.status, 200);
        fx.bm.assert_called_once_with("get_latest_books", json!([]), json!({"size": 2000, "exclude_categories": ["secret", "shared", "no_latest"]}));
    }

    #[tokio::test]
    async fn test_browse_returns_grouped_page_with_display_title_and_cursor() {
        let fx = Fx::new();
        fx.cat.returns("get_hidden_categories", json!(["secret"]));
        let book = json!({"book_id": 7, "category": "5_완결/암살교실", "title": "암살교실 01권", "author": "", "file_path": "5_완결/암살교실/암살교실 01권.pdf", "file_type": "pdf", "file_size": 1, "updated_time": "2024-01-01"});
        fx.bm.returns("get_cover_books_paged", json!([[[book, "암살교실"]], 2295, "next-cur", null]));

        let r = fx.get(&format!("/browse?limit=60&cursor=abc&category={}", enc("5_완결"))).await;

        assert_eq!(r.status, 200);
        let body = r.json();
        assert_eq!(body["status"], "success");
        assert_eq!(body["total"], 2295);
        assert_eq!(body["next_cursor"], "next-cur");
        assert_eq!(body["result"][0]["book_id"], 7);
        assert_eq!(body["result"][0]["display_title"], "암살교실");
        // 관리자는 비노출 카테고리를 걸러내지 않는다.
        fx.bm.assert_called_once_with("get_cover_books_paged", json!([]), json!({"category": "5_완결", "size": 60, "cursor": "abc", "exclude_categories": []}));
    }

    #[tokio::test]
    async fn test_browse_defaults_to_all_and_empty_display_title_for_plain_files() {
        let fx = Fx::new();
        let book = json!({"book_id": 1, "category": "A", "title": "가", "author": "", "file_path": "A/가.epub", "file_type": "epub", "file_size": 1, "updated_time": "2024-01-01"});
        fx.bm.returns("get_cover_books_paged", json!([[[book, ""]], 1, null, null]));

        let r = fx.get("/browse").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"][0]["display_title"], "");
        assert!(r.json()["next_cursor"].is_null());
        fx.bm.assert_called_once_with("get_cover_books_paged", json!([]), json!({"category": "", "size": 60, "cursor": "", "exclude_categories": []}));
    }

    #[tokio::test]
    async fn test_browse_serializes_absolute_file_path_like_python_book_dict() {
        let fx = Fx::new();
        let absolute_path = Book::path_prefix().join("A/book.pdf");
        let book = make_book(json!({"file_path": absolute_path.to_string_lossy()}));
        fx.bm.returns("get_cover_books_paged", json!([[[book, "book"]], 1, null, null]));

        let r = fx.get("/browse").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"][0]["file_path"], "A/book.pdf");
    }

    #[tokio::test]
    async fn test_browse_uncovered_uses_uncovered_query_and_reports_unknown_total() {
        let fx = Fx::new();
        let book = json!({"book_id": 3, "category": "A", "title": "나", "author": "", "file_path": "A/나.pdf", "file_type": "pdf", "file_size": 1, "updated_time": "2024-01-01"});
        fx.bm.returns("get_uncovered_books_paged", json!([[[book, ""]], null, "next-cur", null]));

        let r = fx.get("/browse?uncovered=true&limit=30").await;

        assert_eq!(r.status, 200);
        let body = r.json();
        assert_eq!(body["status"], "success");
        assert!(body["total"].is_null());
        assert_eq!(body["next_cursor"], "next-cur");
        assert_eq!(body["result"][0]["book_id"], 3);
        fx.bm.assert_called_once_with("get_uncovered_books_paged", json!([]), json!({"category": "", "size": 30, "cursor": "", "exclude_categories": []}));
        fx.bm.assert_not_called("get_cover_books_paged");
    }

    #[tokio::test]
    async fn test_viewer_browse_excludes_hidden_categories() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret", "shared"]));
        fx.bm.returns("get_cover_books_paged", json!([[], 0, null, null]));

        let r = fx.get("/browse").await;

        assert_eq!(r.status, 200);
        fx.bm.assert_called_once_with("get_cover_books_paged", json!([]), json!({"category": "", "size": 60, "cursor": "", "exclude_categories": ["secret", "shared"]}));
    }

    #[tokio::test]
    async fn test_viewer_cannot_browse_hidden_category() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["secret"]));

        let r = fx.get("/browse?category=secret/sub").await;

        assert_eq!(r.status, 403);
        fx.bm.assert_not_called("get_cover_books_paged");
    }

    #[tokio::test]
    async fn test_browse_reports_error() {
        let fx = Fx::new();
        fx.bm.returns("get_cover_books_paged", json!([[], 0, null, "invalid cursor"]));

        let r = fx.get("/browse?cursor=bad").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "failure", "error": "invalid cursor"}));
    }

    #[tokio::test]
    async fn test_browse_rejects_out_of_range_limit() {
        let fx = Fx::new();
        assert_eq!(fx.get("/browse?limit=0").await.status, 422);
        assert_eq!(fx.get("/browse?limit=999999").await.status, 422);
    }
}

// ── /pdf-pages/{book_id} ─────────────────────────────────────────────────────

mod test_pdf_pages {
    use super::*;

    #[tokio::test]
    async fn test_returns_response() {
        let fx = Fx::new();
        fx.bm.returns("get_pdf_pages", raw_response("%PDF", Some("application/pdf")));

        let r = fx.get("/pdf-pages/1?start=2&end=4").await;
        assert_eq!(r.status, 200);
    }

    #[tokio::test]
    async fn test_default_params() {
        let fx = Fx::new();
        fx.bm.returns("get_pdf_pages", raw_response("%PDF", None));
        fx.get("/pdf-pages/1").await;
        fx.bm.assert_called_once_with("get_pdf_pages", json!([]), json!({"book_id": 1, "start": 1, "end": 1}));
    }
}

// ── /category-mismatches admin endpoints ─────────────────────────────────────

/// 원본의 `fake_propose(*args, on_progress=None, **kwargs)`에 대응한다.
/// `on_progress`로 이벤트를 차례로 보내고(돌려받은 awaitable은 await), 이벤트마다 `after_event(번호)`를 부른 뒤 최종 결과를 돌려준다.
fn fake_propose(
    events: Vec<Value>,
    final_result: Value,
    after_event: Option<Arc<dyn Fn(usize) + Send + Sync>>,
) -> impl for<'a> Fn(CallArgs<'a>) -> Fut<'a, Result<Value, String>> + Send + Sync + 'static {
    move |c: CallArgs| {
        let (events, final_result, after_event) = (events.clone(), final_result.clone(), after_event.clone());
        Box::pin(async move {
            let cb = c.callbacks.get("on_progress").cloned();
            for (i, event) in events.into_iter().enumerate() {
                if let Some(cb) = &cb {
                    cb(event).await;
                }
                if let Some(f) = &after_event {
                    f(i + 1);
                }
            }
            Ok(json!([final_result, null]))
        })
    }
}

fn proposal_result(total: usize, items: &[Value]) -> Value {
    json!({"content_type": "book", "source_category": "0_inbox", "total_count": total, "processed_count": total, "items": items, "failures": []})
}

fn epub_items(n: usize, extra: Value) -> Vec<Value> {
    (0..n)
        .map(|i| {
            let mut item = json!({"file_path": format!("0_inbox/{i}.epub")});
            for (k, v) in extra.as_object().cloned().unwrap_or_default() {
                item[k] = v;
            }
            item
        })
        .collect()
}

fn batch_sizes(calls: &[CallArgs]) -> Vec<usize> {
    calls.iter().map(|c| c.args[0].as_array().unwrap().len()).collect()
}

/// `mock_bm.apply_category_changes = fake_apply`처럼 호출 인자를 붙잡고 고정 결과를 돌려주는 가짜.
fn capture_apply(captured: Arc<Mutex<Option<(Value, Value)>>>) -> impl for<'a> Fn(CallArgs<'a>) -> Fut<'a, Result<Value, String>> + Send + Sync + 'static {
    move |c: CallArgs| {
        let (items, allowed) = (arg(&c, 0, "items"), arg(&c, 1, "allowed_file_paths"));
        let n = items.as_array().map(|a| a.len()).unwrap_or(0);
        *captured.lock().unwrap() = Some((items, allowed));
        ready(Ok(json!([{"total_count": n, "applied_count": n, "failed_count": 0, "results": []}, null])))
    }
}

mod test_category_mismatch_admin {
    use super::*;

    #[tokio::test]
    async fn test_classify_proposal_comics_prefix_uses_comic_mapping() {
        // /comics 접두사로 마운트된 라우터도 같은 제안 엔드포인트를 쓰는지 본다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        let mappings = json!({"2_science": ["SF"]});
        let result = json!({"source_category": "0_inbox", "total_count": 1, "processed_count": 1, "items": [], "failures": []});
        fx.cm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", mappings);
        fx.cm.returns("propose_category_changes", json!([result, null]));

        let r = fx.post_json("/comics/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"]["started"], true);
        fx.cat.assert_called_once_with("get_all_mappings", json!([]), json!({"content_type": "comic"}));
        assert_eq!(fx.cm.call_count("propose_category_changes"), 1);
        let kwargs = fx.cm.calls("propose_category_changes").last().unwrap().kwargs.clone();
        assert_eq!(kwargs["content_type"], "comic");
    }

    #[tokio::test]
    async fn test_classify_proposal_get_reads_shared_status_file() {
        // 폴링용 GET이 상태 파일 내용을 그대로 돌려준다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_classify_proposal_items", json!([]));
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "source_category": "shared_file", "total_count": 9, "processed_count": 4})));

        let status = fx.get("/categories/classify-proposal").await;

        assert_eq!(status.status, 200);
        assert_eq!(status.json()["result"]["status"], "running");
        assert_eq!(status.json()["result"]["source_category"], "shared_file");
        assert_eq!(status.json()["result"]["processed_count"], 4);
    }

    #[tokio::test]
    async fn test_classify_proposal_passes_delay_option() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        // 상태 파일이 없으면 프로세스 내 메모리 폴백을 쓰는데, 이 폴백은 라우터가 앱과 함께
        // 한 번만 만들어져 다른 테스트가 남긴 running 상태를 물려받을 수 있다.
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.returns("get_all_mappings", json!({}));
        fx.bm.returns("propose_category_changes", json!([{"source_category": "0_inbox", "total_count": 0, "processed_count": 0, "items": [], "failures": []}, null]));

        let r = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox", "use_bookstore": false, "use_content_meta": true, "delay": 2.0})).await;

        assert_eq!(r.status, 200);
        let kwargs = fx.bm.calls("propose_category_changes").last().unwrap().kwargs.clone();
        assert_eq!(kwargs["use_bookstore"], false);
        assert_eq!(kwargs["use_content_meta"], true);
        assert_eq!(kwargs["delay"], 2.0);
    }

    #[tokio::test]
    async fn test_classify_proposal_batches_new_and_updated_items_in_tens() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));

        let placeholders = epub_items(25, json!({"target_category": null, "grade": null}));
        let classified = epub_items(25, json!({"target_category": "3_SF", "grade": "certain"}));
        let mut events: Vec<Value> = placeholders.iter().map(|item| json!({"total_count": 25, "processed_count": 0, "new_items": [item]})).collect();
        events.extend(classified.iter().enumerate().map(|(i, item)| json!({"total_count": 25, "processed_count": i + 1, "updated_items": [item]})));
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(25, &classified), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        let added = fx.cat.calls("add_classify_proposal_items");
        let updated = fx.cat.calls("update_classify_proposal_item_payloads");
        let sizes = |calls: &[RecordedArgs]| calls.iter().map(|call| call.args[0].as_array().unwrap().len()).collect::<Vec<_>>();
        assert_eq!(sizes(&added), [10, 10, 5]);
        assert_eq!(sizes(&updated), [10, 10, 5]);
    }

    #[tokio::test]
    async fn test_classify_proposal_database_flush_runs_off_the_request_thread() {
        // Python은 callback DB write에 asyncio.to_thread를 쓴다. Rust는 전체 background job을
        // TaskRunner worker thread에서 실행하므로 동기 DB flush도 요청 runtime을 막지 않는다.
        let request_thread = std::thread::current().id();
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        let items = epub_items(10, json!({"target_category": "3_SF"}));
        let events = items.iter().enumerate().map(|(i, item)| json!({"total_count": 10, "processed_count": i + 1, "new_items": [item]})).collect();
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(10, &items), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        let writes = fx.cat.calls("add_classify_proposal_items");
        assert_eq!(writes.len(), 1);
        assert_ne!(writes[0].thread_id, request_thread, "DB flush must run on the background task thread");
    }

    #[tokio::test]
    async fn test_classify_proposal_keeps_large_progress_event_as_one_batch() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        let placeholders = epub_items(25, json!({"target_category": null, "grade": null}));
        fx.bm.side_effect("propose_category_changes", fake_propose(vec![json!({"total_count": 25, "processed_count": 0, "new_items": placeholders})], proposal_result(25, &[]), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        let batches = fx.cat.calls("add_classify_proposal_items");
        assert_eq!(batches.iter().map(|call| call.args[0].as_array().unwrap().len()).collect::<Vec<_>>(), [25]);
    }

    #[tokio::test]
    async fn test_classify_proposal_flushes_before_processing_next_batch() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        let items = epub_items(25, json!({"target_category": "3_SF"}));
        let observed_flush_count = Arc::new(Mutex::new(None));
        let (category_mapping, observed) = (fx.cat.clone(), observed_flush_count.clone());
        let after_event: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |processed| {
            if processed == 10 {
                *observed.lock().unwrap() = Some(category_mapping.call_count("add_classify_proposal_items"));
            }
        });
        let events = items.iter().enumerate().map(|(i, item)| json!({"total_count": 25, "processed_count": i + 1, "new_items": [item]})).collect();
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(25, &items), Some(after_event)));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(*observed_flush_count.lock().unwrap(), Some(1));
        let batches = fx.cat.calls("add_classify_proposal_items");
        assert_eq!(batches.iter().map(|call| call.args[0].as_array().unwrap().len()).collect::<Vec<_>>(), [10, 10, 5]);
    }

    #[tokio::test]
    async fn test_classify_proposal_flushes_pending_items_when_manager_returns_error() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        let pending = epub_items(3, json!({"target_category": null}));
        let event = json!({"total_count": 3, "processed_count": 1, "new_items": pending});
        fx.bm.side_effect("propose_category_changes", move |c: CallArgs<'_>| {
            let event = event.clone();
            Box::pin(async move {
                if let Some(callback) = c.callbacks.get("on_progress") {
                    callback(event).await;
                }
                Ok(json!([null, "분류 제안에 실패했습니다."]))
            })
        });

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
        let inserted: Vec<Value> = fx.cat.calls("add_classify_proposal_items").iter().flat_map(|call| call.args[0].as_array().unwrap().clone()).collect();
        assert_eq!(inserted, pending);
    }

    #[tokio::test]
    async fn test_classify_proposal_marks_job_failed_when_manager_panics() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.returns("get_all_mappings", json!({}));
        fx.bm.raises("propose_category_changes", "manager crashed");

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_marks_job_failed_when_item_flush_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.raises("add_classify_proposal_items", "database unavailable");
        let pending = epub_items(3, json!({"target_category": "3_SF"}));
        let events = vec![json!({"total_count": 3, "processed_count": 3, "new_items": pending})];
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(3, &[]), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_marks_job_failed_when_payload_flush_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.raises("update_classify_proposal_item_payloads", "database unavailable");
        let items = epub_items(3, json!({"target_category": "3_SF"}));
        let events = vec![json!({"total_count": 3, "processed_count": 3, "updated_items": items})];
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(3, &[]), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_marks_job_failed_when_progress_status_write_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.raises("merge_classify_proposal_status", "database unavailable");
        let events = vec![json!({"total_count": 3, "processed_count": 1})];
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(3, &[]), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_marks_job_failed_when_final_status_read_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.returns("get_all_mappings", json!({}));
        let reads = Arc::new(Mutex::new(0usize));
        let read_count = reads.clone();
        let state_store = store.clone();
        fx.cat.side_effect("get_classify_proposal_status", move |c| {
            let mut count = read_count.lock().unwrap();
            *count += 1;
            if *count == 1 {
                return ready(Err("database unavailable".to_string()));
            }
            let content_type = arg(&c, 0, "content_type").as_str().unwrap_or("book").to_string();
            let stale_seconds = arg_or(&c, 1, "stale_seconds", json!(300)).as_f64().unwrap_or(300.0);
            ready(Ok(effective(&state_store, &content_type, stale_seconds)))
        });
        fx.bm.side_effect("propose_category_changes", fake_propose(vec![], proposal_result(0, &[]), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_marks_job_failed_when_ready_status_write_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));
        fx.cat.returns("get_all_mappings", json!({}));
        let fail_ready_write = Arc::new(Mutex::new(true));
        let fail_once = fail_ready_write.clone();
        let state_store = store.clone();
        fx.cat.side_effect("set_classify_proposal_status", move |c| {
            let next = arg(&c, 0, "status");
            let content_type = arg(&c, 1, "content_type").as_str().unwrap_or("book").to_string();
            let mut should_fail = fail_once.lock().unwrap();
            if next["status"] == "ready" && *should_fail {
                *should_fail = false;
                return ready(Err("database unavailable".to_string()));
            }
            store_set(&state_store, &next, &content_type);
            ready(Ok(Value::Null))
        });
        fx.bm.side_effect("propose_category_changes", fake_propose(vec![], proposal_result(0, &[]), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_success_polling() {
        // 제안 시작은 202류 응답 대신 started 플래그를 주고, GET으로 완료 상태를 본다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        let mappings = json!({"3_SF": ["과학소설"]});
        let result = json!({"content_type": "book", "source_category": "0_inbox", "total_count": 2, "processed_count": 2, "items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}], "failures": []});
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", mappings);
        // 파일을 미리 idle로 채워 이 테스트가 다른 테스트의 잔여 상태에 기대지 않게 한다.
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));

        // 실제 book_manager처럼 on_progress가 돌려준 awaitable을 await한다.
        fx.bm.side_effect("propose_category_changes", fake_propose(vec![json!({"total_count": 2, "processed_count": 1})], result.clone(), None));
        // 이 테스트는 배관(propose -> ready -> GET)만 보므로, DB 조회 결과를 고정해 GET이 그 값을 그대로 합쳐 돌려주는지 본다.
        fx.cat.returns("get_classify_proposal_items", result["items"].clone());

        let r = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"]["started"], true);
        let status = fx.get("/categories/classify-proposal").await;
        assert_eq!(status.status, 200);
        assert_eq!(status.json()["result"]["status"], "ready");
        let expected_items = json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "book_id": null}]);
        assert_eq!(status.json()["result"]["items"], expected_items);
        assert_eq!(status_of(&store, "book")["status"], "ready");
        assert_eq!(fx.bm.call_count("propose_category_changes"), 1);
        let last = fx.bm.calls("propose_category_changes").last().unwrap().clone();
        assert_eq!(last.kwargs["content_type"], "book");
        assert_eq!(last.kwargs["use_bookstore"], true);
        assert_eq!(last.kwargs["use_content_meta"], true);
        assert!(last.callback_names.contains("on_progress"));
    }

    #[tokio::test]
    async fn test_classify_proposal_progress_tick_exposes_items_so_far() {
        // 제안이 도는 도중 GET은 status: running과 함께 지금까지 DB에 쌓인 항목을 돌려준다.
        // 상태 파일 직접 읽기가 아니라 GET 응답이 상태의 status/카운트와 DB의 항목을 합쳐 돌려주는지로 검증한다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "source_category": "0_inbox", "total_count": 2, "processed_count": 1})));
        let db_items_so_far = json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]);
        fx.cat.returns("get_classify_proposal_items", db_items_so_far.clone());

        let status = fx.get("/categories/classify-proposal").await;

        assert_eq!(status.status, 200);
        assert_eq!(status.json()["result"]["status"], "running");
        assert_eq!(status.json()["result"]["items"], json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "book_id": null}]));
        fx.cat.assert_called_once_with("get_classify_proposal_items", json!([]), json!({"content_type": "book"}));
    }

    #[ignore = "클로저 introspection: endpoint.__closure__의 _run_classify_proposal_job 직접 접근; DI 설계 후 활성화"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_job_flushes_new_items_in_batches_of_10_and_flushes_tail() {
        // 255권을 처리하면 add_classify_proposal_items가 10건씩 25번 + 5건 1번, 총 26번 호출돼야 한다.
        // 마지막 5건(꼬리)이 누락되면 관리자가 목록 끝의 책들을 못 보고 승인하게 된다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "total_count": 0, "processed_count": 0})));

        let all_items = epub_items(255, json!({"target_category": "3_SF"}));
        let events: Vec<Value> = all_items.iter().enumerate().map(|(i, item)| json!({"total_count": 255, "processed_count": i + 1, "new_items": [item]})).collect();
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(255, &all_items), None));
        let router = fx.item_router();

        router.run_classify_proposal_job("0_inbox", true, true, 1.2).await;

        fx.cat.assert_called_once_with("clear_classify_proposal_items", json!([]), json!({"content_type": "book"}));
        let add_calls = fx.cat.calls("add_classify_proposal_items");
        let sizes = batch_sizes(&add_calls);
        let mut expected = vec![10; 25];
        expected.push(5);
        assert_eq!(sizes, expected);
        assert_eq!(sizes.iter().sum::<usize>(), 255);
        let flattened: Vec<Value> = add_calls.iter().flat_map(|c| c.args[0].as_array().unwrap().clone()).collect();
        assert_eq!(flattened, all_items);
    }

    #[tokio::test]
    async fn test_classify_proposal_job_stores_names_first_then_fills_them_in() {
        // 이름이 먼저 깔려야 관리자가 작업 도중에 들어와도 무엇이 대상인지 볼 수 있다.
        // 채우는 쪽이 INSERT면 같은 책이 두 줄로 보이므로 UPDATE여야 한다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));

        let placeholders: Vec<Value> = (0..3).map(|i| json!({"file_path": format!("0_inbox/{i}.epub"), "title": format!("{i}"), "grade": null, "target_category": null})).collect();
        let classified: Vec<Value> = placeholders
            .iter()
            .map(|item| {
                let mut c = item.clone();
                c["grade"] = json!("certain");
                c["target_category"] = json!("3_SF");
                c
            })
            .collect();

        let mut events = vec![json!({"total_count": 3, "processed_count": 0, "new_items": placeholders})];
        events.extend(classified.iter().enumerate().map(|(i, item)| json!({"total_count": 3, "processed_count": i + 1, "updated_items": [item]})));
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(3, &classified), None));

        let response = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["result"]["started"], true);

        let inserted: Vec<Value> = fx.cat.calls("add_classify_proposal_items").iter().flat_map(|c| c.args[0].as_array().unwrap().clone()).collect();
        assert_eq!(inserted, placeholders, "이름 행이 먼저 들어가야 한다");
        let updated: Vec<Value> = fx.cat.calls("update_classify_proposal_item_payloads").iter().flat_map(|c| c.args[0].as_array().unwrap().clone()).collect();
        assert_eq!(updated, classified, "분류 결과는 기존 행을 채워야 한다");
    }

    #[ignore = "클로저 introspection: endpoint.__closure__의 _run_classify_proposal_job 직접 접근; DI 설계 후 활성화"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_job_flushes_the_update_tail() {
        // 꼬리가 누락되면 목록 끝의 책들이 영원히 '분류 중'으로 남아, 다 끝났는데도 승인할 수 없는 행이 생긴다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "total_count": 0, "processed_count": 0})));
        let classified = epub_items(25, json!({"grade": "certain"}));

        let events: Vec<Value> = classified.iter().enumerate().map(|(i, item)| json!({"total_count": 25, "processed_count": i + 1, "updated_items": [item]})).collect();
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(25, &classified), None));
        let router = fx.item_router();

        router.run_classify_proposal_job("0_inbox", true, true, 1.2).await;

        let sizes = batch_sizes(&fx.cat.calls("update_classify_proposal_item_payloads"));
        assert_eq!(sizes, vec![10, 10, 5]);
    }

    #[ignore = "클로저 introspection: endpoint.__closure__의 _run_classify_proposal_job 직접 접근; DI 설계 후 활성화"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_job_batch_size_10_makes_small_categories_visible_before_job_ends() {
        // 25건을 처리하면 10+10+5로 나뉘어 flush돼야 하고, 그중 첫 flush는 11번째 항목을 처리하는 시점(=작업이 끝나기 전)에
        // 이미 일어나 있어야 한다. 배치 크기가 100이면 25건 전부가 끝난 뒤 한 번에만 flush되므로 이 검증에서 실패한다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "total_count": 0, "processed_count": 0})));

        let all_items = epub_items(25, json!({"target_category": "3_SF"}));
        let flush_count_mid_job: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));
        let (cat, slot) = (fx.cat.clone(), flush_count_mid_job.clone());
        let after_event: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |i| {
            if i == 11 {
                *slot.lock().unwrap() = Some(cat.call_count("add_classify_proposal_items"));
            }
        });
        let events: Vec<Value> = all_items.iter().enumerate().map(|(i, item)| json!({"total_count": 25, "processed_count": i + 1, "new_items": [item]})).collect();
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(25, &all_items), Some(after_event)));
        let router = fx.item_router();

        router.run_classify_proposal_job("0_inbox", true, true, 1.2).await;

        assert_eq!(*flush_count_mid_job.lock().unwrap(), Some(1));
        let sizes = batch_sizes(&fx.cat.calls("add_classify_proposal_items"));
        assert_eq!(sizes, vec![10, 10, 5]);
    }

    #[ignore = "create_item_router는 AppDeps를 거치지 않아 with_task_runner의 TaskRunner를 받지 못한다. 설계 문서에 router 쪽 주입 방법이 없다"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_flush_routes_db_write_through_to_thread() {
        // I5: 제안 진행 중 DB flush(add_classify_proposal_items)가 asyncio.to_thread를 거쳐 실행된다.
        // pymysql 호출은 블로킹이라 이벤트 루프 위에서 직접 부르면 단일 워커가 매 10건마다 멈춘다.
        // Rust에서는 `TaskRunner::spawn_blocking_observed`가 블로킹 풀로 넘어간 함수 이름을 기록한다(`with_task_runner`).
        let to_thread_funcs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let spy = to_thread_funcs.clone();
        let fx = Fx::tweaked(move |d| d.with_task_runner(Arc::new(SpyRunner(spy))));
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "total_count": 0, "processed_count": 0})));

        let all_items = epub_items(10, json!({"target_category": "3_SF"}));
        let events: Vec<Value> = all_items.iter().enumerate().map(|(i, item)| json!({"total_count": 10, "processed_count": i + 1, "new_items": [item]})).collect();
        fx.bm.side_effect("propose_category_changes", fake_propose(events, proposal_result(10, &all_items), None));
        let router = fx.item_router();

        router.run_classify_proposal_job("0_inbox", true, true, 1.2).await;

        assert!(to_thread_funcs.lock().unwrap().iter().any(|n| n == "add_classify_proposal_items"));
    }

    #[tokio::test]
    async fn test_classify_proposal_already_running_blocks_restart() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "running", "source_category": "0_inbox"})));

        let r = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"]["already_running"], true);
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_uses_server_side_allowed_paths() {
        // 클라이언트가 보낸 file_path를 그대로 허용 집합으로 쓰면 apply_category_changes의 검증이 통째로 무의미해진다.
        // 항목 목록이 DB(classify_proposal_items)로 옮겨갔으므로, 이 보안 불변식도 DB 기준으로 확인해야 한다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));

        let captured = Arc::new(Mutex::new(None));
        fx.bm.side_effect("apply_category_changes", capture_apply(captured.clone()));

        let r = fx
            .post_json(
                "/categories/classify-proposal/apply",
                &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "5_음악"}, {"file_path": "0_inbox/침입.epub", "target_category": "5_음악"}]}),
            )
            .await;

        assert_eq!(r.status, 200);
        let (items, allowed) = captured.lock().unwrap().clone().expect("apply_category_changes 호출");
        // 서버 제안(DB)에 있던 경로만 허용 집합에 들어간다. 클라이언트가 끼워넣은 경로는 빠진다
        assert_eq!(path_set(&allowed), set_of(&["0_inbox/a.epub"]));
        // 사용자가 화면에서 고친 목적지는 그대로 전달된다
        assert_eq!(items[0]["target_category"], "5_음악");
        // 허용 집합이 실제로 DB 조회에서 나왔는지 — body에서 만들어졌다면 이 호출이 없다.
        // (백그라운드 잡의 병합 단계도 같은 메서드를 한 번 더 부르므로 any_call로 본다.)
        fx.cat.assert_any_call("get_classify_proposal_items", json!([]), json!({"content_type": "book"}));
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_persists_only_allowed_item_callbacks() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        fx.bm.side_effect("apply_category_changes", |c: CallArgs<'_>| {
            Box::pin(async move {
                if let Some(callback) = c.callbacks.get("on_item_done") {
                    callback(json!({"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": null})).await;
                    callback(json!({"file_path": "0_inbox/outsider.epub", "apply_status": "failed", "apply_error": "not allowed"})).await;
                }
                Ok(json!([{"applied_count": 1, "failed_count": 0}, null]))
            })
        });

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "done");
        let updates = fx.cat.calls("update_classify_proposal_item_status");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].args, vec![json!("0_inbox/a.epub"), json!("moved"), json!(null)]);
        assert_eq!(updates[0].kwargs, json!({"content_type": "book"}));
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_marks_job_failed_when_manager_panics() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        fx.bm.raises("apply_category_changes", "manager crashed");

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_panic_does_not_fail_replacement_job() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        let replacement_store = store.clone();
        fx.bm.side_effect("apply_category_changes", move |_| {
            let store = replacement_store.clone();
            Box::pin(async move {
                store.lock().unwrap().insert("book".into(), json!({"status": "applying", "apply_token": "token-B", "content_type": "book"}));
                Err("manager crashed".to_string())
            })
        });

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        let final_status = status_of(&store, "book");
        assert_eq!(final_status["status"], "applying");
        assert_eq!(final_status["apply_token"], "token-B");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_marks_job_failed_when_item_status_write_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        fx.cat.raises("update_classify_proposal_item_status", "database unavailable");
        fx.bm.side_effect("apply_category_changes", |c: CallArgs<'_>| {
            Box::pin(async move {
                if let Some(callback) = c.callbacks.get("on_item_done") {
                    callback(json!({"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": null})).await;
                }
                Ok(json!([{"applied_count": 1, "failed_count": 0}, null]))
            })
        });

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_marks_job_failed_when_progress_status_write_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        fx.cat.raises("merge_classify_proposal_status", "database unavailable");
        fx.bm.side_effect("apply_category_changes", |c: CallArgs<'_>| {
            Box::pin(async move {
                if let Some(callback) = c.callbacks.get("on_progress") {
                    callback(json!({"apply_total_count": 1, "applied_count": 0, "failed_count": 0})).await;
                }
                Ok(json!([{"applied_count": 1, "failed_count": 0}, null]))
            })
        });

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_marks_job_failed_when_done_status_write_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        let fail_done_write = Arc::new(Mutex::new(true));
        let fail_once = fail_done_write.clone();
        let state_store = store.clone();
        fx.cat.side_effect("set_classify_proposal_status", move |c| {
            let next = arg(&c, 0, "status");
            let content_type = arg(&c, 1, "content_type").as_str().unwrap_or("book").to_string();
            let mut should_fail = fail_once.lock().unwrap();
            if next["status"] == "done" && *should_fail {
                *should_fail = false;
                return ready(Err("database unavailable".to_string()));
            }
            store_set(&state_store, &next, &content_type);
            ready(Ok(Value::Null))
        });
        fx.bm.returns("apply_category_changes", json!([{"applied_count": 1, "failed_count": 0}, null]));

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_marks_job_failed_when_final_status_read_fails() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        let reads = Arc::new(Mutex::new(0usize));
        let read_count = reads.clone();
        let state_store = store.clone();
        fx.cat.side_effect("get_classify_proposal_status", move |c| {
            let mut count = read_count.lock().unwrap();
            *count += 1;
            if *count == 2 {
                return ready(Err("database unavailable".to_string()));
            }
            let content_type = arg(&c, 0, "content_type").as_str().unwrap_or("book").to_string();
            let stale_seconds = arg_or(&c, 1, "stale_seconds", json!(300)).as_f64().unwrap_or(300.0);
            ready(Ok(effective(&state_store, &content_type, stale_seconds)))
        });
        fx.bm.returns("apply_category_changes", json!([{"applied_count": 1, "failed_count": 0}, null]));

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        assert_eq!(status_of(&store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_progress_preserves_proposal_counters() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox", "total_count": 1200, "processed_count": 1200})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));
        let observed = Arc::new(Mutex::new(None));
        let (status_store, seen) = (store.clone(), observed.clone());
        fx.bm.side_effect("apply_category_changes", move |c: CallArgs<'_>| {
            let (status_store, seen) = (status_store.clone(), seen.clone());
            Box::pin(async move {
                if let Some(callback) = c.callbacks.get("on_progress") {
                    callback(json!({"apply_total_count": 1, "applied_count": 0, "failed_count": 0})).await;
                }
                *seen.lock().unwrap() = Some(status_of(&status_store, "book"));
                Ok(json!([{"applied_count": 1, "failed_count": 0}, null]))
            })
        });

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(response.status, 200);
        let during_apply = observed.lock().unwrap().clone().unwrap();
        assert_eq!(during_apply["status"], "applying");
        assert_eq!(during_apply["total_count"], 1200);
        assert_eq!(during_apply["processed_count"], 1200);
        assert_eq!(during_apply["apply_total_count"], 1);
        let completed = status_of(&store, "book");
        assert_eq!(completed["status"], "done");
        assert_eq!(completed["total_count"], 1200);
        assert_eq!(completed["processed_count"], 1200);
        assert_eq!(completed["apply_total_count"], 1);
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_rejects_when_not_ready() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "idle"})));

        let r = fx.post_json("/categories/classify-proposal/apply", &json!({"items": []})).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
        assert!(r.json()["error"].as_str().unwrap().contains("제안"));
    }

    #[tokio::test]
    async fn test_classify_proposal_delete_clears_state() {
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready"})));

        let r = fx.delete("/categories/classify-proposal").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"]["cleared"], true);
        let cleared = status_of(&store, "book");
        assert_eq!(cleared["status"], "idle");
        // items는 더 이상 상태 파일에 담지 않는다 — DB(classify_proposal_items)를 지운다
        assert!(cleared.get("items").is_none());
        fx.cat.assert_called_once_with("clear_classify_proposal_items", json!([]), json!({"content_type": "book"}));
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_job_updates_item_status_per_item_in_db() {
        // book_manager가 항목 하나를 처리할 때마다 on_item_done을 불러 update_classify_proposal_item_status가 그 즉시 실행된다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns(
            "get_classify_proposal_items",
            json!([
                {"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"},
                {"file_path": "0_inbox/b.epub", "target_category": "3_SF", "apply_status": "pending"}
            ]),
        );

        // 실제 book_manager처럼 항목을 처리하는 그 순간 on_item_done을 부르고 await한다.
        fx.bm.side_effect("apply_category_changes", |c: CallArgs| {
            Box::pin(async move {
                if let Some(cb) = c.callbacks.get("on_item_done") {
                    cb(json!({"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": null})).await;
                    cb(json!({"file_path": "0_inbox/b.epub", "apply_status": "failed", "apply_error": "파일을 찾을 수 없습니다"})).await;
                }
                Ok(json!([{"total_count": 2, "applied_count": 1, "failed_count": 1, "results": []}, null]))
            })
        });

        let response = fx
            .post_json(
                "/categories/classify-proposal/apply",
                &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/b.epub", "target_category": "3_SF"}]}),
            )
            .await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["result"]["started"], true);

        let done = status_of(&store, "book");
        assert_eq!(done["status"], "done");
        assert_eq!(done["applied_count"], 1);
        assert_eq!(done["failed_count"], 1);
        assert!(done.get("items").is_none());

        // 병합용 clear/add는 더 이상 쓰지 않는다 — 건별 update가 그 일을 대신한다.
        fx.cat.assert_not_called("clear_classify_proposal_items");
        fx.cat.assert_not_called("add_classify_proposal_items");
        let update_calls = fx.cat.calls("update_classify_proposal_item_status");
        assert_eq!(update_calls[0].args, vec![json!("0_inbox/a.epub"), json!("moved"), json!(null)]);
        assert_eq!(update_calls[0].kwargs, json!({"content_type": "book"}));
        assert_eq!(update_calls[1].args, vec![json!("0_inbox/b.epub"), json!("failed"), json!("파일을 찾을 수 없습니다")]);
        assert_eq!(update_calls[1].kwargs, json!({"content_type": "book"}));
    }

    #[ignore = "클로저 introspection: endpoint.__closure__의 _run_classify_proposal_job/_run_classify_apply_job 직접 접근; DI 설계 후 활성화"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_job_handles_exceptions() {
        // 제안/적용 잡이 예외를 던지면 상태를 failed로 굳혀 화면이 계속 회전하지 않게 한다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        fx.cat.returns("get_all_mappings", json!({}));
        let router = fx.item_router();

        fx.bm.raises("propose_category_changes", "boom");
        router.run_classify_proposal_job("0_inbox", true, true, 1.2).await;
        assert_eq!(status_of(&fx.store, "book")["status"], "failed");

        fx.bm.raises("apply_category_changes", "boom");
        router.run_classify_apply_job(vec![], HashSet::new(), false, None).await;
        assert_eq!(status_of(&fx.store, "book")["status"], "failed");
    }

    #[tokio::test]
    async fn test_classify_proposal_applying_second_request_is_refused_while_claimed() {
        // Finding 3: 핸들러가 응답을 돌려주기 전에 동기로 applying을 선점하므로, 백그라운드 작업이 아직 실행되지 않은
        // 시점에도 두 번째 요청은 ready가 아님을 보고 거부돼야 한다.
        let pending_jobs = Arc::new(Mutex::new(Vec::new()));
        let runner = DeferredRunner(pending_jobs.clone());
        let fx = Fx::tweaked(|deps| deps.with_task_runner(Arc::new(runner)));
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]));

        let body = json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]});
        let first = fx.post_json("/categories/classify-proposal/apply", &body).await;
        assert_eq!(first.status, 200);
        assert_eq!(first.json()["result"]["started"], true);
        assert_eq!(pending_jobs.lock().unwrap().len(), 1);
        assert_eq!(status_of(&store, "book")["status"], "applying");

        let second = fx.post_json("/categories/classify-proposal/apply", &body).await;
        assert_eq!(second.status, 200);
        assert_eq!(second.json()["status"], "failure");
        assert!(second.json()["error"].as_str().unwrap().contains("제안"));
    }

    #[ignore = "클로저 introspection: endpoint.__closure__의 _run_classify_apply_job 직접 접근; DI 설계 후 활성화"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_apply_job_no_longer_depends_on_results_key() {
        // on_item_done 콜백 방식은 "results" 키를 읽지 않으므로, 키가 없어도 잡이 정상적으로 끝난다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "applying", "apply_token": "test-token"})));

        // "results" 키가 없다 — 더 이상 이 키를 읽지 않으므로 문제가 안 된다.
        fx.bm.returns("apply_category_changes", json!([{"total_count": 1, "applied_count": 1, "failed_count": 0}, null]));
        let router = fx.item_router();

        router.run_classify_apply_job(vec![json!({"file_path": "0_inbox/a.epub", "target_category": "3_SF"})], set_of(&["0_inbox/a.epub"]), false, Some("test-token")).await;

        let done = status_of(&store, "book");
        assert_eq!(done["status"], "done");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_progress_tick_does_not_revert_status() {
        // Finding 4: 적용 단계 진행률 콜백이 status를 running으로 되돌리지 않는다.
        let pending_jobs = Arc::new(Mutex::new(Vec::new()));
        let runner = DeferredRunner(pending_jobs.clone());
        let fx = Fx::tweaked(|deps| deps.with_task_runner(Arc::new(runner)));
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));

        let seen: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let (st, slot) = (store.clone(), seen.clone());
        fx.bm.side_effect("apply_category_changes", move |c: CallArgs| {
            let (st, slot) = (st.clone(), slot.clone());
            Box::pin(async move {
                if let Some(cb) = c.callbacks.get("on_progress") {
                    // 원본은 반환값을 await하지 않는다. Rust 콜백은 future라 실행하려면 await해야 한다.
                    cb(json!({"total_count": 1, "applied_count": 0, "failed_count": 0})).await;
                }
                *slot.lock().unwrap() = Some(status_of(&st, "book")["status"].clone());
                Ok(json!([{"total_count": 1, "applied_count": 1, "failed_count": 0, "results": [{"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": null}]}, null]))
            })
        });

        let response = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;
        assert_eq!(response.status, 200);
        let job = pending_jobs.lock().unwrap().pop().expect("apply 작업이 대기 중");
        std::thread::spawn(job).join().expect("apply 작업 종료");

        assert_eq!(seen.lock().unwrap().clone().unwrap(), "applying");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_progress_does_not_clobber_propose_counters() {
        // I1: 제안 1,200권 중 50건만 승인해 적용하면, 예전에는 apply의 on_progress가 같은 "total_count" 키로 병합돼
        // 헤더가 "1200 / 50"처럼 뒤바뀌었다. 이제 apply 전용 키(apply_total_count)로 보고하므로 제안 카운터는 그대로다.
        let pending_jobs = Arc::new(Mutex::new(Vec::new()));
        let runner = DeferredRunner(pending_jobs.clone());
        let fx = Fx::tweaked(|deps| deps.with_task_runner(Arc::new(runner)));
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox", "total_count": 1200, "processed_count": 1200})));
        let items = epub_items(50, json!({"target_category": "3_SF"}));
        fx.cat.returns(
            "get_classify_proposal_items",
            json!(items.iter().map(|item| json!({"file_path": item["file_path"], "target_category": item["target_category"], "apply_status": "pending"})).collect::<Vec<_>>()),
        );

        fx.bm.side_effect("apply_category_changes", |c: CallArgs| {
            Box::pin(async move {
                let n = arg(&c, 0, "items").as_array().map(|a| a.len()).unwrap_or(0);
                if let Some(cb) = c.callbacks.get("on_progress") {
                    // 실제 book_manager처럼, 진행률 콜백이 돌려준 awaitable을 await한다.
                    cb(json!({"apply_total_count": n, "applied_count": 10, "failed_count": 0})).await;
                }
                Ok(json!([{"total_count": n, "applied_count": n, "failed_count": 0, "results": []}, null]))
            })
        });
        let body = json!({"items": items.iter().map(|item| json!({"file_path": item["file_path"], "target_category": item["target_category"]})).collect::<Vec<_>>()});
        let response = fx.post_json("/categories/classify-proposal/apply", &body).await;
        assert_eq!(response.status, 200);
        let job = pending_jobs.lock().unwrap().pop().expect("apply 작업이 대기 중");
        std::thread::spawn(job).join().expect("apply 작업 종료");

        let done = status_of(&store, "book");
        // 제안 단계 카운터는 그대로 살아있다 — 승인 50건이 1200을 덮어쓰지 않는다.
        assert_eq!(done["total_count"], 1200);
        assert_eq!(done["processed_count"], 1200);
        assert_eq!(done["apply_total_count"], 50);
        assert_eq!(done["status"], "done");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_accepted_when_status_is_failed() {
        // 승인 도중 중단되면 상태가 applying -> (하트비트 만료) failed로 굳는다. 이때도 남은 pending 행을 다시 승인해
        // 이어갈 수 있어야 한다 — 그러지 않으면 관리자는 파일이 반쯤 옮겨진 채로 아무것도 할 수 없다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "failed", "source_category": "0_inbox", "error": "백엔드가 다시 시작되어 자동 분류가 중단되었습니다. 다시 실행해 주세요."})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));

        let r = fx.post_json("/categories/classify-proposal/apply", &json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]})).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
        assert_eq!(r.json()["result"]["started"], true);
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_rejected_while_applying() {
        // 작업이 이미 도는(applying) 동안에는 새 승인 요청을 거절한다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "applying"})));

        let r = fx.post_json("/categories/classify-proposal/apply", &json!({"items": []})).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_allowed_set_excludes_moved_rows() {
        // moved 행이 allowed에 들어가도 이중 이동은 안 일어나지만, 불필요하므로 애초에 pending 행만 담는다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "ready", "source_category": "0_inbox"})));
        fx.cat.returns(
            "get_classify_proposal_items",
            json!([{"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF", "apply_status": "moved"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF", "apply_status": "pending"}]),
        );

        let captured = Arc::new(Mutex::new(None));
        fx.bm.side_effect("apply_category_changes", capture_apply(captured.clone()));

        let r = fx
            .post_json(
                "/categories/classify-proposal/apply",
                &json!({"items": [{"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF"}]}),
            )
            .await;

        assert_eq!(r.status, 200);
        let (_, allowed) = captured.lock().unwrap().clone().expect("apply_category_changes 호출");
        assert_eq!(path_set(&allowed), set_of(&["0_inbox/still_pending.epub"]));
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_allowed_set_includes_failed_and_moving_rows() {
        // C1: failed 행을 재시도하지 못하면 관리자는 "제안 목록에 없는 파일입니다"만 영원히 다시 보게 된다.
        // I4: moving 행도 재시도할 방법이 있어야 한다 — apply_category_changes가 이동 직전 파일 존재를 다시 확인하므로
        // 이미 끝난 이동은 이중으로 옮겨지지 않는다. moved만 재시도 대상에서 빠진다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "failed", "source_category": "0_inbox"})));
        fx.cat.returns(
            "get_classify_proposal_items",
            json!([
                {"file_path": "0_inbox/interrupted.epub", "target_category": "3_SF", "apply_status": "moving"},
                {"file_path": "0_inbox/rejected.epub", "target_category": "3_SF", "apply_status": "failed"},
                {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF", "apply_status": "pending"},
                {"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF", "apply_status": "moved"},
            ]),
        );

        let captured = Arc::new(Mutex::new(None));
        fx.bm.side_effect("apply_category_changes", capture_apply(captured.clone()));

        let r = fx
            .post_json(
                "/categories/classify-proposal/apply",
                &json!({"items": [{"file_path": "0_inbox/interrupted.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/rejected.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF"}, {"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF"}]}),
            )
            .await;

        assert_eq!(r.status, 200);
        let (_, allowed) = captured.lock().unwrap().clone().expect("apply_category_changes 호출");
        assert_eq!(path_set(&allowed), set_of(&["0_inbox/interrupted.epub", "0_inbox/rejected.epub", "0_inbox/still_pending.epub"]));
    }

    #[ignore = "클로저 introspection: endpoint.__closure__의 _run_classify_apply_job 직접 접근; DI 설계 후 활성화"]
    #[cfg(any())] // 이전 내부 ItemRouter seam은 현재 HTTP API로 대체한다.
    #[tokio::test]
    async fn test_classify_proposal_apply_job_never_overwrites_rows_outside_this_runs_allowed_set() {
        // 재현: 200권 중 80권을 옮긴 뒤 중단. 화면은 GET으로 받은 목록(200권 전부)을 들고 있다가 재승인을 누른다.
        // book_manager는 allowed에 없는(이미 moved인) 항목도 "제안 목록에 없는 파일입니다"로 on_item_done을 부른다.
        // 이 콜백이 file_path로만 매칭해 덮어쓰면 moved였던 행이 failed로 둔갑해 다시는 지우거나 재승인할 수 없는 유령 실패가 된다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "failed", "apply_token": "test-token"})));

        fx.bm.side_effect("apply_category_changes", |c: CallArgs| {
            Box::pin(async move {
                if let Some(cb) = c.callbacks.get("on_item_done") {
                    // 이미 moved라 이번 실행의 allowed에는 없는 행 — book_manager는 여전히 on_item_done을 부르지만 여기서 걸러져야 한다.
                    cb(json!({"file_path": "0_inbox/already_moved.epub", "apply_status": "failed", "apply_error": "제안 목록에 없는 파일입니다"})).await;
                    // 이번 실행이 실제로 허용한 행
                    cb(json!({"file_path": "0_inbox/still_pending.epub", "apply_status": "moved", "apply_error": null})).await;
                }
                Ok(json!([{"total_count": 2, "applied_count": 1, "failed_count": 1, "results": []}, null]))
            })
        });
        let router = fx.item_router();

        // 이번 실행의 allowed에는 still_pending만 있다 — already_moved는 이미 moved라 빠졌다
        router
            .run_classify_apply_job(
                vec![json!({"file_path": "0_inbox/already_moved.epub", "target_category": "3_SF"}), json!({"file_path": "0_inbox/still_pending.epub", "target_category": "3_SF"})],
                set_of(&["0_inbox/still_pending.epub"]),
                false,
                Some("test-token"),
            )
            .await;

        let updated_paths: Vec<Value> = fx.cat.calls("update_classify_proposal_item_status").iter().map(|c| c.args[0].clone()).collect();
        assert!(!updated_paths.contains(&json!("0_inbox/already_moved.epub")));
        assert!(updated_paths.contains(&json!("0_inbox/still_pending.epub")));
    }

    #[tokio::test]
    async fn test_classify_proposal_apply_job_stops_writing_when_token_no_longer_matches() {
        // 다른 승인 작업이 상태의 apply_token을 바꾸면, 이 작업은 더 이상 처리하지 않고 물러난다.
        // 토큰이 바뀌면 진 쪽은 should_continue에서 멈추고, 최종 상태(status/token)도 건드리지 않아야 한다.
        let pending_jobs = Arc::new(Mutex::new(Vec::new()));
        let runner = DeferredRunner(pending_jobs.clone());
        let fx = Fx::tweaked(|deps| deps.with_task_runner(Arc::new(runner)));
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let store = install_classify_status_store(&fx.cat, Some(json!({"status": "ready"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "0_inbox/a.epub", "target_category": "3_SF", "apply_status": "pending"}]));

        let st = store.clone();
        fx.bm.side_effect("apply_category_changes", move |c: CallArgs| {
            let st = st.clone();
            Box::pin(async move {
                if let Some(cb) = c.callbacks.get("on_item_done") {
                    cb(json!({"file_path": "0_inbox/a.epub", "apply_status": "moved", "apply_error": null})).await;
                }
                // 다른 요청이 이 작업을 대체해 토큰을 바꿨다고 가정한다.
                st.lock().unwrap().insert("book".into(), json!({"status": "applying", "apply_token": "token-B", "content_type": "book"}));
                // 상태가 DB로 옮겨가면서 should_continue는 코루틴을 돌려준다. 여기서 await해야 판단 결과를 볼 수 있다.
                let should_continue = c.callbacks.get("should_continue").expect("should_continue 콜백");
                assert_eq!(should_continue(Value::Null).await, json!(false));
                Ok(json!([{"total_count": 1, "applied_count": 1, "failed_count": 0, "results": []}, null]))
            })
        });

        let body = json!({"items": [{"file_path": "0_inbox/a.epub", "target_category": "3_SF"}]});
        let response = fx.post_json("/categories/classify-proposal/apply", &body).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["result"]["started"], true);
        let job = pending_jobs.lock().unwrap().pop().expect("apply 작업이 대기 중");
        std::thread::spawn(job).join().expect("apply 작업 종료");

        // 대체된 뒤에는 이 작업이 상태를 더 이상 덮어쓰지 않는다 - "applying"/"token-B" 그대로여야 한다
        let final_status = status_of(&store, "book");
        assert_eq!(final_status["status"], "applying");
        assert_eq!(final_status["apply_token"], "token-B");
    }

    #[tokio::test]
    async fn test_delete_proposal_item_removes_one_row() {
        // 표에서 책을 지운 뒤 그 행을 없애는 데 쓴다. 남겨 두면 다음 조회에서 이미 없는 파일을 가리키는 행이 되살아난다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "ready"})));
        fx.cat.returns("delete_classify_proposal_item", json!(1));

        let r = fx.delete(&format!("/categories/classify-proposal/item?file_path={}", enc("1_fiction/a.epub"))).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
        assert_eq!(r.json()["result"]["deleted_count"], 1);
        fx.cat.assert_called_once_with("delete_classify_proposal_item", json!(["1_fiction/a.epub"]), json!({"content_type": "book"}));
    }

    #[tokio::test]
    async fn test_delete_proposal_item_rejected_while_job_is_running() {
        // 도는 중에 지우면 그 작업이 방금 찍은 행을 밑에서 없앨 수 있다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "running"})));

        let r = fx.delete(&format!("/categories/classify-proposal/item?file_path={}", enc("1_fiction/a.epub"))).await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
        fx.cat.assert_not_called("delete_classify_proposal_item");
    }

    #[tokio::test]
    async fn test_classify_proposal_get_fills_missing_book_id() {
        // book_id 없이 저장된 옛 항목은 조회할 때 채워 준다. 없으면 표의 편집·삭제 버튼이 안 그려진다.
        use std::os::unix::fs::MetadataExt;

        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        let book = tmp.path().join("1_fiction").join("a.epub");
        std::fs::create_dir_all(book.parent().unwrap()).unwrap();
        std::fs::write(&book, "x").unwrap();
        install_classify_status_store(&fx.cat, Some(json!({"status": "ready"})));
        fx.cat.returns("get_classify_proposal_items", json!([{"file_path": "1_fiction/a.epub"}]));

        let r = fx.get("/categories/classify-proposal").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["result"]["items"][0]["book_id"], json!(std::fs::metadata(&book).unwrap().ino()));
    }

    #[tokio::test]
    async fn test_delete_applied_removes_moved_only_and_returns_count() {
        // DELETE .../applied가 완료(moved) 행만 지우고 지운 개수를 돌려준다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "done"})));
        fx.cat.returns("delete_applied_classify_proposal_items", json!(3));

        let r = fx.delete("/categories/classify-proposal/applied").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
        assert_eq!(r.json()["result"]["deleted_count"], 3);
        fx.cat.assert_called_once_with("delete_applied_classify_proposal_items", json!([]), json!({"content_type": "book"}));
    }

    #[tokio::test]
    async fn test_delete_applied_rejected_while_job_is_running() {
        // 도는 중에 지우면 방금 건별로 기록한 행을 진행 중인 작업 밑에서 지울 수 있다.
        let fx = Fx::new();
        let tmp = TempDir::new().unwrap();
        fx.bm.set_path_prefix(tmp.path());
        install_classify_status_store(&fx.cat, Some(json!({"status": "applying"})));

        let r = fx.delete("/categories/classify-proposal/applied").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
        fx.cat.assert_not_called("delete_applied_classify_proposal_items");
    }

    #[tokio::test]
    async fn test_index_file_success() {
        let fx = Fx::new();
        fx.bm.returns("index_single_file", json!([42, null]));
        let r = fx.post_json("/category-mismatches/index-file", &json!({"file_path": "_epub/test.epub"})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": {"book_id": 42}}));
    }

    #[tokio::test]
    async fn test_index_file_missing_path() {
        let fx = Fx::new();
        let r = fx.post_json("/category-mismatches/index-file", &json!({})).await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn test_index_file_error() {
        let fx = Fx::new();
        fx.bm.returns("index_single_file", json!([null, "file not found"]));
        let r = fx.post_json("/category-mismatches/index-file", &json!({"file_path": "bad.epub"})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
        assert!(r.json()["error"].as_str().unwrap().contains("file not found"));
    }

    #[tokio::test]
    async fn test_delete_file_success() {
        let fx = Fx::new();
        fx.bm.returns("delete_file", json!(["Ok", null]));
        let r = fx.post_json("/category-mismatches/delete-file", &json!({"file_path": "_epub/old.epub"})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": "Ok"}));
    }

    #[tokio::test]
    async fn test_delete_file_missing_path() {
        let fx = Fx::new();
        let r = fx.post_json("/category-mismatches/delete-file", &json!({})).await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn test_delete_file_error() {
        let fx = Fx::new();
        fx.bm.returns("delete_file", json!([null, "permission denied"]));
        let r = fx.post_json("/category-mismatches/delete-file", &json!({"file_path": "bad.epub"})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
    }

    #[tokio::test]
    async fn test_delete_es_doc_success() {
        let fx = Fx::new();
        fx.bm.returns("es_manager.delete", json!(true));
        let r = fx.delete("/category-mismatches/es-doc/1").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
    }

    #[tokio::test]
    async fn test_delete_es_doc_failure() {
        let fx = Fx::new();
        fx.bm.returns("es_manager.delete", json!(false));
        let r = fx.delete("/category-mismatches/es-doc/1").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains('1'));
    }

    #[tokio::test]
    async fn test_reload_success() {
        let fx = Fx::new();
        fx.bm.returns("reload_category", json!([{"reloaded": 3}, null]));
        let r = fx.post_json("/category-mismatches/reload", &json!({"category": "_epub"})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": {"reloaded": 3}}));
    }

    #[tokio::test]
    async fn test_reload_failure() {
        let fx = Fx::new();
        fx.bm.returns("reload_category", json!([null, "reload failed"]));
        let r = fx.post_json("/category-mismatches/reload", &json!({"category": "_epub"})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "failure");
        assert!(r.json()["error"].as_str().unwrap().contains("reload failed"));
    }

    #[tokio::test]
    async fn test_get_details_success() {
        let fx = Fx::new();
        let details = json!({"es": ["a.epub"], "fs": ["b.epub"]});
        // get_category_mismatch_details는 asyncio.to_thread로 부르므로 동기 함수여야 한다.
        fx.bm.returns("get_category_mismatch_details", details.clone());
        let r = fx.get("/category-mismatches/_epub").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": details}));
    }

    #[tokio::test]
    async fn test_get_details_exception() {
        let fx = Fx::new();
        fx.bm.raises("get_category_mismatch_details", "ES down");
        let r = fx.get("/category-mismatches/_epub").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "failure");
        assert_eq!(data["error"], "카테고리 불일치 조회 중 오류가 발생했습니다");
    }
}

// ── /search/bookstore/{store_name} ───────────────────────────────────────────

/// 서점 클래스 가짜. `fake_store.search.return_value`와 `build_search_url.return_value`를 정한다.
fn fake_store(search: Value, url: &str) -> Mock {
    let store = Mock::new();
    store.returns("search", search);
    store.returns("build_search_url", json!(url));
    store
}

mod test_search_bookstore {
    use super::*;

    #[tokio::test]
    async fn test_unknown_store() {
        let fx = Fx::new();
        let r = fx.get("/search/bookstore/unknown?title=test").await;
        assert_eq!(r.status, 404);
    }

    #[tokio::test]
    async fn test_missing_all_params() {
        let fx = Fx::new();
        let r = fx.get("/search/bookstore/yes24").await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn test_yes24_with_results() {
        let store = fake_store(json!([[["Book A", "Author A", "소설", "http://url", null, "9781234567890"]], "Book A", "title"]), "http://search");
        let fx = Fx::tweaked(|d| d.with_bookstore_factory(store_factory(store)));
        let r = fx.get("/search/bookstore/yes24?title=Book+A").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"][0]["title"], "Book A");
        assert_eq!(data["result"][0]["isbn"], "9781234567890");
    }

    #[tokio::test]
    async fn test_yes24_empty_results() {
        let store = fake_store(json!([[], "NoTitle", "title"]), "http://search");
        let fx = Fx::tweaked(|d| d.with_bookstore_factory(store_factory(store)));
        let r = fx.get("/search/bookstore/yes24?title=NoTitle").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "not_found");
        assert_eq!(data["result"], json!([]));
    }

    #[tokio::test]
    async fn test_result_without_isbn() {
        let store = fake_store(json!([[["Book B", "Author B", "소설", "http://url2", null, ""]], "Book B", "title"]), "");
        let fx = Fx::tweaked(|d| d.with_bookstore_factory(store_factory(store)));
        let r = fx.get("/search/bookstore/aladin?title=Book+B").await;
        assert_eq!(r.status, 200);
        assert!(r.json()["result"][0].get("isbn").is_none());
    }

    #[tokio::test]
    async fn test_other_stores() {
        let cases = [
            ("ridi", "RidibooksBookstore"),
            ("naver", "NaverShoppingBookstore"),
            ("naverseries", "NaverSeriesBookstore"),
            ("munpia", "MunpiaBookstore"),
            ("kyobo", "KyoboBookstore"),
            ("joara", "JoaraBookstore"),
            ("naverwebtoon", "NaverWebtoonBookstore"),
            ("kakaowebtoon", "KakaoWebtoonBookstore"),
            ("bomtoon", "BomtoonBookstore"),
            ("lezhin", "LezhinBookstore"),
            ("toptoon", "ToptoonBookstore"),
        ];
        for (store_name, cls) in cases {
            let store = fake_store(json!([[], "q", "title"]), "");
            let fx = Fx::tweaked(|d| d.with_bookstore_factory(store_factory(store)));
            let r = fx.get(&format!("/search/bookstore/{store_name}?title=q")).await;
            assert_eq!(r.status, 200, "store={store_name} cls={cls}");
            assert_eq!(r.json()["status"], "not_found", "store={store_name} cls={cls}");
        }
    }
}

// ── /auth/logout, /auth/me ───────────────────────────────────────────────────

mod test_auth_logout_and_me {
    use super::*;

    #[tokio::test]
    async fn test_logout() {
        let fx = Fx::new();
        let r = fx.post_empty("/auth/logout").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success"}));
    }

    #[tokio::test]
    async fn test_auth_me_returns_payload() {
        let fx = Fx::new();
        let r = fx.get("/auth/me").await;
        assert_eq!(r.status, 200);
        let data = r.json();
        let admin = admin_payload();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"]["email"], admin["email"]);
        assert_eq!(data["result"]["role"], admin["role"]);
        assert_eq!(data["result"]["name"], admin["name"]);
        assert!(data["result"].get("expires_in").is_some());
    }
}

// ── /auth/refresh ────────────────────────────────────────────────────────────

/// `refresh_token_store`를 가짜로 끼운 Fx를 돌려준다. `determine_role`은 항상 "admin"이다.
fn fx_with_refresh_store(rt_store: &Mock) -> Fx {
    let store = rt_store.clone();
    Fx::tweaked(move |d| d.with_role_resolver(Arc::new(FixedRole(Some("admin".to_string())))).with_refresh_token_store(Arc::new(store)))
}

mod test_auth_refresh {
    use super::*;

    #[tokio::test]
    async fn test_missing_token_returns_400() {
        let fx = Fx::new();
        let r = fx.post_empty("/auth/refresh").await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn test_invalid_token_returns_401() {
        let fx = Fx::new();
        let r = fx.post_with_cookie("/auth/refresh", "tm_refresh_token", "bad.token.here").await;
        assert_eq!(r.status, 401);
    }

    #[tokio::test]
    async fn test_success() {
        let rt_store = Mock::new();
        rt_store.returns("rotate", json!("ok"));
        let fx = fx_with_refresh_store(&rt_store);
        let token = backend::auth::create_refresh_token("admin@test.com", "admin", Some("Admin"), None);
        let r = fx.post_with_cookie("/auth/refresh", "tm_refresh_token", &token).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
        assert!(r.json().get("expires_in").is_some());
    }

    #[tokio::test]
    async fn test_unauthorized_email_returns_403() {
        let fx = Fx::new();
        let token = backend::auth::create_refresh_token("stranger@other.com", "viewer", Some("X"), None);
        let r = fx.post_with_cookie("/auth/refresh", "tm_refresh_token", &token).await;
        assert_eq!(r.status, 403);
    }

    #[tokio::test]
    async fn test_rotation_rejection_clears_cookies() {
        let rt_store = Mock::new();
        rt_store.returns("rotate", json!("reused"));
        let fx = fx_with_refresh_store(&rt_store);
        let token = backend::auth::create_refresh_token("admin@test.com", "admin", Some("Admin"), None);
        let r = fx.post_with_cookie("/auth/refresh", "tm_refresh_token", &token).await;
        assert_eq!(r.status, 401);
        assert_eq!(r.json()["detail"], "Invalid refresh token state");
        let cookies = set_cookies(&r);
        assert!(cookies.iter().any(|c| c.contains("tm_access_token=") && c.contains("Max-Age=0")), "{cookies:?}");
        assert!(cookies.iter().any(|c| c.contains("tm_refresh_token=") && c.contains("Max-Age=0")), "{cookies:?}");
    }

    #[tokio::test]
    async fn test_db_error_in_rotate_returns_503() {
        // refresh_token_store.rotate()가 DB 예외를 던지면 503을 반환한다.
        let rt_store = Mock::new();
        rt_store.raises("rotate", "DB connection lost");
        let fx = fx_with_refresh_store(&rt_store);
        let token = backend::auth::create_refresh_token("admin@test.com", "admin", Some("Admin"), None);
        let r = fx.post_with_cookie("/auth/refresh", "tm_refresh_token", &token).await;
        assert_eq!(r.status, 503);
    }

    #[tokio::test]
    async fn test_legacy_refresh_token_without_claims_returns_401() {
        // fid/jti 없는 구형 refresh token은 401과 함께 재로그인 안내 메시지를 반환한다.
        let fx = Fx::new();
        let now = now_plus_3600() - 3600;
        let legacy_payload = json!({"type": "refresh", "email": "admin@test.com", "role": "admin", "exp": now + 3600, "iat": now});
        let secret = std::env::var("TM_JWT_SECRET").unwrap_or_else(|_| "test_jwt_secret_for_testing_minimum_32bytes".to_string());
        let legacy_token = jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256), &legacy_payload, &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes())).unwrap();
        let r = fx.post_with_cookie("/auth/refresh", "tm_refresh_token", &legacy_token).await;
        assert_eq!(r.status, 401);
        assert!(r.json().get("detail").and_then(Value::as_str).unwrap_or("").to_lowercase().contains("log in again"));
    }
}

// ── /auth/google ─────────────────────────────────────────────────────────────

/// Google 검증 seam을 채운 Fx. `client_id`가 None이면 TM_GOOGLE_CLIENT_ID 미설정이다.
fn fx_with_google(client_id: Option<&str>, verify: Result<Value, String>, role: Option<&str>) -> Fx {
    let client_id = client_id.map(str::to_string);
    let role = role.map(str::to_string);
    Fx::tweaked(move |d| {
        let d = d.with_google_verifier(Arc::new(FixedVerifier { client_id, result: verify }));
        // 역할이 없으면 실제 `determine_role`을 쓴다.
        match role {
            Some(role) => d.with_role_resolver(Arc::new(FixedRole(Some(role)))),
            None => d,
        }
    })
}

mod test_auth_google {
    use super::*;

    #[tokio::test]
    async fn test_missing_credential_returns_400() {
        let fx = Fx::new();
        let r = fx.post_json("/auth/google", &json!({})).await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn test_google_api_error_returns_401() {
        let fx = fx_with_google(Some("mock_client_id"), Err("bad".to_string()), None);
        let r = fx.post_json("/auth/google", &json!({"credential": "bad_token"})).await;
        assert_eq!(r.status, 401);
    }

    #[tokio::test]
    async fn test_missing_google_client_id_returns_500() {
        let fx = fx_with_google(None, Ok(json!({})), None);
        let r = fx.post_json("/auth/google", &json!({"credential": "token"})).await;
        assert_eq!(r.status, 500);
    }

    #[tokio::test]
    async fn test_issuer_mismatch_returns_401() {
        let payload = json!({"aud": "expected_client_id", "iss": "https://evil.example.com", "email": "admin@test.com", "email_verified": true});
        let fx = fx_with_google(Some("expected_client_id"), Ok(payload), None);
        let r = fx.post_json("/auth/google", &json!({"credential": "token"})).await;
        assert_eq!(r.status, 401);
    }

    #[tokio::test]
    async fn test_unverified_email_returns_401() {
        let payload = json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "admin@test.com", "email_verified": false});
        let fx = fx_with_google(Some("cid"), Ok(payload), None);
        let r = fx.post_json("/auth/google", &json!({"credential": "token"})).await;
        assert_eq!(r.status, 401);
    }

    #[tokio::test]
    async fn test_unauthorized_email_returns_403() {
        let payload = json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "stranger@other.com", "email_verified": true, "name": "X", "picture": ""});
        let fx = fx_with_google(Some("cid"), Ok(payload), None);
        let r = fx.post_json("/auth/google", &json!({"credential": "token"})).await;
        assert_eq!(r.status, 403);
    }

    #[tokio::test]
    async fn test_success() {
        let payload = json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "admin@test.com", "email_verified": true, "name": "Admin", "picture": "http://pic"});
        let fx = fx_with_google(Some("cid"), Ok(payload), Some("admin"));
        let r = fx.post_json("/auth/google", &json!({"credential": "token"})).await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["email"], "admin@test.com");
        assert_eq!(data["role"], "admin");
        assert!(data.get("expires_in").is_some());
    }
}

mod test_auth_logout {
    use super::*;

    #[tokio::test]
    async fn test_logout_revokes_refresh_family_when_cookie_is_present() {
        let rt_store = Mock::new();
        let store = rt_store.clone();
        let fx = Fx::tweaked(move |d| d.with_refresh_token_store(Arc::new(store)));
        let token = backend::auth::create_refresh_token("admin@test.com", "admin", Some("Admin"), None);
        let r = fx.post_with_cookie("/auth/logout", "tm_refresh_token", &token).await;
        assert_eq!(r.status, 200);
        assert_eq!(rt_store.call_count("revoke_family"), 1);
    }
}

// ── /category-mappings ───────────────────────────────────────────────────────

mod test_category_mappings {
    use super::*;

    #[tokio::test]
    async fn test_get_all() {
        let fx = Fx::new();
        fx.cat.returns("get_all_mappings", json!({"소설": ["fantasy", "romance"]}));
        let r = fx.get("/category-mappings").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": {"소설": ["fantasy", "romance"]}}));
    }

    #[tokio::test]
    async fn test_get_keywords() {
        let fx = Fx::new();
        fx.cat.returns("get_keywords", json!(["fantasy"]));
        let r = fx.get(&format!("/category-mappings/{}", enc("소설"))).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": ["fantasy"]}));
    }

    #[tokio::test]
    async fn test_set_keywords_success() {
        let fx = Fx::new();
        fx.cat.returns("set_keywords", json!(true));
        fx.cat.returns("get_keywords", json!(["drama"]));
        let r = fx.put_json(&format!("/category-mappings/{}", enc("소설")), &json!({"keywords": ["drama"]})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
        assert_eq!(r.json()["result"], json!(["drama"]));
    }

    #[tokio::test]
    async fn test_set_keywords_failure_returns_500() {
        let fx = Fx::new();
        fx.cat.returns("set_keywords", json!(false));
        let r = fx.put_json(&format!("/category-mappings/{}", enc("소설")), &json!({"keywords": ["drama"]})).await;
        assert_eq!(r.status, 500);
    }

    #[tokio::test]
    async fn test_add_keyword_missing_returns_400() {
        let fx = Fx::new();
        let r = fx.post_json(&format!("/category-mappings/{}/keywords", enc("소설")), &json!({})).await;
        assert_eq!(r.status, 400);
    }

    #[tokio::test]
    async fn test_add_keyword_success() {
        let fx = Fx::new();
        fx.cat.returns("add_keyword", json!(true));
        fx.cat.returns("get_keywords", json!(["fantasy", "sci-fi"]));
        let r = fx.post_json(&format!("/category-mappings/{}/keywords", enc("소설")), &json!({"keyword": "sci-fi"})).await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "success");
        assert!(data["result"].as_array().unwrap().contains(&json!("sci-fi")));
    }

    #[tokio::test]
    async fn test_add_keyword_duplicate_is_not_a_failure() {
        // 이미 있는 키워드는 실패가 아니다 — "등록해 달라"는 요청이 이미 충족돼 있다.
        // 예전에는 status를 "duplicate"로 돌려줬고, 공용 응답 처리기가 success가 아닌 것을 전부 오류로 보내는데
        // error 키까지 없어서 화면에는 이유가 안 적힌 실패 창이 떴다 — 등록은 되어 있는데도.
        let fx = Fx::new();
        fx.cat.returns("add_keyword", json!(false));
        fx.cat.returns("get_keywords", json!(["fantasy"]));
        let r = fx.post_json(&format!("/category-mappings/{}/keywords", enc("소설")), &json!({"keyword": "fantasy"})).await;
        assert_eq!(r.status, 200);
        let body = r.json();
        assert_eq!(body["status"], "success");
        assert_eq!(body["result"], json!(["fantasy"]));
        // 아무 일도 안 일어났다는 사실은 경고로 알린다.
        assert!(is_truthy(&body["warning"]));
    }

    #[tokio::test]
    async fn test_remove_keyword_success() {
        let fx = Fx::new();
        fx.cat.returns("remove_keyword", json!(true));
        fx.cat.returns("get_keywords", json!([]));
        let r = fx.delete(&format!("/category-mappings/{}/keywords/fantasy", enc("소설"))).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": []}));
    }

    #[tokio::test]
    async fn test_remove_keyword_not_found_returns_404() {
        let fx = Fx::new();
        fx.cat.returns("remove_keyword", json!(false));
        let r = fx.delete(&format!("/category-mappings/{}/keywords/nonexistent", enc("소설"))).await;
        assert_eq!(r.status, 404);
    }

    #[tokio::test]
    async fn test_delete_category_mapping_success() {
        let fx = Fx::new();
        fx.cat.returns("delete_category", json!(true));
        let r = fx.delete(&format!("/category-mappings/{}", enc("소설"))).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success"}));
    }

    #[tokio::test]
    async fn test_delete_category_mapping_not_found_returns_404() {
        let fx = Fx::new();
        fx.cat.returns("delete_category", json!(false));
        let r = fx.delete(&format!("/category-mappings/{}", enc("없는카테고리"))).await;
        assert_eq!(r.status, 404);
    }

    #[tokio::test]
    async fn test_update_all_success() {
        let fx = Fx::new();
        fx.cat.returns("update_all_mappings", json!(true));
        fx.cat.returns("get_all_mappings", json!({"소설": ["a"]}));
        let r = fx.put_json("/category-mappings", &json!({"mappings": {"소설": ["a"]}})).await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json()["status"], "success");
    }

    #[tokio::test]
    async fn test_update_all_failure_returns_500() {
        let fx = Fx::new();
        fx.cat.returns("update_all_mappings", json!(false));
        let r = fx.put_json("/category-mappings", &json!({"mappings": {"소설": ["a"]}})).await;
        assert_eq!(r.status, 500);
    }
}

// ── /hidden-categories ───────────────────────────────────────────────────────

mod test_hidden_categories {
    use super::*;

    #[tokio::test]
    async fn test_get() {
        let fx = Fx::new();
        fx.cat.returns("get_hidden_categories", json!(["_draft", "_archive"]));
        let r = fx.get("/hidden-categories").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": ["_draft", "_archive"]}));
    }

    #[tokio::test]
    async fn test_get_viewer_hides_hidden_category_names() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_hidden_categories", json!(["_draft", "_archive"]));

        let r = fx.get("/hidden-categories").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": []}));
        fx.cat.assert_not_called("get_hidden_categories");
    }

    #[tokio::test]
    async fn test_set_hidden_success() {
        let fx = Fx::new();
        fx.cat.returns("set_hidden", json!(true));
        fx.cat.returns("get_hidden_categories", json!(["_draft"]));
        let r = fx.post_json("/hidden-categories/_draft", &json!({"hidden": true})).await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "success");
        assert!(data["result"].as_array().unwrap().contains(&json!("_draft")));
    }

    #[tokio::test]
    async fn test_set_hidden_failure_returns_500() {
        let fx = Fx::new();
        fx.cat.returns("set_hidden", json!(false));
        let r = fx.post_json("/hidden-categories/_draft", &json!({"hidden": true})).await;
        assert_eq!(r.status, 500);
    }
}

mod test_latest_excluded_categories {
    use super::*;

    #[tokio::test]
    async fn test_get() {
        let fx = Fx::new();
        fx.cat.returns("get_latest_excluded_categories", json!(["_draft", "_archive"]));
        let r = fx.get("/latest-excluded-categories").await;
        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": ["_draft", "_archive"]}));
    }

    #[tokio::test]
    async fn test_get_viewer_hides_category_names() {
        let fx = Fx::new();
        fx.set_auth(viewer_payload());
        fx.cat.returns("get_latest_excluded_categories", json!(["_draft", "_archive"]));

        let r = fx.get("/latest-excluded-categories").await;

        assert_eq!(r.status, 200);
        assert_eq!(r.json(), json!({"status": "success", "result": []}));
        fx.cat.assert_not_called("get_latest_excluded_categories");
    }

    #[tokio::test]
    async fn test_set_latest_excluded_success() {
        let fx = Fx::new();
        fx.cat.returns("set_latest_excluded", json!(true));
        fx.cat.returns("get_latest_excluded_categories", json!(["_draft"]));
        let r = fx.post_json("/latest-excluded-categories/_draft", &json!({"excluded": true})).await;
        assert_eq!(r.status, 200);
        let data = r.json();
        assert_eq!(data["status"], "success");
        assert!(data["result"].as_array().unwrap().contains(&json!("_draft")));
    }

    #[tokio::test]
    async fn test_set_latest_excluded_failure_returns_500() {
        let fx = Fx::new();
        fx.cat.returns("set_latest_excluded", json!(false));
        let r = fx.post_json("/latest-excluded-categories/_draft", &json!({"excluded": true})).await;
        assert_eq!(r.status, 500);
    }
}

// ── utility function unit tests ──────────────────────────────────────────────

mod test_summarize_request_body {
    use super::*;

    /// _summarize_request_body 유틸리티 함수 (main.py:138-143).
    #[test]
    fn test_list_body() {
        let result = backend::main::summarize_request_body(&json!([1, 2, 3]));
        assert_eq!(result, json!({"type": "list", "length": 3}));
    }

    #[test]
    fn test_none_body() {
        let result = backend::main::summarize_request_body(&Value::Null);
        assert_eq!(result, json!({"type": "none"}));
    }

    #[test]
    fn test_str_body() {
        let result = backend::main::summarize_request_body(&json!("hello"));
        assert_eq!(result, json!({"type": "str", "length": 5}));
    }
}

/// 원격 호스트는 False (main.py:148).
#[test]
fn test_is_local_frontend_origin_remote_returns_false() {
    assert!(!backend::main::is_local_frontend_origin(Some("https://remote.example.com")));
}

/// localhost 계열은 True (main.py:154-156).
#[test]
fn test_is_local_frontend_origin_localhost_returns_true() {
    assert!(backend::main::is_local_frontend_origin(Some("http://localhost:5173")));
    assert!(backend::main::is_local_frontend_origin(Some("http://127.0.0.1:8080")));
    assert!(backend::main::is_local_frontend_origin(Some("http://[::1]:3000")));
}

/// 빈 값/None 은 False (main.py:152-153).
#[test]
fn test_is_local_frontend_origin_empty_returns_false() {
    assert!(!backend::main::is_local_frontend_origin(None));
    assert!(!backend::main::is_local_frontend_origin(Some("")));
}

/// TM_FRONTEND_URL 미설정 시 False (main.py:157).
#[cfg(any())] // 현재 AppDeps/URI 계약에 없는 이전 보조 함수 테스트다.
#[test]
fn test_is_request_from_frontend_host_no_env_returns_false() {
    let _env = EnvGuard::apply(&[("TM_FRONTEND_URL", None)]);
    // 원본은 scope의 server=("testserver", 80)을 쓴다. axum Request에는 서버 정보가 없어 URI의 호스트로 대신한다.
    let request = axum::http::Request::builder().method("GET").uri("http://testserver/").body(()).unwrap();
    assert!(!backend::main::is_request_from_frontend_host(&request));
}

/// 카테고리가 빈 문자열이면 False (main.py:252).
#[cfg(any())] // hidden category 필터는 공개 라우트 통합 테스트에서 검증한다.
#[test]
fn test_category_matches_hidden_empty_category_returns_false() {
    assert!(!backend::main::category_matches_hidden("", &["cat1", "cat2"]));
}

/// viewer 권한에서 hidden 카테고리 책이 유사도 결과에서 제외된다 (main.py:479-480).
#[tokio::test]
async fn test_search_similar_books_filters_hidden_categories_for_viewer() {
    let fx = Fx::new();
    fx.set_auth(viewer_payload());

    let source_book = make_book(json!({"book_id": 1, "category": "visible_cat"}));
    let hidden_similar = make_book(json!({"book_id": 2, "category": "hidden_cat"}));
    let visible_similar = make_book(json!({"book_id": 3, "category": "visible_cat"}));

    fx.bm.returns("get_book", json!([source_book, null]));
    fx.bm.returns("search_similar_books_paged", json!([[hidden_similar, visible_similar], 2, null]));
    fx.cat.returns("get_hidden_categories", json!(["hidden_cat"]));

    let r = fx.get("/similar/1").await;
    assert_eq!(r.status, 200);
    let data = r.json();
    assert_eq!(data["status"], "success");
    assert_eq!(data["total"], 1); // hidden_similar 제외, visible_similar 만 남음
}

// ── Additional edge case tests to reach 100% coverage on backend/main.py ──

#[tokio::test]
async fn test_get_latest_books_error() {
    // 621: get_latest_books returns error
    let fx = Fx::new();
    fx.cat.returns("get_latest_excluded_categories", json!([]));
    fx.bm.returns("get_latest_books", json!([[], 0, "failed to get latest"]));
    let r = fx.get("/latest").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["status"], "failure");
    assert_eq!(r.json()["error"], "failed to get latest");
}

#[tokio::test]
async fn test_reload_progress_flush_exception() {
    // 924-926: exception in _flush_reload_progress_periodically
    let fx = Fx::tweaked(|d| d.with_reload_progress_flush_interval(Duration::from_millis(1)));
    fx.cat.returns("acquire_reload_lock", json!([true, null, null]));
    fx.cat.raises("heartbeat_reload_lock", "heartbeat db failed");

    fx.bm.side_effect("reload_category_mismatches", |_c: CallArgs| {
        Box::pin(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            Ok(json!([{"indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0}, null]))
        })
    });
    let r = fx.post_empty("/category-mismatches/reload-all").await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn test_reload_all_mismatches_job_exception() {
    // 959-962: exception in _run_reload_all_mismatches_job
    let fx = Fx::new();
    fx.cat.returns("acquire_reload_lock", json!([true, null, null]));
    fx.bm.raises("reload_category_mismatches", "mismatch scan crashed");

    let r = fx.post_empty("/category-mismatches/reload-all").await;
    assert_eq!(r.status, 200);
    let call = fx.cat.calls("complete_reload_lock").pop().expect("완료 상태 기록");
    assert_eq!(Value::Array(call.args), json!(["book", "failed", "mismatch scan crashed"]));
    assert_eq!(call.kwargs, json!({"indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0}));
}

#[tokio::test]
async fn test_reload_locks_already_running() {
    // 981-984: reload_category_mismatch_files lock busy
    let fx = Fx::new();
    fx.cat.returns("acquire_reload_lock", json!([false, "lock busy", {"status": "running"}]));
    let r1 = fx.post_json("/category-mismatches/reload-mismatches", &json!({"category": "0_inbox"})).await;
    assert_eq!(r1.status, 200);
    assert_eq!(r1.json()["result"]["already_running"], true);

    // 1002-1005: reload_all_category_mismatches lock busy
    let r2 = fx.post_empty("/category-mismatches/reload-all").await;
    assert_eq!(r2.status, 200);
    assert_eq!(r2.json()["result"]["already_running"], true);
}

#[tokio::test]
async fn test_classify_proposal_restarts_when_the_previous_run_died() {
    // 재배포로 죽은 작업은 새 실행을 막지 않아야 한다.
    // 상태가 running 인 채로 굳으면 버튼을 눌러도 already_running 만 돌아와 화면이 계속 회전했다.
    // 갱신이 끊긴 상태는 죽은 것으로 본다.
    let fx = Fx::new();
    let tmp = TempDir::new().unwrap();
    fx.bm.set_path_prefix(tmp.path());
    install_classify_status_store(&fx.cat, Some(json!({"status": "running", "source_category": "0_inbox", "_age_seconds": 3600})));

    let r = fx.post_json("/categories/classify-proposal", &json!({"category": "0_inbox"})).await;

    assert_eq!(r.status, 200);
    assert!(r.json()["result"].get("already_running").map(Value::is_null).unwrap_or(true));
    assert_eq!(r.json()["result"]["started"], true);
}
