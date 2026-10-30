//! `--rules`·`--builtin-rules` 통합 테스트.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use bookdoctor::cli::{Options, run};
use bookdoctor::rules::{self, parse};
use bookdoctor::scan::scan_file;
use scan_common::{FileReport, Severity};

const REAL: &str = env!("CARGO_BIN_EXE_bookdoctor");

fn rules_file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, body).unwrap();
    p
}

const MARKER_RULE: &str = "rule Marker { meta: severity = \"high\" description = \"시험 표지\" strings: $a = \"EVIL_MARKER\" nocase condition: $a }";

fn json_run(args: &[&str]) -> (i32, String) {
    let mut out = Vec::new();
    let mut argv: Vec<String> = vec!["--json".into(), "--min-severity".into(), "low".into()];
    argv.extend(args.iter().map(|s| (*s).to_owned()));
    let code = run(&argv, &mut out);
    (code, String::from_utf8(out).unwrap())
}

fn opts_with(set: &str) -> Options {
    Options { ruleset: Some(Arc::new(parse(set).unwrap().compile())), ..Options::default() }
}

fn scan_with(set: &str, name: &str, data: &[u8]) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, data).unwrap();
    scan_file(&p, &opts_with(set))
}

#[test]
fn no_rules_means_no_rule_findings_and_the_default_output_is_unchanged() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, "안녕하세요. EVIL_MARKER 가 있는 글입니다.\n".repeat(20)).unwrap();
    let (_, plain) = json_run(&[f.to_str().unwrap()]);
    assert!(!plain.contains("RULE_MATCH"), "{plain}");
    let defaults = Options::default();
    assert!(defaults.ruleset.is_none() && defaults.rule_files.is_empty() && !defaults.builtin_rules);
    // 규칙을 주면 같은 파일에서 일치가 나오고 그 밖의 지표 집합은 그대로다.
    let rf = rules_file(d.path(), "m.rules", MARKER_RULE);
    let (code, with) = json_run(&["--rules", rf.to_str().unwrap(), f.to_str().unwrap()]);
    assert!(code == 0 || code == 1, "{code}");
    assert!(with.contains("RULE_MATCH"), "{with}");
}

#[test]
fn a_rules_file_matches_with_severity_location_and_detail() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, format!("{}EVIL_MARKER{}", "x".repeat(50), "y".repeat(400))).unwrap();
    let rf = rules_file(d.path(), "m.rules", MARKER_RULE);
    let (_, out) = json_run(&["--rules", rf.to_str().unwrap(), f.to_str().unwrap()]);
    let line = out.lines().find(|l| l.contains("RULE_MATCH")).unwrap_or_else(|| panic!("{out}"));
    let v: serde_json::Value = serde_json::from_str(line).unwrap();
    let finding = v["findings"].as_array().unwrap().iter().find(|f| f["code"] == "RULE_MATCH").unwrap();
    assert_eq!(finding["severity"], "HIGH");
    assert_eq!(finding["category"], "security");
    assert_eq!(finding["location"], "Marker");
    let detail = finding["detail"].as_str().unwrap();
    assert!(detail.contains("시험 표지") && detail.contains("$a") && detail.contains("50"), "{detail}");
}

#[test]
fn a_directory_of_rules_files_loads_every_rules_file_and_ignores_others() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("rules");
    fs::create_dir(&dir).unwrap();
    rules_file(&dir, "a.rules", "rule A { strings: $x = \"AAAA_MARK\" condition: $x }");
    rules_file(&dir, "b.rules", "rule B { strings: $x = \"BBBB_MARK\" condition: $x }");
    rules_file(&dir, "notes.txt", "rule C { strings: $x = \"CCCC_MARK\" condition: $x }");
    let f = d.path().join("a.txt");
    fs::write(&f, "AAAA_MARK BBBB_MARK CCCC_MARK ".repeat(5)).unwrap();
    let (_, out) = json_run(&["--rules", dir.to_str().unwrap(), f.to_str().unwrap()]);
    for want in ["\"location\":\"A\"", "\"location\":\"B\""] {
        assert!(out.contains(want), "{want} missing: {out}");
    }
    assert!(!out.contains("\"location\":\"C\""), "{out}");
}

