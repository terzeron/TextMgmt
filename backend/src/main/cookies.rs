//! 인증 쿠키와 JSON 응답 도우미. Python `main.py`의 `_get_cookie_settings`, `_set_auth_cookies`, `CustomJSONResponse`에 대응한다.
//!
//! 쿠키 속성은 환경 변수(`TM_FRONTEND_URL`, `TM_COOKIE_SECURE`, `TM_COOKIE_SAMESITE`)를 호출 시점에 읽어 정한다.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use serde_json::ser::{Formatter, PrettyFormatter, Serializer};

use crate::auth::{ACCESS_COOKIE_NAME, ACCESS_TOKEN_EXPIRATION_SECONDS, REFRESH_COOKIE_NAME, REFRESH_TOKEN_EXPIRATION_SECONDS};

/// `Set-Cookie`를 붙일 수 있는 응답.
pub trait ResponseHeaders {
    fn headers_mut(&mut self) -> &mut HeaderMap;
}

impl ResponseHeaders for Response {
    fn headers_mut(&mut self) -> &mut HeaderMap {
        Response::headers_mut(self)
    }
}

/// 호스트 이름(소문자). `scheme://[user@]host[:port]/..` 꼴만 읽는다.
fn hostname_of(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(stripped) = host_port.strip_prefix('[') { stripped.split(']').next().unwrap_or("") } else { host_port.split(':').next().unwrap_or("") };
    (!host.is_empty()).then(|| host.to_lowercase())
}

pub fn is_local_frontend_origin(frontend_url: Option<&str>) -> bool {
    let Some(host) = frontend_url.filter(|u| !u.is_empty()).and_then(hostname_of) else {
        return false;
    };
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1")
}

fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn resolve_cookie_secure() -> bool {
    let frontend_url = env_var("TM_FRONTEND_URL");
    let explicit = env_var("TM_COOKIE_SECURE").filter(|v| !v.is_empty());
    if is_local_frontend_origin(frontend_url.as_deref()) {
        return explicit.unwrap_or_else(|| "false".to_string()).to_lowercase() == "true";
    }
    if explicit.is_some_and(|v| v.to_lowercase() != "true") {
        log::warn!("TM_COOKIE_SECURE=false is ignored for non-local frontend origins; forcing Secure cookies");
    }
    true
}

/// (Secure 여부, SameSite). SameSite는 "lax" | "strict" | "none"이다.
pub fn _get_cookie_settings() -> (bool, String) {
    let secure = resolve_cookie_secure();
    let mut samesite = match env_var("TM_COOKIE_SAMESITE").unwrap_or_else(|| "lax".to_string()).to_lowercase().as_str() {
        "strict" => "strict",
        "none" => "none",
        _ => "lax",
    };
    // SameSite=None은 Secure가 필수다(브라우저 요구사항).
    if samesite == "none" && !secure {
        log::warn!("SameSite=None requires Secure=True; falling back to SameSite=Lax");
        samesite = "lax";
    }
    (secure, samesite.to_string())
}

fn set_cookie_value(name: &str, value: &str, secure: bool, samesite: &str, max_age: i64) -> HeaderValue {
    // 속성 순서는 Starlette(`http.cookies`)가 만드는 순서와 같다.
    let text = format!("{name}={value}; HttpOnly; Max-Age={max_age}; Path=/; SameSite={samesite}{}", if secure { "; Secure" } else { "" });
    HeaderValue::from_str(&text).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// 접근 쿠키(와 있으면 갱신 쿠키)를 응답에 붙인다. 둘 다 HttpOnly, Path=/다.
pub fn _set_auth_cookies<'a, R: ResponseHeaders>(response: &mut R, access_token: &str, refresh_token: impl Into<Option<&'a str>>) {
    let (secure, samesite) = _get_cookie_settings();
    let headers = response.headers_mut();
    headers.append(header::SET_COOKIE, set_cookie_value(ACCESS_COOKIE_NAME, access_token, secure, &samesite, ACCESS_TOKEN_EXPIRATION_SECONDS));
    if let Some(refresh) = refresh_token.into() {
        headers.append(header::SET_COOKIE, set_cookie_value(REFRESH_COOKIE_NAME, refresh, secure, &samesite, REFRESH_TOKEN_EXPIRATION_SECONDS));
    }
}

