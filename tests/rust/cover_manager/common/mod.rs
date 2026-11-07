//! `cover_manager` 테스트 공용 도구. crate 공용 helper는 `tests/rust/shared/`에서 가져온다.

#![allow(dead_code)]

#[path = "../../shared/clock.rs"]
pub mod clock;
#[path = "../../shared/cover_candidates.rs"]
pub mod cover_candidates;
#[path = "../../shared/epub.rs"]
pub mod epub;
#[path = "../../shared/epub_book.rs"]
pub mod epub_book;
#[path = "../../shared/epub_fixtures.rs"]
pub mod epub_fixtures;
#[path = "../../shared/es_dummy.rs"]
pub mod es_dummy;
#[path = "../../shared/faulty_fs.rs"]
pub mod faulty_fs;
#[path = "../../shared/fsx.rs"]
pub mod fsx;
#[path = "../../shared/image.rs"]
pub mod image;
#[path = "../../shared/log_capture.rs"]
pub mod log_capture;
#[path = "../../shared/setup.rs"]
mod setup;
#[path = "../../shared/zipx.rs"]
pub mod zipx;

#[allow(unused_imports)]
pub use setup::setup_env;
