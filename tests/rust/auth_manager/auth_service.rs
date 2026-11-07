//! `auth_manager::auth_service` 단위 테스트.
#![allow(clippy::await_holding_lock)]

mod common;
use crate::common::auth_fixtures::{FakeRequest, FixedRole, access_token, decode, encode_hs256, fixture, make_request};
use crate::common::clock::now;
use crate::common::env::process_env;
use auth_manager::auth_service;
use auth_manager::auth_service::{FatalConfigError, HttpStatusError};
use serde_json::json;
use std::collections::HashMap;

fn refresh_token(email: &str, role: &str) -> String {
    auth_manager::auth_service::create_refresh_token(email, role, None, None)
}

fn detail_of(err: &HttpStatusError) -> String {
    err.detail.to_lowercase()
}

// ========== determine_role ==========

mod determine_role {
    use super::*;

    #[test]
    fn test_admin_email() {
        let _env = fixture();
        assert_eq!(auth_manager::auth_service::determine_role("admin@example.com").as_deref(), Some("admin"));
    }

    #[test]
    fn test_viewer_email() {
        let _env = fixture();
        assert_eq!(auth_manager::auth_service::determine_role("viewer1@example.com").as_deref(), Some("viewer"));
        assert_eq!(auth_manager::auth_service::determine_role("viewer2@example.com").as_deref(), Some("viewer"));
    }

    #[test]
    fn test_unknown_email() {
        let _env = fixture();
        assert_eq!(auth_manager::auth_service::determine_role("unknown@example.com"), None);
    }

    #[test]
    fn test_empty_email() {
        let _env = fixture();
        assert_eq!(auth_manager::auth_service::determine_role(""), None);
    }

    #[test]
    fn test_reads_current_env_without_reload() {
        let mut env = fixture();
        env.set("TM_ADMIN_EMAIL", "new-admin@example.com");
        env.set("TM_ALLOWED_EMAILS", "new-viewer@example.com");

        assert_eq!(auth_manager::auth_service::determine_role("new-admin@example.com").as_deref(), Some("admin"));
        assert_eq!(auth_manager::auth_service::determine_role("new-viewer@example.com").as_deref(), Some("viewer"));
        assert_eq!(auth_manager::auth_service::determine_role("admin@example.com"), None);
    }
}

#[test]
fn test_auth_requires_jwt_secret() {
    let mut env = fixture();
    env.remove("TM_JWT_SECRET");
    let result = auth_manager::auth_service::load_jwt_secret(&process_env);
    assert!(matches!(result, Err(FatalConfigError(_))), "{result:?}");
}

// ========== 토큰 발급 ==========

mod create_jwt_token {
    use super::*;

    #[test]
    fn test_token_contains_claims() {
        let _env = fixture();
        let token = auth_manager::auth_service::create_jwt_token("admin@example.com", "admin", Some("Admin"), Some("pic.jpg"));
        let payload = decode(&token);
        assert_eq!(payload["email"], "admin@example.com");
        assert_eq!(payload["role"], "admin");
        assert_eq!(payload["name"], "Admin");
        assert_eq!(payload["picture"], "pic.jpg");
        assert_eq!(payload["type"], "access");
        assert!(payload.get("exp").is_some());
        assert!(payload.get("iat").is_some());
    }

    #[test]
    fn test_token_expiration() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let payload = decode(&token);
        let exp = payload["exp"].as_i64().unwrap();
        assert!(exp > now());
        assert!(exp <= now() + 2 * 3600 + 5);
    }
}

mod create_refresh_token {
    use super::*;

