//! 바이너리 진입점 테스트: 실제 실행 파일을 띄워 종료 코드와 출력을 확인한다.

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_search"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("search") && text.contains("사용법") || text.contains("Usage:"), "{text}");
}

#[test]
fn missing_environment_variables_exit_255() {
    let out = bin().arg("제목").env_remove("TM_ES_URL").env_remove("TM_ES_USER").env_remove("TM_ES_PASSWORD").env_remove("TM_ES_BOOK_INDEX").output().unwrap();
    assert_eq!(out.status.code(), Some(255));
    assert!(String::from_utf8(out.stderr).unwrap().contains("TM_ES_BOOK_INDEX"));
}
