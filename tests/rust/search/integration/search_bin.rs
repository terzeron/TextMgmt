//! `search` 바이너리를 실제로 실행해 인자 처리와 종료 코드를 확인한다. ES에는 접속하지 않는다.

use std::process::Command;

fn search() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_search"));
    command.env_clear();
    command
}

#[test]
fn no_args_prints_usage_and_exits_0() {
    let output = search().output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("사용법"), "{text}");
}

#[test]
fn bad_args_exit_2() {
    let output = search().args(["latest", "novels"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn missing_env_exits_255() {
    let output = search().args(["latest", "book"]).output().unwrap();
    assert_eq!(output.status.code(), Some(255));
    assert!(String::from_utf8(output.stderr).unwrap().contains("TM_ES_URL"));
}
