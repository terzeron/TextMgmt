//! scan_common::finding 테스트: 지표 기록과 Report 연동, 경로 정리, JSON 모양.

use std::path::Path;

use scan_common::finding::{Category, FileReport, Format};
use scan_common::{Report, Severity};

#[test]
fn push_records_a_finding_and_report_trait_sees_it() {
    let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
    r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "encoding=cp949");
    assert!(r.has("NOT_UTF8") && !r.has("X"));
    assert_eq!(r.severities().collect::<Vec<_>>(), vec![Severity::High]);
    assert!(!r.is_incomplete());
}

#[test]
fn mark_incomplete_keeps_the_reason() {
    let mut r = FileReport::new(Path::new("a.pdf"), Some(Format::Pdf));
    r.mark_incomplete("skipped");
    assert_eq!(r.reasons, vec!["skipped".to_string()]);
    assert!(r.is_incomplete());
}

#[test]
fn path_keeps_the_original_name_so_quarantine_finds_the_real_file() {
    let r = FileReport::new(Path::new("a\u{1b}b\u{202e}.txt"), None);
    assert_eq!(r.path, "a\u{1b}b\u{202e}.txt");
}

#[cfg(feature = "serde")]
#[test]
fn json_uses_lowercase_category_and_na_viewer() {
    let mut r = FileReport::new(Path::new("a.txt"), Some(Format::Txt));
    r.push(Category::Defect, Severity::High, "NOT_UTF8", "", "d");
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["format"], "txt");
    assert_eq!(v["findings"][0]["category"], "defect");
    assert_eq!(v["findings"][0]["severity"], "HIGH");
    assert_eq!(v["findings"][0]["viewer_blocked"], "n/a");
}

use scan_common::finding::Suppressed;

fn mixed_report() -> FileReport {
    let mut r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    r.push(Category::Defect, Severity::Low, "L1", "", "");
    r.push(Category::Defect, Severity::Low, "L2", "", "");
    r.push(Category::Render, Severity::Medium, "M1", "", "");
    r.push(Category::Security, Severity::High, "H1", "", "");
    r
}

#[test]
fn suppress_below_removes_lower_and_counts_by_severity() {
    let mut r = mixed_report();
    r.suppress_below(Severity::High);
    assert_eq!(r.findings.len(), 1);
    assert!(r.has("H1"));
    assert_eq!(r.suppressed, Suppressed { low: 2, medium: 1, high: 0 });
    assert_eq!(r.suppressed.total(), 3);
    assert!(!r.suppressed.is_empty());
}

#[test]
fn suppress_below_low_keeps_everything() {
    let mut r = mixed_report();
    r.suppress_below(Severity::Low);
    assert_eq!(r.findings.len(), 4);
    assert!(r.suppressed.is_empty());
}

#[test]
fn suppress_below_critical_also_counts_high() {
    let mut r = mixed_report();
    r.suppress_below(Severity::Critical);
    assert!(r.findings.is_empty());
    assert_eq!(r.suppressed, Suppressed { low: 2, medium: 1, high: 1 });
}

#[test]
fn suppress_below_does_not_touch_incomplete_state() {
    let mut r = mixed_report();
    r.mark_incomplete("skipped");
    r.suppress_below(Severity::Critical);
    assert!(r.is_incomplete());
    assert_eq!(r.reasons, vec!["skipped".to_string()]);
}

#[cfg(feature = "serde")]
#[test]
fn suppressed_is_omitted_from_json_when_empty_and_present_otherwise() {
    let mut r = mixed_report();
    let v = serde_json::to_value(&r).unwrap();
    assert!(v.get("suppressed").is_none(), "{v}");
    r.suppress_below(Severity::High);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["suppressed"]["low"], 2);
    assert_eq!(v["suppressed"]["medium"], 1);
    assert_eq!(v["suppressed"]["high"], 0);
}