    #[test]
    fn test_refresh_token_contains_claims() {
        let _env = fixture();
        let token = auth_manager::auth_service::create_refresh_token("admin@example.com", "admin", Some("Admin"), Some("pic.jpg"));
        let payload = decode(&token);
        assert_eq!(payload["email"], "admin@example.com");
        assert_eq!(payload["role"], "admin");
        assert_eq!(payload["type"], "refresh");
        assert_eq!(payload["name"], "Admin");
        assert_eq!(payload["picture"], "pic.jpg");
        assert!(payload["fid"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(payload["jti"].as_str().is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn test_refresh_token_expiration() {
        let _env = fixture();
        let token = refresh_token("admin@example.com", "admin");
        let payload = decode(&token);
        let exp = payload["exp"].as_i64().unwrap();
        assert!(exp > now());
        assert!(exp <= now() + 30 * 24 * 3600 + 5);
    }
}

mod decode_refresh_token {
    use super::*;

    #[test]
    fn test_valid_refresh_token() {
        let _env = fixture();
        let token = refresh_token("admin@example.com", "admin");
        let payload = auth_manager::auth_service::decode_refresh_token(&token).unwrap();
        assert_eq!(payload.email, "admin@example.com");
        assert_eq!(payload.token_type, "refresh");
    }

    #[test]
    fn test_expired_refresh_token() {
        let _env = fixture();
        let expired_payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "exp": now() - 100, "iat": now() - 200});
        let token = encode_hs256(&expired_payload);
        let err = auth_manager::auth_service::decode_refresh_token(&token).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("expired"), "{}", err.detail);
    }

    #[test]
    fn test_access_token_rejected() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let err = auth_manager::auth_service::decode_refresh_token(&token).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("type"), "{}", err.detail);
    }

    #[test]
    fn test_invalid_token() {
        let _env = fixture();
        let err = auth_manager::auth_service::decode_refresh_token("invalid.token.value").unwrap_err();
        assert_eq!(err.status_code, 401);
    }

    #[test]
    fn test_refresh_token_without_rotation_claims_is_rejected() {
        let _env = fixture();
        let payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "exp": now() + 100, "iat": now()});
        let token = encode_hs256(&payload);
        let err = auth_manager::auth_service::decode_refresh_token(&token).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("claims"), "{}", err.detail);
    }

    #[test]
    fn test_refresh_token_missing_fid_only_is_rejected() {
        // fid만 없는 경우도 claims 에러로 거부된다.
        let _env = fixture();
        let payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "jti": "some-jti", "exp": now() + 100, "iat": now()});
        let token = encode_hs256(&payload);
        let err = auth_manager::auth_service::decode_refresh_token(&token).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("claims"), "{}", err.detail);
        assert!(detail_of(&err).contains("log in again"), "{}", err.detail);
    }

    #[test]
    fn test_refresh_token_missing_jti_only_is_rejected() {
        // jti만 없는 경우도 claims 에러로 거부된다.
        let _env = fixture();
        let payload = json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "fid": "some-fid", "exp": now() + 100, "iat": now()});
        let token = encode_hs256(&payload);
        let err = auth_manager::auth_service::decode_refresh_token(&token).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("claims"), "{}", err.detail);
    }
}

// ========== 요청에서 payload 추출 ==========

mod extract_payload {
    use super::*;

    #[test]
    fn test_missing_header() {
        let _env = fixture();
        let req = make_request(None, None);
        let err = auth_manager::auth_service::extract_payload(&req).unwrap_err();
        assert_eq!(err.status_code, 401);
    }

