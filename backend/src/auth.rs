//! 인증. Python `backend/auth.py`에 대응한다. 설계: docs/rust-test-seams.md 3.2.
//!
//! - 환경 변수(`TM_JWT_SECRET`, `TM_ADMIN_EMAIL`, `TM_ALLOWED_EMAILS`)는 호출 시점에 읽는다. Python의 `importlib.reload`가 필요 없다.
//! - Python은 `TM_JWT_SECRET`이 없으면 import 때 프로세스를 끝낸다. Rust는 시작 때 `load_jwt_secret`으로 확인하고,
//!   토큰을 만드는 함수는 비밀키가 없으면 panic한다. 비밀키가 비었을 때 토큰을 검증하는 쪽은 항상 거부한다.
//! - 이 모듈은 HTTP 프레임워크를 모른다. 요청은 `RequestParts`로 추상화하고, axum 연결은 `main::extract`가 한다.

use std::fmt;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde_json::{Value, json};
use sysdeps::EnvFn;

pub const JWT_ALGORITHM: Algorithm = Algorithm::HS256;
pub const ACCESS_TOKEN_EXPIRATION_SECONDS: i64 = 2 * 3600;
pub const REFRESH_TOKEN_EXPIRATION_SECONDS: i64 = 30 * 24 * 3600;
pub const ACCESS_COOKIE_NAME: &str = "tm_access_token";
pub const REFRESH_COOKIE_NAME: &str = "tm_refresh_token";
pub const OBSERVATION_HASH_LENGTH: usize = 16;

/// FastAPI `HTTPException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTTPException {
    pub status_code: u16,
    pub detail: String,
}

impl HTTPException {
    pub fn new(status_code: u16, detail: &str) -> Self {
        Self { status_code, detail: detail.to_string() }
    }
}

impl fmt::Display for HTTPException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status_code, self.detail)
    }
}

impl std::error::Error for HTTPException {}

/// Python `sys.exit(code)`. 프로세스를 끝내야 하는 설정 오류를 값으로 돌려준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemExit(pub i32);

/// 요청에서 인증에 필요한 부분만 읽는 능력. FastAPI `Request`의 `headers`/`cookies`에 대응한다.
pub trait RequestParts {
    fn header(&self, name: &str) -> Option<String>;
    fn cookie(&self, name: &str) -> Option<String>;
}

/// 이메일 -> 역할. 기본 구현은 환경 변수를 본다(Python `determine_role`).
pub trait RoleResolver: Send + Sync {
    fn determine_role(&self, email: &str) -> Option<String> {
        determine_role(email)
    }
}

/// 환경 변수로 역할을 정하는 실제 구현.
pub struct EnvRoleResolver;

impl RoleResolver for EnvRoleResolver {}

/// 토큰 발급·해독 seam. 기본 구현은 모두 실제 JWT다. 테스트는 쓰는 메서드만 바꾼다(Python `monkeypatch.setattr(main_mod, "create_jwt_token", ..)`).
pub trait TokenIssuer: Send + Sync {
    fn create_access_token(&self, email: &str, role: &str, name: &str, picture: &str) -> String {
        create_jwt_token(email, role, Some(name), Some(picture))
    }

    fn create_refresh_token(&self, email: &str, role: &str, name: &str, picture: &str, family_id: &str, token_id: &str) -> String {
        create_refresh_token_with(email, role, Some(name), Some(picture), Some(family_id), Some(token_id))
    }

