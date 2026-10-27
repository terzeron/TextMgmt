//! 오류 응답과 로깅. Python `main.py`의 `exception_handler` 세 개에 대응한다.
//!
//! 라우트는 `Result<_, ApiError>`를 돌려준다. `ApiError`는 FastAPI가 오류를 응답으로 바꾸는 방식과 같은 JSON(`{"detail": ..}`)이 된다.
//! 오류 로그는 요청의 메서드·경로가 필요해서 `error_middleware`가 응답을 보고 한 번만 남긴다(`http_exception_handler` 등과 같은 내용).

use std::error::Error;
use std::future::poll_fn;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::task::Poll;

use axum::body::Body;
use axum::extract::Request;
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub use crate::auth::HTTPException;

pub const GENERIC_SERVER_ERROR_DETAIL: &str = "서버 내부 오류가 발생했습니다";
pub const VALIDATION_ERROR_DETAIL: &str = "입력값이 올바르지 않습니다";
/// 저장소(DB)를 쓸 수 없을 때 503에 쓰는 문구.
pub const SERVICE_UNAVAILABLE_DETAIL: &str = "서비스를 일시적으로 사용할 수 없습니다. 잠시 후 다시 시도해 주세요.";

/// FastAPI `RequestValidationError`: 검증 오류 목록과 받은 본문.
#[derive(Debug, Clone)]
pub struct RequestValidationError {
    errors: Vec<Value>,
    body: Value,
}

impl RequestValidationError {
    pub fn new(errors: Vec<Value>, body: Value) -> Self {
        Self { errors, body }
    }

    /// 입력 위치(`loc`) 하나에 대한 오류 하나.
    pub fn single(loc: &[&str], msg: &str, body: Value) -> Self {
        Self { errors: vec![json!({"loc": loc, "msg": msg, "type": "value_error"})], body }
    }

    pub fn errors(&self) -> &[Value] {
        &self.errors
    }

    pub fn body(&self) -> &Value {
        &self.body
    }
}

impl std::fmt::Display for RequestValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} validation error(s)", self.errors.len())
    }
}

impl Error for RequestValidationError {}

/// 라우트가 돌려주는 오류.
#[derive(Debug, Clone)]
pub enum ApiError {
    /// `HTTPException`: 상태 코드와 `detail`을 그대로 응답한다.
    Http(HTTPException),
    /// 422
    Validation(RequestValidationError),
    /// 예상하지 못한 오류. 500과 일반 문구로 응답하고 사유는 로그에만 남긴다.
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn http(status_code: u16, detail: &str) -> Self {
        ApiError::Http(HTTPException::new(status_code, detail))
    }

    /// 저장소 오류를 503으로 바꾼다. 사유는 호출한 쪽이 먼저 로그에 남긴다.
    pub fn unavailable() -> Self {
        Self::http(503, SERVICE_UNAVAILABLE_DETAIL)
    }
}

impl From<HTTPException> for ApiError {
    fn from(e: HTTPException) -> Self {
        ApiError::Http(e)
    }
}

impl From<RequestValidationError> for ApiError {
    fn from(e: RequestValidationError) -> Self {
        ApiError::Validation(e)
    }
}

/// 상태 코드와 JSON 본문. Starlette `JSONResponse`처럼 공백 없이 쓰고 한글을 이스케이프하지 않는다.
pub fn json_response(status: u16, content: &Value) -> Response {
    let mut resp = Response::new(Body::from(serde_json::to_vec(content).unwrap_or_default()));
    *resp.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    resp
}

fn render(err: &ApiError) -> Response {
    match err {
        ApiError::Http(e) => json_response(e.status_code, &json!({"detail": e.detail})),
        ApiError::Validation(_) => json_response(422, &json!({"detail": VALIDATION_ERROR_DETAIL})),
        ApiError::Internal(_) => json_response(500, &json!({"detail": GENERIC_SERVER_ERROR_DETAIL})),
    }
}

impl IntoResponse for ApiError {
    /// 응답 본문만 만든다. 로그는 `error_middleware`가 이 응답에 붙인 오류를 보고 남긴다.
    fn into_response(self) -> Response {
        let mut resp = render(&self);
        resp.extensions_mut().insert(self);
        resp
    }
}

