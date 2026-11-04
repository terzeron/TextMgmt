//! 라우트 핸들러가 쓰는 추출기. 인증 의존성(`RequireAuth`, `RequireAdmin`, `OptionalAuth`)과 검증 오류를 FastAPI 형식으로 바꾸는 `ApiJson`, `ApiQuery`,
//! 요청한 클라이언트의 IP·User-Agent(`ClientInfo`).
//!
//! 인증 추출기는 핸들러 인자 맨 앞에 둔다. FastAPI도 의존성을 입력 검증보다 먼저 풀어서, 인증 실패(401/403)가 검증 실패(422)보다 먼저 나간다.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequest, FromRequestParts, Query, Request};
use axum::http::HeaderMap;
use axum::http::header;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use super::deps::{AppDeps, AuthMode};
use super::errors::{ApiError, RequestValidationError};
use crate::auth::{optional_auth_with, require_admin_with, require_auth_with};

/// Pydantic·FastAPI bool 문자열 해석. 대소문자를 가리지 않는다. 해석할 수 없으면 `None`.
pub(crate) fn parse_py_bool(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "1" | "on" | "yes" | "t" | "y" => Some(true),
        "false" | "0" | "off" | "no" | "f" | "n" => Some(false),
        _ => None,
    }
}

/// Pydantic bool의 JSON·쿼리 입력 규칙.
pub(super) fn python_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let parsed = match Value::deserialize(d)? {
        Value::Bool(v) => Some(v),
        Value::String(s) => parse_py_bool(&s),
        Value::Number(n) if n.as_f64() == Some(0.0) => Some(false),
        Value::Number(n) if n.as_f64() == Some(1.0) => Some(true),
        _ => None,
    };
    parsed.ok_or_else(|| serde::de::Error::custom("Input should be a valid boolean"))
}

async fn resolve_auth(deps: &AppDeps, parts: &Parts, admin: bool) -> Result<Value, ApiError> {
    let slot = |m: &std::sync::Mutex<Value>| m.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match (deps.auth_mode(), admin) {
        (AuthMode::Fixed(v), _) => Ok(v.clone()),
        (AuthMode::Split { auth, .. }, false) => Ok(auth.clone()),
        (AuthMode::Split { admin, .. }, true) => Ok(admin.clone()),
        (AuthMode::AuthOnly(v), false) => Ok(v.clone()),
        (AuthMode::Shared(slots), false) => Ok(slot(&slots.auth)),
        (AuthMode::Shared(slots), true) => Ok(slot(&slots.admin)),
        (AuthMode::Cookies, false) => {
            let payload = require_auth_with(parts, deps.role_resolver()).await?;
            require_active_session(deps, &payload).await?;
            Ok(payload)
        }
        (AuthMode::Cookies | AuthMode::AuthOnly(_), true) => {
            let payload = require_admin_with(parts, deps.role_resolver()).await?;
            require_active_session(deps, &payload).await?;
            Ok(payload)
        }
    }
}

/// 폐기 결과를 다음 요청부터 반영한다. 저장소 오류도 인증 성공으로 처리하지 않는다.
async fn require_active_session(deps: &AppDeps, payload: &Value) -> Result<(), ApiError> {
    let family_id = payload.get("fid").and_then(Value::as_str).filter(|fid| !fid.is_empty()).ok_or_else(|| ApiError::http(401, "Session token required"))?.to_string();
    let email = payload.get("email").and_then(Value::as_str).unwrap_or_default().to_string();
    let unavailable = |error: String| {
        log::warn!("Access session validation failed: {error}");
        ApiError::unavailable()
    };
    let store = deps.refresh_store().map_err(&unavailable)?;
    let active =
        tokio::task::spawn_blocking(move || store.is_session_active(&family_id, &email)).await.map_err(|error| unavailable(error.to_string()))?.map_err(|error| unavailable(error.to_string()))?;
    if active { Ok(()) } else { Err(ApiError::http(401, "Session is no longer active")) }
}

/// `Depends(require_auth)`: admin 또는 viewer. 값은 토큰 payload(역할은 지금 정책으로 덮어쓴다)다.
pub struct RequireAuth(pub Value);

/// `Depends(require_admin)`
pub struct RequireAdmin(pub Value);

/// `Depends(optional_auth)`: 인증에 실패하면 None. 오류를 만들지 않는다.
///
/// Python의 `optional_auth`는 `dependency_overrides`와 상관없이 항상 실제 인증을 한다. 여기서도 `AuthMode`를 무시하고 실제 인증을 한다.
pub struct OptionalAuth(pub Option<Value>);

impl FromRequestParts<Arc<AppDeps>> for RequireAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, deps: &Arc<AppDeps>) -> Result<Self, Self::Rejection> {
        Ok(Self(resolve_auth(deps, parts, false).await?))
    }
}

