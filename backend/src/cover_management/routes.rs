//! `/admin/book-covers/*` 라우트. Python `create_cover_router`에 대응한다. 앱에는 `main`의 `all_routes()`가 `routes()`를 합친다.
//!
//! 서비스 호출은 `deps.task_runner()`로 넘기고(Python `asyncio.to_thread`) 응답이 끝나면 취소 표시를 켠다.
//! 요청이 끊겨 핸들러가 버려져도(axum은 future를 버린다) 작업 스레드의 `check_cancelled`가 멈춘다.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::handler::Handler;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ApplyCoverModel, ApplyImageModel, ApplyUrlModel, BookCoverModel, CoverHooks, CoverManagement, with_cancel_scope};
use crate::epub_cover::{CoverError, MAX_IMAGE_BYTES};
use crate::main::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, RequestValidationError, RequireAdmin, Routes};

const FAILED_DETAIL: &str = "표지 작업을 완료하지 못했습니다. 다시 시도하세요";

pub fn routes() -> Routes {
    // base64 본문이 이미지 상한(15MB)의 4/3배라서 axum의 기본 2MiB 상한을 넓힌다.
    let image_body = DefaultBodyLimit::max(MAX_IMAGE_BYTES.div_ceil(3) * 4 + 64 * 1024);
    Routes::new()
        .get("/admin/book-covers/directories", &["require_admin"], directories)
        .get("/admin/book-covers/missing", &["require_admin"], missing)
        .get("/admin/book-covers/candidates/{store_name}", &["require_admin"], candidates)
        .get("/admin/book-covers/candidate-image/{candidate_id}", &["require_admin"], candidate_image)
        .post("/admin/book-covers/apply", &["require_admin"], apply)
        .post("/admin/book-covers/apply-url", &["require_admin"], apply_url)
        .post("/admin/book-covers/apply-image", &["require_admin"], apply_image.layer(image_body))
        .post("/admin/book-covers/cache/clear", &["require_admin"], clear_cache)
        .get("/admin/book-covers/preview", &["require_admin"], preview)
}

// ---------------------------------------------------------------------------
// 작업 실행
// ---------------------------------------------------------------------------

struct Shared<R> {
    result: Option<R>,
    waker: Option<Waker>,
}

/// 작업 스레드가 값을 채우면 깨어나는 future(비동기 일회용 채널).
struct Pending<R> {
    shared: Arc<Mutex<Shared<R>>>,
    cancelled: Arc<AtomicBool>,
    /// 연결이 끊겼는지 확인하는 함수. 있으면 주기적으로 깨어나 확인한다.
    probe: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ticker: bool,
}

impl<R: Send + 'static> Future for Pending<R> {
    type Output = Option<R>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<R>> {
        let finished = {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            match shared.result.take() {
                Some(result) => Some(result),
                None => {
                    shared.waker = Some(cx.waker().clone());
                    None
                }
            }
        };
        if let Some(result) = finished {
            return Poll::Ready(Some(result));
        }
        if let Some(probe) = self.probe.clone() {
            if probe() {
                self.cancelled.store(true, Ordering::SeqCst);
                return Poll::Ready(None);
            }
            if !self.ticker {
                self.ticker = true;
                let (shared, cancelled) = (self.shared.clone(), self.cancelled.clone());
                let waker = cx.waker().clone();
                std::thread::spawn(move || {
                    // 작업이 끝나거나 요청이 끝나 취소 표시가 켜질 때까지 주기적으로 깨운다.
                    while !cancelled.load(Ordering::SeqCst) && shared.lock().map(|s| s.result.is_none()).unwrap_or(false) {
                        std::thread::sleep(Duration::from_millis(50));
                        waker.wake_by_ref();
                    }
                });
            }
        }
        Poll::Pending
    }
}

/// 핸들러가 끝나거나 버려지면 취소 표시를 켠다(Python `finally: cancelled.set()`).
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn to_api_error(err: CoverError) -> ApiError {
    if err.is_cancelled() {
        ApiError::http(499, "표지 요청을 취소했습니다")
    } else if err.is_io() {
        log::warn!("Cover management failed: {err}");
        ApiError::http(503, FAILED_DETAIL)
    } else {
        ApiError::http(409, &err.to_string())
    }
}

fn root_and_hooks(deps: &AppDeps) -> Result<(std::path::PathBuf, CoverHooks), ApiError> {
    let manager = deps.book_manager().map_err(|e| {
        log::warn!("Cover management failed: {e}");
        ApiError::http(503, FAILED_DETAIL)
    })?;
    Ok((manager.path_prefix(), deps.cover_hooks()))
}

