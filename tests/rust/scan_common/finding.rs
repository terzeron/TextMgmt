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
fn path_is_sanitized_of_control_characters() {
    let r = FileReport::new(Path::new("a\u{1b}b.txt"), None);
    assert!(!r.path.contains('\u{1b}'));
    assert!(r.path.ends_with("b.txt"));
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
