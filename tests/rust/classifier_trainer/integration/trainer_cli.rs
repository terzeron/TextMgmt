//! 학습 CLI 진입점: 사용법과 인자 오류. 실제 ES 학습은 여기서 돌리지 않는다.

use std::process::Command;

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_classifier_trainer")).args(args).env_remove("TM_ES_URL").output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn no_arguments_prints_usage_and_exits_1() {
    let (code, stdout, _) = run(&[]);
    assert_eq!(code, 1);
    assert!(stdout.contains("Usage: classifier_trainer"), "{stdout}");
    assert!(stdout.contains("--per-category"));
}

#[test]
fn help_prints_usage_and_exits_0() {
    let (code, stdout, _) = run(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Usage: classifier_trainer"));
}

#[test]
fn missing_output_exits_1() {
    let (code, stdout, stderr) = run(&["--per-category", "100"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--output"), "{stderr}");
    assert!(stdout.contains("Usage"));
}

#[test]
fn bad_number_exits_1() {
    let (code, _, stderr) = run(&["--output", "/nonexistent/m.bin", "--per-category", "많이"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--per-category"), "{stderr}");
}

#[test]
fn unknown_option_exits_1() {
    let (code, _, stderr) = run(&["--output", "m.bin", "--what"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("--what"), "{stderr}");
}

#[test]
fn missing_es_env_fails_before_training() {
    let (code, _, stderr) = run(&["--output", "/nonexistent/m.bin"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("ES"), "{stderr}");
}