/// 서비스 작업을 작업 스레드에서 돌리고 결과를 기다린다. `watch`면 연결이 끊겼는지 확인한다.
async fn run<R, F>(deps: &Arc<AppDeps>, watch: bool, job: F) -> ApiResult<R>
where
    R: Send + 'static,
    F: FnOnce(&CoverManagement) -> Result<R, CoverError> + Send + 'static,
{
    let (root, hooks) = root_and_hooks(deps)?;
    let cancelled = Arc::new(AtomicBool::new(false));
    let _guard = CancelOnDrop(cancelled.clone());
    let shared = Arc::new(Mutex::new(Shared { result: None, waker: None }));
    let probe: Option<Arc<dyn Fn() -> bool + Send + Sync>> = if watch && hooks.has_disconnect_probe() {
        let hooks = hooks.clone();
        Some(Arc::new(move || hooks.is_disconnected()))
    } else {
        None
    };
    let (sink, flag) = (shared.clone(), cancelled.clone());
    deps.task_runner().spawn_blocking_observed("cover_management");
    deps.task_runner().spawn(
        "cover-management",
        Box::new(move || {
            let service = CoverManagement::with_hooks(&root, hooks);
            let outcome = with_cancel_scope(Arc::new(move || flag.load(Ordering::SeqCst)), || job(&service));
            let waker = {
                let mut shared = sink.lock().unwrap_or_else(|e| e.into_inner());
                shared.result = Some(outcome);
                shared.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }),
    );
    match (Pending { shared, cancelled, probe, ticker: false }).await {
        Some(result) => result.map_err(to_api_error),
        None => Err(ApiError::http(499, "표지 요청을 취소했습니다")),
    }
}

fn success(result: Value) -> Json<Value> {
    Json(json!({"status": "success", "result": result}))
}

// ---------------------------------------------------------------------------
// 쿼리 검증 (FastAPI Query)
// ---------------------------------------------------------------------------

fn invalid(loc: &[&str], msg: &str) -> ApiError {
    RequestValidationError::single(loc, msg, Value::Null).into()
}

fn max_length(name: &'static str, value: &str, max: usize) -> ApiResult<()> {
    if value.chars().count() > max { Err(invalid(&["query", name], &format!("String should have at most {max} characters"))) } else { Ok(()) }
}

fn min_length(name: &'static str, value: &str) -> ApiResult<()> {
    if value.is_empty() { Err(invalid(&["query", name], "String should have at least 1 character")) } else { Ok(()) }
}

/// FastAPI의 bool 쿼리 해석.
fn parse_bool(name: &'static str, value: Option<&str>) -> ApiResult<bool> {
    match value.map(str::to_lowercase).as_deref() {
        None => Ok(false),
        Some("true" | "1" | "on" | "yes" | "t" | "y") => Ok(true),
        Some("false" | "0" | "off" | "no" | "f" | "n") => Ok(false),
        Some(_) => Err(invalid(&["query", name], "Input should be a valid boolean, unable to interpret input")),
    }
}

// ---------------------------------------------------------------------------
// 핸들러
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DirectoryQuery {
    #[serde(default)]
    directory: String,
}

async fn directories(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiQuery(query): ApiQuery<DirectoryQuery>) -> ApiResult<Json<Value>> {
    max_length("directory", &query.directory, 4096)?;
    let result = run(&deps, false, move |service| match service.hooks().directories_hook() {
        Some(f) => f(&query.directory).map(Value::Array).map_err(CoverError::from),
        None => service.directories(&query.directory).map(Value::Array),
    })
    .await?;
    Ok(success(result))
}

#[derive(Deserialize)]
struct MissingQuery {
    #[serde(default)]
    directory: String,
    #[serde(default)]
    cursor: String,
    limit: Option<String>,
    covered: Option<String>,
}

async fn missing(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiQuery(query): ApiQuery<MissingQuery>) -> ApiResult<Json<Value>> {
    max_length("directory", &query.directory, 4096)?;
    max_length("cursor", &query.cursor, 10)?;
    let limit = match query.limit.as_deref() {
        None => 60,
        Some(text) => match text.trim().parse::<i64>() {
            Ok(n) if (1..=100).contains(&n) => n as usize,
            Ok(n) if n < 1 => return Err(invalid(&["query", "limit"], "Input should be greater than or equal to 1")),
            Ok(_) => return Err(invalid(&["query", "limit"], "Input should be less than or equal to 100")),
            Err(_) => return Err(invalid(&["query", "limit"], "Input should be a valid integer, unable to parse string as an integer")),
        },
    };
    let covered = parse_bool("covered", query.covered.as_deref())?;
    let result = run(&deps, false, move |service| service.missing(&query.directory, &query.cursor, limit, covered)).await?;
    Ok(success(result))
}

#[derive(Deserialize)]
struct CandidatesQuery {
    file_path: Option<String>,
    include_covered: Option<String>,
    refresh: Option<String>,
}

async fn candidates(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(store_name): Path<String>, ApiQuery(query): ApiQuery<CandidatesQuery>) -> ApiResult<Json<Value>> {
    let Some(file_path) = query.file_path else { return Err(invalid(&["query", "file_path"], "Field required")) };
    min_length("file_path", &file_path)?;
    max_length("file_path", &file_path, 4096)?;
    let include_covered = parse_bool("include_covered", query.include_covered.as_deref())?;
    let refresh = parse_bool("refresh", query.refresh.as_deref())?;
    let result = run(&deps, true, move |service| match service.hooks().candidates_hook() {
        Some(f) => f(&store_name, &file_path),
        None => service.candidates(&store_name, &file_path, include_covered, refresh),
    })
    .await?;
    Ok(success(result))
}

fn image_response(data: Vec<u8>, cache_control: &'static str) -> Response {
    let mut response = data.into_response();
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control));
    response
}