impl FromRequestParts<Arc<AppDeps>> for RequireAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, deps: &Arc<AppDeps>) -> Result<Self, Self::Rejection> {
        Ok(Self(resolve_auth(deps, parts, true).await?))
    }
}

impl FromRequestParts<Arc<AppDeps>> for OptionalAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, deps: &Arc<AppDeps>) -> Result<Self, Self::Rejection> {
        let mut payload = optional_auth_with(parts, deps.role_resolver()).await;
        if let Some(value) = &payload
            && require_active_session(deps, value).await.is_err()
        {
            payload = None;
        }
        Ok(Self(payload))
    }
}

/// JSON 본문. 형식이 틀리면 FastAPI처럼 422(`RequestValidationError`)다.
pub struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|ct| {
            let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            ct == "application/json" || ct.strip_prefix("application/").is_some_and(|subtype| subtype.len() > "+json".len() && subtype.ends_with("+json"))
        });
        if !is_json {
            return Err(RequestValidationError::single(&["body"], "Input should be a valid JSON body", Value::Null).into());
        }
        let bytes = axum::body::Bytes::from_request(req, state).await.map_err(|e| RequestValidationError::single(&["body"], &e.to_string(), Value::Null))?;
        let raw: Value = serde_json::from_slice(&bytes).map_err(|e| RequestValidationError::single(&["body"], &format!("JSON decode error: {e}"), Value::Null))?;
        match serde_json::from_value::<T>(raw.clone()) {
            Ok(v) => Ok(Self(v)),
            Err(e) => Err(RequestValidationError::single(&["body"], &e.to_string(), raw).into()),
        }
    }
}

/// 쿼리 문자열. 형식이 틀리면 422다.
pub struct ApiQuery<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for ApiQuery<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(v)) => Ok(Self(v)),
            Err(e) => Err(RequestValidationError::single(&["query"], &e.to_string(), Value::Null).into()),
        }
    }
}

// ---- 요청한 클라이언트의 IP와 User-Agent. Python `_client_ip`, `_request_ip_prefix`, `_request_user_agent`. ----
//
// 실제 경로는 클라이언트 -> Cloudflare -> Traefik -> 백엔드 pod다. 이 구성에서
// - 접속 상대(peer)는 Traefik pod IP이고,
// - `X-Forwarded-For` 첫 항목은 Cloudflare edge IP다(Cloudflare가 XFF를 보내지 않아 Traefik이 자기가 본 peer로 새로 만든다).
//
// 둘 다 프록시 IP라서, Cloudflare가 원 클라이언트 IP로 채워 주는 `CF-Connecting-IP`를 먼저 쓴다.
// 헤더는 위조할 수 있으므로 표시·감사 전용이다. 인가 판단에는 쓰지 않는다.
fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
}

/// 실제 접속 클라이언트 IP. 알 수 없으면 빈 문자열이다.
pub fn _client_ip(headers: &HeaderMap, client_host: Option<&str>) -> String {
    for name in ["CF-Connecting-IP", "True-Client-IP"] {
        let value = header_text(headers, name).trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    let forwarded = header_text(headers, "X-Forwarded-For");
    if !forwarded.is_empty() {
        let first_hop = forwarded.split(',').next().unwrap_or("").trim();
        if !first_hop.is_empty() {
            return first_hop.to_string();
        }
    }
    client_host.unwrap_or("").to_string()
}

/// 관측 로그용 IP 프리픽스. IPv4는 /24, IPv6는 /48까지만 남긴다.
pub fn _request_ip_prefix(headers: &HeaderMap, client_host: Option<&str>) -> String {
    let ip = _client_ip(headers, client_host);
    if ip.is_empty() {
        return String::new();
    }
    let (sep, keep) = if ip.contains(':') { (':', 3) } else { ('.', 3) };
    ip.split(sep).take(keep).collect::<Vec<_>>().join(&sep.to_string())
}

pub fn _request_user_agent(headers: &HeaderMap) -> String {
    header_text(headers, "User-Agent").to_string()
}

/// 로그인·갱신 때 세션 저장소에 남기는 접속 정보.
#[derive(Debug, Clone, Default)]
pub struct ClientInfo {
    pub ip: String,
    pub user_agent: String,
    pub ip_prefix: String,
}

impl ClientInfo {
    pub fn from_request(headers: &HeaderMap, client_host: Option<&str>) -> Self {
        Self { ip: _client_ip(headers, client_host), user_agent: _request_user_agent(headers), ip_prefix: _request_ip_prefix(headers, client_host) }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // peer 주소는 서버가 `into_make_service_with_connect_info::<SocketAddr>()`로 켜야 있다. 없으면(테스트) 모른다.
        let host = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_string());
        Ok(Self::from_request(&parts.headers, host.as_deref()))
    }
}
