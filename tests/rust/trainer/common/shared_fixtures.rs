//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use trainer::{CliIo, Deps, main_with};

pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained")
}

pub fn run(args: &[&str], deps: &Deps) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}
