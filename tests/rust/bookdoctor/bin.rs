//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

#[test]
fn no_args_prints_usage_and_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_bookdoctor")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("bookdoctor") && text.contains("사용법"), "{text}");
}

#[test]
fn json_mode_keeps_stdout_clean_and_writes_the_summary_line_to_stderr() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("empty.txt"), b"").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bookdoctor")).args(["--json", "-r"]).arg(d.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "{}", String::from_utf8_lossy(&out.stdout));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("발견 0건") && err.contains("억제 MEDIUM 1건"), "{err}");
}

#[test]
fn text_mode_writes_the_summary_line_to_stdout_only() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("empty.txt"), b"").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_bookdoctor")).arg("-r").arg(d.path()).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("억제 MEDIUM 1건"));
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}
