use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_move_semi_clusters"))
}

#[test]
fn no_arguments_print_the_usage_and_exit_zero() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("사용법") && text.contains("move_semi_clusters run"), "{text}");
}

#[test]
fn run_with_a_missing_report_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    let out = bin().args(["run", "--library-root"]).arg(tmp.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8(out.stderr).unwrap().contains("hybrid_clustering_report.json"));
}
