//! 인증 라우트: Google 로그인, 토큰 갱신, 내 정보, 로그아웃, 로그인 세션 관리(admin).
//! Python `main.py`의 `/auth/*`에 대응한다. 라우트 파일의 작성 예시로도 쓴다(`main.rs` 머리말 참고).
//!
//! 로그에는 이메일·family_id·토큰·쿠키 원문을 남기지 않고 `observation_hash` 값만 남긴다.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use super::client_info::ClientInfo;
use super::cookies::{_clear_auth_cookies, _set_auth_cookies};
use super::deps::AppDeps;
use super::errors::{ApiError, ApiResult, json_response};
use super::extract::{ApiJson, ApiQuery, RequireAdmin, RequireAuth};
use super::routes::Routes;
use crate::auth::{ACCESS_TOKEN_EXPIRATION_SECONDS, REFRESH_COOKIE_NAME, REFRESH_TOKEN_EXPIRATION_SECONDS, RequestParts, observation_hash};
use crate::refresh_token_store::Rotation;

const GOOGLE_ISSUERS: [&str; 2] = ["accounts.google.com", "https://accounts.google.com"];

/// family_id는 `uuid4().hex`라서 32자 소문자 hex다.
const FAMILY_ID_LENGTH: usize = 32;
const SESSION_STATUS_FILTERS: [&str; 2] = ["active", "all"];
const MAX_SESSION_PAGE_SIZE: i64 = 100;

pub(super) fn routes() -> Routes {
    Routes::new()
        .post("/auth/google", &[], verify_google_token)
        .post("/auth/refresh", &[], refresh_access_token)
        .get("/auth/me", &["require_auth"], auth_me)
        .post("/auth/logout", &[], logout)
        .get("/auth/sessions", &["require_admin"], list_login_sessions)
        .delete("/auth/sessions/{session_id}", &["require_admin"], revoke_login_session)
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Python 진리값: None, 빈 문자열/목록/객체, 0, false는 거짓이다.
fn is_falsy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => !b,
        Some(Value::Number(n)) => n.as_f64() == Some(0.0),
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
    }
}

fn internal(context: &str, err: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(format!("{context}: {err}"))
}

/// 요청자의 refresh 쿠키에서 family_id를 얻는다. 없거나 유효하지 않으면 None.
fn current_refresh_family_id(deps: &AppDeps, headers: &HeaderMap) -> Option<String> {
    let token = headers.cookie(REFRESH_COOKIE_NAME).filter(|t| !t.is_empty())?;
    let payload = deps.tokens().decode_refresh_token(&token).ok()?;
    payload.get("fid").and_then(Value::as_str).map(str::to_string)
}

/// refresh 회전 거부 사건을 마스킹된 필드로만 기록한다.
///
/// `reuse-detected`가 같은 브라우저의 멀티탭 동시성에서 오는지, 다른 환경에서 복사된 상태에서 오는지 구분하는 관측 로그다.
/// UA/IP 해시는 진단 신호일 뿐이며 인가 판단에 쓰지 않는다. 원문 토큰·쿠키·user-agent·전체 IP·전체 이메일은 남기지 않는다.
fn log_refresh_rotation_rejected(deps: &AppDeps, client: &ClientInfo, status: &str, email: &str, family_id: &str, token_id: &str) {
    // 관측 실패가 refresh 응답을 막지 않아야 한다.
    let observation = deps.refresh_store().map_err(|e| e.to_string()).and_then(|s| s.get_token_observation(token_id).map_err(|e| e.to_string())).unwrap_or_else(|e| {
        log::debug!("refresh rotation observation lookup failed: {e}");
        None
    });
    let replaced = observation.as_ref().and_then(|o| o.get("replaced_by_present")).and_then(Value::as_bool).unwrap_or(false);
    log::warn!(
        "Refresh token rotation rejected event=refresh-rotation-rejected status={status} email_hash={} family_hash={} jti_hash={} replaced_by_present={replaced} request_user_agent_hash={} request_ip_prefix_hash={}",
        observation_hash(email),
        observation_hash(family_id),
        observation_hash(token_id),
        observation_hash(&client.user_agent),
        observation_hash(&client.ip_prefix),
    );
}

/// 접근·갱신 토큰을 만들고 갱신 토큰을 세션 저장소에 기록한다.
fn issue_auth_tokens(deps: &AppDeps, email: &str, role: &str, name: &str, picture: &str, client: &ClientInfo) -> ApiResult<(String, String)> {
    let issued_at = deps.clock().now() as i64;
    let tokens = deps.tokens();
    let refresh_token_id = tokens.new_id();
    let family_id = tokens.new_id();
    let access = tokens.create_access_token(email, role, name, picture);
    let refresh = tokens.create_refresh_token(email, role, name, picture, &family_id, &refresh_token_id);
    let store = deps.refresh_store().map_err(|e| internal("refresh token store", e))?;
    store
        .store_issued(&refresh_token_id, &family_id, email, issued_at, issued_at + REFRESH_TOKEN_EXPIRATION_SECONDS, Some(&client.ip), Some(&client.user_agent))
        .map_err(|e| internal("store_issued", e))?;
    Ok((access, refresh))
}

