//! 인코딩 손상 탐지기 테스트 (tests/test_detect_mojibake.py 이식).
//! 판정 로직은 결정론적이다. 학습도 확률도 없이 문자만 센다.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use common::*;
use detect_mojibake::*;
use serde_json::Value;

fn hanja() -> String {
    HANJA_CLASSIC_UNIT.repeat(20)
}
fn english() -> String {
    ENGLISH_UNIT.repeat(20)
}
fn word_list() -> String {
    KOREAN_WORD_LIST_UNIT.repeat(20)
}
fn quiet(_: &str) {}

fn write(p: &Path, text: &str) {
    fs::write(p, text).unwrap();
}

// ---- korean_likeness ----

#[test]
fn likeness_is_none_when_too_few_syllables() {
    assert_eq!(korean_likeness("가나다"), None);
    assert_eq!(korean_likeness(""), None);
    assert_eq!(korean_likeness(&"가".repeat(MIN_SYLLABLES_FOR_LIKENESS - 1)), None);
}

#[test]
fn likeness_is_computed_at_the_syllable_floor() {
    assert_eq!(korean_likeness(&"이".repeat(MIN_SYLLABLES_FOR_LIKENESS)), Some(1.0));
}

#[test]
fn likeness_is_high_for_real_korean() {
    assert!(korean_likeness(&clean_korean()).unwrap() >= 0.20);
}

#[test]
fn likeness_is_near_zero_for_corrupted_text() {
    assert!(korean_likeness(&corrupted()).unwrap() < MOJIBAKE_LIKENESS_THRESHOLD);
}

#[test]
fn likeness_ignores_non_hangul_characters() {
    let base = "이다는을가".repeat(10);
    let with_hanja = format!("{base}{}", "孟子曰 ".repeat(50));
    assert_eq!(korean_likeness(&base), korean_likeness(&with_hanja));
}

// ---- classify_text ----

#[test]
fn classify_text_table() {
    let cases: Vec<(String, &str)> = vec![
        (clean_korean(), VERDICT_CLEAN),
        (corrupted(), VERDICT_CORRUPTED),
        (hanja(), VERDICT_UNDETERMINED),
        (english(), VERDICT_UNDETERMINED),
        (word_list(), VERDICT_CLEAN),
        (String::new(), VERDICT_UNREADABLE),
        ("짧은 글".to_string(), VERDICT_UNREADABLE),
    ];
    for (text, expected) in cases {
        assert_eq!(classify_text(&text).0, expected, "{}", text.chars().take(20).collect::<String>());
    }
}

#[test]
fn hanja_heavy_document_is_never_called_corrupted() {
    assert_eq!(classify_text(&hanja()), (VERDICT_UNDETERMINED, None));
}

#[test]
fn classify_returns_likeness_for_decided_verdicts() {
    for text in [clean_korean(), corrupted()] {
        let l = classify_text(&text).1.unwrap();
        assert!((0.0..=1.0).contains(&l));
    }
}

#[test]
fn english_document_with_hangul_noise_is_undetermined() {
    let text = format!("{}{}", ENGLISH_UNIT.repeat(20).repeat(12), "뷁뾃쎯퉳깗".repeat(12));
    assert!(korean_likeness(&text).is_some());
    assert_eq!(classify_text(&text).0, VERDICT_UNDETERMINED);
}

#[test]
fn classify_text_is_undetermined_when_hangul_is_too_sparse_to_measure() {
    let text = format!("{}{}", "가나다라마바사아자차".repeat(4), "x".repeat(300));
    assert!(text.chars().count() >= 200);
    assert_eq!(classify_text(&text), (VERDICT_UNDETERMINED, None));
}

// ---- 파일 검사 ----

#[test]
fn inspect_file_reports_verdict() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("clean.txt");
    write(&p, &clean_korean().repeat(4));
    let r = inspect_file(p.to_str().unwrap());
    assert_eq!(r.verdict, VERDICT_CLEAN);
    assert!(r.size.unwrap() > 0);
    assert_eq!(r.reason.as_deref(), Some(""));
}

