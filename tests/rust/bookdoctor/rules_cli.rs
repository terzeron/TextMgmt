//! 규칙 매칭 통합 테스트. 명령행은 내장 규칙만 쓰므로, 여기서는 `Options.ruleset`에 시험 규칙을 넣어 매칭 동작을 본다.

mod common;

use std::fs;
use std::sync::Arc;

use bookdoctor::cli::Options;
use bookdoctor::rules::{self, parse};
use bookdoctor::scan::scan_file;
use scan_common::{FileReport, Severity};

const MARKER_RULE: &str = "rule Marker { meta: severity = \"high\" description = \"시험 표지\" strings: $a = \"EVIL_MARKER\" nocase condition: $a }";

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
fn no_ruleset_means_no_rule_findings() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, "안녕하세요. EVIL_MARKER 가 있는 글입니다.\n".repeat(20)).unwrap();
    assert!(Options::default().ruleset.is_none());
    assert!(!scan_file(&f, &Options::default()).has("RULE_MATCH"));
    assert!(scan_file(&f, &opts_with(MARKER_RULE)).has("RULE_MATCH"));
}

#[test]
fn a_rule_matches_with_severity_location_and_detail() {
    let r = scan_with(MARKER_RULE, "a.txt", format!("{}EVIL_MARKER{}", "x".repeat(50), "y".repeat(400)).as_bytes());
    let finding = r.findings.iter().find(|f| f.code == "RULE_MATCH").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.location, "Marker");
    assert!(finding.detail.contains("시험 표지") && finding.detail.contains("$a") && finding.detail.contains("50"), "{}", finding.detail);
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
    let low = "rule LowOne { meta: severity = \"low\" strings: $a = \"EVIL_MARKER\" condition: $a }";
    let mut r = scan_with(low, "a.txt", "EVIL_MARKER ".repeat(10).as_bytes());
    assert!(r.has("RULE_MATCH"));
    r.suppress_below(Severity::High);
    assert!(!r.has("RULE_MATCH"), "low must be hidden by the default min severity");
}

#[test]
fn a_path_that_cannot_be_read_is_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let compiled = parse(MARKER_RULE).unwrap().compile();
    let mut report = FileReport::new(d.path(), None);
    rules::scan_report(d.path(), &compiled, &mut report);
    assert!(report.incomplete && report.reasons.iter().any(|m| m == "rules: file could not be read"), "{:?}", report.reasons);
}
