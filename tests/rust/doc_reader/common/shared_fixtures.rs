//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/doc_reader/fixtures").join(name)
}
