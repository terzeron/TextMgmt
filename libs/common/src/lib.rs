//! 여러 crate가 함께 쓰는 기능.
//!
//! 시스템 능력(`sysdeps`)이나 특정 문서 형식(`doc_reader`, `htmlkit`)에 속하지 않는 것만 둔다.
//! 모듈 하나는 역할 하나이고, 두 crate 이상이 쓸 때만 여기에 올린다.

#[cfg(feature = "http_client")]
pub mod http_client;
pub mod json_field_reader;
pub mod text_sanitizer;
pub mod url_parser;
#[cfg(feature = "xml_parser")]
pub mod xml_parser;
#[cfg(feature = "zip_reader")]
pub mod zip_reader;

/// trait 기본 구현이 아직 없는 메서드가 돌려주는 오류 문구. 테스트 대역은 쓰는 메서드만 구현하고 나머지는 이 오류를 낸다.
pub const UNIMPLEMENTED: &str = "unimplemented";

/// panic 페이로드의 메시지. `panic!`이 넘긴 `&str`이나 `String`이 아니면 `None`이다. 없을 때의 문구는 호출자가 정한다.
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> Option<&str> {
    payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str))
}
