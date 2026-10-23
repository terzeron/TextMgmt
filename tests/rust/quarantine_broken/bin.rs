//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quarantine_broken"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("quarantine_broken") && text.contains("사용법") || text.contains("Usage:"), "{text}");
}

#[test]
fn undo_without_a_manifest_logs_an_error_and_exits_one() {
    let d = tempfile::tempdir().unwrap();
    let out = bin().args(["undo", "--manifest", d.path().join("none.json").to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stderr).unwrap().contains("ERROR"));
}