/// 두 쿠키를 `Max-Age=0`으로 지운다.
pub fn _clear_auth_cookies<R: ResponseHeaders>(response: &mut R) {
    let (secure, samesite) = _get_cookie_settings();
    let headers = response.headers_mut();
    headers.append(header::SET_COOKIE, set_cookie_value(ACCESS_COOKIE_NAME, "", secure, &samesite, 0));
    headers.append(header::SET_COOKIE, set_cookie_value(REFRESH_COOKIE_NAME, "", secure, &samesite, 0));
}

/// Python `custom_jsonable_encoder`: 문자열을 건드리지 않는다(한글이 이스케이프되지 않는다). JSON 값은 이미 그 상태라 복사만 한다.
pub fn custom_jsonable_encoder(value: &Value) -> Value {
    value.clone()
}

/// `json.dumps(indent=2, separators=(",", ":"), ensure_ascii=False)`와 같은 모양을 만드는 formatter.
struct IndentedCompactColon(PrettyFormatter<'static>);

impl Formatter for IndentedCompactColon {
    fn begin_array<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.begin_array(w)
    }

    fn end_array<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_array(w)
    }

    fn begin_array_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W, first: bool) -> std::io::Result<()> {
        self.0.begin_array_value(w, first)
    }

    fn end_array_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_array_value(w)
    }

    fn begin_object<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.begin_object(w)
    }

    fn end_object<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_object(w)
    }

    fn begin_object_key<W: ?Sized + std::io::Write>(&mut self, w: &mut W, first: bool) -> std::io::Result<()> {
        self.0.begin_object_key(w, first)
    }

    fn begin_object_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b":")
    }

    fn end_object_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        self.0.end_object_value(w)
    }
}

/// Python `CustomJSONResponse`: 들여쓰기 2칸, 구분자 `,`와 `:`, 한글 그대로.
pub struct CustomJSONResponse {
    pub status_code: u16,
    pub body: Vec<u8>,
    headers: HeaderMap,
}

impl CustomJSONResponse {
    pub fn new(content: Value) -> Self {
        let mut body = Vec::new();
        let mut ser = Serializer::with_formatter(&mut body, IndentedCompactColon(PrettyFormatter::with_indent(b"  ")));
        // Value 직렬화는 실패하지 않는다.
        let _ = content.serialize(&mut ser);
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Self { status_code: 200, body, headers }
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
}

impl ResponseHeaders for CustomJSONResponse {
    fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
}

impl IntoResponse for CustomJSONResponse {
    fn into_response(self) -> Response {
        let mut resp = Response::new(Body::from(self.body));
        *resp.status_mut() = StatusCode::from_u16(self.status_code).unwrap_or(StatusCode::OK);
        *resp.headers_mut() = self.headers;
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hostname_parsing() {
        assert_eq!(hostname_of("http://localhost:3000").as_deref(), Some("localhost"));
        assert_eq!(hostname_of("https://User@App.Example.com:8443/x?y#z").as_deref(), Some("app.example.com"));
        assert_eq!(hostname_of("http://[::1]:3000/").as_deref(), Some("::1"));
        assert_eq!(hostname_of("localhost:3000"), None);
        assert!(is_local_frontend_origin(Some("http://127.0.0.1:3000")));
        assert!(!is_local_frontend_origin(Some("https://app.example.com")));
        assert!(!is_local_frontend_origin(None));
    }

    #[test]
    fn custom_json_matches_python_dumps_indent_2() {
        let resp = CustomJSONResponse::new(json!({"k": "한글", "list": [1, {"x": []}], "empty": {}}));
        let expected = "{\n  \"k\":\"한글\",\n  \"list\":[\n    1,\n    {\n      \"x\":[]\n    }\n  ],\n  \"empty\":{}\n}";
        assert_eq!(String::from_utf8(resp.body.clone()).unwrap(), expected);
        assert_eq!(resp.headers().get("content-type").unwrap(), "application/json");
    }

    #[test]
    fn set_cookie_attribute_order_matches_starlette() {
        let v = set_cookie_value("tm_access_token", "t", true, "lax", 7200);
        assert_eq!(v.to_str().unwrap(), "tm_access_token=t; HttpOnly; Max-Age=7200; Path=/; SameSite=lax; Secure");
    }
}
