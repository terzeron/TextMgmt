//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

use classifier::vectorize::field_text;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// field_text, softmax
// ---------------------------------------------------------------------------

#[test]
fn field_text_matches_python_field_text() {
    let doc = json!({"text": "abcdef", "name": "vol_12345_x9_77.epub", "author": ["가", "", null, "나"], "n": 0, "t": 12, "b": true});
    assert_eq!(field_text(&doc, "text", 3), "abc");
    assert_eq!(field_text(&doc, "text", 0), "abcdef");
    assert_eq!(field_text(&doc, "text", 100), "abcdef");
    // name: 두 자리 이상 숫자열만 공백으로, 그 뒤에 자른다.
    assert_eq!(field_text(&doc, "name", 0), "vol_ _x9_ .epub");
    assert_eq!(field_text(&doc, "name", 6), "vol_ _");
    assert_eq!(field_text(&doc, "author", 0), "가 나");
    assert_eq!(field_text(&doc, "missing", 0), "");
    assert_eq!(field_text(&doc, "n", 0), "");
    assert_eq!(field_text(&doc, "t", 0), "12");
    assert_eq!(field_text(&doc, "b", 0), "True");
    assert_eq!(field_text(&json!({"text": "가나다라"}), "text", 2), "가나");
    assert_eq!(field_text(&Value::Null, "text", 0), "");
}
