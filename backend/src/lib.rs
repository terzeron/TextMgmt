//! TextMgmt 백엔드.

// 테스트가 `backend::main::app()`을 쓰도록 설계했다. 바이너리 진입점은 `src/bin/backend.rs`다.
#![allow(special_module_name)]

pub mod auth;
pub mod book;
pub mod book_classifier;
pub mod book_manager;
pub mod bookstore;
pub mod bookstore_cache;
pub mod category_mapping;
pub mod classifier;
pub mod comics;
pub mod comics_manager;
pub mod cover_browse;
pub mod cover_management;
pub mod cover_sources;
pub mod cover_status;
pub mod db;
pub mod direct_cover_url;
pub mod epub_cover;
pub mod es_manager;
pub mod image_edit;
pub mod main;
pub mod pdf;
pub mod refresh_token_store;
pub mod view_history_store;
