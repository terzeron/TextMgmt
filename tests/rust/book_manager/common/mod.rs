//! `book_manager` 테스트 공용 도구. crate 공용 helper는 `tests/rust/shared/`에서 가져온다.
//! HTTP 라우터 준비물(`TestClient`, `Fx`)은 backend 테스트에만 있다.

#![allow(dead_code)]

#[path = "../../shared/category_apply.rs"]
pub mod category_apply;
#[path = "../../shared/clock.rs"]
pub mod clock;
#[path = "../../shared/cover_candidates.rs"]
pub mod cover_candidates;
#[path = "../../shared/cover_docs.rs"]
pub mod cover_docs;
#[path = "../../shared/cover_env.rs"]
pub mod cover_env;
#[path = "../../shared/dyn_es.rs"]
pub mod dyn_es;
#[path = "../../shared/edits.rs"]
pub mod edits;
#[path = "../../shared/env.rs"]
pub mod env;
#[path = "../../shared/epub.rs"]
pub mod epub;
#[path = "../../shared/epub_book.rs"]
pub mod epub_book;
#[path = "../../shared/epub_fixtures.rs"]
pub mod epub_fixtures;
#[path = "../../shared/es_dummy.rs"]
pub mod es_dummy;
#[path = "../../shared/es_fake.rs"]
pub mod es_fake;
#[path = "../../shared/faulty_fs.rs"]
pub mod faulty_fs;
#[path = "../../shared/file_reader_fakes.rs"]
pub mod file_reader_fakes;
#[path = "../../shared/fsx.rs"]
pub mod fsx;
#[path = "../../shared/image.rs"]
pub mod image;
#[path = "../../shared/log_capture.rs"]
pub mod log_capture;
#[path = "../../shared/pdf.rs"]
pub mod pdf;
#[path = "../../shared/pdf_edit_env.rs"]
pub mod pdf_edit_env;
#[path = "../../shared/pdf_env.rs"]
pub mod pdf_env;
#[path = "../../shared/preview_epubs.rs"]
pub mod preview_epubs;
#[path = "../../shared/raster.rs"]
pub mod raster;
#[path = "../../shared/runner.rs"]
pub mod runner;
#[path = "../../shared/setup.rs"]
mod setup;
#[path = "../../shared/shared_fixtures.rs"]
pub mod shared_fixtures;
#[path = "../../shared/zipx.rs"]
pub mod zipx;

#[allow(unused_imports)]
pub use setup::{ES_FROM_FIXTURE, MYSQL_FROM_FIXTURE, project_root, setup_env};
