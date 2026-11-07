//! `search_manager` 테스트 공용 도구. crate 공용 helper는 `tests/rust/shared/`에서 가져온다.

#![allow(dead_code)]

use std::time::Duration;

use search_manager::HttpTransport;

#[path = "../../shared/clock.rs"]
pub mod clock;
#[path = "../../shared/env.rs"]
pub mod env;
#[path = "../../shared/es_dummy.rs"]
pub mod es_dummy;
#[path = "../../shared/faulty_fs.rs"]
pub mod faulty_fs;
#[path = "../../shared/setup.rs"]
mod setup;

#[allow(unused_imports)]
pub use setup::setup_env;

/// 로컬 테스트 서버로 보내는 http 전송.
pub fn transport(url: &str) -> HttpTransport {
    HttpTransport::new(url, "elastic", "pw", "books", Duration::from_secs(5))
}
