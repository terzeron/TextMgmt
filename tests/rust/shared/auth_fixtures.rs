//! 인증 테스트 준비물: 고정 비밀키로 토큰 만들기·풀기, 가짜 요청(`FakeRequest`), 고정 역할(`FixedRole`), 환경 고정(`fixture`).
//! `auth_manager`와 backend 테스트가 함께 쓴다. 포함하는 쪽의 `common`에 `env`(`EnvGuard`)가 있어야 한다.

#![allow(dead_code)]

use std::collections::HashMap;

use auth_manager::auth_service::{RequestParts, RoleResolver};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::Value;

use super::env::EnvGuard;

pub const SECRET: &str = "testsecret123";

pub struct FakeRequest {
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
}

impl RequestParts for FakeRequest {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.get(name).cloned()
    }
}

pub fn decode(token: &str) -> Value {
    jsonwebtoken::decode::<Value>(token, &DecodingKey::from_secret(SECRET.as_bytes()), &Validation::new(Algorithm::HS256)).unwrap().claims
}

pub fn encode_hs256(payload: &Value) -> String {
    encode_hs256_with_secret(payload, SECRET)
}

/// 임의 payload로 HS256 토큰을 만든다. 구형·만료 토큰처럼 운영 코드가 만들지 않는 토큰을 시험할 때 쓴다.
pub fn encode_hs256_with_secret(payload: &Value, secret: &str) -> String {
    jsonwebtoken::encode(&Header::new(Algorithm::HS256), payload, &EncodingKey::from_secret(secret.as_bytes())).unwrap()
}

pub fn fixture() -> EnvGuard {
    let mut env = EnvGuard::new();
    env.set("TM_JWT_SECRET", SECRET);
    env.set("TM_ADMIN_EMAIL", "admin@example.com");
    env.set("TM_ALLOWED_EMAILS", "viewer1@example.com,viewer2@example.com");
    env
}

pub fn make_request(token: Option<&str>, cookie_token: Option<&str>) -> FakeRequest {
    let mut headers = HashMap::new();
    let mut cookies = HashMap::new();
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        headers.insert("Authorization".to_string(), format!("Bearer {t}"));
    }
    if let Some(c) = cookie_token.filter(|c| !c.is_empty()) {
        cookies.insert("tm_access_token".to_string(), c.to_string());
    }
    FakeRequest { headers, cookies }
}

/// `RoleResolver` fake. 어떤 이메일이든 같은 역할을 돌려준다 (구 `.determine_role(|_| ..)` 클로저).
pub struct FixedRole(pub Option<String>);

impl RoleResolver for FixedRole {
    fn determine_role(&self, _email: &str) -> Option<String> {
        self.0.clone()
    }
}

pub fn access_token(email: &str, role: &str) -> String {
    auth_manager::auth_service::create_jwt_token(email, role, None, None)
}