#[test]
fn a_bad_or_missing_rules_file_exits_2_with_file_and_line() {
    let d = tempfile::tempdir().unwrap();
    let bad = rules_file(d.path(), "bad.rules", "rule X {\n strings:\n  $a = \"ab\" fullword\n condition: $a }");
    let f = d.path().join("a.txt");
    fs::write(&f, "text").unwrap();
    let out = Command::new(REAL).arg("--rules").arg(&bad).arg(&f).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bad.rules:3:"), "{err}");
    let missing = Command::new(REAL).arg("--rules").arg(d.path().join("nope.rules")).arg(&f).output().unwrap();
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("nope.rules"));
    // 같은 이름의 규칙이 두 파일에 있으면 오류다.
    let a = rules_file(d.path(), "a.rules", MARKER_RULE);
    let b = rules_file(d.path(), "b.rules", MARKER_RULE);
    let dup = Command::new(REAL).arg("--rules").arg(&a).arg("--rules").arg(&b).arg(&f).output().unwrap();
    assert_eq!(dup.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&dup.stderr).contains("duplicate"), "{}", String::from_utf8_lossy(&dup.stderr));
}

#[test]
fn files_over_16_mib_are_matched_in_the_prefix_and_marked_incomplete() {
    let mut data = b"{\\rtf1 EVIL_MARKER ".to_vec();
    data.extend(std::iter::repeat_n(b' ', 17 * 1024 * 1024));
    data.push(b'}');
    let r = scan_with(MARKER_RULE, "big.rtf", &data);
    assert!(r.has("RULE_MATCH"), "{:?}", r.findings);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("first 16 MiB")), "{:?}", r.reasons);
}

#[test]
fn a_rule_scan_that_hits_the_work_limit_is_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, "a".repeat(100_000)).unwrap();
    let compiled = parse(&(0..50).map(|i| format!("rule R{i} {{ strings: $a = \"{}b{i}\" condition: $a }}\n", "a".repeat(40))).collect::<String>()).unwrap().compile();
    let mut report = FileReport::new(&f, None);
    rules::scan_report_with_budget(&f, &compiled, &mut report, 100_000);
    assert!(report.incomplete && report.reasons.iter().any(|m| m.contains("work limit")), "{:?}", report.reasons);
}

#[test]
fn rules_apply_to_nested_files_under_recursion() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("outer.zip");
    common::zip_of(&p, &[("inner.rtf", b"{\\rtf1 hello EVIL_MARKER}"), ("plain.bin", b"EVIL_MARKER not a container")]);
    let r = scan_file(&p, &opts_with(MARKER_RULE));
    let nested = r.findings.iter().find(|f| f.code == "RULE_MATCH" && f.location.starts_with("inner.rtf!")).unwrap_or_else(|| panic!("{:?} {:?}", r.findings, r.reasons));
    assert_eq!(nested.severity, Severity::High);
}

#[test]
fn the_min_severity_filter_applies_to_rule_matches() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, "EVIL_MARKER ".repeat(10)).unwrap();
    let rf = rules_file(d.path(), "low.rules", "rule LowOne { meta: severity = \"low\" strings: $a = \"EVIL_MARKER\" condition: $a }");
    let mut out = Vec::new();
    run(&["--json".into(), "--rules".into(), rf.to_string_lossy().into_owned(), f.to_string_lossy().into_owned()], &mut out);
    assert!(!String::from_utf8(out).unwrap().contains("RULE_MATCH"), "low must be hidden by the default min severity");
    let (_, shown) = json_run(&["--rules", rf.to_str().unwrap(), f.to_str().unwrap()]);
    assert!(shown.contains("RULE_MATCH"));
}

#[test]
fn isolate_passes_the_rules_to_the_child() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, "EVIL_MARKER ".repeat(10)).unwrap();
    let rf = rules_file(d.path(), "m.rules", MARKER_RULE);
    let out = Command::new(REAL).args(["--isolate", "--json", "--min-severity", "low", "--rules"]).arg(&rf).arg(&f).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("RULE_MATCH"), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
}
