//! `common::json_field_reader` 단위 테스트.

use common::json_field_reader::{has_content, str_field};
use serde_json::{Value, json};

#[test]
fn has_content_treats_only_null_and_empty_containers_as_missing() {
    assert!(!has_content(None));
    assert!(!has_content(Some(&Value::Null)));
    assert!(!has_content(Some(&json!(""))));
    assert!(!has_content(Some(&json!([]))));
    assert!(!has_content(Some(&json!({}))));
    assert!(has_content(Some(&json!(0))));
    assert!(has_content(Some(&json!(false))));
    assert!(has_content(Some(&json!("x"))));
    assert!(has_content(Some(&json!([1]))));
    assert!(has_content(Some(&json!({"x": 1}))));
}

#[test]
fn str_field_reads_strings_and_returns_empty_for_missing_or_other_types() {
    let value = json!({"title": "책", "count": 3, "none": null});
    assert_eq!(str_field(&value, "title"), "책");
    assert_eq!(str_field(&value, "count"), "");
    assert_eq!(str_field(&value, "none"), "");
    assert_eq!(str_field(&value, "missing"), "");
    assert_eq!(str_field(&json!(["title"]), "title"), "");
}
