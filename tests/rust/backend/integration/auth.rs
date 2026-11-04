//! 원본: tests/test_auth.py
//!
//! JWT 발급·검증, 권한 의존성(`require_auth`/`require_admin`), 쿠키 설정, Google OAuth 로그인, refresh 회전 관측 로그,
//! 접속 IP 추출, admin 로그인 세션 관리 API 테스트. Python의 autouse `setup_env` fixture는 `fixture()`(`common::env::EnvGuard`)로 옮겼다.
//! Python `monkeypatch.setattr(main_mod, ...)`는 `AppDeps`의 `with_google_verifier`/`with_role_resolver`/`with_refresh_token_store`로 옮겼다.
//! 토큰 발급 함수의 반환값을 고정해야 하는 `test_auth_refresh_and_logout`은 `AppDeps::with_token_issuer(Arc<dyn TokenIssuer>)` seam으로 풀었다.
//!
//! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.2, 3.4, 3.9, 3.10):
//! - `backend::auth_service` (환경 변수는 호출 시점에 읽는다. Python의 `importlib.reload` 불필요)
//!   - `determine_role(email: &str) -> Option<String>`
//!   - `create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String` (4인자 통일안. None은 빈 문자열)
//!   - `create_refresh_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String`
//!   - `decode_refresh_token(token: &str) -> Result<Value, HTTPException>`, `_extract_payload<R: RequestParts>(req: &R) -> Result<Value, HTTPException>`
//!   - `async require_auth<R: RequestParts>(req: &R) -> Result<Value, HTTPException>`, `async require_admin<R: RequestParts>(req: &R) -> Result<Value, HTTPException>`
//!   - `require_auth_with(req, resolver: &dyn RoleResolver)`, `require_admin_with(..)` (async, 같은 반환형. `monkeypatch.setattr(auth_mod, "determine_role", ..)` 대응)
//!   - `trait RequestParts { fn header(&self, name: &str) -> Option<String>; fn cookie(&self, name: &str) -> Option<String>; }`
//!   - `struct HTTPException { pub status_code: u16, pub detail: String }` + `HTTPException::new(u16, &str)`, `struct SystemExit(pub i32)`
//!   - `load_jwt_secret(env: EnvFn) -> Result<String, SystemExit>` (`EnvFn = &dyn Fn(&str) -> Option<String>`. Python `import backend.auth` 시 TM_JWT_SECRET 없으면 SystemExit)
//!   - `ACCESS_TOKEN_EXPIRATION_SECONDS: i64`
//! - `backend::main`
//!   - `app() -> axum::Router`, `app_with(deps: AppDeps) -> axum::Router`, `AppDeps::new(auth: AuthMode) -> AppDeps`
//!   - `AuthMode::{Cookies, Fixed(Value), Split { auth: Value, admin: Value }, AuthOnly(Value), Shared(..)}`: `require_auth`/`require_admin` dependency_overrides 대응
//!     (`AuthOnly`는 require_auth만 고정하고 require_admin은 실제 구현을 쓴다. 구 `Real`/`Overrides`는 `Cookies`/`Split`)
//!   - `AppDeps` 소비형 빌더: `.with_google_verifier(Arc<dyn GoogleVerifier>)`, `.with_role_resolver(Arc<dyn RoleResolver>)`, `.with_refresh_token_store(Arc<dyn RefreshTokenStoreApi>)`
//!   - `trait GoogleVerifier { fn client_id(&self) -> Option<String>; fn verify(&self, credential: &str, audience: &str) -> Result<Value, String>; }`
//!   - `trait RoleResolver { fn determine_role(&self, email: &str) -> Option<String>; }`
//!   - (모든 trait 메서드에 기본 구현이 있다. fake는 쓰는 메서드만 구현한다)
//!   - `trait TokenIssuer` (`backend::auth_service`, 모든 메서드에 기본 구현): `create_access_token`, `create_refresh_token`, `decode_refresh_token`, `new_id`. `AppDeps::with_token_issuer(Arc<dyn TokenIssuer>)`
//!   - `ACCESS_COOKIE_NAME`, `REFRESH_COOKIE_NAME: &str`, `observation_hash(&str) -> String`
//!   - `_get_cookie_settings() -> (bool, String)`, `_set_auth_cookies(&mut CustomJSONResponse, &str, &str)`, `_clear_auth_cookies(&mut CustomJSONResponse)`
//!   - `CustomJSONResponse::{new(Value) -> Self, headers(&self) -> &HeaderMap}`, `custom_jsonable_encoder(&Value) -> Value`
//!   - `_client_ip(headers: &HeaderMap, client_host: Option<&str>) -> String`, `_request_ip_prefix(headers: &HeaderMap, client_host: Option<&str>) -> String`
//!   - `async validation_exception_handler(&Parts, &RequestValidationError) -> Response`, `async http_exception_handler(&Parts, &HTTPException) -> Response`,
//!     `async general_exception_handler(&Parts, &(dyn Error + Send + Sync)) -> Response`, `RequestValidationError::new(Vec<Value>, Value /*body*/)`
//!   - `route_table() -> Vec<RouteInfo>`, `struct RouteInfo { path: String, methods: Vec<String>, dependencies: Vec<String> }` (FastAPI `route.dependant.dependencies` 대응)
//! - `backend::refresh_token_store`: `trait RefreshTokenStoreApi`(store_issued, rotate, revoke_family, get_token_observation, list_sessions, revoke_session),
//!   `SessionQuery { status, current_family_id, page, page_size, email }`, `StoreError::Unsupported(String)`
//! - 로그 캡처는 `common::log_capture::LogCapture`(`log` crate 기반), 환경 변수는 `common::env::EnvGuard`.
//! - dev-dependency: `jsonwebtoken`, `tokio`
//!
//! 토큰 관련 대역: `decode_refresh_token`/`create_*_token` 전역 덮어쓰기는 seam이 없다. refresh 쿠키는 `crafted_refresh()`가 실제 서명한 토큰으로 만든다.

#![allow(clippy::await_holding_lock)]

#[path = "../common/mod.rs"]
mod common;
use std::sync::{Arc};
use axum::Router;
use axum::http::{Method, Request};
use backend::auth_service::{ACCESS_TOKEN_EXPIRATION_SECONDS, HTTPException, TokenIssuer};
use backend::main::{ACCESS_COOKIE_NAME, app_with, AppDeps, AuthMode, GoogleVerifier, observation_hash, REFRESH_COOKIE_NAME, RequestValidationError, RoleResolver};
use backend::refresh_token_store::{RefreshTokenStoreApi};
use common::env::EnvGuard;
use common::log_capture::LogCapture;
use common::{Response, TestClient};
use serde_json::{json, Value};
use axum::body::Body;
use tower::ServiceExt;
use crate::common::shared_fixtures::{now};
use crate::common::shared_fixtures::{FakeStore, decode, encode_hs256, fixture, make_request};

