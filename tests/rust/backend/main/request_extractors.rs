//! `backend::main::request_extractors` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{ip_headers};
use crate::common::shared_fixtures::{DEFAULT_CLIENT_HOST};

#[test]
fn test_prefers_cf_connecting_ip_over_proxy_hops() {
    // 운영 경로에서 XFF 첫 항목은 Cloudflare edge IP 라 신뢰할 수 없다.
    let headers = ip_headers(&[("CF-Connecting-IP", "203.0.113.77"), ("X-Forwarded-For", "104.22.17.33")]);
    assert_eq!(backend::main::_client_ip(&headers, Some("10.1.202.80")), "203.0.113.77");
}

#[test]
fn test_falls_back_to_true_client_ip() {
    let headers = ip_headers(&[("True-Client-IP", "203.0.113.78"), ("X-Forwarded-For", "104.22.17.33")]);
    assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "203.0.113.78");
}

#[test]
fn test_falls_back_to_forwarded_for_without_cloudflare() {
    // Cloudflare 를 거치지 않는 경로에서는 XFF 첫 항목을 쓴다.
    let headers = ip_headers(&[("X-Forwarded-For", "198.51.100.23, 10.1.2.3")]);
    assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "198.51.100.23");
}

#[test]
fn test_falls_back_to_peer_when_no_headers() {
    assert_eq!(backend::main::_client_ip(&ip_headers(&[]), Some(DEFAULT_CLIENT_HOST)), "192.0.2.9");
}

#[test]
fn test_ignores_blank_headers() {
    let headers = ip_headers(&[("CF-Connecting-IP", "  "), ("X-Forwarded-For", " , 10.1.2.3")]);
    assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "192.0.2.9");
}

#[test]
fn test_returns_empty_when_no_client_info() {
    assert_eq!(backend::main::_client_ip(&ip_headers(&[]), None), "");
}

#[test]
fn test_keeps_full_ipv6() {
    let headers = ip_headers(&[("CF-Connecting-IP", "2001:db8:abcd:1234::1")]);
    assert_eq!(backend::main::_client_ip(&headers, Some(DEFAULT_CLIENT_HOST)), "2001:db8:abcd:1234::1");
}

mod request_ip_prefix {
    use crate::common::shared_fixtures::{DEFAULT_CLIENT_HOST};
    use crate::common::shared_fixtures::{ip_headers};


    #[test]
    fn test_masks_cf_connecting_ip() {
        // 프리픽스도 프록시 IP 가 아니라 실제 클라이언트 IP 를 기준으로 만든다.
        let headers = ip_headers(&[("CF-Connecting-IP", "203.0.113.77"), ("X-Forwarded-For", "104.22.17.33")]);
        assert_eq!(backend::main::_request_ip_prefix(&headers, Some(DEFAULT_CLIENT_HOST)), "203.0.113");
    }

    #[test]
    fn test_uses_forwarded_for_first_hop_ipv4() {
        let headers = ip_headers(&[("X-Forwarded-For", "198.51.100.23, 10.1.2.3")]);
        assert_eq!(backend::main::_request_ip_prefix(&headers, Some(DEFAULT_CLIENT_HOST)), "198.51.100");
    }

    #[test]
    fn test_truncates_ipv6_to_48_bits() {
        let headers = ip_headers(&[("X-Forwarded-For", "2001:db8:abcd:1234::1")]);
        assert_eq!(backend::main::_request_ip_prefix(&headers, Some(DEFAULT_CLIENT_HOST)), "2001:db8:abcd");
    }

    #[test]
    fn test_falls_back_to_client_host() {
        assert_eq!(backend::main::_request_ip_prefix(&ip_headers(&[]), Some(DEFAULT_CLIENT_HOST)), "192.0.2");
    }

    #[test]
    fn test_returns_empty_when_no_client_info() {
        assert_eq!(backend::main::_request_ip_prefix(&ip_headers(&[]), None), "");
    }
}
