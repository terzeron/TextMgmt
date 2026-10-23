//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cluster_hybrid_genres"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("cluster_hybrid_genres") && text.contains("사용법") || text.contains("Usage:"), "{text}");
}

#[test]
fn an_empty_source_directory_writes_an_empty_report() {
    let d = tempfile::Builder::new().prefix("cl").tempdir().unwrap();
    let src = d.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let report = d.path().join("r.json");
    let out = bin().args(["--source-dir", src.to_str().unwrap(), "--report", report.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(report.exists());
}