fn access_token(email: &str, role: &str) -> String {
    backend::auth_service::create_jwt_token(email, role, None, None)
}

// ---- HTTP 응답 헬퍼 ----

fn set_cookies(resp: &Response) -> Vec<String> {
    resp.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.clone()).collect()
}

/// Python `resp.headers.get("set-cookie", "")`: 같은 헤더가 여러 개면 쉼표로 이어 붙인다.
fn set_cookie_joined(resp: &Response) -> String {
    set_cookies(resp).join(", ")
}

fn cookie_value(resp: &Response, name: &str) -> Option<String> {
    set_cookies(resp).iter().find_map(|c| c.strip_prefix(&format!("{name}="))).map(|rest| rest.split(';').next().unwrap_or("").to_string())
}

async fn post_empty(client: &TestClient, path: &str) -> Response {
    client.request(Method::POST, path, None, Vec::new()).await
}

/// Python `auth_client` fixture: require_auth/require_admin을 고정 payload로 바꾼 앱 (`AuthMode::Split`).
fn auth_deps() -> AppDeps {
    AppDeps::new(AuthMode::Split { auth: json!({"email": "user@example.com", "role": "user"}), admin: json!({"email": "admin@example.com", "role": "admin"}) })
}

fn auth_app(deps: AppDeps) -> Router {
    app_with(deps)
}

fn client(app: &Router) -> TestClient {
    TestClient::with_app(app.clone())
}

fn refresh_client(app: &Router, token: &str) -> TestClient {
    client(app).cookie(REFRESH_COOKIE_NAME, token)
}

/// `decode_refresh_token`을 가로채는 seam이 없으므로 같은 비밀키로 서명한 실제 refresh 토큰을 쓴다.
fn crafted_refresh(email: &str, fid: &str, jti: &str) -> String {
    encode_hs256(&json!({"type": "refresh", "email": email, "role": "user", "name": "n", "picture": "p", "fid": fid, "jti": jti, "iat": now(), "exp": now() + 3600}))
}

// TODO(common): FixedRole, FakeGoogle, FakeStore는 main_mock, view_history_api도 쓴다. common/auth_fakes.rs로 옮긴다.

/// `RoleResolver` fake. 어떤 이메일이든 같은 역할을 돌려준다 (구 `.determine_role(|_| ..)` 클로저).
struct FixedRole(Option<String>);

impl RoleResolver for FixedRole {
    fn determine_role(&self, _email: &str) -> Option<String> {
        self.0.clone()
    }
}

fn role(r: &str) -> Arc<FixedRole> {
    Arc::new(FixedRole(Some(r.to_string())))
}

fn no_role() -> Arc<FixedRole> {
    Arc::new(FixedRole(None))
}

/// `GoogleVerifier` fake. `client_id`와 검증 결과를 정한다 (구 `.google_client_id`, `.verify_google_token`).
type VerifyFn = dyn Fn(&str, &str) -> Result<Value, String> + Send + Sync;

struct FakeGoogle {
    client_id: Option<String>,
    verify: Box<VerifyFn>,
}

impl GoogleVerifier for FakeGoogle {
    fn client_id(&self) -> Option<String> {
        self.client_id.clone()
    }

    fn verify(&self, credential: &str, audience: &str) -> Result<Value, String> {
        (self.verify)(credential, audience)
    }
}

fn fake_google(client_id: &str, verify: impl Fn(&str, &str) -> Result<Value, String> + Send + Sync + 'static) -> Arc<FakeGoogle> {
    Arc::new(FakeGoogle { client_id: Some(client_id.to_string()), verify: Box::new(verify) })
}

/// `TokenIssuer` fake: 접근 토큰은 "jwt", 갱신 토큰은 "rjwt"로 고정하고, "rjwt"는 family1 토큰으로 해독한다.
/// 그 밖의 토큰은 실제 해독으로 넘긴다(앞쪽 요청은 `crafted_refresh`가 만든 실제 토큰을 쓴다).
struct FixedTokens;

impl TokenIssuer for FixedTokens {
    fn create_access_token(&self, _email: &str, _role: &str, _name: &str, _picture: &str, _family_id: &str) -> String {
        "jwt".to_string()
    }

    fn create_refresh_token(&self, _email: &str, _role: &str, _name: &str, _picture: &str, _family_id: &str, _token_id: &str) -> String {
        "rjwt".to_string()
    }

    fn decode_refresh_token(&self, token: &str) -> Result<Value, HTTPException> {
        if token == "rjwt" {
            return Ok(json!({"type": "refresh", "email": "e@example.com", "fid": "family1", "jti": "new-token"}));
        }
        backend::auth_service::decode_refresh_token(token)
    }
}
#[test]
fn test_default_name_and_picture() {
    let _env = fixture();
    let token = access_token("admin@example.com", "admin");
    let payload = decode(&token);
    assert_eq!(payload["name"], "");
    assert_eq!(payload["picture"], "");
}

// ========== Google 로그인 / refresh / logout ==========

fn valid_google_claims(audience: &str) -> Value {
    json!({"aud": audience, "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": true, "name": "N", "picture": "P"})
}

#[tokio::test]
async fn test_auth_google_success() {
    let _env = fixture();
    let deps =
        auth_deps().with_google_verifier(fake_google("cid", |_credential, audience| Ok(valid_google_claims(audience)))).with_role_resolver(role("user")).with_refresh_token_store(FakeStore::new());
    let app = auth_app(deps);

    let resp = client(&app).post_json("/auth/google", &json!({"credential": "c"})).await;
    assert_eq!(resp.status, 200);
    let data = resp.json();
    assert_eq!(data["status"], "success");
    // 선제적 갱신을 위한 expires_in 필드 포함 확인
    assert!(data.get("expires_in").is_some());
    assert_eq!(data["expires_in"], json!(ACCESS_TOKEN_EXPIRATION_SECONDS));
}

