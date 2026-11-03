//! ES REST 클라이언트 테스트. 실제 ES 대신 한 번만 응답하는 로컬 HTTP 서버를 쓴다.

#[path = "../common/tls_server.rs"]
mod tls_server;

use std::path::Path;

use es_client::{EsClient, EsError};

fn client(url: &str) -> EsClient {
    EsClient::new(url, "elastic", "pw", "books")
}

#[test]
fn from_env_reports_the_first_missing_variable() {
    // 다른 테스트와 환경을 공유하지 않도록 이 테스트만 변수를 건드린다.
    for key in ["TM_ES_BOOK_INDEX", "TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD"] {
        unsafe { std::env::remove_var(key) };
    }
    assert!(matches!(EsClient::from_env(), Err(EsError::MissingEnv(k)) if k == "TM_ES_BOOK_INDEX"));
    unsafe {
        std::env::set_var("TM_ES_BOOK_INDEX", "books");
        std::env::set_var("TM_ES_URL", "http://127.0.0.1:1");
        std::env::set_var("TM_ES_USER", "u");
    }
    assert!(matches!(EsClient::from_env(), Err(EsError::MissingEnv(k)) if k == "TM_ES_PASSWORD"));
    unsafe { std::env::set_var("TM_ES_PASSWORD", "p") };
    assert!(EsClient::from_env().is_ok());
}

#[test]
fn https_uses_the_configured_ca_and_verifies_the_server_certificate() {
    let (url, server) = tls_server::serve_tls_once();
    let ca_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/common/tls/local-es.crt");
    let client = EsClient::new_with_ca(&url, "u", "p", "books", Some(&ca_path)).unwrap();

    assert_eq!(client.connect_with_retry(1, &|_| panic!("one successful TLS request should not retry")).unwrap()["version"]["number"], "9.1.0");
    server.join().unwrap();
}

#[test]
fn https_rejects_an_untrusted_server_certificate() {
    let (url, server) = tls_server::serve_tls_once();
    let error = EsClient::new(&url, "u", "p", "books").connect_with_retry(1, &|_| {}).unwrap_err();

    assert!(matches!(error, EsError::Transport(_)), "{error}");
    server.join().unwrap();
}

#[test]
fn connect_with_retry_gives_up_after_the_configured_attempts_and_sleeps_between() {
    let waits = std::cell::RefCell::new(Vec::new());
    // 닫힌 포트: 연결이 항상 거부된다.
    let c = client("http://127.0.0.1:1");
    let err = c.connect_with_retry(3, &|secs| waits.borrow_mut().push(secs)).unwrap_err();
    assert!(matches!(err, EsError::Transport(_)), "{err}");
    // 시도 3번 사이에 2번 쉰다. 2**(n+1)초, 최대 10초.
    assert_eq!(*waits.borrow(), [2, 4]);
}

// ---- 분기 보강 ----

#[test]
fn error_messages_are_readable() {
    assert!(EsError::Transport("refused".into()).to_string().contains("연결 오류"));
    assert!(EsError::Parse("x".into()).to_string().contains("해석 실패"));
    assert!(EsError::Unsupported.to_string().contains("지원하지 않는"));
    assert!(EsError::MissingEnv("K".into()).to_string().contains("K"));
}
