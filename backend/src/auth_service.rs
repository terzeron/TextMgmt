//! 인증. 설계: docs/rust-test-seams.md 3.2.
//!
//! - 환경 변수(`TM_JWT_SECRET`, `TM_ADMIN_EMAIL`, `TM_ALLOWED_EMAILS`)는 호출 시점에 읽는다.
//! - 시작 때 `load_jwt_secret`으로 비밀키를 확인하고,
//!   토큰을 만드는 함수는 비밀키가 없으면 panic한다. 비밀키가 비었을 때 토큰을 검증하는 쪽은 항상 거부한다.
//! - 이 모듈은 HTTP 프레임워크를 모른다. 요청은 `RequestParts`로 추상화하고, axum 연결은 `main::extract`가 한다.

use std::fmt;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Deserializer, Serialize};
use sysdeps::EnvFn;

pub const JWT_ALGORITHM: Algorithm = Algorithm::HS256;
pub const ACCESS_TOKEN_EXPIRATION_SECONDS: i64 = 2 * 3600;
pub const REFRESH_TOKEN_EXPIRATION_SECONDS: i64 = 30 * 24 * 3600;
pub const ACCESS_COOKIE_NAME: &str = "tm_access_token";
pub const REFRESH_COOKIE_NAME: &str = "tm_refresh_token";
pub const OBSERVATION_HASH_LENGTH: usize = 16;

/// HTTP 상태 코드와 응답 detail을 담은 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStatusError {
    pub status_code: u16,
    pub detail: String,
}

impl HttpStatusError {
    pub fn new(status_code: u16, detail: &str) -> Self {
        Self { status_code, detail: detail.to_string() }
    }
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status_code, self.detail)
    }
}

impl std::error::Error for HttpStatusError {}

/// JWT payload. 접근 토큰과 갱신 토큰이 같은 모양을 쓴다.
///
/// 모든 필드에 기본값이 있다. 빠진 claim과 `null` claim은 빈 값으로 읽는다.
/// `fid`/`jti`가 빈 토큰은 만들 때 그 claim을 넣지 않는다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Claims {
    #[serde(rename = "type", deserialize_with = "null_as_default")]
    pub token_type: String,
    #[serde(deserialize_with = "null_as_default")]
    pub email: String,
    #[serde(deserialize_with = "null_as_default")]
    pub role: String,
    #[serde(deserialize_with = "null_as_default")]
    pub name: String,
    #[serde(deserialize_with = "null_as_default")]
    pub picture: String,
    #[serde(rename = "fid", deserialize_with = "null_as_default", skip_serializing_if = "String::is_empty")]
    pub family_id: String,
    #[serde(rename = "jti", deserialize_with = "null_as_default", skip_serializing_if = "String::is_empty")]
    pub token_id: String,
    #[serde(deserialize_with = "seconds")]
    pub exp: i64,
    #[serde(deserialize_with = "seconds")]
    pub iat: i64,
}

impl Claims {
    /// 역할만 있는 payload. 테스트 대역이 주로 쓴다.
    pub fn with_role(role: &str) -> Self {
        Self { role: role.to_string(), ..Self::default() }
    }

    pub fn is_viewer(&self) -> bool {
        self.role == "viewer"
    }
}

fn null_as_default<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// 정수 초. 실수(`1.7e9`)는 소수점을 버린다. `null`은 0이다.
fn seconds<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Seconds {
        Int(i64),
        Float(f64),
    }
    Ok(match Option::<Seconds>::deserialize(d)? {
        Some(Seconds::Int(n)) => n,
        Some(Seconds::Float(f)) => f as i64,
        None => 0,
    })
}

/// 프로세스를 끝내야 하는 설정 오류. 종료 코드를 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FatalConfigError(pub i32);

/// 요청에서 인증에 필요한 부분(헤더, 쿠키)만 읽는 능력.
pub trait RequestParts {
    fn header(&self, name: &str) -> Option<String>;
    fn cookie(&self, name: &str) -> Option<String>;
}

/// 이메일 -> 역할. 기본 구현은 환경 변수를 본다.
pub trait RoleResolver: Send + Sync {
    fn determine_role(&self, email: &str) -> Option<String> {
        determine_role(email)
    }
}

/// 환경 변수로 역할을 정하는 실제 구현.
pub struct EnvRoleResolver;

impl RoleResolver for EnvRoleResolver {}

