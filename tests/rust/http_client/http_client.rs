//! `common::http_client` 단위 테스트: 응답 값 객체, 오류 문구, agent 설정.
//! 소켓을 거치는 세션 테스트는 integration/http_client.rs에 있다.

use std::io::Write;
use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
use std::time::Duration;

use http_client::{HttpError, HttpResponse, http_client_with_timeout};

#[test]
fn errors_have_stable_display_text() {
    assert_eq!(HttpError::Timeout.to_string(), "HTTP 요청 시간 초과");
    assert_eq!(HttpError::Connection("closed".into()).to_string(), "HTTP 연결 실패: closed");
    assert_eq!(HttpError::Other("bad status".into()).to_string(), "HTTP 오류: bad status");
}

#[test]
fn from_text_exposes_status_text_and_json() {
    let r = HttpResponse::from_text(200, "{\"a\": 1}");
    assert_eq!(r.status_code, 200);
    assert_eq!(r.json()["a"], 1);
    assert_eq!(HttpResponse::from_text(200, "한글").text(), "한글");
}

#[test]
fn debug_output_contains_response_metadata() {
    let r = HttpResponse::from_text(204, "");
    let debug = format!("{r:?}");
    assert!(debug.contains("status_code: 204"));
    assert!(debug.contains("headers: []"));
}

#[test]
fn from_chunks_concatenates_chunks_and_marks_the_body_closed() {
    let closed = Arc::new(AtomicBool::new(false));
    let r = HttpResponse::from_chunks(200, vec![], vec![b"ab".to_vec(), b"cd".to_vec()], closed.clone());
    assert!(!closed.load(Ordering::SeqCst));
    assert_eq!(r.text(), "abcd");
    assert!(closed.load(Ordering::SeqCst), "본문을 다 읽으면 닫아야 한다");
}

#[test]
fn dropping_an_unread_response_closes_the_body() {
    let closed = Arc::new(AtomicBool::new(false));
    let r = HttpResponse::from_chunks(200, vec![], vec![b"ab".to_vec()], closed.clone());
    drop(r);
    assert!(closed.load(Ordering::SeqCst));
}

#[test]
fn next_chunk_streams_until_exhausted() {
    let closed = Arc::new(AtomicBool::new(false));
    let mut r = HttpResponse::from_chunks(200, vec![], vec![b"ab".to_vec(), b"cd".to_vec()], closed);
    assert_eq!(r.next_chunk().unwrap().unwrap(), b"ab");
    assert_eq!(r.next_chunk().unwrap().unwrap(), b"cd");
    assert!(r.next_chunk().is_none());
}

#[test]
fn redirect_statuses_expose_the_location_header_case_insensitively() {
    for status in [301u16, 302, 303, 307, 308] {
        let r = HttpResponse::from_chunks(status, vec![("Location".into(), "https://x.test/a".into())], vec![], Arc::new(AtomicBool::new(false)));
        assert!(r.is_redirect(), "status={status}");
        assert_eq!(r.location(), Some("https://x.test/a"));
    }
    let ok = HttpResponse::from_text(200, "");
    assert!(!ok.is_redirect());
    assert_eq!(ok.location(), None);
    assert_eq!(ok.header("Content-Type"), None);
}

#[test]
fn http_client_with_timeout_reports_unreadable_or_empty_ca_files() {
    let timeout = Duration::from_secs(1);
    assert!(http_client_with_timeout(timeout, None).is_ok());

    let missing = std::path::Path::new("/nonexistent/ca.pem");
    let error = http_client_with_timeout(timeout, Some(missing)).unwrap_err();
    assert!(error.starts_with("CA 인증서를 읽지 못했다"), "{error}");

    let mut empty = tempfile::NamedTempFile::new().unwrap();
    empty.write_all(b"not a pem").unwrap();
    let error = http_client_with_timeout(timeout, Some(empty.path())).unwrap_err();
    assert!(error.starts_with("CA 인증서 PEM에 X.509 certificate가 없다"), "{error}");
}
