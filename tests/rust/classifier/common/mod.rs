//! 분류기 테스트 공용 도구. backend 공용 도구(`tests/rust/backend/common`) 중 backend와 axum을 쓰지 않는 파일만 가져온다.

#![allow(dead_code)]

#[path = "../../backend/common/env.rs"]
pub mod env;
#[path = "../../backend/common/epub.rs"]
pub mod epub;
#[path = "../../backend/common/es_dummy.rs"]
pub mod es_dummy;
#[path = "../../backend/common/faulty_fs.rs"]
pub mod faulty_fs;
#[path = "../../backend/common/image.rs"]
pub mod image;
#[path = "../../backend/common/log_capture.rs"]
pub mod log_capture;
#[path = "../../backend/common/setup.rs"]
mod setup;
pub mod shared_fixtures;
#[path = "../../backend/common/zipx.rs"]
pub mod zipx;

pub use setup::setup_env;
