//! `bookdoctor::rule_engine` 단위 테스트.

mod support;
use std::fs;
use std::sync::Arc;
use bookdoctor::cli_parser::Options;
use bookdoctor::{FileReport, rule_engine, Severity};
use bookdoctor::rule_engine::{parse};
use bookdoctor::format_dispatcher::scan_file;
use std::time::{Duration, Instant};

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
    rule_engine::scan_report_with_budget(&f, &compiled, &mut report, 100_000);
    assert!(report.incomplete && report.reasons.iter().any(|m| m.contains("work limit")), "{:?}", report.reasons);
}

#[test]
fn rules_apply_to_nested_files_under_recursion() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("outer.zip");
    support::zip_of(&p, &[("inner.rtf", b"{\\rtf1 hello EVIL_MARKER}"), ("plain.bin", b"EVIL_MARKER not a container")]);
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
    rule_engine::scan_report(d.path(), &compiled, &mut report);
    assert!(report.incomplete && report.reasons.iter().any(|m| m == "rules: file could not be read"), "{:?}", report.reasons);
}

#[test]
fn mutated_rule_sources_and_random_data_never_panic_or_stall() {
    // 모든 문법 요소가 든 규칙 텍스트를 변조해 파싱하고, 파싱이 되면 무작위 데이터에 적용한다. panic·지연이 없어야 한다.
    let base = "rule A { meta: severity = \"high\" description = \"d\" strings: $a = \"abc\" nocase wide $b = { 4D ?? 90 00 } $c = \"x\\x41y\" ascii condition: ( $a and #b >= 1 ) or $c at 3 or 2 of them or not $a }\nrule B { strings: $z = \"zz\" condition: any of them }";
    let mut seed = 0x5EED_1234u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as usize
    };
    let data: Vec<u8> = (0..4096).map(|_| (next() & 0xFF) as u8).collect();
    let started = Instant::now();
    let (mut parsed, mut rejected) = (0, 0);
    for _ in 0..3000 {
        let mut bytes = base.as_bytes().to_vec();
        for _ in 0..1 + next() % 4 {
            let at = next() % bytes.len();
            match next() % 3 {
                0 => bytes[at] = (next() & 0xFF) as u8,
                1 => bytes.insert(at, (next() & 0xFF) as u8),
                _ => {
                    bytes.remove(at);
                }
            }
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        match parse(&text) {
            Ok(set) => {
                parsed += 1;
                let _ = set.scan(&data);
                let _ = set.scan(b"");
            }
            Err(_) => rejected += 1,
        }
    }
    assert!(started.elapsed() < Duration::from_secs(60), "{:?}", started.elapsed());
    assert!(parsed > 0 && rejected > 0, "the mutations should produce both valid and invalid sources ({parsed}/{rejected})");
}
