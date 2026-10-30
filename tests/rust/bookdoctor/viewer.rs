//! `viewer_blocked` 표 테스트. 표가 낡지 않도록 스캐너 소스와 대조한다.

mod common;

use std::collections::BTreeSet;

use bookdoctor::cli::Options;
use bookdoctor::report::write_json_line;
use bookdoctor::scan::scan_file;
use bookdoctor::viewer::{EPUB_CODES, PDF_CODES, TXT_CODES, viewer_blocked};
use regex::Regex;
use scan_common::{Category, Format, ViewerBlocked};

const EPUB_SECURITY_RS: &str = include_str!("../../../utils/bookdoctor/src/epub/security.rs");
const PDF_SECURITY_RS: &str = include_str!("../../../utils/bookdoctor/src/pdf/security.rs");
const TXT_RS: &str = include_str!("../../../utils/bookdoctor/src/txt.rs");

fn opts() -> Options {
    Options { no_poppler: true, ..Options::default() }
}

#[test]
fn epub_script_is_blocked() {
    for code in ["SCRIPT_TAG", "EVENT_HANDLER_ATTR", "JS_URL", "REMOTE_SCRIPT", "SCRIPT_FILE", "SCRIPT_MEDIA_TYPE", "ACTIVE_ELEMENT", "FORM_ELEMENT", "REMOTE_FORM_ACTION", "META_REFRESH"] {
        assert_eq!(viewer_blocked(Format::Epub, code), ViewerBlocked::Blocked, "{code}");
    }
}

#[test]
fn epub_unfiltered_risks_are_not_blocked() {
    for code in ["SVG_FOREIGN_OBJECT", "XINCLUDE", "XXE_ENTITY", "ENTITY_DECLARATION", "EXTERNAL_DTD", "REMOTE_RESOURCE", "BASE_HREF", "REMOTE_CSS_IMPORT"] {
        assert_eq!(viewer_blocked(Format::Epub, code), ViewerBlocked::NotBlocked, "{code}");
    }
}

#[test]
fn pdf_javascript_is_not_blocked() {
    // pdfjsEnv.js의 PDF_DOCUMENT_OPTIONS에는 enableScripting·isEvalSupported가 없다.
    for code in ["JavaScript action", "exploit JavaScript", "Launch action", "embedded executable content"] {
        assert_eq!(viewer_blocked(Format::Pdf, code), ViewerBlocked::NotBlocked, "{code}");
    }
}

#[test]
fn txt_html_is_blocked_but_display_spoofing_is_not() {
    assert_eq!(viewer_blocked(Format::Txt, "HTML_IN_TEXT"), ViewerBlocked::Blocked);
    assert_eq!(viewer_blocked(Format::Txt, "BIDI_CONTROL"), ViewerBlocked::NotBlocked);
    assert_eq!(viewer_blocked(Format::Txt, "ZERO_WIDTH"), ViewerBlocked::NotBlocked);
}

#[test]
fn defect_is_not_applicable() {
    for (f, code) in [(Format::Epub, "HEAD_MISSING"), (Format::Epub, "ZIP_PATH_TRAVERSAL"), (Format::Epub, "ELF_BINARY_CONTENT"), (Format::Pdf, "NO_EOF_MARKER"), (Format::Txt, "EMPTY_TEXT"), (Format::Txt, "NO_SUCH_CODE"), (Format::Epub, "MOJIBAKE")] {
        assert_eq!(viewer_blocked(f, code), ViewerBlocked::NotApplicable, "{code}");
    }
}

#[test]
fn every_table_code_exists_in_its_scanner_source() {
    for (code, _) in EPUB_CODES {
        assert!(EPUB_SECURITY_RS.contains(&format!("\"{code}\"")), "EPUB 표의 {code}가 security.rs에 없다");
    }
    for (code, _) in PDF_CODES {
        assert!(PDF_SECURITY_RS.contains(&format!("\"{code}\"")), "PDF 표의 {code}가 security.rs에 없다");
    }
    for (code, _) in TXT_CODES {
        assert!(TXT_RS.contains(&format!("\"{code}\"")), "TXT 표의 {code}가 txt.rs에 없다");
    }
}