async fn verify_google_token(State(deps): State<Arc<AppDeps>>, client: ClientInfo, ApiJson(body): ApiJson<Value>) -> ApiResult<Response> {
    // `request_body: dict`: 객체가 아니면 FastAPI가 422로 거부한다.
    if !body.is_object() {
        return Err(super::errors::RequestValidationError::single(&["body"], "Input should be a valid dictionary", body.clone()).into());
    }
    let credential = body.get("credential");
    if is_falsy(credential) {
        return Err(ApiError::http(400, "Credential is required"));
    }
    let verifier = deps.google();
    let Some(client_id) = verifier.client_id() else {
        return Err(ApiError::http(500, "Google client ID is not configured"));
    };

    deps.task_runner().spawn_blocking_observed("verify_oauth2_token");
    let credential = credential.and_then(Value::as_str).unwrap_or("").to_string();
    let verified = tokio::task::spawn_blocking({
        let verifier = verifier.clone();
        move || verifier.verify(&credential, &client_id)
    })
    .await
    .map_err(|e| internal("verify_oauth2_token", e))?;
    let result = match verified {
        Ok(claims) => claims,
        Err(err) => {
            log::error!("Google token verification failed: {err}");
            return Err(ApiError::http(401, "Invalid Google token"));
        }
    };

    let issuer = str_of(&result, "iss");
    if !GOOGLE_ISSUERS.contains(&issuer) {
        log::error!("Google token issuer mismatch: {issuer}");
        return Err(ApiError::http(401, "Invalid token issuer"));
    }
    let email = str_of(&result, "email");
    if is_falsy(result.get("email_verified")) {
        log::warn!("Google login rejected because email is not verified: email_hash={}", observation_hash(email));
        return Err(ApiError::http(401, "Email is not verified"));
    }
    let (name, picture) = (str_of(&result, "name"), str_of(&result, "picture"));

    // 서버 쪽에서 역할을 정한다.
    let Some(role) = deps.role_resolver().determine_role(email) else {
        log::warn!("Unauthorized email login attempt: email_hash={}", observation_hash(email));
        return Err(ApiError::http(403, "Access denied"));
    };

    let (access, refresh) = issue_auth_tokens(&deps, email, &role, name, picture, &client)?;
    let mut response = json_response(200, &json!({"status": "success", "email": email, "name": name, "picture": picture, "role": role, "expires_in": ACCESS_TOKEN_EXPIRATION_SECONDS}));
    _set_auth_cookies(&mut response, &access, refresh.as_str());
    Ok(response)
}

async fn refresh_access_token(State(deps): State<Arc<AppDeps>>, headers: HeaderMap, client: ClientInfo) -> ApiResult<Response> {
    let Some(refresh_token) = headers.cookie(REFRESH_COOKIE_NAME).filter(|t| !t.is_empty()) else {
        return Err(ApiError::http(400, "Refresh token is required"));
    };
    let payload = deps.tokens().decode_refresh_token(&refresh_token)?;

    let email = str_of(&payload, "email");
    let Some(role) = deps.role_resolver().determine_role(email) else {
        return Err(ApiError::http(403, "Access denied"));
    };

    let current_token_id = str_of(&payload, "jti");
    let family_id = str_of(&payload, "fid");
    let (name, picture) = (str_of(&payload, "name"), str_of(&payload, "picture"));
    let issued_at = deps.clock().now() as i64;
    let new_token_id = deps.tokens().new_id();
    let rotated = deps.refresh_store().and_then(|store| {
        store.rotate(current_token_id, &new_token_id, family_id, email, issued_at, issued_at + REFRESH_TOKEN_EXPIRATION_SECONDS, Some(&client.ip), Some(&client.user_agent)).map_err(|e| e.to_string())
    });
    let rotation = match rotated {
        Ok(rotation) => rotation,
        Err(e) => {
            log::error!("refresh_token_store.rotate() failed for email_hash={}: {e}", observation_hash(email));
            return Err(ApiError::unavailable());
        }
    };
    if rotation != Rotation::Ok {
        log_refresh_rotation_rejected(&deps, &client, rotation.as_str(), email, family_id, current_token_id);
        let mut response = json_response(401, &json!({"detail": "Invalid refresh token state"}));
        _clear_auth_cookies(&mut response);
        return Ok(response);
    }

    let access = deps.tokens().create_access_token(email, &role, name, picture);
    let new_refresh = deps.tokens().create_refresh_token(email, &role, name, picture, family_id, &new_token_id);
    let mut response = json_response(200, &json!({"status": "success", "expires_in": ACCESS_TOKEN_EXPIRATION_SECONDS}));
    _set_auth_cookies(&mut response, &access, new_refresh.as_str());
    Ok(response)
}