/// 토큰 발급·해독 seam. 기본 구현은 모두 실제 JWT다. 테스트는 쓰는 메서드만 바꾼다.
pub trait TokenIssuer: Send + Sync {
    fn create_access_token(&self, email: &str, role: &str, name: &str, picture: &str, family_id: &str) -> String {
        create_session_jwt_token(email, role, name, picture, family_id)
    }

    fn create_refresh_token(&self, email: &str, role: &str, name: &str, picture: &str, family_id: &str, token_id: &str) -> String {
        create_refresh_token_with(email, role, Some(name), Some(picture), Some(family_id), Some(token_id))
    }

    fn decode_refresh_token(&self, token: &str) -> Result<Claims, HttpStatusError> {
        decode_refresh_token(token)
    }

    /// `uuid.uuid4().hex`
    fn new_id(&self) -> String {
        uuid_hex()
    }
}

/// 실제 토큰 발급기.
pub struct JwtIssuer;

impl TokenIssuer for JwtIssuer {}

// ---------------------------------------------------------------------------
// 환경
// ---------------------------------------------------------------------------

fn env_var(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

/// 시작 때 한 번 부른다. 비밀키가 없으면 `FatalConfigError(-1)`이다.
pub fn load_jwt_secret(env: EnvFn) -> Result<String, FatalConfigError> {
    match env("TM_JWT_SECRET") {
        Some(secret) if !secret.is_empty() => Ok(secret),
        _ => {
            log::error!("The environment variable TM_JWT_SECRET is not set.");
            Err(FatalConfigError(-1))
        }
    }
}

/// 지금 환경의 JWT 비밀키. 없으면 빈 문자열이다.
pub fn jwt_secret() -> String {
    env_var("TM_JWT_SECRET")
}

pub fn tm_admin_email() -> String {
    env_var("TM_ADMIN_EMAIL")
}

fn allowed_emails() -> Vec<String> {
    env_var("TM_ALLOWED_EMAILS").split(',').map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).collect()
}

/// 관리자 이메일이면 "admin", 허용 목록이면 "viewer", 아니면 None.
///
/// 빈 이메일은 `TM_ADMIN_EMAIL`이 비어 있어도 항상 거부한다.
pub fn determine_role(email: &str) -> Option<String> {
    if email.is_empty() {
        return None;
    }
    if email == tm_admin_email() {
        return Some("admin".to_string());
    }
    if allowed_emails().iter().any(|e| e == email) {
        return Some("viewer".to_string());
    }
    None
}

