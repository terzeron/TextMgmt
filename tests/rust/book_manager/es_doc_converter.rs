//! `book_manager::es_doc_converter` 단위 테스트.

use base64::Engine;
use book_manager::{decode_category_cursor, encode_category_cursor};
use serde_json::json;

#[test]
fn test_encode_decode_category_cursor() {
    let cur = encode_category_cursor(&json!([1, "val"]));
    assert_eq!(decode_category_cursor(&cur), Some(json!([1, "val"])));
    assert_eq!(decode_category_cursor("invalid_base64!!!"), None);
    let not_list = base64::engine::general_purpose::URL_SAFE.encode(br#"{"a": 1}"#);
    assert_eq!(decode_category_cursor(&not_list), None);
}
