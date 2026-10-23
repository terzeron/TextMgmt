//! TextMgmt 백엔드. Python `backend/*.py`를 Rust로 옮기는 중이다.

// 테스트가 `backend::main::app()`을 쓰도록 설계했다. 바이너리 진입점은 `src/bin/backend.rs`다.
#![allow(special_module_name)]

pub mod es_manager;
pub mod main;
pub mod db;
pub mod view_history_store;
pub mod category_mapping;
pub mod refresh_token_store;
pub mod book;
pub mod image_edit;
pub mod cover_browse;
pub mod cover_sources;
pub mod direct_cover_url;
pub mod comics;
pub mod comics_manager;
