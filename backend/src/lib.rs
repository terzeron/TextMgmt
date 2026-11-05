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