// ---------------------------------------------------------------------------
// 시간·식별자·해시
// ---------------------------------------------------------------------------

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `uuid.uuid4().hex`와 같은 모양(소문자 hex 32자, 버전 4)의 무작위 식별자.
pub fn uuid_hex() -> String {
    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_ok();
    if !filled {
        // /dev/urandom이 없는 환경의 대비책. 예측하기 어렵지 않으므로 인가 판단에는 쓰지 않는다.
        use std::hash::{BuildHasher, Hasher};
        for chunk in bytes.chunks_mut(8) {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 관측 로그용 단방향 해시: `HMAC-SHA256(secret, "tm-observation|" + value)`의 hex 앞 16자.
///
/// refresh 회전 실패를 "같은 브라우저의 동시 요청"과 "다른 환경에서 복사된 상태"로 나누려는 진단 신호다.
/// 원문(이메일, family_id, jti, user-agent, IP)을 로그에 남기지 않고 같은 값끼리 상관관계만 본다. 인가 판단에는 쓰지 않는다.
pub fn observation_hash(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    // jsonwebtoken의 HS256 서명은 HMAC-SHA256이다. 서명 결과(base64url)를 hex로 바꿔 쓴다.
    let key = EncodingKey::from_secret(jwt_secret().as_bytes());
    let message = format!("tm-observation|{value}");
    let Ok(signature) = jsonwebtoken::crypto::sign(message.as_bytes(), &key, JWT_ALGORITHM) else {
        return String::new();
    };
    let hex: String = URL_SAFE_NO_PAD.decode(&signature).unwrap_or_default().iter().map(|b| format!("{b:02x}")).collect();
    hex.chars().take(OBSERVATION_HASH_LENGTH).collect()
}

// ---------------------------------------------------------------------------
// 토큰
// ---------------------------------------------------------------------------

fn sign(payload: &Claims) -> String {
    let secret = jwt_secret();
    assert!(!secret.is_empty(), "The environment variable TM_JWT_SECRET is not set.");
    jsonwebtoken::encode(&Header::new(JWT_ALGORITHM), payload, &EncodingKey::from_secret(secret.as_bytes())).expect("JWT 서명")
}

/// 만료(`exp`)를 필수로 검사한다. 시계 오차 허용(leeway)은 두지 않는다(PyJWT 기본값과 같다).
fn decode_claims(token: &str) -> Result<Claims, ErrorKind> {
    let secret = jwt_secret();
    if secret.is_empty() {
        return Err(ErrorKind::InvalidSignature);
    }
    let mut validation = Validation::new(JWT_ALGORITHM);
    validation.leeway = 0;
    jsonwebtoken::decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation).map(|data| data.claims).map_err(|e| e.into_kind())
}

/// 접근 토큰(2시간). `name`/`picture`가 None이면 빈 문자열이다.
pub fn create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String {
    create_session_jwt_token(email, role, name.unwrap_or(""), picture.unwrap_or(""), "")
}

/// 운영용 접근 토큰. 세션 식별자가 없는 기존 토큰은 HTTP 인증에서 거부한다.
pub fn create_session_jwt_token(email: &str, role: &str, name: &str, picture: &str, family_id: &str) -> String {
    let now = now_secs();
    sign(&Claims {
        token_type: "access".to_string(),
        email: email.to_string(),
        role: role.to_string(),
        name: name.to_string(),
        picture: picture.to_string(),
        family_id: family_id.to_string(),
        token_id: String::new(),
        exp: now + ACCESS_TOKEN_EXPIRATION_SECONDS,
        iat: now,
    })
}

/// 갱신 토큰(30일). family/token id를 안 주면 새로 만든다.
pub fn create_refresh_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String {
    create_refresh_token_with(email, role, name, picture, None, None)
}

pub fn create_refresh_token_with(email: &str, role: &str, name: Option<&str>, picture: Option<&str>, family_id: Option<&str>, token_id: Option<&str>) -> String {
    let now = now_secs();
    let new_or = |id: Option<&str>| id.filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(uuid_hex);
    sign(&Claims {
        token_type: "refresh".to_string(),
        email: email.to_string(),
        role: role.to_string(),
        name: name.unwrap_or("").to_string(),
        picture: picture.unwrap_or("").to_string(),
        family_id: new_or(family_id),
        token_id: new_or(token_id),
        exp: now + REFRESH_TOKEN_EXPIRATION_SECONDS,
        iat: now,
    })
}

pub fn decode_refresh_token(token: &str) -> Result<Claims, HttpStatusError> {
    let payload = decode_claims(token).map_err(|kind| match kind {
        ErrorKind::ExpiredSignature => HttpStatusError::new(401, "Refresh token expired"),
        _ => HttpStatusError::new(401, "Invalid refresh token"),
    })?;
    if payload.token_type != "refresh" {
        return Err(HttpStatusError::new(401, "Invalid token type"));
    }
    if payload.family_id.is_empty() || payload.token_id.is_empty() {
        return Err(HttpStatusError::new(401, "Refresh token missing required claims. Please log in again."));
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// 요청 검사
// ---------------------------------------------------------------------------

/// `Authorization: Bearer`가 있으면 그것을, 비었으면 접근 쿠키를 쓴다.
pub fn extract_payload<R: RequestParts + ?Sized>(req: &R) -> Result<Claims, HttpStatusError> {
    let header = req.header("Authorization").unwrap_or_default();
    let mut token = header.strip_prefix("Bearer ").unwrap_or("").to_string();
    if token.is_empty() {
        token = req.cookie(ACCESS_COOKIE_NAME).unwrap_or_default();
    }
    if token.is_empty() {
        return Err(HttpStatusError::new(401, "Missing authentication token"));
    }
    let payload = decode_claims(&token).map_err(|kind| match kind {
        ErrorKind::ExpiredSignature => HttpStatusError::new(401, "Token expired"),
        _ => HttpStatusError::new(401, "Invalid token"),
    })?;
    if payload.token_type != "access" {
        return Err(HttpStatusError::new(401, "Invalid token type"));
    }
    Ok(payload)
}

/// 토큰의 역할이 지금 정책과 같아야 한다. 허용 목록에서 빠진 사용자와 강등된 관리자는 토큰이 살아 있어도 거부한다.
fn resolve_current_role(payload: &Claims, resolver: &dyn RoleResolver) -> Result<String, HttpStatusError> {
    match resolver.determine_role(&payload.email) {
        Some(role) if payload.role == role => Ok(role),
        _ => Err(HttpStatusError::new(403, "Access denied")),
    }
}

pub async fn require_auth_with<R: RequestParts + ?Sized>(req: &R, resolver: &dyn RoleResolver) -> Result<Claims, HttpStatusError> {
    let payload = extract_payload(req)?;
    let role = resolve_current_role(&payload, resolver)?;
    if role != "admin" && role != "viewer" {
        return Err(HttpStatusError::new(403, "Insufficient permissions"));
    }
    Ok(Claims { role, ..payload })
}

pub async fn require_admin_with<R: RequestParts + ?Sized>(req: &R, resolver: &dyn RoleResolver) -> Result<Claims, HttpStatusError> {
    let payload = extract_payload(req)?;
    let role = resolve_current_role(&payload, resolver)?;
    if role != "admin" {
        return Err(HttpStatusError::new(403, "Admin access required"));
    }
    Ok(Claims { role, ..payload })
}

pub async fn require_auth<R: RequestParts + ?Sized>(req: &R) -> Result<Claims, HttpStatusError> {
    require_auth_with(req, &EnvRoleResolver).await
}

pub async fn require_admin<R: RequestParts + ?Sized>(req: &R) -> Result<Claims, HttpStatusError> {
    require_admin_with(req, &EnvRoleResolver).await
}

/// 인증에 실패하면 None. 오류를 만들지 않는다.
pub async fn optional_auth_with<R: RequestParts + ?Sized>(req: &R, resolver: &dyn RoleResolver) -> Option<Claims> {
    require_auth_with(req, resolver).await.ok()
}

pub async fn optional_auth<R: RequestParts + ?Sized>(req: &R) -> Option<Claims> {
    optional_auth_with(req, &EnvRoleResolver).await
}

// ---------------------------------------------------------------------------
// axum 요청 연결
// ---------------------------------------------------------------------------

/// `Cookie` 헤더(여러 개 가능)에서 `name`의 값을 찾는다. 같은 이름이 여러 번이면 뒤의 것이 이긴다.
fn cookie_from_headers(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for value in headers.get_all("cookie") {
        let Ok(text) = value.to_str() else { continue };
        for pair in text.split(';') {
            let Some((key, val)) = pair.split_once('=') else { continue };
            if key.trim() == name {
                let val = val.trim();
                let val = val.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(val);
                found = Some(val.to_string());
            }
        }
    }
    found
}

impl RequestParts for axum::http::HeaderMap {
    fn header(&self, name: &str) -> Option<String> {
        self.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
    }

    fn cookie(&self, name: &str) -> Option<String> {
        cookie_from_headers(self, name)
    }
}

impl RequestParts for axum::http::request::Parts {
    fn header(&self, name: &str) -> Option<String> {
        self.headers.header(name)
    }

    fn cookie(&self, name: &str) -> Option<String> {
        self.headers.cookie(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn uuid_hex_looks_like_uuid4_hex() {
        let id = uuid_hex();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(&id[12..13], "4");
        assert_ne!(id, uuid_hex());
    }

    #[test]
    fn cookie_parsing_handles_spaces_quotes_and_duplicates() {
        let mut headers = HeaderMap::new();
        headers.append("cookie", "a=1; tm_access_token=\"abc\" ; b=2".parse().unwrap());
        headers.append("cookie", "tm_access_token=def".parse().unwrap());
        assert_eq!(cookie_from_headers(&headers, "tm_access_token").as_deref(), Some("def"));
        assert_eq!(cookie_from_headers(&headers, "b").as_deref(), Some("2"));
        assert_eq!(cookie_from_headers(&headers, "missing"), None);
    }

    #[test]
    fn observation_hash_matches_hmac_sha256_reference() {
        // SAFETY: 이 lib 단위 테스트 바이너리에서 TM_JWT_SECRET을 바꾸는 테스트는 이것 하나다.
        unsafe { std::env::set_var("TM_JWT_SECRET", "testsecret123") };
        // 기준값: HMAC-SHA256(key=b"testsecret123", msg=b"tm-observation|observed-family")의 hex 앞 16자
        assert_eq!(observation_hash("observed-family"), "f4927567f8b53241");
        assert_eq!(observation_hash(""), "");
        unsafe { std::env::remove_var("TM_JWT_SECRET") };
    }
}