/// Python `_summarize_request_body`: 본문 내용 대신 모양만 남긴다(키 이름 20개, 길이).
pub fn summarize_request_body(body: &Value) -> Value {
    match body {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            json!({"type": "dict", "keys": keys.into_iter().take(20).collect::<Vec<_>>(), "key_count": map.len()})
        }
        Value::Array(items) => json!({"type": "list", "length": items.len()}),
        Value::Null => json!({"type": "none"}),
        Value::String(s) => json!({"type": "str", "length": s.chars().count()}),
        Value::Bool(b) => json!({"type": "bool", "length": if *b { 4 } else { 5 }}),
        Value::Number(n) => json!({"type": if n.is_f64() { "float" } else { "int" }, "length": n.to_string().len()}),
    }
}

fn log_validation(method: &Method, path: &str, exc: &RequestValidationError) {
    log::error!("[422] {method} {path}");
    // 입력 값(`input`)에는 비밀(credential 등)이 들어 있을 수 있어서 위치·메시지·종류만 남긴다.
    let errors: Vec<Value> = exc.errors().iter().map(|e| json!({"loc": e.get("loc"), "msg": e.get("msg"), "type": e.get("type")})).collect();
    log::error!("Validation error: {}", Value::Array(errors));
    log::error!("Request body summary: {}", summarize_request_body(exc.body()));
}

fn log_http(method: &Method, path: &str, exc: &HTTPException) {
    log::error!("[{}] {method} {path} - {}", exc.status_code, exc.detail);
}

fn log_general(method: &Method, path: &str, exc: &(dyn Error + Send + Sync)) {
    log::error!("[500] {method} {path}");
    log::error!("Exception: {exc}");
}

/// 검증 오류를 기록하고 422 응답을 만든다.
pub async fn validation_exception_handler(parts: &Parts, exc: &RequestValidationError) -> Response {
    log_validation(&parts.method, parts.uri.path(), exc);
    render(&ApiError::Validation(exc.clone()))
}

/// `HTTPException`을 기록하고 같은 상태 코드의 응답을 만든다.
pub async fn http_exception_handler(parts: &Parts, exc: &HTTPException) -> Response {
    log_http(&parts.method, parts.uri.path(), exc);
    render(&ApiError::Http(exc.clone()))
}

/// 처리하지 못한 오류를 기록하고 500 응답을 만든다. 사유는 응답에 담지 않는다.
pub async fn general_exception_handler(parts: &Parts, exc: &(dyn Error + Send + Sync)) -> Response {
    log_general(&parts.method, parts.uri.path(), exc);
    render(&ApiError::Internal(exc.to_string()))
}

#[derive(Debug)]
struct PanicError(String);

impl std::fmt::Display for PanicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for PanicError {}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".to_string())
}

/// 핸들러가 panic해도 연결을 끊지 않고 500 응답을 돌려준다(Starlette `ServerErrorMiddleware`).
pub async fn error_middleware(req: Request, next: Next) -> Response {
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    let mut fut = Box::pin(next.run(req));
    let outcome = poll_fn(|cx| match catch_unwind(AssertUnwindSafe(|| fut.as_mut().poll(cx))) {
        Ok(Poll::Pending) => Poll::Pending,
        Ok(Poll::Ready(resp)) => Poll::Ready(Ok(resp)),
        Err(payload) => Poll::Ready(Err(panic_message(payload.as_ref()))),
    })
    .await;
    let mut resp = match outcome {
        Ok(resp) => resp,
        Err(message) => {
            log_general(&method, &path, &PanicError(message));
            return render(&ApiError::Internal(String::new()));
        }
    };
    match resp.extensions_mut().remove::<ApiError>() {
        Some(ApiError::Http(e)) => log_http(&method, &path, &e),
        Some(ApiError::Validation(e)) => log_validation(&method, &path, &e),
        Some(ApiError::Internal(msg)) => log_general(&method, &path, &PanicError(msg)),
        None if resp.status() == StatusCode::METHOD_NOT_ALLOWED && resp.headers().get(header::CONTENT_TYPE).is_none() => {
            // axum의 기본 405는 본문이 없다. FastAPI와 같은 JSON으로 바꾼다.
            let e = HTTPException::new(405, "Method Not Allowed");
            log_http(&method, &path, &e);
            let mut replaced = render(&ApiError::Http(e));
            if let Some(allow) = resp.headers().get(header::ALLOW) {
                replaced.headers_mut().insert(header::ALLOW, allow.clone());
            }
            return replaced;
        }
        None => {}
    }
    resp
}

/// 없는 경로. FastAPI의 404와 같은 `{"detail": "Not Found"}`다.
pub async fn not_found() -> ApiError {
    ApiError::http(404, "Not Found")
}
