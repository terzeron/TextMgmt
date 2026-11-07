//! `backend::api_router::error_responses` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use serde_json::{Value, json};

/// 요청 본문 요약 함수.
#[test]
fn test_list_body() {
    let result = backend::api_router::summarize_request_body(&json!([1, 2, 3]));
    assert_eq!(result, json!({"type": "list", "length": 3}));
}

#[test]
fn test_none_body() {
    let result = backend::api_router::summarize_request_body(&Value::Null);
    assert_eq!(result, json!({"type": "none"}));
}

#[test]
fn test_str_body() {
    let result = backend::api_router::summarize_request_body(&json!("hello"));
    assert_eq!(result, json!({"type": "str", "length": 5}));
}

#[test]
fn test_scalar_bodies_report_type_and_printed_length() {
    let summarize = backend::api_router::summarize_request_body;
    assert_eq!(summarize(&json!(true)), json!({"type": "bool", "length": 4}));
    assert_eq!(summarize(&json!(false)), json!({"type": "bool", "length": 5}));
    assert_eq!(summarize(&json!(1234)), json!({"type": "int", "length": 4}));
    assert_eq!(summarize(&json!(1.5)), json!({"type": "float", "length": 3}));
}

#[test]
fn test_validation_error_displays_its_error_count() {
    let error = backend::api_router::RequestValidationError::new(vec![json!({}), json!({})], Value::Null);
    assert_eq!(error.to_string(), "2 validation error(s)");
}