#[tokio::test]
async fn test_auth_google_errors() {
    let _env = fixture();
    let resp = TestClient::with_app(auth_app(auth_deps())).post_json("/auth/google", &json!({})).await;
    assert_eq!(resp.status, 400);

    let deps = auth_deps().with_google_verifier(fake_google("cid", |_credential, _audience| Err("nope".to_string())));
    let resp = TestClient::with_app(auth_app(deps)).post_json("/auth/google", &json!({"credential": "c"})).await;
    assert_eq!(resp.status, 401);

    let deps =
        auth_deps().with_google_verifier(fake_google("cid", |_credential, _audience| Ok(json!({"aud": "cid", "iss": "https://evil.example.com", "email": "e@example.com", "email_verified": true}))));
    let resp = TestClient::with_app(auth_app(deps)).post_json("/auth/google", &json!({"credential": "c"})).await;
    assert_eq!(resp.status, 401);

    let deps = auth_deps()
        .with_google_verifier(fake_google("cid", |_credential, _audience| Ok(json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": false}))));
    let resp = TestClient::with_app(auth_app(deps)).post_json("/auth/google", &json!({"credential": "c"})).await;
    assert_eq!(resp.status, 401);

    let deps = auth_deps()
        .with_google_verifier(fake_google("cid", |_credential, _audience| Ok(json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "e@example.com", "email_verified": true}))))
        .with_role_resolver(no_role());
    let resp = TestClient::with_app(auth_app(deps)).post_json("/auth/google", &json!({"credential": "c"})).await;
    assert_eq!(resp.status, 403);
}

#[tokio::test]
async fn test_auth_refresh_and_logout() {
    let _env = fixture();
    // 쿠키 없는 refresh는 400
    let resp = post_empty(&TestClient::with_app(auth_app(auth_deps())), "/auth/refresh").await;
    assert_eq!(resp.status, 400);

    let store = FakeStore::rotating("ok");
    // `AppDeps::with_token_issuer`가 Python의 `create_jwt_token`/`create_refresh_token`/`decode_refresh_token` 덮어쓰기를 대신한다.
    let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(store.clone()).with_token_issuer(Arc::new(FixedTokens));
    let app = auth_app(deps);
    let token = crafted_refresh("e@example.com", "family1", "old-token");

    let resp = post_empty(&refresh_client(&app, &token), "/auth/refresh").await;
    assert_eq!(resp.status, 200);
    // sliding expiration: refresh token도 새로 발급되어야 함
    assert!(cookie_value(&resp, REFRESH_COOKIE_NAME).is_some());
    assert_eq!(cookie_value(&resp, REFRESH_COOKIE_NAME).as_deref(), Some("rjwt"));
    // 선제적 갱신을 위한 expires_in 필드 포함 확인
    let refresh_data = resp.json();
    assert!(refresh_data.get("expires_in").is_some());
    assert_eq!(refresh_data["expires_in"], json!(ACCESS_TOKEN_EXPIRATION_SECONDS));

    // Python TestClient는 응답의 Set-Cookie를 쿠키 저장소에 보관한다. 같은 동작을 직접 옮긴다.
    let jar = format!("{ACCESS_COOKIE_NAME}={}; {REFRESH_COOKIE_NAME}={}", cookie_value(&resp, ACCESS_COOKIE_NAME).unwrap_or_default(), cookie_value(&resp, REFRESH_COOKIE_NAME).unwrap());

    let resp = client(&app).header("cookie", &jar).get("/auth/me").await;
    assert_eq!(resp.status, 200);
    // /auth/me도 expires_in(잔여 초) 반환
    let me_data = resp.json();
    assert!(me_data["result"].get("expires_in").is_some());
    assert!(me_data["result"]["expires_in"].is_i64() || me_data["result"]["expires_in"].is_u64());

    // 저장소 쿠키에 refresh 토큰이 남아 있으므로 쿠키를 직접 주지 않아도 family가 폐기된다.
    let resp = post_empty(&client(&app).header("cookie", &jar), "/auth/logout").await;
    assert_eq!(resp.status, 200);
    assert_eq!(*store.revoked_families.lock().unwrap(), vec![("family1".to_string(), "logout".to_string())]);

    // logout 응답이 쿠키를 지웠으므로 저장소는 비어 있고, 이번에는 쿠키를 명시한다.
    let resp = post_empty(&refresh_client(&app, &token), "/auth/logout").await;
    assert_eq!(resp.status, 200);
    assert_eq!(*store.revoked_families.lock().unwrap(), vec![("family1".to_string(), "logout".to_string()), ("family1".to_string(), "logout".to_string())]);
}

#[tokio::test]
async fn test_auth_refresh_rejected_token_state_clears_cookies() {
    let _env = fixture();
    let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(FakeStore::rotating("reused"));
    let app = auth_app(deps);
    let token = crafted_refresh("e@example.com", "family1", "old-token");

    let resp = post_empty(&refresh_client(&app, &token), "/auth/refresh").await;

    assert_eq!(resp.status, 401);
    assert_eq!(resp.json()["detail"], "Invalid refresh token state");
    let set_cookie = set_cookie_joined(&resp);
    assert!(set_cookie.contains(&format!("{ACCESS_COOKIE_NAME}=")), "{set_cookie}");
    assert!(set_cookie.contains(&format!("{REFRESH_COOKIE_NAME}=")), "{set_cookie}");
    assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");
}

#[tokio::test]
async fn test_logout_ignores_invalid_refresh_token() {
    let _env = fixture();
    let store = FakeStore::new();
    // 서명이 맞지 않는 쿠키는 실제 `decode_refresh_token`이 401로 거부한다.
    let deps = auth_deps().with_refresh_token_store(store.clone());
    let app = auth_app(deps);

    let resp = post_empty(&refresh_client(&app, "bad"), "/auth/logout").await;

    assert_eq!(resp.status, 200);
    assert!(store.revoked_families.lock().unwrap().is_empty());
    let set_cookie = set_cookie_joined(&resp);
    assert!(set_cookie.contains(&format!("{ACCESS_COOKIE_NAME}=")), "{set_cookie}");
    assert!(set_cookie.contains(&format!("{REFRESH_COOKIE_NAME}=")), "{set_cookie}");
    assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");
}

#[tokio::test]
async fn test_auth_me_expires_in_with_real_token() {
    // 실제 JWT로 /auth/me 호출 시 expires_in이 남은 초를 정확히 반환하는지 검증.
    let _env = fixture();
    let store = FakeStore::new();
    store.store_issued("token", &family_a(), "admin@example.com", now(), now() + 3600, None, None).unwrap();
    let token = backend::auth_service::create_session_jwt_token("admin@example.com", "admin", "A", "P", &family_a());

    // 인증을 고정하지 않고(`AuthMode::Cookies`) 실제 토큰으로 호출
    let app = auth_app(AppDeps::new(AuthMode::Cookies).with_role_resolver(role("admin")).with_refresh_token_store(store));
    let resp = client(&app).cookie(ACCESS_COOKIE_NAME, &token).get("/auth/me").await;
    assert_eq!(resp.status, 200);
    let data = resp.json();
    let expires_in = data["result"]["expires_in"].as_i64().unwrap();
    // 방금 생성한 토큰이므로 남은 시간은 만료 시간에 근접해야 함 (±5초 오차 허용)
    assert!((ACCESS_TOKEN_EXPIRATION_SECONDS - 5..=ACCESS_TOKEN_EXPIRATION_SECONDS).contains(&expires_in), "expires_in={expires_in}");
}

// ========== Admin 로그인 세션 관리 API ==========

fn family_a() -> String {
    "a".repeat(32)
}

fn family_b() -> String {
    "b".repeat(32)
}

fn session_item(family_id: &str, overrides: &[(&str, Value)]) -> Value {
    let mut item = json!({
        "session_id": family_id,
        "session_label": format!("{}...", &family_id[..8]),
        "email": "admin@example.com",
        "status": "active",
        "created_at": 1786500000,
        "last_seen_at": 1786501200,
        "expires_at": 1787106000,
        "revoked_at": null,
        "revoke_reason": null,
        "token_count": 6,
        "valid_token_count": 1,
        "is_current": false
    });
    for (k, v) in overrides {
        item[*k] = v.clone();
    }
    item
}

fn session_page(items: Vec<Value>) -> Value {
    let n = items.len();
    json!({"items": items, "pagination": {"page": 1, "pageSize": 50, "totalItems": n, "totalPages": 1}, "summary": {"active": n, "expired": 0, "revoked": 0, "total": n}})
}
#[tokio::test]
async fn test_returns_session_page() {
    let _env = fixture();
    let store = FakeStore::listing(Ok(session_page(vec![session_item(&family_a(), &[("is_current", json!(true))])])));
    let deps = auth_deps().with_refresh_token_store(store.clone());

    let resp = client(&auth_app(deps)).get("/auth/sessions").await;

    assert_eq!(resp.status, 200);
    let body = resp.json();
    assert_eq!(body["status"], "success");
    assert_eq!(body["result"]["items"][0]["is_current"], json!(true));
    assert_eq!(body["result"]["summary"]["total"], json!(1));
    // 기본값: 활성만, 1페이지 50건
    let captured = store.list_calls.lock().unwrap().last().cloned().unwrap_or(Value::Null);
    assert_eq!(captured["status"], "active");
    assert_eq!(captured["page"], json!(1));
    assert_eq!(captured["page_size"], json!(50));
}

#[tokio::test]
async fn test_passes_query_params_through() {
    let _env = fixture();
    let store = FakeStore::listing(Ok(session_page(vec![])));
    let deps = auth_deps().with_refresh_token_store(store.clone());

    let resp = client(&auth_app(deps)).get("/auth/sessions?page=2&pageSize=10&status=all&email=viewer%40example.com").await;

    assert_eq!(resp.status, 200);
    let captured = store.list_calls.lock().unwrap().last().cloned().unwrap_or(Value::Null);
    assert_eq!(captured["page"], json!(2));
    assert_eq!(captured["page_size"], json!(10));
    assert_eq!(captured["status"], "all");
    assert_eq!(captured["email"], "viewer@example.com");
}

#[tokio::test]
async fn test_marks_requester_session_as_current() {
    let _env = fixture();
    let store = FakeStore::listing(Ok(session_page(vec![])));
    let deps = auth_deps().with_refresh_token_store(store.clone());
    let token = crafted_refresh("e@example.com", &family_a(), "j");

    refresh_client(&auth_app(deps), &token).get("/auth/sessions").await;

    assert_eq!(store.list_calls.lock().unwrap().last().unwrap()["current_family_id"], json!(family_a()));
}

#[tokio::test]
async fn test_invalid_refresh_cookie_is_ignored() {
    let _env = fixture();
    let store = FakeStore::listing(Ok(session_page(vec![])));
    let deps = auth_deps().with_refresh_token_store(store.clone());

    let resp = refresh_client(&auth_app(deps), "garbage").get("/auth/sessions").await;

    // 잘못된 refresh 쿠키가 관리자 목록 조회를 막지 않는다
    assert_eq!(resp.status, 200);
    assert!(store.list_calls.lock().unwrap().last().unwrap()["current_family_id"].is_null());
}

#[tokio::test]
async fn test_rejects_unknown_status_filter() {
    let _env = fixture();
    let deps = auth_deps().with_refresh_token_store(FakeStore::listing(Ok(session_page(vec![]))));

    let resp = client(&auth_app(deps)).get("/auth/sessions?status=bogus").await;

    assert_eq!(resp.status, 400);
    assert!(resp.json()["detail"].as_str().unwrap().contains("status"));
}

#[tokio::test]
async fn test_rejects_out_of_range_pagination() {
    let _env = fixture();
    let deps = auth_deps().with_refresh_token_store(FakeStore::listing(Ok(session_page(vec![]))));
    let app = auth_app(deps);

    assert_eq!(client(&app).get("/auth/sessions?page=0").await.status, 422);
    assert_eq!(client(&app).get("/auth/sessions?pageSize=0").await.status, 422);
    // pageSize 상한 100
    assert_eq!(client(&app).get("/auth/sessions?pageSize=101").await.status, 422);
    assert_eq!(client(&app).get("/auth/sessions?pageSize=100").await.status, 200);
}

#[tokio::test]
async fn test_store_failure_returns_503() {
    let _env = fixture();
    let deps = auth_deps().with_refresh_token_store(FakeStore::listing(Err("db down".to_string())));

    let resp = client(&auth_app(deps)).get("/auth/sessions").await;

    assert_eq!(resp.status, 503);
}

#[tokio::test]
async fn test_response_excludes_token_internals() {
    let _env = fixture();
    let deps = auth_deps().with_refresh_token_store(FakeStore::listing(Ok(session_page(vec![session_item(&family_a(), &[])]))));

    let body = client(&auth_app(deps)).get("/auth/sessions").await.text();

    assert!(!body.contains("jti"));
    assert!(!body.contains("replaced_by"));
    assert!(!body.contains(REFRESH_COOKIE_NAME));
    assert!(!body.contains(ACCESS_COOKIE_NAME));
}

mod test_revoke_login_session {
    use super::*;

    #[tokio::test]
    async fn test_revokes_one_family() {
        let _env = fixture();
        let store = FakeStore::revoking(Ok(json!({"found": true, "revoked": true, "session": session_item(&family_a(), &[("status", json!("revoked")), ("revoke_reason", json!("admin-revoked"))])})));
        let deps = auth_deps().with_refresh_token_store(store.clone());

        let resp = client(&auth_app(deps)).delete(&format!("/auth/sessions/{}", family_a())).await;

        assert_eq!(resp.status, 200);
        let result = resp.json()["result"].clone();
        assert_eq!(store.revoke_session_calls.lock().unwrap().last().unwrap()["family_id"], json!(family_a()));
        assert_eq!(result["revoked"], json!(true));
        assert_eq!(result["revoked_current"], json!(false));
        assert_eq!(result["status"], "revoked");
        assert_eq!(result["revoke_reason"], "admin-revoked");
        // 본인 세션이 아니면 쿠키를 지우지 않는다
        assert!(!set_cookie_joined(&resp).contains("Max-Age=0"));
    }

    #[tokio::test]
    async fn test_revoking_current_session_clears_cookies() {
        let _env = fixture();
        let store = FakeStore::revoking(Ok(json!({"found": true, "revoked": true, "session": session_item(&family_a(), &[("status", json!("revoked")), ("revoke_reason", json!("admin-revoked"))])})));
        let deps = auth_deps().with_refresh_token_store(store);
        let token = crafted_refresh("e@example.com", &family_a(), "j");

        let resp = refresh_client(&auth_app(deps), &token).delete(&format!("/auth/sessions/{}", family_a())).await;

        assert_eq!(resp.status, 200);
        assert_eq!(resp.json()["result"]["revoked_current"], json!(true));
        let set_cookie = set_cookie_joined(&resp);
        assert!(set_cookie.contains(&format!("{ACCESS_COOKIE_NAME}=")), "{set_cookie}");
        assert!(set_cookie.contains(&format!("{REFRESH_COOKIE_NAME}=")), "{set_cookie}");
        assert!(set_cookie.contains("Max-Age=0"), "{set_cookie}");
    }

    #[tokio::test]
    async fn test_already_inactive_family_still_succeeds() {
        let _env = fixture();
        let deps = auth_deps().with_refresh_token_store(FakeStore::revoking(Ok(
            json!({"found": true, "revoked": false, "session": session_item(&family_a(), &[("status", json!("revoked")), ("revoke_reason", json!("logout"))])}),
        )));

        let resp = client(&auth_app(deps)).delete(&format!("/auth/sessions/{}", family_b())).await;

        assert_eq!(resp.status, 200);
        let result = resp.json()["result"].clone();
        assert_eq!(result["revoked"], json!(false));
        assert_eq!(result["status"], "revoked");
        assert_eq!(result["revoke_reason"], "logout");
    }

    #[tokio::test]
    async fn test_unknown_family_returns_404() {
        let _env = fixture();
        let deps = auth_deps().with_refresh_token_store(FakeStore::revoking(Ok(json!({"found": false, "revoked": false, "session": null}))));

        let resp = client(&auth_app(deps)).delete(&format!("/auth/sessions/{}", family_b())).await;

        assert_eq!(resp.status, 404);
    }

    #[tokio::test]
    async fn test_malformed_session_id_returns_400() {
        let _env = fixture();
        let store = FakeStore::revoking(Ok(json!({"found": true, "revoked": true, "session": session_item(&family_a(), &[])})));
        let deps = auth_deps().with_refresh_token_store(store.clone());
        let app = auth_app(deps);

        for bad in ["short".to_string(), "A".repeat(32), "z".repeat(32), "a".repeat(31), "a".repeat(33)] {
            let resp = client(&app).delete(&format!("/auth/sessions/{bad}")).await;
            assert_eq!(resp.status, 400, "{bad}");
        }
        // 형식 검증 실패 시 store 를 건드리지 않는다
        assert!(store.revoke_session_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_store_failure_returns_503() {
        let _env = fixture();
        let deps = auth_deps().with_refresh_token_store(FakeStore::revoking(Err("db down".to_string())));

        assert_eq!(client(&auth_app(deps)).delete(&format!("/auth/sessions/{}", family_a())).await.status, 503);
    }

    #[tokio::test]
    async fn test_audit_log_masks_target_session() {
        let _env = fixture();
        let deps = auth_deps().with_refresh_token_store(FakeStore::revoking(Ok(json!({"found": true, "revoked": true, "session": session_item(&family_a(), &[("status", json!("revoked"))])}))));
        let app = auth_app(deps);

        let logs = LogCapture::start(log::Level::Warn);
        client(&app).delete(&format!("/auth/sessions/{}", family_a())).await;

        let text = logs.text();
        assert!(text.contains(&format!("session_hash={}", observation_hash(&family_a()))), "{text}");
        // 대상 family_id 원문은 남기지 않는다
        assert!(!text.contains(&family_a()), "{text}");
    }
}

/// 세션 관리 엔드포인트는 admin 전용이다.
#[tokio::test]
async fn test_viewer_is_forbidden() {
    let _env = fixture();
    // require_admin은 고정하지 않았으므로(`AuthOnly`) 실제 의존성이 동작한다
    let app = auth_app(AppDeps::new(AuthMode::AuthOnly(json!({"email": "viewer@example.com", "role": "viewer"}))));

    let status = client(&app).get("/auth/sessions").await.status;
    assert!(status == 401 || status == 403, "GET /auth/sessions -> {status}");
    let status = client(&app).delete(&format!("/auth/sessions/{}", family_a())).await.status;
    assert!(status == 401 || status == 403, "DELETE /auth/sessions/<id> -> {status}");
}

#[tokio::test]
async fn test_unauthenticated_is_rejected() {
    let _env = fixture();
    let c = TestClient::new();
    assert_eq!(c.get("/auth/sessions").await.status, 401);
    assert_eq!(c.delete(&format!("/auth/sessions/{}", family_a())).await.status, 401);
}

/// 엔드포인트가 require_admin 의존성을 명시적으로 갖는지 확인한다.
#[test]
fn test_endpoints_declare_admin_dependency() {
    let _env = fixture();
    let routes: Vec<backend::main::RouteInfo> = backend::main::route_table().into_iter().filter(|r| r.path.starts_with("/auth/sessions")).collect();
    assert!(!routes.is_empty(), "세션 관리 라우트를 찾지 못했습니다");
    for route in routes {
        assert!(route.dependencies.iter().any(|d| d == "require_admin"), "{:?} {} 에 require_admin 이 없습니다", route.methods, route.path);
    }
}

/// 로그인·갱신 시점의 실제 접속 IP 와 User-Agent 가 세션 저장소까지 전달되는지 본다.
const REAL_IP: &str = "203.0.113.77";

const PROXY_IP: &str = "104.22.17.33";

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn with_client_headers(c: TestClient) -> TestClient {
    c.header("CF-Connecting-IP", REAL_IP).header("X-Forwarded-For", PROXY_IP).header("User-Agent", UA)
}

#[tokio::test]
async fn test_google_login_records_real_client_ip() {
    let _env = fixture();
    let store = FakeStore::new();
    let deps =
        auth_deps().with_google_verifier(fake_google("cid", |_credential, audience| Ok(valid_google_claims(audience)))).with_role_resolver(role("user")).with_refresh_token_store(store.clone());

    let resp = with_client_headers(client(&auth_app(deps))).post_json("/auth/google", &json!({"credential": "c"})).await;

    assert_eq!(resp.status, 200);
    let captured = store.issued.lock().unwrap().last().cloned().unwrap_or(Value::Null);
    assert_eq!(captured["client_ip"], REAL_IP);
    assert_ne!(captured["client_ip"], PROXY_IP);
    assert_eq!(captured["user_agent"], UA);
}

#[tokio::test]
async fn test_refresh_updates_client_info() {
    // 세션의 IP/UA 는 마지막 갱신 값이므로 refresh 마다 다시 전달돼야 한다.
    let _env = fixture();
    let store = FakeStore::rotating("ok");
    let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(store.clone());
    let token = crafted_refresh("e@example.com", "F", "J");

    let resp = post_empty(&with_client_headers(refresh_client(&auth_app(deps), &token)), "/auth/refresh").await;

    assert_eq!(resp.status, 200);
    let captured = store.rotated.lock().unwrap().last().cloned().unwrap_or(Value::Null);
    assert_eq!(captured["client_ip"], REAL_IP);
    assert_eq!(captured["user_agent"], UA);
}

#[tokio::test]
async fn test_sessions_api_exposes_client_info() {
    let _env = fixture();
    let session = json!({
        "session_id": "a".repeat(32),
        "session_label": "aaaaaaaa...",
        "email": "admin@example.com",
        "status": "active",
        "created_at": 1,
        "last_seen_at": 2,
        "expires_at": 3,
        "revoked_at": null,
        "revoke_reason": null,
        "token_count": 1,
        "valid_token_count": 1,
        "client_ip": REAL_IP,
        "user_agent": UA,
        "user_agent_summary": "Chrome 131 / macOS 10.15",
        "is_current": false
    });
    let page = json!({"items": [session], "pagination": {"page": 1, "pageSize": 50, "totalItems": 1, "totalPages": 1}, "summary": {"active": 1, "expired": 0, "revoked": 0, "total": 1}});
    let deps = auth_deps().with_refresh_token_store(FakeStore::listing(Ok(page)));

    let item = client(&auth_app(deps)).get("/auth/sessions").await.json()["result"]["items"][0].clone();

    assert_eq!(item["client_ip"], REAL_IP);
    assert_eq!(item["user_agent"], UA);
    assert_eq!(item["user_agent_summary"], "Chrome 131 / macOS 10.15");
}

// 실제 서명·인증과 상태를 보관하는 저장소 대역으로 세션 폐기를 검증한다.
async fn session_login(app: &Router) -> (String, String, String) {
    let response = client(app).post_json("/auth/google", &json!({"credential":"valid"})).await;
    assert_eq!(response.status, 200);
    let access = cookie_value(&response, ACCESS_COOKIE_NAME).unwrap();
    let refresh = cookie_value(&response, REFRESH_COOKIE_NAME).unwrap();
    let fid = backend::auth_service::decode_refresh_token(&refresh).unwrap()["fid"].as_str().unwrap().to_string();
    (access, refresh, fid)
}

fn session_app(store: Arc<FakeStore>) -> Router {
    auth_app(
        AppDeps::new(AuthMode::Cookies)
            .with_refresh_token_store(store)
            .with_google_verifier(fake_google("cid", |_, _| Ok(json!({"iss":"https://accounts.google.com","email":"admin@example.com","email_verified":true})))),
    )
}

#[tokio::test]
async fn test_session_access_revoked_after_logout() {
    let _env = fixture();
    let app = session_app(FakeStore::new());
    let (access, refresh, _) = session_login(&app).await;
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &access).get("/auth/me").await.status, 200);
    let response = post_empty(&client(&app).cookie(REFRESH_COOKIE_NAME, &refresh), "/auth/logout").await;
    assert_eq!(response.status, 200);
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &access).get("/auth/me").await.status, 401);
    assert_eq!(client(&app).header("authorization", &format!("Bearer {access}")).get("/auth/me").await.status, 401);
}

