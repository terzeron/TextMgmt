//! TXT 검사 테스트.

mod common;

use bookdoctor::cli::Options;
use bookdoctor::scan::scan_file;
use scan_common::{Category, FileReport, Severity};

const CLEAN: &str = "한글 소설입니다.\n두 번째 줄입니다.\n";

fn scan_bytes(bytes: &[u8]) -> FileReport {
    let f = common::file_with("a.txt", bytes);
    scan_file(f.path(), &Options::default())
}

fn scan_str(s: &str) -> FileReport {
    scan_bytes(s.as_bytes())
}

fn find<'a>(r: &'a FileReport, code: &str) -> &'a scan_common::Finding {
    r.findings.iter().find(|x| x.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", r.findings))
}

fn has(r: &FileReport, code: &str) -> bool {
    r.findings.iter().any(|x| x.code == code)
}

#[test]
fn utf8_bom_is_clean() {
    let r = scan_bytes(b"\xEF\xBB\xBF\xED\x95\x9C\xEA\xB8\x80\n");
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn utf16_bom_is_not_utf8_and_does_not_panic() {
    let r = scan_bytes(b"\xFF\xFEh\x00i\x00");
    let d = find(&r, "NOT_UTF8");
    assert_eq!((d.severity, d.detail.as_str()), (Severity::High, "encoding=utf-16le"));
    assert_eq!(d.category, Category::Defect);
    assert!(!has(&r, "BINARY_CONTENT"));
    assert!(!r.incomplete, "{:?}", r.reasons);
}

#[test]
fn utf16_be_bom_is_not_utf8() {
    let r = scan_bytes(b"\xFE\xFF\x00h\x00i");
    assert_eq!(find(&r, "NOT_UTF8").detail, "encoding=utf-16be");
    assert!(!has(&r, "BINARY_CONTENT"));
    assert!(!has(&r, "INVALID_UTF8"));
}

#[test]
fn cp949_text_is_not_utf8() {
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("한글 소설입니다");
    let r = scan_bytes(&bytes);
    let d = find(&r, "NOT_UTF8");
    assert_eq!((d.severity, d.detail.as_str()), (Severity::High, "encoding=cp949"));
}

#[test]
fn unknown_encoding_is_reported_as_unknown() {
    let r = scan_bytes(b"ok\xff\xff\xff");
    assert_eq!(find(&r, "NOT_UTF8").detail, "encoding=unknown");
}

#[test]
fn flags_not_utf8() {
    assert!(has(&scan_bytes(b"ok\xff"), "NOT_UTF8"));
}

#[test]
fn does_not_flag_not_utf8_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "NOT_UTF8"));
}

#[test]
fn flags_invalid_utf8() {
    let r = scan_bytes(b"ok\xff");
    let d = find(&r, "INVALID_UTF8");
    assert_eq!((d.category, d.severity, d.detail.as_str()), (Category::Defect, Severity::High, "offset=2"));
    // NOT_UTF8이 먼저, INVALID_UTF8이 나중에 나온다.
    let pos = |c: &str| r.findings.iter().position(|x| x.code == c).unwrap();
    assert!(pos("NOT_UTF8") < pos("INVALID_UTF8"));
}

#[test]
fn does_not_flag_invalid_utf8_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "INVALID_UTF8"));
}

#[test]
fn invalid_utf8_skips_text_checks() {
    let r = scan_bytes(b"ok\xff\0\0");
    assert!(!has(&r, "BINARY_CONTENT"));
}

#[test]
fn flags_replacement_char() {
    let r = scan_str("한글\u{FFFD}소설\n");
    let d = find(&r, "REPLACEMENT_CHAR");
    assert_eq!((d.category, d.severity), (Category::Defect, Severity::Medium));
}

#[test]
fn does_not_flag_replacement_char_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "REPLACEMENT_CHAR"));
}

#[test]
fn flags_binary_content_on_nul() {
    let r = scan_bytes(b"abc\0def\n");
    let d = find(&r, "BINARY_CONTENT");
    assert_eq!((d.category, d.severity), (Category::Defect, Severity::High));
}

#[test]
fn does_not_flag_an_isolated_nul_in_a_large_text() {
    let mut s = "abcdefghij\n".repeat(50_000);
    s.insert(1000, '\0');
    s.insert(200_000, '\0');
    assert!(!has(&scan_str(&s), "BINARY_CONTENT"));
}

#[test]
fn flags_binary_content_when_nuls_are_dense() {
    let mut b = vec![0u8; 4096];
    b.extend_from_slice(b"text");
    assert!(has(&scan_bytes(&b), "BINARY_CONTENT"));
}

#[test]
fn executable_disguised_as_txt_is_still_critical_after_the_density_rule() {
    // 밀도 규칙은 BINARY_CONTENT만 바꾼다. 머리 표지로 가려내는 DISGUISED_EXECUTABLE은 NUL이 섞여도 남아야 한다.
    // PE는 `MZ`만으로는 실행 파일이 아니므로 e_lfanew(0x3C)가 가리키는 자리에 `PE\0\0`를 둔다.
    let mut pe = vec![0u8; 4096];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C] = 0x80;
    pe[0x80..0x84].copy_from_slice(b"PE\0\0");
    let mut elf = vec![0u8; 4096];
    elf[..4].copy_from_slice(b"\x7fELF");
    for (name, bytes) in [("pe", pe), ("elf", elf)] {
        let r = scan_bytes(&bytes);
        assert!(has(&r, "DISGUISED_EXECUTABLE"), "{name}: {:?}", r.findings);
    }
}

