//! 명령행 파싱과 `run` 종료 코드 테스트.

use bookdoctor::cli::{parse, run};
use scan_common::Category;

fn args(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

#[test]
fn parse_reads_every_option() {
    let o = parse(&args("-r --json --report r.json --workers 3 --limit 5 --max-total-mb 9 --only security,defect --pdf-text --no-poppler a")).unwrap();
    assert!(o.recursive && o.json && o.pdf_text && o.no_poppler);
    assert_eq!(o.report, Some("r.json".into()));
    assert_eq!((o.workers, o.limit, o.max_total_mb), (3, 5, 9));
    assert_eq!(o.only, vec![Category::Security, Category::Defect]);
    assert_eq!(o.paths, vec![std::path::PathBuf::from("a")]);
}

#[test]
fn parse_rejects_unknown_only_category() {
    let e = parse(&args("--only bogus x")).unwrap_err();
    assert!(e.contains("bogus"), "{e}");
}

#[test]
fn parse_requires_a_path_and_positive_limits() {
    assert_eq!(parse(&[]).unwrap_err(), "the following arguments are required: PATH...");
    assert_eq!(parse(&args("--max-total-mb 0 x")).unwrap_err(), "Limits must be positive");
}

#[test]
fn clean_empty_dir_exits_0() {
    let d = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    assert_eq!(run(&["-r".into(), d.path().to_string_lossy().into_owned()], &mut out), 0);
}
