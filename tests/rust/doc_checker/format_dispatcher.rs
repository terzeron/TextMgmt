//! `doc_checker::format_dispatcher` 단위 테스트.

mod support;
use container_parser::magic_sniffer::Sniffed;
use doc_checker::CheckOptions;
use doc_checker::format_dispatcher::{dispatch_format, scan_file};
use doc_checker::{Category, FileReport, Format, Severity};
use std::path::Path;

#[test]
fn dispatch_follows_content_for_pdf_that_is_a_zip() {
    assert_eq!(dispatch_format(Path::new("a.pdf"), Sniffed::Zip), Some(Format::Epub));
}

#[test]
fn dispatch_follows_extension_otherwise() {
    assert_eq!(dispatch_format(Path::new("a.txt"), Sniffed::Unknown), Some(Format::Txt));
    assert_eq!(dispatch_format(Path::new("a.epub"), Sniffed::Unknown), Some(Format::Epub));
    assert_eq!(dispatch_format(Path::new("a.dat"), Sniffed::Pdf), None);
}

fn epub_with_script(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    support::epub_with_chapter(&p, &support::xhtml(&format!("<p>본문</p><script>{script}</script>")));
    (d, p)
}

#[test]
fn scan_file_adds_combined_suspicious_for_obfuscation_plus_network() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    let r = scan_file(&p, &CheckOptions::default());
    let f = r.findings.iter().find(|f| f.code == "COMBINED_SUSPICIOUS").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert_eq!(f.detail, "OBFUSCATION + NETWORK_API");
}

#[test]
fn scan_file_does_not_escalate_a_single_medium_signal() {
    let (_d, p) = epub_with_script("var x = atob('aGk=');");
    assert!(!scan_file(&p, &CheckOptions::default()).has("COMBINED_SUSPICIOUS"));
}

#[test]
fn scan_file_fills_risk_score_and_a_clean_file_stays_below_high() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    assert!(scan_file(&p, &CheckOptions::default()).risk_score >= 70);
    let d = tempfile::tempdir().unwrap();
    let clean = d.path().join("ok.epub");
    support::make_epub(&clean);
    // make_epub의 기본형은 LOW 지표(NAV_MISSING)만 낸다. LOW는 10점이다.
    let r = scan_file(&clean, &CheckOptions::default());
    assert!(r.findings.iter().all(|f| f.severity == Severity::Low), "{:?}", r.findings);
    assert_eq!(r.risk_score, if r.findings.is_empty() { 0 } else { 10 });
}

#[test]
fn risk_score_keeps_counting_findings_hidden_by_the_output_filter() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    let mut r = scan_file(&p, &CheckOptions::default());
    let before = r.risk_score;
    r.suppress_below(Severity::Critical);
    assert!(r.findings.is_empty());
    assert_eq!(r.risk_score, before);
}

// --- zip · docx · hwpx 디스패치 ---

#[test]
fn dispatch_routes_zip_docx_and_hwpx_by_extension() {
    assert_eq!(dispatch_format(Path::new("a.zip"), Sniffed::Zip), Some(Format::Zip));
    assert_eq!(dispatch_format(Path::new("a.zip"), Sniffed::Unknown), Some(Format::Zip));
    for name in ["a.docx", "a.DOCM", "a.dotx", "a.dotm"] {
        assert_eq!(dispatch_format(Path::new(name), Sniffed::Zip), Some(Format::Docx), "{name}");
        assert_eq!(dispatch_format(Path::new(name), Sniffed::Unknown), Some(Format::Docx), "{name}");
    }
    assert_eq!(dispatch_format(Path::new("a.hwpx"), Sniffed::Zip), Some(Format::Hwpx));
}

#[test]
fn dispatch_follows_pdf_content_for_zip_family_extensions_and_keeps_epub() {
    for name in ["a.zip", "a.docx", "a.hwpx"] {
        assert_eq!(dispatch_format(Path::new(name), Sniffed::Pdf), Some(Format::Pdf), "{name}");
    }
    assert_eq!(dispatch_format(Path::new("a.epub"), Sniffed::Zip), Some(Format::Epub));
    assert_eq!(dispatch_format(Path::new("a.docs"), Sniffed::Zip), None);
}

