//! bookdoctor 테스트 준비물. 검사 준비물은 doc_checker 테스트의 것을 그대로 쓰고, 명령행 실행만 더한다.
#![allow(dead_code)]

#[path = "../../doc_checker/support/mod.rs"]
mod checker;
#[allow(unused_imports)] // 테스트 바이너리마다 쓰는 준비물이 다르다.
pub use checker::*;

pub mod shared_fixtures {
    #[allow(unused_imports)] // 테스트 바이너리마다 쓰는 준비물이 다르다.
    pub use super::checker::shared_fixtures::*;

    use std::path::Path;

    use bookdoctor::cli_parser::{execute, parse};
    use bookdoctor::option_model::Options;

    /// `root`를 코퍼스 루트로 삼아 명령을 실행한다. 휴지통은 `root/0_trash`다.
    pub fn run_in(root: &Path, args: &[&str]) -> (i32, String) {
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        let code = match parse(&argv) {
            Ok(o) => execute(&Options { library_root: root.to_path_buf(), ..o }, &mut out, false, true),
            Err(_) => 2,
        };
        (code, String::from_utf8_lossy(&out).into_owned())
    }
}
