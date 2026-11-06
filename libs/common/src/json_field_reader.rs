//! JSON 객체에서 필드 값을 읽는다. 외부 API 응답, ES 문서, 메타데이터를 읽는 쪽이 함께 쓴다.

use serde_json::Value;

/// 값이 있고 내용이 있는가: null과 빈 문자열·빈 배열·빈 객체만 없는 것으로 본다. 숫자와 불리언은 값이 있다.
pub fn has_content(value: Option<&Value>) -> bool {
    value.is_some_and(|v| match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    })
}

/// 문자열 필드. 필드가 없거나 문자열이 아니면 빈 문자열이다.
pub fn str_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
