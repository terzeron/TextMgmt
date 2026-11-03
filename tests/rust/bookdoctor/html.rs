//! 독립 HTML 테스트. 규칙은 EPUB의 HTML 규칙을 재사용한다.

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli::Options;
use bookdoctor::scan::scan_file;
use bookdoctor::{FileReport, Format};

fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, bytes).unwrap();
    (d, p)
}

fn scan(name: &str, bytes: &[u8]) -> FileReport {
    let (_d, p) = write(name, bytes);
    scan_file(&p, &Options::default())
}

fn utf16le_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    out.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
    out
}

#[test]
fn plain_html_is_clean() {
    let r = scan("a.html", b"<!DOCTYPE html><html><head><title>t</title></head><body><p>hello</p></body></html>");
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn saved_pages_that_start_with_a_comment_or_meta_are_html_not_a_mismatch() {
    for head in ["<!-- saved from url=(0030)http://example.com/ -->\n<html><body>hi</body></html>", "<meta charset=\"utf-8\"><p>fragment only</p>", "\u{feff}  <p>bom and spaces</p>", "plain text with no tags at all"] {
        let r = scan("a.html", head.as_bytes());
        assert_eq!(r.format, Some(Format::Html), "{head}");
        assert!(!r.has("FORMAT_MISMATCH"), "{head}: {:?}", r.findings);
    }
}

#[test]
fn script_event_handler_and_remote_script_use_the_epub_rules() {
    let page = r#"<html><body onload="x()"><script src="http://evil.example/a.js"></script><script>var a = 1;</script></body></html>"#;
    let r = scan("a.html", page.as_bytes());
    for code in ["EVENT_HANDLER_ATTR", "REMOTE_SCRIPT", "SCRIPT_TAG"] {
        assert!(r.has(code), "{code}: {:?}", r.findings);
    }
    // 위치는 파일 이름이 아니라 검사 대상 이름이다.
    assert!(r.findings.iter().all(|f| !f.location.is_empty()));
}

#[test]
fn binary_content_named_html_is_a_mismatch() {
    let pdf = scan("a.html", b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n");
    assert!(pdf.has("FORMAT_MISMATCH"), "{:?}", pdf.findings);
    assert_eq!(pdf.format, Some(Format::Pdf));
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    assert!(scan("b.htm", &pe).has("DISGUISED_EXECUTABLE"));
    let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.extend_from_slice(&[0u8; 600]);
    let r = scan("c.html", &ole);
    assert!(r.has("FORMAT_MISMATCH") && r.incomplete, "unanalyzed content must not be clean: {:?} {:?}", r.findings, r.reasons);
    let r = scan("d.html", b"{\\rtf1{\\*\\objclass X}}");
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
}

#[test]
fn large_html_is_incomplete_after_the_limit() {
    let mut page = b"<html><script src=\"http://evil.example/a.js\"></script>".to_vec();
    page.extend(std::iter::repeat_n(b' ', 17 * 1024 * 1024));
    let r = scan("big.html", &page);
    assert!(r.has("REMOTE_SCRIPT"), "{:?}", r.findings);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("limit")), "{:?}", r.reasons);
}

#[test]
fn utf16_html_is_decoded() {
    let r = scan("a.html", &utf16le_bom("<html><script src=\"http://evil.example/a.js\"></script></html>"));
    assert!(r.has("REMOTE_SCRIPT"), "{:?}", r.findings);
}

// --- 인코딩 우회 (EPUB의 `check_member`와 같은 처리) ---

const REMOTE: &str = r#"<html><script src="http://evil.example/a.js"></script></html>"#;

#[test]
fn utf32_html_is_incomplete_because_it_cannot_be_inspected() {
    let mut bytes = vec![0xFF, 0xFE, 0, 0];
    bytes.extend(REMOTE.chars().flat_map(|c| (c as u32).to_le_bytes()));
    let r = scan("a.html", &bytes);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.to_lowercase().contains("utf-32")), "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn nul_bytes_in_utf8_html_are_not_reported_clean() {
    let mut bytes = REMOTE.as_bytes().to_vec();
    bytes.splice(6..6, [0u8]);
    let r = scan("a.html", &bytes);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("NUL")), "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn utf16_bom_followed_by_ascii_markup_is_still_scanned() {
    // `FF FE` 뒤에 ASCII 마크업이 오면 UTF-16 변환 결과는 깨진 글자다. 바이트를 그대로 읽은 텍스트도 함께 검사한다.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend_from_slice(REMOTE.as_bytes());
    assert!(scan("a.html", &bytes).has("REMOTE_SCRIPT"));
}

#[test]
fn a_utf8_bom_does_not_hide_the_markup() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(REMOTE.as_bytes());
    assert!(scan("a.html", &bytes).has("REMOTE_SCRIPT"));
}

#[test]
fn legacy_encoded_pages_are_still_scanned() {
    // EUC-KR 한글(0xB0 0xA1)이 든 페이지도 ASCII 마크업은 그대로 검사한다.
    let mut bytes = b"<html><body>".to_vec();
    bytes.extend_from_slice(&[0xB0, 0xA1, 0xB3, 0xAA]);
    bytes.extend_from_slice(br#"<script src="http://evil.example/a.js"></script></body></html>"#);
    assert!(scan("a.html", &bytes).has("REMOTE_SCRIPT"));
}

#[test]
fn a_missing_or_unreadable_html_path_is_incomplete() {
    use bookdoctor::html::scan_html_file;
    use bookdoctor::FileReport;
    use bookdoctor::sniff::Sniffed;
    let d = tempfile::tempdir().unwrap();
    let mut r = FileReport::new(&d.path().join("none.html"), None);
    scan_html_file(&d.path().join("none.html"), Sniffed::Html, &mut r);
    assert!(r.reasons.iter().any(|m| m == "HTML file could not be opened"), "{:?}", r.reasons);
    // 디렉터리는 열리지만 읽지 못한다.
    let mut r = FileReport::new(d.path(), None);
    scan_html_file(d.path(), Sniffed::Html, &mut r);
    assert!(r.reasons.iter().any(|m| m == "HTML file could not be read"), "{:?}", r.reasons);
}