#[test]
fn inspect_file_detects_corrupted_file() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("broken.txt");
    write(&p, &corrupted());
    assert_eq!(inspect_file(p.to_str().unwrap()).verdict, VERDICT_CORRUPTED);
}

#[test]
fn inspect_file_on_missing_path() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(inspect_file(d.path().join("없음.txt").to_str().unwrap()).verdict, VERDICT_UNREADABLE);
}

#[test]
fn unreadable_reason_distinguishes_empty_from_short() {
    let d = tempfile::tempdir().unwrap();
    let (empty, short) = (d.path().join("empty.txt"), d.path().join("short.txt"));
    write(&empty, "");
    write(&short, "짧은 글");
    assert_eq!(inspect_file(empty.to_str().unwrap()).reason.as_deref(), Some("missing_or_empty"));
    assert_eq!(inspect_file(short.to_str().unwrap()).reason.as_deref(), Some("too_short"));
}

#[test]
fn inspect_file_rounds_likeness_to_four_places_half_even() {
    // 음절 32개 중 1개가 흔한 음절이면 0.03125. Python round(0.03125, 4)는 0.0312.
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("tie.txt");
    let body = format!("{}{}", "이", "쀍".repeat(31));
    // 한글 비율 하한과 길이 하한을 맞추려고 음절을 반복한다(비율은 그대로).
    write(&p, &body.repeat(8));
    let r = inspect_file(p.to_str().unwrap());
    assert_eq!(r.likeness, Some(0.0312));
}

// ---- list_library_files ----

// ---- scan ----

#[test]
fn scan_counts_verdicts_and_keeps_every_non_clean_file() {
    let d = tempfile::tempdir().unwrap();
    let cat = d.path().join("3_무협");
    fs::create_dir(&cat).unwrap();
    write(&cat.join("clean.txt"), &clean_korean().repeat(4));
    write(&cat.join("broken.txt"), &corrupted());
    write(&cat.join("english.txt"), &english().repeat(4));
    write(&cat.join("empty.txt"), "");

    let report = scan(d.path(), &ScanOptions { workers: 1, limit: 0, progress_every: 2000 }, &quiet);
    assert_eq!(report.total, 4);
    assert_eq!(report.counts[VERDICT_CLEAN], 1);
    assert_eq!(report.counts[VERDICT_CORRUPTED], 1);
    let paths = |v: &[FileResult]| v.iter().map(|r| r.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&report.corrupted), [cat.join("broken.txt").to_string_lossy()]);
    assert_eq!(paths(&report.undetermined), [cat.join("english.txt").to_string_lossy()]);
    assert_eq!(paths(&report.unreadable), [cat.join("empty.txt").to_string_lossy()]);
    serde_json::to_string(&report).unwrap();
}

#[test]
fn scan_respects_limit_and_logs_progress() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..3 {
        write(&d.path().join(format!("{i}.txt")), &clean_korean());
    }
    let lines = Mutex::new(Vec::<String>::new());
    let log = |m: &str| lines.lock().unwrap().push(m.to_string());
    let report = scan(d.path(), &ScanOptions { workers: 1, limit: 2, progress_every: 1 }, &log);
    assert_eq!(report.total, 2);
    assert!(lines.lock().unwrap().iter().any(|l| l.contains("1/2")));
}

#[test]
fn scan_groups_files_outside_the_default_root_as_other() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("3_판타지")).unwrap();
    write(&d.path().join("3_판타지/a.txt"), &clean_korean());
    let report = scan(d.path(), &ScanOptions { workers: 1, limit: 0, progress_every: 2000 }, &quiet);
    assert_eq!(report.per_category.keys().collect::<Vec<_>>(), ["(기타)"]);
}