#[tokio::test]
async fn test_session_access_admin_revocation_is_scoped_to_one_family() {
    let _env = fixture();
    let store = FakeStore::revoking(Ok(json!({"found":true,"revoked":true,"session":{"status":"revoked"}})));
    let app = session_app(store);
    let (first, _, fid) = session_login(&app).await;
    let (second, _, _) = session_login(&app).await;
    let response = client(&app).cookie(ACCESS_COOKIE_NAME, &second).delete(&format!("/auth/sessions/{fid}")).await;
    assert_eq!(response.status, 200);
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &first).get("/auth/me").await.status, 401);
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &second).get("/auth/me").await.status, 200);
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &first).get("/auth/sessions").await.status, 401);
}

#[tokio::test]
async fn test_session_access_rejects_unbound_missing_and_mismatched_sessions() {
    let _env = fixture();
    let app = session_app(FakeStore::new());
    let (_, _, fid) = session_login(&app).await;
    for family in [Value::Null, json!(""), json!(family_a()), json!(fid)] {
        let email = if family == json!(fid) { "viewer1@example.com" } else { "admin@example.com" };
        let role = if email.starts_with("viewer") { "viewer" } else { "admin" };
        let token = encode_hs256(&json!({"type":"access","email":email,"role":role,"fid":family,"iat":now(),"exp":now()+3600}));
        assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &token).get("/auth/me").await.status, 401);
    }
}