    #[test]
    fn test_valid_token() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(Some(&token), None);
        let payload = auth_manager::auth_service::extract_payload(&req).unwrap();
        assert_eq!(payload.email, "admin@example.com");
        assert_eq!(payload.role, "admin");
    }

    #[test]
    fn test_valid_cookie_token() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(None, Some(&token));
        let payload = auth_manager::auth_service::extract_payload(&req).unwrap();
        assert_eq!(payload.email, "admin@example.com");
        assert_eq!(payload.role, "admin");
    }

    #[test]
    fn test_expired_token() {
        let _env = fixture();
        let expired_payload = json!({"type": "access", "email": "admin@example.com", "role": "admin", "exp": now() - 100, "iat": now() - 200});
        let token = encode_hs256(&expired_payload);
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::extract_payload(&req).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("expired"), "{}", err.detail);
    }

    #[test]
    fn test_invalid_token() {
        let _env = fixture();
        let req = make_request(Some("invalid.token.value"), None);
        let err = auth_manager::auth_service::extract_payload(&req).unwrap_err();
        assert_eq!(err.status_code, 401);
    }

    #[test]
    fn test_refresh_token_rejected() {
        let _env = fixture();
        let token = refresh_token("admin@example.com", "admin");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::extract_payload(&req).unwrap_err();
        assert_eq!(err.status_code, 401);
        assert!(detail_of(&err).contains("type"), "{}", err.detail);
    }

    #[test]
    fn test_bearer_header_takes_priority_over_cookie() {
        let _env = fixture();
        let header_token = access_token("admin@example.com", "admin");
        let cookie_token = access_token("viewer1@example.com", "viewer");
        let req = make_request(Some(&header_token), Some(&cookie_token));
        let payload = auth_manager::auth_service::extract_payload(&req).unwrap();
        assert_eq!(payload.email, "admin@example.com");
    }

    #[test]
    fn test_empty_bearer_falls_back_to_cookie() {
        let _env = fixture();
        let cookie_token = access_token("viewer1@example.com", "viewer");
        let req = FakeRequest { headers: HashMap::from([("Authorization".to_string(), "Bearer ".to_string())]), cookies: HashMap::from([("tm_access_token".to_string(), cookie_token)]) };

        let payload = auth_manager::auth_service::extract_payload(&req).unwrap();
        assert_eq!(payload.email, "viewer1@example.com");
    }
}

mod require_auth {
    use super::*;

    #[tokio::test]
    async fn test_admin_passes() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(Some(&token), None);
        let payload = auth_manager::auth_service::require_auth(&req).await.unwrap();
        assert_eq!(payload.role, "admin");
    }

    #[tokio::test]
    async fn test_viewer_passes() {
        let _env = fixture();
        let token = access_token("viewer1@example.com", "viewer");
        let req = make_request(Some(&token), None);
        let payload = auth_manager::auth_service::require_auth(&req).await.unwrap();
        assert_eq!(payload.role, "viewer");
    }

    #[tokio::test]
    async fn test_cookie_token_passes() {
        let _env = fixture();
        let token = access_token("viewer1@example.com", "viewer");
        let req = make_request(None, Some(&token));
        let payload = auth_manager::auth_service::require_auth(&req).await.unwrap();
        assert_eq!(payload.role, "viewer");
    }

    #[tokio::test]
    async fn test_invalid_role_fails() {
        let _env = fixture();
        let token = access_token("nobody@example.com", "unknown");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::require_auth(&req).await.unwrap_err();
        assert_eq!(err.status_code, 403);
    }

    #[tokio::test]
    async fn test_revoked_viewer_fails() {
        let _env = fixture();
        let token = access_token("viewer1@example.com", "viewer");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::require_auth_with(&req, &FixedRole(None)).await.unwrap_err();
        assert_eq!(err.status_code, 403);
        assert_eq!(err.detail, "Access denied");
    }

    #[tokio::test]
    async fn test_role_mismatch_fails() {
        let _env = fixture();
        let token = access_token("viewer1@example.com", "viewer");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::require_auth_with(&req, &FixedRole(Some("admin".to_string()))).await.unwrap_err();
        assert_eq!(err.status_code, 403);
        assert_eq!(err.detail, "Access denied");
    }

    #[tokio::test]
    async fn test_no_token_fails() {
        let _env = fixture();
        let req = make_request(None, None);
        let err = auth_manager::auth_service::require_auth(&req).await.unwrap_err();
        assert_eq!(err.status_code, 401);
    }
}

mod require_admin {
    use super::*;

