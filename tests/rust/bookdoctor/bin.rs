//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

#[test]
fn no_args_prints_usage_and_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_bookdoctor")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("bookdoctor") && text.contains("사용법"), "{text}");
}
