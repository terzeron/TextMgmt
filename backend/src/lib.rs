//! TextMgmt 백엔드.

// 테스트가 `backend::main::app()`을 쓰도록 설계했다. 바이너리 진입점은 `src/bin/backend.rs`다.
#![allow(special_module_name)]

pub mod auth_service;
pub mod book_manager;
pub mod book_metadata;
pub mod comics_manager;
pub mod comics_metadata;
pub mod cover_management;
pub mod cover_sources;
pub mod cover_status;
pub mod direct_cover_url;
pub mod epub_cover;
pub mod epub_metadata_reader;
pub mod es_manager;
pub(crate) mod image_edit;
pub mod main;
pub mod pdf_page_reader;
pub mod refresh_token_store;
pub mod stderr_logger;
pub mod task_runner;
pub mod view_history_store;

use serde_json::Value;

/// 값이 있고 내용이 있는가: null과 빈 문자열·빈 배열·빈 객체만 없는 것으로 본다. 숫자와 불리언은 값이 있다.
pub(crate) fn has_content(value: Option<&Value>) -> bool {
    value.is_some_and(|v| match v {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    })
}