/// security.rs에서 `"UPPER_SNAKE"` 리터럴(6자 이상)을 모두 뽑는다. 코드가 아닌 문자열은 `NOT_CODES`로 뺀다.
fn epub_scanner_codes() -> BTreeSet<String> {
    // 현재 security.rs에는 코드가 아닌 대문자 리터럴이 없다. 생기면 여기에 사유와 함께 적는다.
    const NOT_CODES: &[&str] = &[];
    let re = Regex::new(r#""([A-Z][A-Z_]{5,})""#).unwrap();
    re.captures_iter(EPUB_SECURITY_RS).map(|c| c[1].to_string()).filter(|c| !NOT_CODES.contains(&c.as_str())).collect()
}

#[test]
fn every_epub_scanner_code_is_classified() {
    let table: BTreeSet<&str> = EPUB_CODES.iter().map(|(c, _)| *c).collect();
    let missing: Vec<String> = epub_scanner_codes().into_iter().filter(|c| !table.contains(c.as_str())).collect();
    assert!(missing.is_empty(), "viewer.rs 표에 분류가 없는 EPUB 코드: {missing:?}");
    assert!(epub_scanner_codes().len() > 50, "추출이 너무 적다");
}

/// pdf/security.rs에서 지표 종류 문자열을 뽑는다. `Severity::X, "kind"`, `Severity::X }, "kind"`, `kind: "kind"` 위치다.
fn pdf_scanner_kinds() -> BTreeSet<String> {
    let re = Regex::new(r#"(?:Severity::\w+(?:\s*\})?\s*,|kind:)\s*"([^"]+)""#).unwrap();
    re.captures_iter(PDF_SECURITY_RS).map(|c| c[1].to_string()).collect()
}

#[test]
fn every_pdf_scanner_kind_is_classified() {
    let table: BTreeSet<&str> = PDF_CODES.iter().map(|(c, _)| *c).collect();
    let kinds = pdf_scanner_kinds();
    let missing: Vec<&String> = kinds.iter().filter(|k| !table.contains(k.as_str())).collect();
    assert!(missing.is_empty(), "viewer.rs 표에 분류가 없는 PDF 종류: {missing:?}");
    assert!(kinds.len() > 20, "추출이 너무 적다: {kinds:?}");
}

#[test]
fn every_txt_security_code_is_classified() {
    let re = Regex::new(r#"Category::Security,\s*(?:Severity::\w+,\s*)?"([A-Z_]+)""#).unwrap();
    let found: BTreeSet<String> = re.captures_iter(TXT_RS).map(|c| c[1].to_string()).collect();
    let table: BTreeSet<&str> = TXT_CODES.iter().map(|(c, _)| *c).collect();
    assert_eq!(found.iter().map(String::as_str).collect::<BTreeSet<_>>(), table);
}

fn scripted_epub() -> common::NamedFile {
    let f = common::epub_named("s.epub");
    common::epub_with_chapter(f.path(), &common::xhtml("<script>eval(x)</script><p>hi</p>"));
    f
}

fn value_of(r: &scan_common::FileReport, code: &str) -> ViewerBlocked {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", r.findings)).viewer_blocked
}

#[test]
fn annotate_sets_every_finding() {
    let e = scripted_epub();
    let r = scan_file(e.path(), &opts());
    assert_eq!(value_of(&r, "SCRIPT_TAG"), ViewerBlocked::Blocked);
    assert_eq!(value_of(&r, "DYNAMIC_CODE_EXEC"), ViewerBlocked::Blocked);
    for f in &r.findings {
        assert_eq!(f.viewer_blocked, viewer_blocked(Format::Epub, &f.code), "{}", f.code);
        if f.category != Category::Security {
            assert_eq!(f.viewer_blocked, ViewerBlocked::NotApplicable, "{}", f.code);
        }
    }
    let t = common::file_with("a.txt", "<script>alert(1)</script>\n본문입니다.\u{202E}\n".as_bytes());
    let r = scan_file(t.path(), &opts());
    assert_eq!(value_of(&r, "HTML_IN_TEXT"), ViewerBlocked::Blocked);
    assert_eq!(value_of(&r, "BIDI_CONTROL"), ViewerBlocked::NotBlocked);
}

#[test]
fn json_shows_viewer_blocked_values() {
    let e = scripted_epub();
    let r = scan_file(e.path(), &opts());
    let mut out = Vec::new();
    write_json_line(&r, &mut out).unwrap();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("\"viewer_blocked\":\"blocked\""), "{s}");
    let t = common::file_with("b.txt", "<script>x</script>\n\u{202E}한글\n".as_bytes());
    let mut out = Vec::new();
    write_json_line(&scan_file(t.path(), &opts()), &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("\"viewer_blocked\":\"not_blocked\""));
    let d = common::file_with("c.txt", b"\xEF\xBF\xBD\xEF\xBF\xBD\xEF\xBF\xBD\n<p>x</p>\n");
    let mut out = Vec::new();
    write_json_line(&scan_file(d.path(), &opts()), &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("\"viewer_blocked\":\"n/a\""));
}

#[test]
fn only_filter_keeps_viewer_values() {
    let e = scripted_epub();
    let r = scan_file(e.path(), &Options { only: vec![Category::Security], ..opts() });
    assert!(!r.findings.is_empty());
    assert!(r.findings.iter().all(|f| f.category == Category::Security));
    assert_eq!(value_of(&r, "SCRIPT_TAG"), ViewerBlocked::Blocked);
}

#[test]
fn executable_format_is_not_applicable_to_the_viewer() {
    for code in ["EXECUTABLE_FILE", "EXECUTABLE_EXTENSION_NO_MAGIC", "ANYTHING"] {
        assert_eq!(viewer_blocked(Format::Executable, code), ViewerBlocked::NotApplicable, "{code}");
    }
}
