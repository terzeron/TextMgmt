//! 텍스트·JSON·리포트 파일 출력 테스트.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use bookdoctor::report::{Aggregate, write_json_line, write_report_file, write_text};
use scan_common::{Category, FileReport, Format, Severity, Summary};

fn flagged(path: &Path) -> FileReport {
    let mut r = FileReport::new(path, Some(Format::Epub));
    r.push(Category::Security, Severity::High, "EPUB_SCRIPT", "a.xhtml", "script \u{1b}[31m\nelement");
    r
}

#[test]
fn json_line_has_category_severity_code_and_viewer() {
    let mut out = Vec::new();
    write_json_line(&flagged(Path::new("a.epub")), &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("\"category\":\"security\""), "{s}");
    assert!(s.contains("\"severity\":\"HIGH\""), "{s}");
    assert!(s.contains("\"code\":\"EPUB_SCRIPT\""), "{s}");
    assert!(s.contains("\"viewer_blocked\":\"n/a\""), "{s}");
    assert_eq!(s.matches('\n').count(), 1);
}

#[test]
fn file_name_with_control_chars_never_breaks_json_or_text() {
    let r = flagged(Path::new("a\u{1b}[31m\n.txt"));
    let mut json = Vec::new();
    write_json_line(&r, &mut json).unwrap();
    let json = String::from_utf8(json).unwrap();
    let body = json.strip_suffix('\n').unwrap();
    assert!(!body.contains('\u{1b}') && !body.contains('\n'));
    serde_json::from_str::<serde_json::Value>(body).unwrap();

    let mut text = Vec::new();
    write_text(&r, &mut text).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(text.lines().count() >= 2);
    assert!(text.chars().all(|c| c == '\n' || !c.is_control()), "{text:?}");
}

#[test]
fn non_utf8_file_name_is_lossy_not_a_panic() {
    let r = flagged(Path::new(OsStr::from_bytes(b"\xff.txt")));
    let mut text = Vec::new();
    write_text(&r, &mut text).unwrap();
    assert!(String::from_utf8(text).unwrap().contains(".txt"));
    let mut json = Vec::new();
    write_json_line(&r, &mut json).unwrap();
    assert!(String::from_utf8(json).unwrap().contains(".txt"));
}

#[test]
fn report_file_contains_only_flagged_files_and_summary() {
    let clean = FileReport::new(Path::new("ok.epub"), Some(Format::Epub));
    let bad = flagged(Path::new("bad.epub"));
    let mut agg = Aggregate::new();
    agg.add(&clean);
    agg.add(&bad);
    let mut summary = Summary { scanned: 2, has_findings: true, ..Summary::default() };
    summary.by_severity.insert("HIGH", 1);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("r.json");
    write_report_file(&path, &agg, &summary, "/books").unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 1);
    assert_eq!(v["summary"]["scanned"], 2);
    assert_eq!(v["summary"]["by_severity"]["HIGH"], 1);
    assert_eq!(v["summary"]["by_category"]["security"], 1);
    assert_eq!(v["summary"]["incomplete"], 0);
    assert_eq!(v["root"], "/books");
    assert!(v["scanned_at"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn clean_file_prints_nothing() {
    let clean = FileReport::new(Path::new("ok.epub"), Some(Format::Epub));
    let (mut t, mut j) = (Vec::new(), Vec::new());
    write_text(&clean, &mut t).unwrap();
    write_json_line(&clean, &mut j).unwrap();
    assert!(t.is_empty() && j.is_empty());
}

#[test]
fn incomplete_file_is_kept_and_printed_with_reasons() {
    let mut r = FileReport::new(Path::new("x.pdf"), Some(Format::Pdf));
    r.mark_incomplete("unreadable: boom");
    let mut t = Vec::new();
    write_text(&r, &mut t).unwrap();
    assert!(String::from_utf8(t).unwrap().contains("unreadable: boom"));
    let mut agg = Aggregate::new();
    agg.add(&r);
    assert_eq!((agg.incomplete(), agg.findings()), (1, 0));
}

#[test]
fn bidi_file_name_is_hidden_in_text_but_kept_in_json() {
    let r = flagged(Path::new("evil\u{202e}gpj.exe"));
    let mut text = Vec::new();
    write_text(&r, &mut text).unwrap();
    assert!(!String::from_utf8(text).unwrap().contains('\u{202e}'));
    let mut json = Vec::new();
    write_json_line(&r, &mut json).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["path"], "evil\u{202e}gpj.exe");
}
