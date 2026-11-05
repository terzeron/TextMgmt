//! request_schema 통합 테스트.

#[path = "../common/mod.rs"]
mod common;
use axum::http::Method;
use backend::auth_service::Claims;
use backend::main::{AppDeps, AuthMode, app_with};
use common::TestClient;

#[tokio::test]
async fn missing_required_body_fields_are_rejected() {
    common::setup_env();
    let auth = Claims { email: "admin@test.com".into(), role: "admin".into(), exp: 4_000_000_000, ..Claims::default() };
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(auth))));
    let expected: Vec<(String, String, Vec<String>)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/required_body_fields.json"))).unwrap();

    for (method, template, required) in expected {
        let path = template.replace("{book_id}", "1").replace("{category}", "contract-test").replace("{keyword}", "test").replace("{store_name}", "test");
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let response = client.request(method.clone(), &path, Some("application/json"), b"{}".to_vec()).await;
        assert_eq!(response.status, 422, "{method} {template} requires {required:?}; response: {}", response.text());
    }
}

#[tokio::test]
async fn openapi_json_body_examples_pass_deserialization() {
    common::setup_env();
    let auth = Claims { email: "admin@test.com".into(), role: "admin".into(), exp: 4_000_000_000, ..Claims::default() };
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(auth))));
    let examples: Vec<(String, String, String, serde_json::Value)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/json_body_samples.json"))).unwrap();

    for (method, template, content_type, body) in examples {
        let path = template.replace("{book_id}", "1").replace("{category}", "contract-test").replace("{keyword}", "test").replace("{store_name}", "test");
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let response = client.request(method.clone(), &path, Some(&content_type), serde_json::to_vec(&body).unwrap()).await;
        assert_ne!(response.status, 422, "schema example rejected: {method} {template} {body}; response: {}", response.text());
    }
}
