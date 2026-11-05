//! 여러 crate가 함께 쓰는 기능.
//!
//! 시스템 능력(`sysdeps`)이나 특정 문서 형식(`doc_reader`, `htmlkit`)에 속하지 않는 것만 둔다.
//! 모듈 하나는 역할 하나이고, 두 crate 이상이 쓸 때만 여기에 올린다.

#[cfg(feature = "http_client")]
pub mod http_client;
pub mod json_field_reader;
pub mod url_parser;