    #[tokio::test]
    async fn test_admin_passes() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(Some(&token), None);
        let payload = auth_manager::auth_service::require_admin(&req).await.unwrap();
        assert_eq!(payload.role, "admin");
    }

    #[tokio::test]
    async fn test_admin_cookie_passes() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(None, Some(&token));
        let payload = auth_manager::auth_service::require_admin(&req).await.unwrap();
        assert_eq!(payload.role, "admin");
    }

    #[tokio::test]
    async fn test_viewer_blocked() {
        let _env = fixture();
        let token = access_token("viewer1@example.com", "viewer");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::require_admin(&req).await.unwrap_err();
        assert_eq!(err.status_code, 403);
        assert!(detail_of(&err).contains("admin"), "{}", err.detail);
    }

    #[tokio::test]
    async fn test_admin_revoked_fails() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::require_admin_with(&req, &FixedRole(None)).await.unwrap_err();
        assert_eq!(err.status_code, 403);
        assert_eq!(err.detail, "Access denied");
    }

    #[tokio::test]
    async fn test_admin_demoted_to_viewer_fails() {
        let _env = fixture();
        let token = access_token("admin@example.com", "admin");
        let req = make_request(Some(&token), None);
        let err = auth_manager::auth_service::require_admin_with(&req, &FixedRole(Some("viewer".to_string()))).await.unwrap_err();
        assert_eq!(err.status_code, 403);
        assert_eq!(err.detail, "Access denied");
    }

    #[tokio::test]
    async fn test_no_token_fails() {
        let _env = fixture();
        let req = make_request(None, None);
        let err = auth_manager::auth_service::require_admin(&req).await.unwrap_err();
        assert_eq!(err.status_code, 401);
    }
}

// ---- 환경 변수 누락 검사 ----

#[test]
fn test_auth_requires_jwt_secret_os_environ() {
    // 환경 변수를 지우고 설정 로더를 다시 부른다.
    let mut env = fixture();
    env.remove("TM_JWT_SECRET");
    let result = auth_manager::auth_service::load_jwt_secret(&process_env);
    assert!(matches!(result, Err(FatalConfigError(_))), "{result:?}");
}

/// determine_role 이 "admin"/"viewer" 이외 역할을 반환할 때 require_auth 가 403 을 올린다.
#[tokio::test]
async fn test_require_auth_non_standard_role_raises_403() {
    let _env = fixture();
    let token = access_token("admin@example.com", "superuser");
    let req = FakeRequest { headers: HashMap::from([("Authorization".to_string(), format!("Bearer {token}"))]), cookies: HashMap::new() };

    let err = auth_manager::auth_service::require_auth_with(&req, &FixedRole(Some("superuser".to_string()))).await.unwrap_err();
    assert_eq!(err.status_code, 403);
    assert!(detail_of(&err).contains("permissions"), "{}", err.detail);
}

#[test]
fn test_observation_hash_of_empty_value_is_empty_string() {
    common::setup_env();
    assert_eq!(auth_service::observation_hash(""), "");
}

#[test]
fn test_load_jwt_secret_returns_a_configured_secret_and_errors_render_status_and_detail() {
    let _env = fixture();
    assert!(auth_manager::auth_service::load_jwt_secret(&process_env).is_ok_and(|secret| !secret.is_empty()));
    assert_eq!(HttpStatusError::new(401, "Invalid token").to_string(), "401: Invalid token");
}

#[test]
fn test_issued_at_accepts_a_float_or_null() {
    let _env = fixture();
    let float_iat = encode_hs256(&json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "fid": "f", "jti": "j", "exp": now() + 100, "iat": 1_700_000_000.75}));
    assert_eq!(auth_manager::auth_service::decode_refresh_token(&float_iat).unwrap().iat, 1_700_000_000);
    let null_iat = encode_hs256(&json!({"type": "refresh", "email": "admin@example.com", "role": "admin", "fid": "f", "jti": "j", "exp": now() + 100, "iat": null}));
    assert_eq!(auth_manager::auth_service::decode_refresh_token(&null_iat).unwrap().iat, 0);
}

#[test]
fn test_decoding_without_a_secret_is_unauthorized() {
    let mut env = fixture();
    let token = refresh_token("admin@example.com", "admin");
    env.remove("TM_JWT_SECRET");
    assert_eq!(auth_manager::auth_service::decode_refresh_token(&token).unwrap_err().status_code, 401);
}

#[tokio::test]
async fn test_optional_auth_returns_claims_or_none() {
    let _env = fixture();
    let token = access_token("admin@example.com", "admin");
    assert_eq!(auth_manager::auth_service::optional_auth(&make_request(Some(&token), None)).await.map(|claims| claims.role).as_deref(), Some("admin"));
    assert!(auth_manager::auth_service::optional_auth(&make_request(None, None)).await.is_none());
}
