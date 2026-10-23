//! 라우트 핸들러가 쓰는 추출기. 인증 의존성(`RequireAuth`, `RequireAdmin`, `OptionalAuth`)과 검증 오류를 FastAPI 형식으로 바꾸는 `ApiJson`, `ApiQuery`.
//!
//! 인증 추출기는 핸들러 인자 맨 앞에 둔다. FastAPI도 의존성을 입력 검증보다 먼저 풀어서, 인증 실패(401/403)가 검증 실패(422)보다 먼저 나간다.

use std::sync::Arc;

use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::header;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::deps::{AppDeps, AuthMode};
use super::errors::{ApiError, RequestValidationError};
use crate::auth::{HTTPException, optional_auth_with, require_admin_with, require_auth_with};

async fn resolve_auth(deps: &AppDeps, parts: &Parts, admin: bool) -> Result<Value, HTTPException> {
    let slot = |m: &std::sync::Mutex<Value>| m.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match (deps.auth_mode(), admin) {
        (AuthMode::Fixed(v), _) => Ok(v.clone()),
        (AuthMode::Split { auth, .. }, false) => Ok(auth.clone()),
        (AuthMode::Split { admin, .. }, true) => Ok(admin.clone()),
        (AuthMode::AuthOnly(v), false) => Ok(v.clone()),
        (AuthMode::Shared(slots), false) => Ok(slot(&slots.auth)),
        (AuthMode::Shared(slots), true) => Ok(slot(&slots.admin)),
        (AuthMode::Cookies, false) => require_auth_with(parts, deps.role_resolver()).await,
        (AuthMode::Cookies | AuthMode::AuthOnly(_), true) => require_admin_with(parts, deps.role_resolver()).await,
    }
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
        Ok(Self(optional_auth_with(parts, deps.role_resolver()).await))
    }
}

/// JSON 본문. 형식이 틀리면 FastAPI처럼 422(`RequestValidationError`)다.
pub struct ApiJson<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for ApiJson<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let is_json = req.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_none_or(|ct| {
            let ct = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            ct == "application/json" || ct.ends_with("+json")
        });
        let bytes = axum::body::Bytes::from_request(req, state).await.map_err(|e| RequestValidationError::single(&["body"], &e.to_string(), Value::Null))?;
        if !is_json {
            return Err(RequestValidationError::single(&["body"], "Input should be a valid JSON body", Value::Null).into());
        }
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
