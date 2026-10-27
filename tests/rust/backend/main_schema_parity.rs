//! Required JSON body fields from FastAPI OpenAPI versus Rust request extractors.

mod common;

use axum::http::Method;
use backend::main::{AppDeps, AuthMode, app_with};
use common::TestClient;
use serde_json::json;

#[tokio::test]
async fn missing_required_python_body_fields_are_rejected_by_rust() {
    common::setup_env();
    let auth = json!({"email":"admin@test.com", "role":"admin", "exp": 4_000_000_000_i64});
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(auth))));
    let expected: Vec<(String, String, Vec<String>)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/python_required_body_fields.json"))).unwrap();

    for (method, template, required) in expected {
        let path = template.replace("{book_id}", "1").replace("{category}", "contract-test").replace("{keyword}", "test").replace("{store_name}", "test");
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let response = client.request(method.clone(), &path, Some("application/json"), b"{}".to_vec()).await;
        assert_eq!(response.status, 422, "{method} {template} requires {required:?}; response: {}", response.text());
    }
}

#[tokio::test]
async fn python_openapi_json_body_examples_pass_rust_deserialization() {
    common::setup_env();
    let auth = json!({"email":"admin@test.com", "role":"admin", "exp": 4_000_000_000_i64});
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(auth))));
    let examples: Vec<(String, String, String, serde_json::Value)> =
        serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/python_json_body_samples.json"))).unwrap();

    for (method, template, content_type, body) in examples {
        let path = template.replace("{book_id}", "1").replace("{category}", "contract-test").replace("{keyword}", "test").replace("{store_name}", "test");
        let method = Method::from_bytes(method.as_bytes()).unwrap();
        let response = client.request(method.clone(), &path, Some(&content_type), serde_json::to_vec(&body).unwrap()).await;
        assert_ne!(response.status, 422, "Python schema example rejected: {method} {template} {body}; response: {}", response.text());
    }
}
