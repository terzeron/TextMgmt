//! 텍스트·JSON·리포트 파일 출력 테스트.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use bookdoctor::report::{Aggregate, finding_style, write_json_line, write_report_file, write_text};
use scan_common::color::Style;
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
    write_text(&r, &mut text, false).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(text.lines().count() >= 2);
    assert!(text.chars().all(|c| c == '\n' || !c.is_control()), "{text:?}");
}

#[test]
fn non_utf8_file_name_is_lossy_not_a_panic() {
    let r = flagged(Path::new(OsStr::from_bytes(b"\xff.txt")));
    let mut text = Vec::new();
    write_text(&r, &mut text, false).unwrap();
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
    write_text(&clean, &mut t, false).unwrap();
    write_json_line(&clean, &mut j).unwrap();
    assert!(t.is_empty() && j.is_empty());
}

#[test]
fn incomplete_file_is_kept_and_printed_with_reasons() {
    let mut r = FileReport::new(Path::new("x.pdf"), Some(Format::Pdf));
    r.mark_incomplete("unreadable: boom");
    let mut t = Vec::new();
    write_text(&r, &mut t, false).unwrap();
    assert!(String::from_utf8(t).unwrap().contains("unreadable: boom"));
    let mut agg = Aggregate::new();
    agg.add(&r);
    assert_eq!((agg.incomplete(), agg.findings()), (1, 0));
}

#[test]
fn bidi_file_name_is_hidden_in_text_but_kept_in_json() {
    let r = flagged(Path::new("evil\u{202e}gpj.exe"));
    let mut text = Vec::new();
    write_text(&r, &mut text, false).unwrap();
    assert!(!String::from_utf8(text).unwrap().contains('\u{202e}'));
    let mut json = Vec::new();
    write_json_line(&r, &mut json).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(v["path"], "evil\u{202e}gpj.exe");
}

#[test]
fn escape_and_bidi_in_the_path_are_hidden_in_text_but_the_real_file_is_found() {
    use bookdoctor::cli::Options;
    use bookdoctor::scan::scan_file;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("ev\u{1b}[31m\u{202e}il.txt");
    std::fs::write(&p, [0u8; 4096]).unwrap();
    let r = scan_file(&p, &Options { no_poppler: true, ..Options::default() });
    // 열고 옮기는 쪽이 쓰는 경로는 원문이다.
    assert_eq!(Path::new(&r.path), p.as_path());
    assert!(Path::new(&r.path).exists());
    let mut text = Vec::new();
    write_text(&r, &mut text, false).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(!text.is_empty());
    assert!(!text.contains('\u{1b}') && !text.contains('\u{202e}'), "{text:?}");
}

#[test]
fn aggregate_counts_suppressed_even_for_files_that_print_nothing() {
    use bookdoctor::report::write_summary_line;
    let mut only_low = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    only_low.push(Category::Defect, Severity::Low, "L", "", "");
    only_low.push(Category::Defect, Severity::Medium, "M", "", "");
    only_low.suppress_below(Severity::High);
    let mut agg = Aggregate::new();
    agg.add(&only_low);
    assert_eq!(agg.suppressed().low, 1);
    assert_eq!(agg.suppressed().medium, 1);
    assert_eq!(agg.findings(), 0);
    let mut out = Vec::new();
    write_summary_line(&agg, &Summary { scanned: 1, ..Summary::default() }, &mut out).unwrap();
    let line = String::from_utf8(out).unwrap();
    assert!(line.contains("억제 LOW 1건, MEDIUM 1건"), "{line}");
}

#[test]
fn summary_line_omits_suppressed_part_when_nothing_was_suppressed() {
    use bookdoctor::report::write_summary_line;
    let agg = Aggregate::new();
    let mut out = Vec::new();
    write_summary_line(&agg, &Summary { scanned: 3, ..Summary::default() }, &mut out).unwrap();
    let line = String::from_utf8(out).unwrap();
    assert!(!line.contains("억제"), "{line}");
}

