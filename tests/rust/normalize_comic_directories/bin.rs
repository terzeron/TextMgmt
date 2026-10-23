use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_normalize_comic_directories"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("사용법") && text.contains("normalize_comic_directories run"), "{text}");
}

#[test]
fn a_missing_root_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let out = bin().args(["run", "--root"]).arg(tmp.path().join("none")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr).unwrap().contains("not a directory"));
}

#[test]
fn a_dry_run_over_an_empty_root_reports_zero_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let report = tmp.path().join("r.tsv");
    let out = bin().args(["run", "--root"]).arg(tmp.path()).arg("--report").arg(&report).arg("--cache").arg(tmp.path().join("c.jsonl")).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8(out.stdout).unwrap().contains("Directories scanned: 0"));
    assert!(report.exists());
}
