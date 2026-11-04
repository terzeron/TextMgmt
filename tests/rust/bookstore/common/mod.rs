//! bookstore 테스트 공용 모듈.
#![allow(dead_code)]

pub mod shared_fixtures;

/// `env_helpers`가 부르는 자리. 서점 코드는 backend 환경 변수(`TM_BOOK_DIR`, MySQL, ES)를 읽지 않으므로 맞출 값이 없다.
pub fn setup_env() {}