#[tokio::test]
async fn test_session_access_fails_closed_when_store_is_unavailable() {
    let _env = fixture();
    let store = FakeStore::new();
    let app = session_app(store.clone());
    let (access, _, _) = session_login(&app).await;
    *store.sessions.lock().unwrap() = Some(Err("database unavailable".into()));
    let response = client(&app).cookie(ACCESS_COOKIE_NAME, &access).get("/auth/me").await;
    assert_eq!(response.status, 503);
    assert!(!response.json().to_string().contains("database unavailable"));
}

#[tokio::test]
async fn test_session_access_logout_without_refresh_cookie_revokes_session() {
    let _env = fixture();
    let app = session_app(FakeStore::new());
    let (access, _, _) = session_login(&app).await;
    let response = post_empty(&client(&app).cookie(ACCESS_COOKIE_NAME, &access), "/auth/logout").await;
    assert_eq!(response.status, 200);
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &access).get("/auth/me").await.status, 401);
}

#[tokio::test]
async fn test_session_access_refresh_keeps_family_and_prior_access_valid() {
    let _env = fixture();
    let app = session_app(FakeStore::new());
    let (access, refresh, fid) = session_login(&app).await;
    let response = post_empty(&client(&app).cookie(REFRESH_COOKIE_NAME, &refresh), "/auth/refresh").await;
    assert_eq!(response.status, 200);
    let next_access = cookie_value(&response, ACCESS_COOKIE_NAME).unwrap();
    let payload = backend::auth_service::_extract_payload(&make_request(Some(&next_access), None)).unwrap();
    assert_eq!(payload["fid"], fid);
    for token in [access, next_access] {
        assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &token).get("/auth/me").await.status, 200);
    }
}

