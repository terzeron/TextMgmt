//! 명령행 파싱과 `run` 종료 코드 테스트.

use bookdoctor::cli::{Options, effective_workers, parse, print_usage, run};
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

#[test]
fn effective_workers_are_capped_by_memory() {
    // 한도 256MB 기준: workers <= 256 / max-total-mb, 최소 1.
    assert_eq!(effective_workers(&Options::default()), 1);
    assert_eq!(effective_workers(&Options { workers: 2, max_total_mb: 128, ..Options::default() }), 2);
    assert_eq!(effective_workers(&Options { workers: 8, max_total_mb: 64, ..Options::default() }), 4);
    assert_eq!(effective_workers(&Options { workers: 8, max_total_mb: 1000, ..Options::default() }), 1);
    // 0은 parse가 먼저 거른다.
    assert!(parse(&args("--max-total-mb 0 a")).is_err());
}

#[test]
fn usage_explains_the_memory_cap_on_workers() {
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("256 / max-total-mb"), "{text}");
}
