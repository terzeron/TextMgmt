//! `auth_manager` 테스트 공용 도구. crate 공용 helper는 `tests/rust/shared/`에서 가져온다.

#![allow(dead_code)]

#[path = "../../shared/auth_fixtures.rs"]
pub mod auth_fixtures;
#[path = "../../shared/clock.rs"]
pub mod clock;
#[path = "../../shared/env.rs"]
pub mod env;
#[path = "../../shared/log_capture.rs"]
pub mod log_capture;
#[path = "../../shared/setup.rs"]
mod setup;

#[allow(unused_imports)]
pub use setup::setup_env;