#[tokio::test]
async fn test_session_access_logout_with_invalid_refresh_cookie_revokes_session() {
    let _env = fixture();
    let app = session_app(FakeStore::new());
    let (access, _, _) = session_login(&app).await;
    let cookies = format!("{ACCESS_COOKIE_NAME}={access}; {REFRESH_COOKIE_NAME}=invalid");
    assert_eq!(post_empty(&client(&app).header("cookie", &cookies), "/auth/logout").await.status, 200);
    assert_eq!(client(&app).cookie(ACCESS_COOKIE_NAME, &access).get("/auth/me").await.status, 401);
}

#[tokio::test]
async fn test_session_access_optional_auth_treats_revoked_token_as_anonymous() {
    let _env = fixture();
    let app = session_app(FakeStore::new());
    let (access, refresh, _) = session_login(&app).await;
    post_empty(&client(&app).cookie(REFRESH_COOKIE_NAME, &refresh), "/auth/logout").await;
    let logs = LogCapture::start(log::Level::Error);
    let response = client(&app).cookie(ACCESS_COOKIE_NAME, &access).post_json("/logs/client-error", &json!({"error_type":"CUSTOM_ERROR","message":"test","url":"/"})).await;
    assert_eq!(response.status, 200);
    assert!(logs.text().contains("user=anonymous(anonymous)"));
}

