use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reload_samgukji_epubs"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("사용법") && text.contains("reload_samgukji_epubs --list"), "{text}");
}

#[test]
fn list_reads_tm_book_dir_from_the_environment() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("2_소설역사");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("삼국지 1.epub"), b"x").unwrap();
    let out = bin().arg("--list").env("TM_BOOK_DIR", tmp.path()).current_dir(tmp.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().contains("대상: 1개 EPUB"));
}

#[test]
fn a_missing_target_directory_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    let out = bin().arg("--list").env("TM_BOOK_DIR", tmp.path()).current_dir(tmp.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stderr).unwrap().contains("대상 디렉토리를 찾을 수 없습니다"));
}

#[test]
fn apply_in_a_directory_that_is_not_the_repo_exits_one_without_launching() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("2_소설역사");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("삼국지 1.epub"), b"x").unwrap();
    let out = bin().arg("--apply").env("TM_BOOK_DIR", tmp.path()).current_dir(tmp.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}
