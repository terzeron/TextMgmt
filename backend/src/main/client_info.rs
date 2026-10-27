//! 요청한 클라이언트의 IP와 User-Agent. Python `_client_ip`, `_request_ip_prefix`, `_request_user_agent`.
//!
//! 실제 경로는 클라이언트 -> Cloudflare -> Traefik -> 백엔드 pod다. 이 구성에서
//! - 접속 상대(peer)는 Traefik pod IP이고,
//! - `X-Forwarded-For` 첫 항목은 Cloudflare edge IP다(Cloudflare가 XFF를 보내지 않아 Traefik이 자기가 본 peer로 새로 만든다).
//!
//! 둘 다 프록시 IP라서, Cloudflare가 원 클라이언트 IP로 채워 주는 `CF-Connecting-IP`를 먼저 쓴다.
//! 헤더는 위조할 수 있으므로 표시·감사 전용이다. 인가 판단에는 쓰지 않는다.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;

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
