//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_detect_mojibake"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("detect_mojibake") && text.contains("사용법") || text.contains("Usage:"), "{text}");
}

#[test]
fn check_reports_a_file_as_json() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    std::fs::write(&f, "짧은 글").unwrap();
    let out = bin().args(["check", "--file", f.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().contains("\"verdict\""));
}

#[test]
fn bad_arguments_exit_two_and_runtime_errors_log_with_a_timestamp() {
    assert_eq!(bin().arg("bogus").output().unwrap().status.code(), Some(2));
    let out = bin().args(["check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stderr).unwrap().contains("ERROR"));
}