    fn decode_refresh_token(&self, token: &str) -> Result<Value, HTTPException> {
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

/// 시작 때 한 번 부른다. 비밀키가 없으면 `SystemExit(-1)`이다(Python `sys.exit(-1)`).
pub fn load_jwt_secret(env: EnvFn) -> Result<String, SystemExit> {
    match env("TM_JWT_SECRET") {
        Some(secret) if !secret.is_empty() => Ok(secret),
        _ => {
            log::error!("The environment variable TM_JWT_SECRET is not set.");
            Err(SystemExit(-1))
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
/// Python은 `TM_ADMIN_EMAIL`이 비었을 때 빈 이메일을 admin으로 판정한다. Rust는 빈 이메일을 항상 거부한다.
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

fn base64url_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => continue,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// 관측 로그용 단방향 해시. Python `observation_hash`: `HMAC-SHA256(secret, "tm-observation|" + value)`의 hex 앞 16자.
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
    let hex: String = base64url_decode(&signature).iter().map(|b| format!("{b:02x}")).collect();
    hex.chars().take(OBSERVATION_HASH_LENGTH).collect()
}

// ---------------------------------------------------------------------------
// 토큰
// ---------------------------------------------------------------------------

fn sign(payload: &Value) -> String {
    let secret = jwt_secret();
    assert!(!secret.is_empty(), "The environment variable TM_JWT_SECRET is not set.");
    jsonwebtoken::encode(&Header::new(JWT_ALGORITHM), payload, &EncodingKey::from_secret(secret.as_bytes())).expect("JWT 서명")
}

/// 만료(`exp`)를 필수로 검사한다. 시계 오차 허용(leeway)은 두지 않는다(PyJWT 기본값과 같다).
fn decode_claims(token: &str) -> Result<Value, ErrorKind> {
    let secret = jwt_secret();
    if secret.is_empty() {
        return Err(ErrorKind::InvalidSignature);
    }
    let mut validation = Validation::new(JWT_ALGORITHM);
    validation.leeway = 0;
    jsonwebtoken::decode::<Value>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation).map(|data| data.claims).map_err(|e| e.into_kind())
}

/// 접근 토큰(2시간). `name`/`picture`가 None이면 빈 문자열이다.
pub fn create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String {
    let now = now_secs();
    sign(&json!({
        "type": "access", "email": email, "role": role, "name": name.unwrap_or(""), "picture": picture.unwrap_or(""),
        "exp": now + ACCESS_TOKEN_EXPIRATION_SECONDS, "iat": now,
    }))
}

/// 갱신 토큰(30일). family/token id를 안 주면 새로 만든다.
pub fn create_refresh_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String {
    create_refresh_token_with(email, role, name, picture, None, None)
}

pub fn create_refresh_token_with(email: &str, role: &str, name: Option<&str>, picture: Option<&str>, family_id: Option<&str>, token_id: Option<&str>) -> String {
    let now = now_secs();
    let family_id = family_id.filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(uuid_hex);
    let token_id = token_id.filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(uuid_hex);
    sign(&json!({
        "type": "refresh", "email": email, "role": role, "name": name.unwrap_or(""), "picture": picture.unwrap_or(""),
        "fid": family_id, "jti": token_id, "exp": now + REFRESH_TOKEN_EXPIRATION_SECONDS, "iat": now,
    }))
}

fn claim_present(payload: &Value, key: &str) -> bool {
    match payload.get(key) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

pub fn decode_refresh_token(token: &str) -> Result<Value, HTTPException> {
    let payload = decode_claims(token).map_err(|kind| match kind {
        ErrorKind::ExpiredSignature => HTTPException::new(401, "Refresh token expired"),
        _ => HTTPException::new(401, "Invalid refresh token"),
    })?;
    if payload.get("type").and_then(Value::as_str) != Some("refresh") {
        return Err(HTTPException::new(401, "Invalid token type"));
    }
    if !claim_present(&payload, "fid") || !claim_present(&payload, "jti") {
        return Err(HTTPException::new(401, "Refresh token missing required claims. Please log in again."));
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// 요청 검사
// ---------------------------------------------------------------------------

/// `Authorization: Bearer`가 있으면 그것을, 비었으면 접근 쿠키를 쓴다.
pub fn _extract_payload<R: RequestParts + ?Sized>(req: &R) -> Result<Value, HTTPException> {
    let header = req.header("Authorization").unwrap_or_default();
    let mut token = header.strip_prefix("Bearer ").unwrap_or("").to_string();
    if token.is_empty() {
        token = req.cookie(ACCESS_COOKIE_NAME).unwrap_or_default();
    }
    if token.is_empty() {
        return Err(HTTPException::new(401, "Missing authentication token"));
    }
    let payload = decode_claims(&token).map_err(|kind| match kind {
        ErrorKind::ExpiredSignature => HTTPException::new(401, "Token expired"),
        _ => HTTPException::new(401, "Invalid token"),
    })?;
    if payload.get("type").and_then(Value::as_str) != Some("access") {
        return Err(HTTPException::new(401, "Invalid token type"));
    }
    Ok(payload)
}

/// 토큰의 역할이 지금 정책과 같아야 한다. 허용 목록에서 빠진 사용자와 강등된 관리자는 토큰이 살아 있어도 거부한다.
fn resolve_current_role(payload: &Value, resolver: &dyn RoleResolver) -> Result<String, HTTPException> {
    let email = payload.get("email").and_then(Value::as_str).unwrap_or("");
    let current = resolver.determine_role(email);
    let token_role = payload.get("role").and_then(Value::as_str);
    match current {
        Some(role) if token_role == Some(role.as_str()) => Ok(role),
        _ => Err(HTTPException::new(403, "Access denied")),
    }
}

fn with_role(mut payload: Value, role: &str) -> Value {
    payload["role"] = Value::String(role.to_string());
    payload
}

pub async fn require_auth_with<R: RequestParts + ?Sized>(req: &R, resolver: &dyn RoleResolver) -> Result<Value, HTTPException> {
    let payload = _extract_payload(req)?;
    let role = resolve_current_role(&payload, resolver)?;
    if role != "admin" && role != "viewer" {
        return Err(HTTPException::new(403, "Insufficient permissions"));
    }
    Ok(with_role(payload, &role))
}

pub async fn require_admin_with<R: RequestParts + ?Sized>(req: &R, resolver: &dyn RoleResolver) -> Result<Value, HTTPException> {
    let payload = _extract_payload(req)?;
    let role = resolve_current_role(&payload, resolver)?;
    if role != "admin" {
        return Err(HTTPException::new(403, "Admin access required"));
    }
    Ok(with_role(payload, &role))
}

pub async fn require_auth<R: RequestParts + ?Sized>(req: &R) -> Result<Value, HTTPException> {
    require_auth_with(req, &EnvRoleResolver).await
}

pub async fn require_admin<R: RequestParts + ?Sized>(req: &R) -> Result<Value, HTTPException> {
    require_admin_with(req, &EnvRoleResolver).await
}

/// 인증에 실패하면 None. 오류를 만들지 않는다.
pub async fn optional_auth_with<R: RequestParts + ?Sized>(req: &R, resolver: &dyn RoleResolver) -> Option<Value> {
    require_auth_with(req, resolver).await.ok()
}

pub async fn optional_auth<R: RequestParts + ?Sized>(req: &R) -> Option<Value> {
    optional_auth_with(req, &EnvRoleResolver).await
}

// ---------------------------------------------------------------------------
// axum 요청 연결
// ---------------------------------------------------------------------------

/// `Cookie` 헤더(여러 개 가능)에서 `name`의 값을 찾는다. 같은 이름이 여러 번이면 뒤의 것이 이긴다(Starlette와 같다).
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
    fn base64url_decode_matches_known_value() {
        // "any carnal pleas" -> YW55IGNhcm5hbCBwbGVhcw
        assert_eq!(base64url_decode("YW55IGNhcm5hbCBwbGVhcw"), b"any carnal pleas");
        assert_eq!(base64url_decode("-_8"), vec![0xfb, 0xff]);
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
    fn observation_hash_matches_python_hmac_sha256() {
        // SAFETY: 이 lib 단위 테스트 바이너리에서 TM_JWT_SECRET을 바꾸는 테스트는 이것 하나다.
        unsafe { std::env::set_var("TM_JWT_SECRET", "testsecret123") };
        // python3: hmac.new(b"testsecret123", b"tm-observation|observed-family", hashlib.sha256).hexdigest()[:16]
        assert_eq!(observation_hash("observed-family"), "f4927567f8b53241");
        assert_eq!(observation_hash(""), "");
        unsafe { std::env::remove_var("TM_JWT_SECRET") };
    }
}