async fn auth_me(State(deps): State<Arc<AppDeps>>, RequireAuth(payload): RequireAuth) -> Json<Value> {
    let exp = payload.get("exp").and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0);
    let remaining = (exp - deps.clock().now() as i64).max(0);
    Json(
        json!({"status": "success", "result": {"email": str_of(&payload, "email"), "role": str_of(&payload, "role"), "name": str_of(&payload, "name"), "picture": str_of(&payload, "picture"), "expires_in": remaining}}),
    )
}

async fn logout(State(deps): State<Arc<AppDeps>>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(refresh_token) = headers.cookie(REFRESH_COOKIE_NAME).filter(|t| !t.is_empty()) {
        match deps.tokens().decode_refresh_token(&refresh_token) {
            Ok(payload) => {
                let store = deps.refresh_store().map_err(|e| internal("refresh token store", e))?;
                store.revoke_family(str_of(&payload, "fid"), "logout").map_err(|e| internal("revoke_family", e))?;
            }
            Err(_) => log::debug!("Ignoring invalid refresh token during logout"),
        }
    }
    let mut response = json_response(200, &json!({"status": "success"}));
    _clear_auth_cookies(&mut response);
    Ok(response)
}

// === 로그인 세션 관리 API (admin 전용) ===

#[derive(Deserialize)]
struct SessionsQuery {
    page: Option<i64>,
    #[serde(rename = "pageSize")]
    page_size: Option<i64>,
    status: Option<String>,
    email: Option<String>,
}

fn out_of_range(name: &str, bound: &str) -> ApiError {
    super::errors::RequestValidationError::single(&["query", name], bound, Value::Null).into()
}

/// 서버 쪽 refresh 세션(family) 목록. jti·토큰 값은 노출하지 않는다.
async fn list_login_sessions(State(deps): State<Arc<AppDeps>>, RequireAdmin(_): RequireAdmin, ApiQuery(q): ApiQuery<SessionsQuery>, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(50);
    if page < 1 {
        return Err(out_of_range("page", "Input should be greater than or equal to 1"));
    }
    if !(1..=MAX_SESSION_PAGE_SIZE).contains(&page_size) {
        return Err(out_of_range("pageSize", "Input should be between 1 and 100"));
    }
    let status = q.status.unwrap_or_else(|| "active".to_string());
    if !SESSION_STATUS_FILTERS.contains(&status.as_str()) {
        return Err(ApiError::http(400, &format!("status must be one of: {}", SESSION_STATUS_FILTERS.join(", "))));
    }
    let query = crate::refresh_token_store::SessionQuery {
        status,
        current_family_id: current_refresh_family_id(&deps, &headers),
        page: page as u32,
        page_size: page_size as u32,
        email: q.email.filter(|e| !e.is_empty()),
    };
    let listed = deps.refresh_store().and_then(|store| store.list_sessions(&query).map_err(|e| e.to_string()));
    match listed {
        Ok(result) => Ok(Json(json!({"status": "success", "result": result}))),
        Err(e) => {
            log::error!("refresh_token_store.list_sessions() failed: {e}");
            Err(ApiError::unavailable())
        }
    }
}

fn is_family_id(value: &str) -> bool {
    value.len() == FAMILY_ID_LENGTH && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 세션(family) 하나를 폐기한다. 되돌릴 수 없으므로 단일 family로만 범위를 제한한다.
async fn revoke_login_session(State(deps): State<Arc<AppDeps>>, RequireAdmin(admin): RequireAdmin, Path(session_id): Path<String>, headers: HeaderMap) -> ApiResult<Response> {
    if !is_family_id(&session_id) {
        return Err(ApiError::http(400, "Invalid session id"));
    }
    let revoked = deps.refresh_store().and_then(|store| store.revoke_session(&session_id).map_err(|e| e.to_string()));
    let outcome = match revoked {
        Ok(outcome) => outcome,
        Err(e) => {
            log::error!("refresh_token_store.revoke_session() failed: {e}");
            return Err(ApiError::unavailable());
        }
    };
    if is_falsy(outcome.get("found")) {
        return Err(ApiError::http(404, "Session not found"));
    }

    let revoked_current = current_refresh_family_id(&deps, &headers).as_deref() == Some(session_id.as_str());
    // 관리자 행위 감사 로그. 관리자와 대상 세션은 해시로만 남긴다.
    log::warn!("Admin admin_hash={} revoked login session session_hash={} revoked_current={revoked_current}", observation_hash(str_of(&admin, "email")), observation_hash(&session_id));

    let session = outcome.get("session").filter(|s| !s.is_null()).cloned().unwrap_or_else(|| json!({}));
    let mut response = json_response(
        200,
        &json!({"status": "success", "result": {
            "session_id": session_id,
            "revoked": outcome.get("revoked").cloned().unwrap_or(Value::Null),
            "revoked_current": revoked_current,
            "status": session.get("status").and_then(Value::as_str).unwrap_or("revoked"),
            "revoke_reason": session.get("revoke_reason").cloned().unwrap_or(Value::Null),
        }}),
    );
    if revoked_current {
        // 본인 세션을 폐기했으면 쿠키도 지워 프런트가 미인증 흐름으로 넘어가게 한다.
        _clear_auth_cookies(&mut response);
    }
    Ok(response)
}
