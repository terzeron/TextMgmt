//! `backend::main::error_responses` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use serde_json::{Value, json};

/// 요청 본문 요약 함수.
#[test]
fn test_list_body() {
    let result = backend::main::summarize_request_body(&json!([1, 2, 3]));
    assert_eq!(result, json!({"type": "list", "length": 3}));
}

#[test]
fn test_none_body() {
    let result = backend::main::summarize_request_body(&Value::Null);
    assert_eq!(result, json!({"type": "none"}));
}

#[test]
fn test_str_body() {
    let result = backend::main::summarize_request_body(&json!("hello"));
    assert_eq!(result, json!({"type": "str", "length": 5}));
}