const SOURCE: &str = "https://app.example.com";

const ATTACKER: &str = "https://evil.example.com";

fn setup() -> EnvGuard {
    common::setup_env();
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", SOURCE);
    env
}

// Origin 자동 주입이 없는 실제 HTTP 요청으로 차단 여부를 검사한다.
async fn request(app: &Router, method: Method, path: &str, headers: &[(&str, &str)], body: &str) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.clone().oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap()
}

#[tokio::test]
async fn cookie_mutations_reject_untrusted_or_missing_source_before_side_effects() {
    let _env = setup();
    let store = FakeStore::new();
    let app = session_app(store.clone());
    let (access, refresh, _) = session_login(&app).await;
    let cookies = format!("{ACCESS_COOKIE_NAME}={access}; {REFRESH_COOKIE_NAME}={refresh}");
    for source in [ATTACKER, "null", "https://app.example.com.evil.test", "http://app.example.com", "https://app.example.com:8443"] {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            let response = request(&app, method, "/auth/logout", &[("cookie", &cookies), ("origin", source)], "").await;
            assert_eq!(response.status(), 403, "source={source}");
        }
    }
    for headers in [
        vec![("cookie", cookies.as_str())],
        vec![("cookie", cookies.as_str()), ("referer", "https://app.example.com.evil.test/path")],
        vec![("cookie", cookies.as_str()), ("referer", "https://app.example.com@evil.test/path")],
        vec![("cookie", cookies.as_str()), ("origin", ATTACKER), ("referer", "https://app.example.com/path")],
        vec![("cookie", cookies.as_str()), ("origin", SOURCE), ("origin", ATTACKER)],
        vec![("cookie", cookies.as_str()), ("referer", "https://app.example.com/path"), ("referer", "https://evil.example.com/path")],
        vec![("cookie", cookies.as_str()), ("authorization", "Bearer invalid")],
    ] {
        assert_eq!(request(&app, Method::POST, "/auth/logout", &headers, "").await.status(), 403);
    }
    assert_eq!(request(&app, Method::POST, "/auth/refresh", &[("cookie", &cookies), ("origin", ATTACKER)], "").await.status(), 403);
    assert!(store.revoked_families.lock().unwrap().is_empty());
    assert!(store.rotated.lock().unwrap().is_empty());
    assert_eq!(request(&app, Method::GET, "/auth/me", &[("cookie", &cookies), ("origin", ATTACKER)], "").await.status(), 200);
}

