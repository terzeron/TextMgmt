//! `backend::api_router::auth_cookies` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::auth_fixtures::fixture;
use backend::api_router::{ACCESS_COOKIE_NAME, CustomJSONResponse, REFRESH_COOKIE_NAME};
use serde_json::json;

#[test]
fn test_default_values() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.remove("TM_COOKIE_SECURE");
    env.remove("TM_COOKIE_SAMESITE");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(!secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_secure_true() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "https://app.example.com");
    env.set("TM_COOKIE_SECURE", "true");
    env.remove("TM_COOKIE_SAMESITE");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_samesite_strict() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.remove("TM_COOKIE_SECURE");
    env.set("TM_COOKIE_SAMESITE", "strict");
    let (_secure, samesite) = backend::api_router::get_cookie_settings();
    assert_eq!(samesite, "strict");
}

#[test]
fn test_invalid_samesite_falls_back_to_lax() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.remove("TM_COOKIE_SECURE");
    env.set("TM_COOKIE_SAMESITE", "invalid_value");
    let (_secure, samesite) = backend::api_router::get_cookie_settings();
    assert_eq!(samesite, "lax");
}

#[test]
fn test_samesite_none_with_secure_true() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.set("TM_COOKIE_SECURE", "true");
    env.set("TM_COOKIE_SAMESITE", "none");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(secure);
    assert_eq!(samesite, "none");
}

#[test]
fn test_samesite_none_without_secure_falls_back_to_lax() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.set("TM_COOKIE_SECURE", "false");
    env.set("TM_COOKIE_SAMESITE", "none");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(!secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_non_local_origin_forces_secure_even_when_disabled() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "https://app.example.com");
    env.set("TM_COOKIE_SECURE", "false");
    env.remove("TM_COOKIE_SAMESITE");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_cookie_settings_defaults() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.remove("TM_COOKIE_SECURE");
    env.remove("TM_COOKIE_SAMESITE");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(!secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_cookie_settings_none_requires_secure() {
    let mut env = fixture();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.set("TM_COOKIE_SECURE", "false");
    env.set("TM_COOKIE_SAMESITE", "none");
    let (secure, samesite) = backend::api_router::get_cookie_settings();
    assert!(!secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_set_and_clear_auth_cookies() {
    let mut env = fixture();
    env.set("TM_COOKIE_SECURE", "true");
    env.set("TM_COOKIE_SAMESITE", "strict");
    let mut response = CustomJSONResponse::new(json!({"ok": true}));
    backend::api_router::set_auth_cookies(&mut response, "access-token", "refresh-token");
    let cookies: Vec<String> = response.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cookies.iter().any(|cookie| cookie.contains(ACCESS_COOKIE_NAME)), "{cookies:?}");
    assert!(cookies.iter().any(|cookie| cookie.contains(REFRESH_COOKIE_NAME)), "{cookies:?}");

    let mut response2 = CustomJSONResponse::new(json!({"ok": true}));
    backend::api_router::clear_auth_cookies(&mut response2);
    let cookies2: Vec<String> = response2.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert!(cookies2.iter().any(|cookie| cookie.contains(&format!("{ACCESS_COOKIE_NAME}="))), "{cookies2:?}");
    assert!(cookies2.iter().any(|cookie| cookie.contains(&format!("{REFRESH_COOKIE_NAME}="))), "{cookies2:?}");
}

#[test]
fn test_custom_jsonable_encoder_preserves_strings() {
    let _env = fixture();
    let payload = json!({"k": "한글", "list": ["테스트", {"x": "값"}]});
    let encoded = backend::api_router::custom_jsonable_encoder(&payload);
    assert_eq!(encoded["k"], "한글");
    assert_eq!(encoded["list"][0], "테스트");
}

/// 원격 호스트는 false다.
#[test]
fn test_is_local_frontend_origin_remote_returns_false() {
    assert!(!backend::api_router::is_local_frontend_origin(Some("https://remote.example.com")));
}

/// localhost 계열은 true다.
#[test]
fn test_is_local_frontend_origin_localhost_returns_true() {
    assert!(backend::api_router::is_local_frontend_origin(Some("http://localhost:5173")));
    assert!(backend::api_router::is_local_frontend_origin(Some("http://127.0.0.1:8080")));
    assert!(backend::api_router::is_local_frontend_origin(Some("http://[::1]:3000")));
}

/// 빈 값/없음은 false다.
#[test]
fn test_is_local_frontend_origin_empty_returns_false() {
    assert!(!backend::api_router::is_local_frontend_origin(None));
    assert!(!backend::api_router::is_local_frontend_origin(Some("")));
}
