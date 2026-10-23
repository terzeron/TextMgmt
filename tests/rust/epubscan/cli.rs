//! epubscan 명령행 테스트.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use epubscan::cli;
use serde_json::Value;

struct Run {
    code: i32,
    out: String,
}

fn quiet(_: &str) {}

fn run(repo: &Path, argv: &[&str]) -> Run {
    let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = cli::run(&args, repo, &mut out, &quiet);
    Run { code, out: String::from_utf8(out).unwrap() }
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

struct Env {
    _d: tempfile::TempDir,
    root: PathBuf,
}

fn env() -> Env {
    let d = tempfile::Builder::new().prefix("epubscan").tempdir().unwrap();
    let root = d.path().to_path_buf();
    Env { _d: d, root }
}

fn clean_epub(path: &Path) {
    epub_with_chapter(path, &xhtml("<p>안녕</p>"));
}

fn bad_epub(path: &Path) {
    epub_with_chapter(path, &xhtml("<script>eval(1)</script>"));
}

// ---- structure ----

#[test]
fn structure_file_mode_prints_one_json_line_per_file() {
    let e = env();
    let f = e.root.join("p.epub");
    make_epub_with(&f, EpubSpec { chapter: prefixed_chapter(), ..EpubSpec::default() });
    let r = run(&e.root, &["structure", "--file", p(&f)]);
    assert_eq!(r.code, 0);
    assert_eq!(serde_json::from_str::<Value>(r.out.trim()).unwrap()["issues"][0], "prefixed_root");
}

#[test]
fn structure_scan_writes_the_report_and_prints_a_summary() {
    let e = env();
    make_epub_with(&e.root.join("p.epub"), EpubSpec { chapter: prefixed_chapter(), ..EpubSpec::default() });
    let report = e.root.join("out/r.json");
    let r = run(&e.root, &["structure", "--root", p(&e.root), "--workers", "1", "--report", p(&report)]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("prefixed_root") && r.out.contains("문제 1건") && r.out.contains("리포트:"));
    assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&report).unwrap()).unwrap()["flagged_count"], 1);
}

#[test]
fn structure_without_report_writes_a_per_directory_file() {
    let e = env();
    let (a, b) = (e.root.join("library_a"), e.root.join("library_b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    make_epub(&b.join("b.epub"));
    assert_eq!(run(&e.root, &["structure", "--root", p(&a), "--workers", "1"]).code, 0);
    assert_eq!(run(&e.root, &["structure", "--root", p(&b), "--workers", "1", "--limit", "5"]).code, 0);
    let mut names: Vec<String> = fs::read_dir(e.root.join("tmp")).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names, ["epub_structure_report_library_a.json", "epub_structure_report_library_b.json"]);
}

// ---- scan ----

#[test]
fn scan_reports_findings_with_exit_code_one() {
    let e = env();
    let f = e.root.join("bad.epub");
    bad_epub(&f);
    let r = run(&e.root, &["scan", p(&f)]);
    assert_eq!(r.code, 1);
    assert!(r.out.contains("[FINDINGS]") && r.out.contains("[HIGH] SCRIPT_TAG |"), "{}", r.out);
}

#[test]
fn scan_clean_file_exits_zero_and_says_it_is_not_a_guarantee() {
    let e = env();
    let f = e.root.join("ok.epub");
    clean_epub(&f);
    let r = run(&e.root, &["scan", p(&f)]);
    assert_eq!(r.code, 0);
    assert!(r.out.contains("[NONE]") && r.out.contains("not a safety guarantee"));
}

#[test]
fn scan_incomplete_exits_two_unless_there_are_findings() {
    let e = env();
    let f = e.root.join("junk.epub");
    fs::write(&f, b"not a zip").unwrap();
    let r = run(&e.root, &["scan", p(&f)]);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("[INCOMPLETE]") && r.out.contains("note: ") && r.out.contains("Cannot open as ZIP EPUB"));

    // 불완전한 파일이 섞여 있어도 지표가 있으면 1이다. 지표를 2 뒤에 숨기지 않는다.
    let g = e.root.join("bad.epub");
    bad_epub(&g);
    assert_eq!(run(&e.root, &["scan", p(&f), p(&g)]).code, 1);
}

#[test]
fn scan_directories_respect_the_recursive_flag() {
    let e = env();
    let sub = e.root.join("lib/sub");
    fs::create_dir_all(&sub).unwrap();
    bad_epub(&e.root.join("lib/top.epub"));
    bad_epub(&sub.join("deep.epub"));
    fs::write(e.root.join("lib/notes.txt"), "x").unwrap();
    let flat = run(&e.root, &["scan", "--json", p(&e.root.join("lib"))]);
    let deep = run(&e.root, &["scan", "-r", "--json", p(&e.root.join("lib"))]);
    assert_eq!(serde_json::from_str::<Value>(&flat.out).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(serde_json::from_str::<Value>(&deep.out).unwrap().as_array().unwrap().len(), 2);
}

#[test]
fn scan_json_output_is_an_array_of_reports() {
    let e = env();
    let f = e.root.join("bad.epub");
    bad_epub(&f);
    let r = run(&e.root, &["scan", "--json", p(&f)]);
    let v: Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v[0]["status"], "FINDINGS");
    assert_eq!(v[0]["findings"][0]["severity"], "HIGH");
}

#[test]
fn scan_limits_are_passed_through() {
    let e = env();
    let f = e.root.join("ok.epub");
    clean_epub(&f);
    let r = run(&e.root, &["scan", "--max-member-mb", "1", "--max-total-mb", "1", "--max-files", "2", "--workers", "1", p(&f)]);
    assert_eq!(r.code, 2);
    assert!(r.out.contains("Too many entries"));
}

#[test]
fn scan_with_nothing_to_scan_or_bad_options_exits_two() {
    let e = env();
    assert_eq!(run(&e.root, &["scan", p(&e.root.join("missing"))]).code, 2);
    let empty = e.root.join("empty");
    fs::create_dir(&empty).unwrap();
    assert_eq!(run(&e.root, &["scan", p(&empty)]).code, 2);
    assert_eq!(run(&e.root, &["scan"]).code, 2);
    for bad in [&["scan", "--max-files", "0", "x"][..], &["scan", "--max-files", "abc", "x"], &["scan", "--max-total-mb", "-1", "x"], &["scan", "--nope", "x"], &["scan", "--max-files"]] {
        assert_eq!(run(&e.root, bad).code, 2, "{bad:?}");
    }
}

#[test]
fn unknown_command_and_help() {
    let e = env();
    assert_eq!(run(&e.root, &["bogus"]).code, 2);
    let h = run(&e.root, &["--help"]);
    assert_eq!(h.code, 0);
    assert!(h.out.contains("사용법:") && h.out.contains("epubscan"));
    assert_eq!(run(&e.root, &["structure", "--workers", "x"]).code, 2);
    assert_eq!(run(&e.root, &["structure", "--bogus"]).code, 2);
}

#[test]
fn no_arguments_print_purpose_and_usage_for_both_subcommands() {
    let e = env();
    for argv in [&[][..], &["--help"], &["-h"]] {
        let r = run(&e.root, argv);
        assert_eq!(r.code, 0, "{argv:?}");
        for needle in ["epubscan", "EPUB", "사용법:", "structure", "scan", "--root", "--file", "--report", "-r", "--json", "--max-member-mb", "--max-total-mb", "--max-files", "--workers", "종료 코드"]
        {
            assert!(r.out.contains(needle), "{needle} 없음: {}", r.out);
        }
    }
}