#[tokio::test]
async fn trusted_origin_or_referer_allows_cookie_mutations() {
    let _env = setup();
    let store = FakeStore::new();
    let app = session_app(store.clone());
    for source in [vec![("origin", SOURCE)], vec![("referer", "https://app.example.com/reader?book=1")]] {
        let (_, refresh, _) = session_login(&app).await;
        let cookies = format!("{REFRESH_COOKIE_NAME}={refresh}");
        let mut headers = source;
        headers.push(("cookie", &cookies));
        let response = request(&app, Method::POST, "/auth/logout", &headers, "").await;
        assert_eq!(response.status(), 200);
    }
    assert_eq!(store.revoked_families.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn cookie_requests_fail_closed_without_frontend_configuration() {
    let mut env = setup();
    let store = FakeStore::new();
    let app = session_app(store.clone());
    let (_, refresh, _) = session_login(&app).await;
    env.remove("TM_FRONTEND_URL");
    let cookies = format!("{REFRESH_COOKIE_NAME}={refresh}");
    for source in [vec![("origin", SOURCE)], vec![("referer", "https://app.example.com/path")]] {
        let mut headers = source;
        headers.push(("cookie", &cookies));
        assert_eq!(request(&app, Method::POST, "/auth/logout", &headers, "").await.status(), 403);
    }
    assert!(store.revoked_families.lock().unwrap().is_empty());
}

#[tokio::test]
async fn google_login_requires_trusted_source_even_without_cookies() {
    let _env = setup();
    let store = FakeStore::new();
    let app = session_app(store.clone());
    for source in [None, Some(ATTACKER), Some("null")] {
        let mut headers = vec![("content-type", "application/json")];
        if let Some(source) = source {
            headers.push(("origin", source));
        }
        assert_eq!(request(&app, Method::POST, "/auth/google", &headers, r#"{"credential":"valid"}"#).await.status(), 403);
    }
    assert!(store.issued.lock().unwrap().is_empty());
    let response = request(&app, Method::POST, "/auth/google", &[("content-type", "application/json"), ("origin", SOURCE)], r#"{"credential":"valid"}"#).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["access-control-allow-origin"], SOURCE);
    assert_eq!(store.issued.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn bearer_only_cli_requests_and_anonymous_debug_logs_remain_usable() {
    let _env = setup();
    let store = FakeStore::new();
    let app = session_app(store.clone());
    let (access, _, _) = session_login(&app).await;
    let bearer = format!("Bearer {access}");
    assert_eq!(request(&app, Method::POST, "/auth/logout", &[("authorization", &bearer)], "").await.status(), 200);
    assert_eq!(store.revoked_families.lock().unwrap().len(), 1);
    assert_eq!(
        request(&app, Method::POST, "/logs/client-error", &[("content-type", "application/json"), ("origin", ATTACKER)], r#"{"error_type":"CUSTOM_ERROR","message":"test","url":"/"}"#)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn json_content_type_is_required_and_allows_application_json_suffixes() {
    let _env = setup();
    let app = session_app(FakeStore::new());
    let body = r#"{"error_type":"CUSTOM_ERROR","message":"test","url":"/"}"#;
    for content_type in [None, Some(""), Some("text/plain"), Some("text/plain+json"), Some("application/x-www-form-urlencoded"), Some("multipart/form-data")] {
        let headers = content_type.map(|value| vec![("content-type", value)]).unwrap_or_default();
        assert_eq!(request(&app, Method::POST, "/logs/client-error", &headers, body).await.status(), 422, "content-type={content_type:?}");
    }
    for content_type in ["application/json", "application/json; charset=utf-8", "application/problem+json", "APPLICATION/JSON"] {
        assert_eq!(request(&app, Method::POST, "/logs/client-error", &[("content-type", content_type)], body).await.status(), 200, "{content_type}");
    }
}

#[tokio::test]
async fn test_handlers_direct() {
    let _env = fixture();
    let logs = LogCapture::start(log::Level::Warn);
    let (parts, _) = axum::http::Request::builder().method("GET").uri("/x").body(()).unwrap().into_parts();

    let secret = "super-secret-token";
    let exc = RequestValidationError::new(vec![json!({"loc": ["body"], "msg": "bad", "type": "value_error"})], json!({"credential": secret}));
    let resp = backend::main::validation_exception_handler(&parts, &exc).await;
    assert_eq!(resp.status().as_u16(), 422);
    assert!(!logs.text().contains(secret));

    let http_exc = HTTPException::new(404, "no");
    let resp = backend::main::http_exception_handler(&parts, &http_exc).await;
    assert_eq!(resp.status().as_u16(), 404);

    let boom = std::io::Error::other("boom");
    let resp = backend::main::general_exception_handler(&parts, &boom).await;
    assert_eq!(resp.status().as_u16(), 500);
}

const RAW_EMAIL: &str = "observed-user@example.com";

const RAW_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) ObservedBrowser/123.45";

const RAW_IP: &str = "203.0.113.77";

/// 거부된 refresh 응답, 로그 텍스트, 쿠키로 보낸 원문 토큰을 돌려준다.
async fn reject_refresh(status: &str) -> (Response, String, String) {
    let store = FakeStore::rotating(status).with_observation(Ok(Some(json!({"replaced_by_present": true, "revoked_at": 1, "revoke_reason": "rotated"}))));
    let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(store);
    let app = auth_app(deps);
    let raw_cookie = crafted_refresh(RAW_EMAIL, "observed-family", "observed-jti");
    let logs = LogCapture::start(log::Level::Warn);
    let resp = post_empty(&refresh_client(&app, &raw_cookie).header("User-Agent", RAW_UA).header("X-Forwarded-For", &format!("{RAW_IP}, 10.0.0.1")), "/auth/refresh").await;
    (resp, logs.text(), raw_cookie)
}

#[tokio::test]
async fn test_emits_structured_masked_event() {
    let _env = fixture();
    let (resp, text, _raw_cookie) = reject_refresh("reused").await;

    assert_eq!(resp.status, 401);
    assert!(text.contains("event=refresh-rotation-rejected"), "{text}");
    assert!(text.contains("status=reused"), "{text}");
    assert!(text.contains("replaced_by_present=true"), "{text}");
    // 모든 식별자는 HMAC 해시로만 남는다
    for (field, raw) in [("email_hash", RAW_EMAIL), ("family_hash", "observed-family"), ("jti_hash", "observed-jti"), ("request_user_agent_hash", RAW_UA), ("request_ip_prefix_hash", "203.0.113")]
    {
        let expected = format!("{field}={}", observation_hash(raw));
        assert!(text.contains(&expected), "{expected} not in {text}");
    }
}

#[tokio::test]
async fn test_never_logs_raw_identifiers() {
    // 원문 토큰·쿠키·전체 이메일·전체 IP·raw user-agent 는 로그에 남지 않는다.
    let _env = fixture();
    let (_, text, raw_cookie) = reject_refresh("reused").await;

    assert!(!text.contains(RAW_EMAIL));
    assert!(!text.contains(RAW_UA));
    assert!(!text.contains(RAW_IP)); // /24 프리픽스도 해시로만 기록
    assert!(!text.contains(&raw_cookie));
    assert!(!text.contains("observed-family"));
    assert!(!text.contains("observed-jti"));
}

#[tokio::test]
async fn test_observation_lookup_failure_does_not_break_response() {
    // 관측 조회가 실패해도 refresh 응답 경로는 그대로 401 을 반환한다.
    let _env = fixture();
    let store = FakeStore::rotating("reused").with_observation(Err("store down".to_string()));
    let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(store);
    let app = auth_app(deps);

    let logs = LogCapture::start(log::Level::Warn);
    let resp = post_empty(&refresh_client(&app, &crafted_refresh(RAW_EMAIL, "F", "J")), "/auth/refresh").await;

    assert_eq!(resp.status, 401);
    assert!(logs.text().contains("event=refresh-rotation-rejected"), "{}", logs.text());
    assert!(logs.text().contains("replaced_by_present=false"), "{}", logs.text());
}

#[tokio::test]
async fn test_successful_refresh_emits_no_observation_event() {
    let _env = fixture();
    let deps = auth_deps().with_role_resolver(role("user")).with_refresh_token_store(FakeStore::rotating("ok"));
    let app = auth_app(deps);

    let logs = LogCapture::start(log::Level::Warn);
    let resp = post_empty(&refresh_client(&app, &crafted_refresh(RAW_EMAIL, "F", "J")), "/auth/refresh").await;

    assert_eq!(resp.status, 200);
    assert!(!logs.text().contains("event=refresh-rotation-rejected"), "{}", logs.text());
}