async fn candidate_image(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(candidate_id): Path<String>) -> ApiResult<Response> {
    let data = run(&deps, true, move |service| service.image(&candidate_id)).await?;
    Ok(image_response(data, "private, max-age=3600"))
}

/// 저장 뒤 검색 색인의 파일 크기를 갱신한다. 실패해도 저장은 끝났으므로 경고만 붙인다.
async fn complete_apply(deps: &Arc<AppDeps>, mut result: Value) -> ApiResult<Json<Value>> {
    let book_id = result.get("book_id").and_then(Value::as_i64).unwrap_or(0);
    let file_size = result.get("file_size").cloned().unwrap_or(Value::Null);
    let manager = deps.book_manager();
    let outcome = run_index_update(deps, manager, book_id, file_size).await;
    if let Err(message) = outcome {
        log::warn!("Cover saved; ES metadata update failed: {message}");
        result["warning"] = json!("EPUB 표지를 저장했습니다. 검색 색인의 파일 크기는 갱신하지 못했습니다");
    }
    Ok(success(result))
}

async fn run_index_update(deps: &Arc<AppDeps>, manager: Result<Arc<dyn crate::main::BookManagerApi>, String>, book_id: i64, file_size: Value) -> Result<(), String> {
    let manager = manager?;
    let shared = Arc::new(Mutex::new(Shared { result: None, waker: None }));
    let sink = shared.clone();
    deps.task_runner().spawn_blocking_observed("update_index");
    deps.task_runner().spawn(
        "cover-index-update",
        Box::new(move || {
            let es = manager.es_manager();
            let outcome: Result<(), String> = (|| {
                let doc = es.search_by_id(book_id).map_err(|e| e.to_string())?;
                if doc.as_ref().is_some_and(|d| !d.is_null()) && !es.update(book_id, &json!({"file_size": file_size})).map_err(|e| e.to_string())? {
                    return Err("ES update failed".to_string());
                }
                Ok(())
            })();
            let waker = {
                let mut shared = sink.lock().unwrap_or_else(|e| e.into_inner());
                shared.result = Some(outcome);
                shared.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }),
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    (Pending { shared, cancelled, probe: None, ticker: false }).await.unwrap_or_else(|| Err("취소".to_string()))
}

async fn apply(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiJson(body): ApiJson<ApplyCoverModel>) -> ApiResult<Json<Value>> {
    let result = run(&deps, false, move |service| service.apply(&body.file_path, &body.candidate_id, &body.revision)).await?;
    complete_apply(&deps, result).await
}

async fn apply_url(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiJson(body): ApiJson<ApplyUrlModel>) -> ApiResult<Json<Value>> {
    let result = run(&deps, false, move |service| service.apply_url(&body.file_path, &body.url, &body.revision, body.replace_existing)).await?;
    complete_apply(&deps, result).await
}

async fn apply_image(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiJson(body): ApiJson<ApplyImageModel>) -> ApiResult<Json<Value>> {
    let result = run(&deps, false, move |service| service.apply_image(&body.file_path, &body.image_data, &body.revision)).await?;
    complete_apply(&deps, result).await
}

async fn clear_cache(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiJson(body): ApiJson<BookCoverModel>) -> ApiResult<Json<Value>> {
    let result = run(&deps, false, move |service| service.clear_cache(&body.file_path, &body.revision)).await?;
    Ok(success(result))
}

#[derive(Deserialize)]
struct PreviewQuery {
    file_path: Option<String>,
    v: Option<String>,
    original: Option<String>,
    w: Option<String>,
}

async fn preview(_admin: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiQuery(query): ApiQuery<PreviewQuery>) -> ApiResult<Response> {
    let Some(file_path) = query.file_path else { return Err(invalid(&["query", "file_path"], "Field required")) };
    min_length("file_path", &file_path)?;
    max_length("file_path", &file_path, 4096)?;
    max_length("v", query.v.as_deref().unwrap_or(""), 200)?;
    let original = parse_bool("original", query.original.as_deref())?;
    let width = match query.w.as_deref() {
        None => None,
        Some(text) => Some(text.trim().parse::<i64>().map_err(|_| invalid(&["query", "w"], "Input should be a valid integer, unable to parse string as an integer"))?),
    };
    // 허용하지 않는 폭은 서비스가 기본 폭으로 바꾼다.
    let width = width.and_then(|w| u32::try_from(w).ok());
    let data = run(&deps, false, move |service| service.preview_width(&file_path, original, width)).await?;
    Ok(image_response(data, "private, no-store"))
}
