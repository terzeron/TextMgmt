//! 원본: tests/test_client_error_log.py
//!
//! `POST /logs/client-error` 엔드포인트: 익명·로그인 사용자 기록, 검증 오류, 입력 상한과 로그 위조 방어(CWE-117, CWE-770).
//!
//! 가정한 API:
//! - `backend::main::app() -> axum::Router` (common::TestClient가 쓴다)
//! - `backend::auth::create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String` (4인자 통일안. 이 파일은 `jwt()`로 감싼다)
//! - `backend::auth::ACCESS_COOKIE_NAME: &str`
//! - `backend::main::CLIENT_ERROR_FIELD_LIMITS: LazyLock<HashMap<&'static str, usize>>` (필드 이름 -> 최대 길이)
//! - 핸들러는 `log::error!`로 `[CLIENT_ERROR] type=..., user=email(role)` 줄과 `Component Stack:`/`Stack Trace:` 줄을 남긴다.
//!   `caplog`는 `common::log_capture::LogCapture`(스레드 로컬, `log` crate 기반)로 대신한다. 설계: docs/rust-test-seams.md 3.2, 3.10.

mod common;

use backend::auth::{ACCESS_COOKIE_NAME, create_jwt_token};
use backend::main::CLIENT_ERROR_FIELD_LIMITS;
use common::TestClient;
use common::log_capture::LogCapture;
use log::Level;
use serde_json::{Value, json};

// TODO(common): `common::fx::jwt(email, role)`로 옮긴다. cover, pdf_pages, book_preview도 같은 헬퍼가 필요하다(문서 3.2).
fn jwt(email: &str, role: &str) -> String {
    create_jwt_token(email, role, None, None)
}

fn client() -> TestClient {
    TestClient::new()
}

#[tokio::test]
async fn test_log_client_error_anonymous() {
    // 비로그인(익명) 사용자의 에러 로그 전송 성공 및 로그 확인
    let payload = json!({
        "error_type": "REACT_RENDER_ERROR",
        "message": "Cannot read properties of null (reading 'map')",
        "stack": "TypeError: Cannot read properties of null\n    at BookView.jsx:42:15",
        "component_stack": "\n    at BookView\n    at App",
        "url": "http://localhost:3000/book-view/1",
        "user_agent": "Mozilla/5.0 TestBrowser",
        "timestamp": "2026-08-30T11:30:00Z",
    });
    let logs = LogCapture::start(Level::Error);
    let response = client().post_json("/logs/client-error", &payload).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json(), json!({"status": "ok"}));
    let text = logs.text();
    assert!(text.contains("[CLIENT_ERROR] type=REACT_RENDER_ERROR, user=anonymous(anonymous)"), "{text}");
    assert!(text.contains("Cannot read properties of null"), "{text}");
    assert!(text.contains("Component Stack:"), "{text}");
    assert!(text.contains("Stack Trace:"), "{text}");
}

#[tokio::test]
async fn test_log_client_error_authenticated() {
    // 로그인된 사용자의 에러 로그 전송 시 사용자 이메일 및 역할 로깅 확인
    common::setup_env();
    let token = jwt("viewer@test.com", "viewer");
    let payload = json!({
        "error_type": "WINDOW_ERROR",
        "message": "Uncaught SyntaxError: Unexpected token",
        "url": "http://localhost:3000/viewer/epub/123",
    });
    let logs = LogCapture::start(Level::Error);
    let response = client().cookie(ACCESS_COOKIE_NAME, &token).post_json("/logs/client-error", &payload).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json(), json!({"status": "ok"}));
    let text = logs.text();
    assert!(text.contains("[CLIENT_ERROR] type=WINDOW_ERROR, user=viewer@test.com(viewer)"), "{text}");
}

#[tokio::test]
async fn test_log_client_error_invalid_error_type() {
    // 허용되지 않은 error_type 전송 시 422 검증 오류
    let payload = json!({
        "error_type": "INVALID_ERROR_TYPE",
        "message": "Some error",
        "url": "http://localhost:3000/",
    });
    let response = client().post_json("/logs/client-error", &payload).await;
    assert_eq!(response.status, 422);
}

#[tokio::test]
async fn test_log_client_error_missing_required_fields() {
    // 필수 필드(message, url 등) 누락 시 422 검증 오류
    // message와 url이 없다.
    let payload = json!({"error_type": "UNHANDLED_PROMISE"});
    let response = client().post_json("/logs/client-error", &payload).await;
    assert_eq!(response.status, 422);
}