/// `MZ` 머리, e_lfanew=`offset`, 그 자리의 `PE\0\0`. 파일 길이는 `len`이다.
fn pe_bytes(offset: u32, len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&offset.to_le_bytes());
    let at = offset as usize;
    b[at..at + 4].copy_from_slice(b"PE\0\0");
    b
}

fn min_pe() -> Vec<u8> {
    pe_bytes(0x40, 0x100)
}

fn elf() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(b"\x7fELF");
    b
}

fn macho() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(&[0xCF, 0xFA, 0xED, 0xFE]);
    b
}

fn scan_named(name: &str, bytes: &[u8]) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    std::fs::write(&p, bytes).unwrap();
    scan_file(&p, &CheckOptions::default())
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a doc_checker::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing in {:?}", r.findings))
}

#[test]
fn exe_with_pe_header_is_confirmed() {
    let r = scan_named("a.exe", &min_pe());
    assert_eq!(r.format, Some(Format::Executable));
    let f = finding(&r, "EXECUTABLE_FILE");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(f.detail.contains("PE"), "{}", f.detail);
    assert!(!r.has("EXECUTABLE_EXTENSION_NO_MAGIC"));
}

#[test]
fn elf_so_is_confirmed() {
    let r = scan_named("libx.so", &elf());
    assert!(finding(&r, "EXECUTABLE_FILE").detail.contains("ELF"));
}

#[test]
fn macho_dylib_is_confirmed() {
    let r = scan_named("libx.dylib", &macho());
    assert!(finding(&r, "EXECUTABLE_FILE").detail.contains("Mach-O"));
}

#[test]
fn exe_extension_with_text_content_is_not_confirmed() {
    let r = scan_named("a.exe", "그냥 글이다\n".repeat(20).as_bytes());
    assert_eq!(r.format, Some(Format::Executable));
    assert!(!r.has("EXECUTABLE_FILE"));
    let f = finding(&r, "EXECUTABLE_EXTENSION_NO_MAGIC");
    assert_eq!((f.category, f.severity), (Category::Defect, Severity::Low));
}

#[test]
fn mz_without_pe_header_is_not_confirmed() {
    let mut b = b"MZ this is only text that starts with MZ".to_vec();
    b.resize(0x100, b' ');
    let r = scan_named("a.exe", &b);
    assert!(!r.has("EXECUTABLE_FILE") && r.has("EXECUTABLE_EXTENSION_NO_MAGIC"));
}

#[test]
fn pe_with_e_lfanew_beyond_the_head_window_is_confirmed() {
    let r = scan_named("big.dll", &pe_bytes(70_000, 70_100));
    assert!(r.has("EXECUTABLE_FILE"), "{:?}", r.findings);
}

#[test]
fn extension_match_is_case_insensitive() {
    assert!(scan_named("A.EXE", &min_pe()).has("EXECUTABLE_FILE"));
}

#[test]
fn any_executable_magic_counts_for_any_executable_extension() {
    // 확장자가 .so인데 PE 내용이어도 실행 파일은 맞다(종류는 상세에 그대로 적는다).
    let r = scan_named("odd.so", &min_pe());
    assert!(finding(&r, "EXECUTABLE_FILE").detail.contains("PE"));
}

#[test]
fn dispatch_sends_executable_extensions_to_the_executable_scanner() {
    assert_eq!(dispatch_format(Path::new("a.exe"), Sniffed::Pe), Some(Format::Executable));
    assert_eq!(dispatch_format(Path::new("a.so"), Sniffed::Unknown), Some(Format::Executable));
    // 기존 포맷은 바뀌지 않는다.
    assert_eq!(dispatch_format(Path::new("a.txt"), Sniffed::Pe), Some(Format::Txt));
    assert_eq!(dispatch_format(Path::new("a.pdf"), Sniffed::Pdf), Some(Format::Pdf));
}

#[test]
fn txt_with_executable_content_still_reports_disguised_executable() {
    let r = scan_named("note.txt", &min_pe());
    let f = finding(&r, "DISGUISED_EXECUTABLE");
    assert_eq!(f.severity, Severity::Critical);
    assert!(!r.has("EXECUTABLE_FILE"));
}