#[test]
fn scan_report_json_keeps_python_key_order() {
    let d = tempfile::tempdir().unwrap();
    write(&d.path().join("a.txt"), &clean_korean());
    let report = scan(d.path(), &ScanOptions { workers: 1, limit: 0, progress_every: 2000 }, &quiet);
    let v: Value = serde_json::to_value(&report).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["scanned_at", "root", "threshold", "total", "counts", "elapsed_sec", "per_category", "corrupted", "unreadable", "undetermined"]);
    assert!(v["scanned_at"].as_str().unwrap().ends_with("+00:00"));
}

// ---- 요약 출력 ----

fn base_report() -> ScanReport {
    serde_json::from_value(serde_json::json!({
        "scanned_at": "2026-09-18T00:00:00+00:00", "root": "/mnt/data/text", "threshold": MOJIBAKE_LIKENESS_THRESHOLD,
        "total": 4, "counts": {"clean": 2, "corrupted": 1, "undetermined": 1}, "elapsed_sec": 120.0,
        "per_category": {"3_판타지": {"clean": 1, "corrupted": 1}, "9_격언명언": {"clean": 1}},
        "corrupted": [], "unreadable": [], "undetermined": []
    }))
    .unwrap()
}

fn summary(r: &ScanReport) -> String {
    let mut out = Vec::new();
    print_summary(r, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn print_summary_lists_categories_with_damage() {
    let out = summary(&base_report());
    assert!(out.contains("검사 4건"));
    assert!(out.contains("손상이 있는 카테고리 1개"));
    assert!(out.contains("3_판타지"));
    assert!(!out.contains("9_격언명언"));
    assert!(out.contains("임계값 likeness < 0.1"));
}

#[test]
fn print_summary_without_damage_skips_the_table() {
    let mut r = base_report();
    r.counts = [("clean".to_string(), 4)].into_iter().collect();
    r.per_category = [("3_판타지".to_string(), [("clean".to_string(), 4)].into_iter().collect())].into_iter().collect();
    assert!(!summary(&r).contains("손상이 있는 카테고리"));
}

#[test]
fn print_summary_survives_zero_total() {
    let mut r = base_report();
    r.total = 0;
    r.counts.clear();
    r.per_category.clear();
    assert!(summary(&r).contains("검사 0건"));
}

#[test]
fn print_summary_formats_thousands_and_percent_like_python() {
    let mut r = base_report();
    r.total = 12345;
    r.counts = [("corrupted".to_string(), 1234)].into_iter().collect();
    let out = summary(&r);
    assert!(out.contains("1,234  (10.00%)"), "{out}");
}

// ---- CLI ----

struct Cli {
    code: i32,
    out: String,
}

fn run_cli(repo_root: &Path, argv: &[&str]) -> Cli {
    let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = cli::run(&args, repo_root, &mut out, &quiet);
    Cli { code, out: String::from_utf8(out).unwrap() }
}

#[test]
fn cli_check_prints_one_json_line_per_file() {
    let d = tempfile::tempdir().unwrap();
    let (clean, broken) = (d.path().join("clean.txt"), d.path().join("broken.txt"));
    write(&clean, &clean_korean());
    write(&broken, &corrupted());
    let r = run_cli(d.path(), &["check", "--file", clean.to_str().unwrap(), "--file", broken.to_str().unwrap()]);
    assert_eq!(r.code, 0);
    let verdicts: Vec<String> = r.out.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["verdict"].as_str().unwrap().to_string()).collect();
    assert_eq!(verdicts, [VERDICT_CLEAN, VERDICT_CORRUPTED]);
}

#[test]
fn cli_check_without_file_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run_cli(d.path(), &["check"]).code, 1);
}

#[test]
fn cli_unknown_command_or_option_exits_2_like_argparse() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(run_cli(d.path(), &["bogus"]).code, 2);
    assert_eq!(run_cli(d.path(), &["scan", "--nope"]).code, 2);
    assert_eq!(run_cli(d.path(), &["scan", "--workers", "x"]).code, 2);
}

