//! 명령행 파싱과 `run` 종료 코드 테스트.

mod common;

use std::path::Path;

use bookdoctor::cli::{Options, effective_workers, parse, print_usage, run};
use scan_common::{Category, Severity};

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

#[test]
fn usage_advises_external_memory_limit_and_worker_choice() {
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("MemoryMax"), "{text}");
    assert!(text.contains("워커 2개가 빠르고 8개는 오히려 느리다"), "{text}");
    assert!(text.contains("워커가 1개"), "{text}");
}

// --- --min-severity ---

fn write_file(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(dir.join(name), bytes).unwrap();
}

/// `con.txt`는 LOW 1건(예약 장치 이름), `empty.txt`는 MEDIUM 1건(빈 파일)만 낸다.
fn low_and_medium_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "con.txt", common::clean_korean().as_bytes());
    write_file(d.path(), "empty.txt", b"");
    d
}

fn run_in(dir: &Path, extra: &[&str]) -> (i32, String) {
    let mut argv: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
    argv.push("-r".into());
    argv.push(dir.to_string_lossy().into_owned());
    let mut out = Vec::new();
    let code = run(&argv, &mut out);
    (code, String::from_utf8(out).unwrap())
}

#[test]
fn parse_min_severity_defaults_to_high_and_reads_each_value() {
    assert_eq!(parse(&args("a")).unwrap().min_severity, Severity::High);
    assert_eq!(Options::default().min_severity, Severity::High);
    for (word, want) in [("low", Severity::Low), ("medium", Severity::Medium), ("high", Severity::High), ("critical", Severity::Critical)] {
        assert_eq!(parse(&args(&format!("--min-severity {word} a"))).unwrap().min_severity, want, "{word}");
    }
}

#[test]
fn parse_rejects_unknown_min_severity() {
    let e = parse(&args("--min-severity bogus x")).unwrap_err();
    assert!(e.contains("invalid choice") && e.contains("bogus"), "{e}");
}

#[test]
fn default_output_hides_low_and_medium_and_exits_0() {
    let d = low_and_medium_dir();
    let (code, out) = run_in(d.path(), &[]);
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("[LOW]") && !out.contains("[MEDIUM]"), "{out}");
    assert!(!out.contains("con.txt") && !out.contains("empty.txt"), "{out}");
    assert!(out.contains("발견 0건"), "{out}");
    assert!(out.contains("억제 LOW 1건, MEDIUM 1건"), "{out}");
}

#[test]
fn min_severity_low_shows_everything_and_exits_1() {
    let d = low_and_medium_dir();
    let (code, out) = run_in(d.path(), &["--min-severity", "low"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[LOW] FILENAME_RESERVED_NAME"), "{out}");
    assert!(out.contains("[MEDIUM] EMPTY_FILE"), "{out}");
    assert!(!out.contains("억제"), "{out}");
}

#[test]
fn min_severity_medium_shows_medium_but_not_low() {
    let d = low_and_medium_dir();
    let (code, out) = run_in(d.path(), &["--min-severity", "medium"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("[MEDIUM] EMPTY_FILE") && !out.contains("[LOW]"), "{out}");
    assert!(out.contains("억제 LOW 1건") && !out.contains("MEDIUM 1건"), "{out}");
}

#[test]
fn invalid_min_severity_exits_2() {
    let d = low_and_medium_dir();
    let (code, _) = run_in(d.path(), &["--min-severity", "bogus"]);
    assert_eq!(code, 2);
}

#[test]
fn file_that_could_not_be_scanned_still_exits_2_under_default_filter() {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "big.txt", &vec![b'a'; 2 * 1024 * 1024]);
    let (code, out) = run_in(d.path(), &["--max-total-mb", "1"]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("INCOMPLETE"), "{out}");
}

#[test]
fn high_findings_are_still_shown_by_default() {
    let d = tempfile::tempdir().unwrap();
    let text = common::clean_korean();
    let (cp949, _, _) = encoding_rs::EUC_KR.encode(&text);
    write_file(d.path(), "old.txt", &cp949);
    let (code, out) = run_in(d.path(), &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("NOT_UTF8"), "{out}");
}

#[test]
fn mojibake_high_survives_the_default_filter_so_quarantine_still_works() {
    let d = tempfile::tempdir().unwrap();
    write_file(d.path(), "broken.txt", common::corrupted().as_bytes());
    let (code, out) = run_in(d.path(), &["--json"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("\"code\":\"MOJIBAKE\""), "{out}");
}

#[test]
fn report_file_summary_has_suppressed_counts() {
    let d = low_and_medium_dir();
    let report = d.path().join("out").with_extension("json");
    let argv: Vec<String> = ["--report".to_string(), report.to_string_lossy().into_owned(), "-r".to_string(), d.path().to_string_lossy().into_owned()].into();
    let mut sink = Vec::new();
    assert_eq!(run(&argv, &mut sink), 0);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(v["summary"]["suppressed"]["low"], 1, "{v}");
    assert_eq!(v["summary"]["suppressed"]["medium"], 1, "{v}");
    assert_eq!(v["summary"]["suppressed"]["high"], 0, "{v}");
}

#[test]
fn usage_documents_min_severity() {
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("--min-severity"), "{text}");
}

// --- --color ---

#[test]
fn parse_color_defaults_to_auto_and_reads_each_value() {
    use scan_common::color::ColorMode;
    assert_eq!(parse(&args("a")).unwrap().color, ColorMode::Auto);
    assert_eq!(parse(&args("--color always a")).unwrap().color, ColorMode::Always);
    assert_eq!(parse(&args("--color never a")).unwrap().color, ColorMode::Never);
    let e = parse(&args("--color rainbow a")).unwrap_err();
    assert!(e.contains("invalid choice") && e.contains("rainbow"), "{e}");
}

#[test]
fn color_always_paints_structure_errors_red() {
    let d = low_and_medium_dir();
    let (_, out) = run_in(d.path(), &["--min-severity", "low", "--color", "always"]);
    assert!(out.contains("\u{1b}[31m  [LOW] FILENAME_RESERVED_NAME"), "{out:?}");
    assert!(out.contains("\u{1b}[31m  [MEDIUM] EMPTY_FILE"), "{out:?}");
}

#[test]
fn piped_output_has_no_ansi_in_auto_mode_and_never_mode() {
    let d = low_and_medium_dir();
    for extra in [vec!["--min-severity", "low"], vec!["--min-severity", "low", "--color", "never"]] {
        let (_, out) = run_in(d.path(), &extra);
        assert!(!out.contains('\u{1b}'), "{extra:?}: {out:?}");
    }
}

#[test]
fn json_output_has_no_ansi_even_with_color_always() {
    let d = low_and_medium_dir();
    let (_, out) = run_in(d.path(), &["--json", "--min-severity", "low", "--color", "always"]);
    assert!(out.contains("EMPTY_FILE") && !out.contains('\u{1b}'), "{out:?}");
}

#[test]
fn usage_documents_color() {
    let mut out = Vec::new();
    print_usage(&mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("--color"));
}
