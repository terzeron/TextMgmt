//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_epubscan"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("epubscan") && text.contains("사용법") || text.contains("Usage:"), "{text}");
}

#[test]
fn scanning_an_unreadable_file_exits_two() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("junk.epub");
    std::fs::write(&f, b"x").unwrap();
    let out = bin().args(["scan", f.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stdout).unwrap().contains("INCOMPLETE"));
}

#[test]
fn structure_of_a_single_file_prints_json() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("junk.epub");
    std::fs::write(&f, b"x").unwrap();
    let out = bin().args(["structure", "--file", f.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().contains("not_zip"));
}