// --- CLI ---

#[test]
fn confirmed_executable_flag_is_set_only_for_confirmed_files() {
    assert!(scan_named("a.exe", &min_pe()).confirmed_executable);
    assert!(!scan_named("a.exe", b"plain text\n".repeat(30).as_slice()).confirmed_executable);
    assert!(!scan_named("a.txt", &min_pe()).confirmed_executable);
}

// --- 최종 리뷰 반영 ---

#[test]
fn executable_with_a_pdf_marker_in_the_head_is_still_confirmed() {
    let mut b = pe_bytes(0x200, 0x300);
    b[0x80..0x85].copy_from_slice(b"%PDF-");
    assert!(scan_named("evil.exe", &b).has("EXECUTABLE_FILE"));
}

#[test]
fn far_pe_with_a_pdf_marker_in_the_head_is_still_confirmed() {
    let mut b = pe_bytes(70_000, 70_100);
    b[0x80..0x85].copy_from_slice(b"%PDF-");
    assert!(scan_named("evil.dll", &b).has("EXECUTABLE_FILE"));
}

#[test]
fn txt_with_pe_and_a_pdf_marker_still_reports_disguised_executable() {
    let mut b = pe_bytes(0x200, 0x300);
    b[0x80..0x85].copy_from_slice(b"%PDF-");
    assert!(scan_named("evil.txt", &b).has("DISGUISED_EXECUTABLE"));
}

#[test]
fn data_like_executable_extension_does_not_get_zero_tail_or_sparse_findings() {
    let mut b: Vec<u8> = (0..8192u32).map(|i| (i % 251) as u8 + 1).collect();
    b.extend(vec![0u8; 8192]);
    let r = scan_named("firmware.bin", &b);
    assert!(!r.has("ZERO_TAIL") && !r.has("SPARSE_FILE"), "{:?}", r.findings);
    assert!(r.has("EXECUTABLE_EXTENSION_NO_MAGIC"));
}

#[test]
fn executable_over_the_size_limit_is_still_confirmed_and_not_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("big.exe");
    let mut b = pe_bytes(0x40, 0x100);
    b.resize(2 * 1024 * 1024, 0xAA);
    std::fs::write(&p, &b).unwrap();
    let r = scan_file(&p, &CheckOptions { max_total_mb: 1, ..CheckOptions::default() });
    assert!(r.has("EXECUTABLE_FILE"), "{:?} {:?}", r.findings, r.reasons);
    assert!(!r.incomplete);
}

#[test]
fn non_executable_files_keep_the_size_limit_and_zero_tail_checks() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("big.txt");
    std::fs::write(&p, vec![b'a'; 2 * 1024 * 1024]).unwrap();
    assert!(scan_file(&p, &CheckOptions { max_total_mb: 1, ..CheckOptions::default() }).incomplete);
}

mod html_scanner {
    use crate::support::shared_fixtures::scan;
    use doc_checker::Format;

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
        for head in [
            "<!-- saved from url=(0030)http://example.com/ -->\n<html><body>hi</body></html>",
            "<meta charset=\"utf-8\"><p>fragment only</p>",
            "\u{feff}  <p>bom and spaces</p>",
            "plain text with no tags at all",
        ] {
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
        use container_parser::magic_sniffer::Sniffed;
        use doc_checker::FileReport;
        use doc_checker::format_dispatcher::scan_html_file;
        let d = tempfile::tempdir().unwrap();
        let mut r = FileReport::new(&d.path().join("none.html"), None);
        scan_html_file(&d.path().join("none.html"), Sniffed::Html, &mut r);
        assert!(r.reasons.iter().any(|m| m == "HTML file could not be opened"), "{:?}", r.reasons);
        // 디렉터리는 열리지만 읽지 못한다.
        let mut r = FileReport::new(d.path(), None);
        scan_html_file(d.path(), Sniffed::Html, &mut r);
        assert!(r.reasons.iter().any(|m| m == "HTML file could not be read"), "{:?}", r.reasons);
    }
}