// --- 무인증 경로 입력 상한과 로그 위조 방어 (CWE-117, CWE-770) ---

#[tokio::test]
async fn test_log_client_error_truncates_oversized_fields() {
    // 상한을 넘는 필드는 거부하지 않고 잘라서 기록한다.
    let payload = json!({
        "error_type": "CUSTOM_ERROR",
        "message": "A".repeat(10000),
        "stack": "B".repeat(50000),
        "component_stack": "C".repeat(50000),
        "url": format!("http://localhost/{}", "D".repeat(10000)),
        "user_agent": "E".repeat(5000),
        "timestamp": "F".repeat(5000),
    });
    let logs = LogCapture::start(Level::Error);
    let response = client().post_json("/logs/client-error", &payload).await;

    assert_eq!(response.status, 200);
    let text = logs.text();
    let message_limit = CLIENT_ERROR_FIELD_LIMITS["message"];
    let stack_limit = CLIENT_ERROR_FIELD_LIMITS["stack"];
    assert!(text.contains(&"A".repeat(message_limit)));
    assert!(!text.contains(&"A".repeat(message_limit + 1)));
    assert!(text.contains(&"B".repeat(stack_limit)));
    assert!(!text.contains(&"B".repeat(stack_limit + 1)));
}

#[tokio::test]
async fn test_log_client_error_strips_newlines_from_single_line_fields() {
    // message 의 개행으로 가짜 로그 줄을 심을 수 없어야 한다.
    let payload = json!({
        "error_type": "CUSTOM_ERROR",
        "message": "real error\n[CLIENT_ERROR] type=FAKE, user=admin@evil.com(admin)",
        "url": "http://localhost/x",
    });
    let logs = LogCapture::start(Level::Error);
    let response = client().post_json("/logs/client-error", &payload).await;

    assert_eq!(response.status, 200);
    let logged = logs.records().into_iter().map(|r| r.message).find(|m| m.contains("[CLIENT_ERROR] type=")).expect("[CLIENT_ERROR] 기록");
    // 개행이 공백이 되어 한 줄로 남는다. 새 줄이 생기지 않으므로 로그를 읽을 때
    // 위조된 항목이 별도 기록으로 보이지 않는다.
    assert!(!logged.contains('\n'), "{logged}");
    assert!(logged.contains("real error [CLIENT_ERROR] type=FAKE"), "{logged}");
}

#[tokio::test]
async fn test_log_client_error_escapes_newlines_in_stack() {
    // stack 은 여러 줄이 의미를 가지므로 개행을 보존한다.
    let payload = json!({
        "error_type": "CUSTOM_ERROR",
        "message": "boom",
        "stack": "TypeError: boom\n[CLIENT_ERROR] type=FAKE, user=admin@evil.com(admin)\n    at B.jsx:2:2",
        "url": "http://localhost/x",
    });
    let logs = LogCapture::start(Level::Error);
    let response = client().post_json("/logs/client-error", &payload).await;

    assert_eq!(response.status, 200);
    let messages: Vec<String> = logs.records().into_iter().map(|r| r.message).collect();
    let stack_record = messages.iter().find(|m| m.contains("Stack Trace:")).expect("Stack Trace 기록");
    assert!(stack_record.contains(r"[CLIENT_ERROR] type=FAKE, user=admin@evil.com(admin)\n"), "{stack_record}");
    let other_records: Vec<&String> = messages.iter().filter(|m| !m.contains("Stack Trace:")).collect();
    assert!(!other_records.iter().any(|record| record.contains("[CLIENT_ERROR] type=FAKE")), "{other_records:?}");
}

#[tokio::test]
async fn test_log_client_error_accepts_realistic_payload_unchanged() {
    // 정상 클라이언트가 보내는 크기는 잘리지 않아야 한다(프론트는 이미 자른다).
    let message = "Cannot read properties of null (reading 'map')";
    let url = format!("https://tm.terzeron.com/book-edit/204252164?category={}", "%EC%97%AD".repeat(40));
    let payload: Value = json!({
        "error_type": "REACT_RENDER_ERROR",
        "message": message,
        "stack": format!("TypeError\n{}", "    at Component.jsx:10:5\n".repeat(100)),
        "url": url,
        "user_agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36",
    });
    let logs = LogCapture::start(Level::Error);
    let response = client().post_json("/logs/client-error", &payload).await;

    assert_eq!(response.status, 200);
    let text = logs.text();
    assert!(text.contains(message));
    assert!(text.contains(&url), "422 자 URL 이 그대로 남아야 한다");
}