#[test]
fn flags_binary_content_on_control_ratio() {
    let mut s = "가".repeat(90);
    s.push_str(&"\u{1}".repeat(10));
    assert!(has(&scan_str(&s), "BINARY_CONTENT"));
}

#[test]
fn does_not_flag_binary_content_on_clean_text() {
    assert!(!has(&scan_str("탭\t과 줄바꿈\r\n과 폼피드\u{c}는 괜찮다\n"), "BINARY_CONTENT"));
    // 제어문자 1% 이하는 통과.
    let mut s = "가".repeat(200);
    s.push('\u{1}');
    assert!(!has(&scan_str(&s), "BINARY_CONTENT"));
}

#[test]
fn flags_bidi_control_once_per_file() {
    let r = scan_str("가\u{202E}나\u{2066}다\n");
    let d = find(&r, "BIDI_CONTROL");
    assert_eq!((d.category, d.severity), (Category::Security, Severity::Medium));
    assert_eq!(d.location, "char 1");
    assert_eq!(d.detail, "count=2");
    assert_eq!(r.findings.iter().filter(|x| x.code == "BIDI_CONTROL").count(), 1);
}

#[test]
fn does_not_flag_bidi_control_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "BIDI_CONTROL"));
}

#[test]
fn flags_zero_width() {
    let r = scan_str("가\u{200B}나\u{2060}다\n");
    let d = find(&r, "ZERO_WIDTH");
    assert_eq!((d.category, d.severity), (Category::Security, Severity::Medium));
    assert_eq!((d.location.as_str(), d.detail.as_str()), ("char 1", "count=2"));
}

#[test]
fn does_not_flag_zero_width_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "ZERO_WIDTH"));
}

#[test]
fn zero_width_leading_feff_is_clean_but_inner_feff_is_flagged() {
    assert!(!has(&scan_str("\u{FEFF}가나\n"), "ZERO_WIDTH"));
    assert!(has(&scan_str("가\u{FEFF}나\n"), "ZERO_WIDTH"));
}

#[test]
fn flags_html_in_text() {
    let r = scan_str("본문\n< Script src=x>\n");
    let d = find(&r, "HTML_IN_TEXT");
    assert_eq!((d.category, d.severity), (Category::Security, Severity::Medium));
    assert_eq!((d.location.as_str(), d.detail.as_str()), ("line 2", "count=1"));
    assert!(has(&scan_str("click javascript:alert(1)\n"), "HTML_IN_TEXT"));
}

#[test]
fn does_not_flag_html_in_text_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "HTML_IN_TEXT"));
    assert!(!has(&scan_str("a < b 이고 scripture 이다\n"), "HTML_IN_TEXT"));
}

#[test]
fn flags_mixed_newlines() {
    let r = scan_str("a\r\nb\nc\n");
    let d = find(&r, "MIXED_NEWLINES");
    assert_eq!((d.category, d.severity), (Category::Render, Severity::Low));
    assert!(has(&scan_str("a\r\nb\rc\r\n"), "MIXED_NEWLINES"));
    assert!(has(&scan_str("a\rb\nc\n"), "MIXED_NEWLINES"));
}

#[test]
fn does_not_flag_mixed_newlines_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "MIXED_NEWLINES"));
    assert!(!has(&scan_str("a\r\nb\r\nc\r\n"), "MIXED_NEWLINES"));
    assert!(!has(&scan_str("a\rb\rc\r"), "MIXED_NEWLINES"));
}

#[test]
fn flags_long_line() {
    let s = format!("짧다\n{}\n", "가".repeat(100_001));
    let r = scan_str(&s);
    let d = find(&r, "LONG_LINE");
    assert_eq!((d.category, d.severity, d.location.as_str()), (Category::Render, Severity::Low, "line 2"));
}

#[test]
fn does_not_flag_long_line_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "LONG_LINE"));
    assert!(!has(&scan_str(&"가".repeat(100_000)), "LONG_LINE"));
}

#[test]
fn flags_empty_text() {
    let r = scan_str(" \n\t\r\n ");
    let d = find(&r, "EMPTY_TEXT");
    assert_eq!((d.category, d.severity), (Category::Defect, Severity::Medium));
    assert!(has(&scan_bytes(b"\xEF\xBB\xBF \n"), "EMPTY_TEXT"));
}

#[test]
fn does_not_flag_empty_text_on_clean_text() {
    assert!(!has(&scan_str(CLEAN), "EMPTY_TEXT"));
}

#[test]
fn clean_text_has_no_findings() {
    let r = scan_str(CLEAN);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn oversized_file_is_truncated_and_marked_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.txt");
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(64 * 1024 * 1024 + 1).unwrap();
    let r = scan_file(&path, &Options::default());
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|x| x == "truncated: only first 64 MiB scanned"), "{:?}", r.reasons);
}