#[test]
fn summary_line_lists_only_nonzero_suppressed_severities() {
    use bookdoctor::report::write_summary_line;
    let mut r = FileReport::new(Path::new("a.epub"), Some(Format::Epub));
    r.push(Category::Defect, Severity::Medium, "M", "", "");
    r.suppress_below(Severity::High);
    let mut agg = Aggregate::new();
    agg.add(&r);
    let mut out = Vec::new();
    write_summary_line(&agg, &Summary::default(), &mut out).unwrap();
    let line = String::from_utf8(out).unwrap();
    assert!(line.contains("억제 MEDIUM 1건") && !line.contains("LOW"), "{line}");
}

// --- 색 ---

fn finding_of(category: Category, severity: Severity, code: &str) -> scan_common::Finding {
    let mut r = FileReport::new(Path::new("a"), None);
    r.push(category, severity, code, "", "");
    r.findings.remove(0)
}

#[test]
fn finding_style_rules() {
    let s = |c, v, code| finding_style(&finding_of(c, v, code));
    assert_eq!(s(Category::Render, Severity::Low, "X"), Some(Style::Red));
    assert_eq!(s(Category::Defect, Severity::High, "X"), Some(Style::Red));
    assert_eq!(s(Category::Security, Severity::High, "X"), Some(Style::Purple));
    assert_eq!(s(Category::Security, Severity::Critical, "X"), Some(Style::Red));
    assert_eq!(s(Category::Security, Severity::Medium, "X"), None);
    assert_eq!(s(Category::Security, Severity::Low, "X"), None);
    // 암호화는 분류·심각도와 무관하게 cyan이다.
    assert_eq!(s(Category::Render, Severity::High, "ENCRYPTED"), Some(Style::Cyan));
}

#[test]
fn write_text_paints_whole_finding_line_when_color_is_on() {
    let r = flagged(Path::new("a.epub"));
    let mut out = Vec::new();
    write_text(&r, &mut out, true).unwrap();
    let text = String::from_utf8(out).unwrap();
    let painted: Vec<&str> = text.lines().filter(|l| l.contains("\u{1b}[35m")).collect();
    assert_eq!(painted.len(), 1, "{text:?}");
    assert!(painted[0].starts_with("\u{1b}[35m  [HIGH] EPUB_SCRIPT") && painted[0].ends_with("\u{1b}[0m"), "{text:?}");
}

#[test]
fn write_text_without_color_has_no_escape_at_all() {
    let r = flagged(Path::new("a.epub"));
    let mut out = Vec::new();
    write_text(&r, &mut out, false).unwrap();
    assert!(!String::from_utf8(out).unwrap().contains('\u{1b}'));
}

#[test]
fn encrypted_incomplete_reason_is_cyan_and_other_reasons_are_not() {
    let mut r = FileReport::new(Path::new("a.pdf"), Some(Format::Pdf));
    r.mark_incomplete("encrypted: string values may be unreadable");
    r.mark_incomplete("skipped: too big");
    let mut out = Vec::new();
    write_text(&r, &mut out, true).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\u{1b}[36m  INCOMPLETE encrypted: string values may be unreadable\u{1b}[0m"), "{text:?}");
    assert!(text.contains("\n  INCOMPLETE skipped: too big\n"), "{text:?}");
}

#[test]
fn escape_in_file_name_and_detail_never_reaches_output_raw() {
    let r = flagged(Path::new("a\u{1b}[31m.txt"));
    let mut out = Vec::new();
    write_text(&r, &mut out, true).unwrap();
    let text = String::from_utf8(out).unwrap();
    // 우리가 붙인 시작·끝 코드 한 쌍(칠한 줄 1개)만 있어야 한다.
    assert_eq!(text.matches('\u{1b}').count(), 2, "{text:?}");
}