#[test]
fn cli_scan_writes_report_and_summary() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("text/3_판타지");
    fs::create_dir_all(&root).unwrap();
    write(&root.join("clean.txt"), &clean_korean());
    write(&root.join("broken.txt"), &corrupted());
    let report_path = d.path().join("out/report.json");
    let r = run_cli(d.path(), &["scan", "--root", d.path().join("text").to_str().unwrap(), "--workers", "1", "--report", report_path.to_str().unwrap()]);
    assert_eq!(r.code, 0);
    let v: Value = serde_json::from_str(&fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(v["total"], 2);
    assert_eq!(v["counts"][VERDICT_CORRUPTED], 1);
    assert_eq!(Path::new(v["corrupted"][0]["path"].as_str().unwrap()).file_name().unwrap(), "broken.txt");
    assert!(r.out.contains("리포트:"));
}

#[test]
fn cli_without_report_writes_a_per_directory_file() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path();
    let (a, b) = (repo.join("library_a"), repo.join("library_b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    write(&a.join("a.txt"), &clean_korean());
    write(&b.join("b.txt"), &clean_korean());
    assert_eq!(run_cli(repo, &["scan", "--root", a.to_str().unwrap(), "--workers", "1"]).code, 0);
    assert_eq!(run_cli(repo, &["scan", "--root", b.to_str().unwrap(), "--workers", "1"]).code, 0);
    let mut names: Vec<String> = fs::read_dir(repo.join("tmp")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["mojibake_report_library_a.json", "mojibake_report_library_b.json"]);
    let v: Value = serde_json::from_str(&fs::read_to_string(repo.join("tmp/mojibake_report_library_b.json")).unwrap()).unwrap();
    assert_eq!(v["root"], b.to_str().unwrap());
}

// ---- 리포트 경로 ----

// ---- 사용법 ----

#[test]
fn no_arguments_and_help_print_purpose_and_usage() {
    let d = tempfile::tempdir().unwrap();
    for argv in [&[][..], &["--help"], &["-h"]] {
        let r = run_cli(d.path(), argv);
        assert_eq!(r.code, 0, "{argv:?}");
        for needle in ["detect_mojibake", "인코딩이 손상", "사용법:", "scan", "check", "--root", "--file", "--workers", "--limit", "--report"] {
            assert!(r.out.contains(needle), "{needle} 없음: {}", r.out);
        }
    }
    // 아무것도 스캔하지 않는다.
    assert!(!d.path().join("tmp").exists());
}

// ---- 분기 보강 ----

#[test]
fn files_without_extractable_text_get_a_distinct_reason() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("blob.bin");
    fs::write(&p, b"binary").unwrap();
    let r = inspect_file(p.to_str().unwrap());
    assert_eq!((r.verdict.as_str(), r.reason.as_deref()), (VERDICT_UNREADABLE, Some("no_text_extracted")));
}

#[test]
fn scan_options_default_to_two_workers_and_progress_every_2000() {
    let o = ScanOptions::default();
    assert_eq!((o.workers, o.limit, o.progress_every), (DEFAULT_WORKERS, 0, 2000));
}

#[test]
fn cli_accepts_inline_values_and_rejects_extra_positionals_and_unwritable_reports() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("text");
    fs::create_dir(&root).unwrap();
    write(&root.join("a.txt"), &clean_korean());
    let report = d.path().join("r.json");
    let inline = [format!("--root={}", root.display()), format!("--report={}", report.display()), "--workers=1".to_string()];
    let refs: Vec<&str> = std::iter::once("scan").chain(inline.iter().map(String::as_str)).collect();
    assert_eq!(run_cli(d.path(), &refs).code, 0);
    assert!(report.exists());

    assert_eq!(run_cli(d.path(), &["scan", "extra"]).code, 2);

    let blocker = d.path().join("file");
    fs::write(&blocker, "x").unwrap();
    let bad = blocker.join("r.json");
    assert_eq!(run_cli(d.path(), &["scan", "--root", root.to_str().unwrap(), "--report", bad.to_str().unwrap()]).code, 1);
}
