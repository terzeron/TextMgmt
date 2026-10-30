//! 디스패치와 파일 단위 검사 테스트.

mod common;

use std::path::Path;

use bookdoctor::cli::run;
use bookdoctor::scan::dispatch_format;
use scan_common::Format;
use scan_common::sniff::Sniffed;

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

#[test]
fn missing_path_is_incomplete_exit_2() {
    let d = tempfile::tempdir().unwrap();
    let missing = d.path().join("none.epub");
    let mut out = Vec::new();
    assert_eq!(run(&[missing.to_string_lossy().into_owned()], &mut out), 2);
    assert!(String::from_utf8(out).unwrap().contains("unreadable"));
}

#[cfg(unix)]
#[test]
fn unreadable_subdir_is_reported_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    // root는 권한 검사를 우회하므로 이 시나리오를 만들 수 없다.
    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let sub = d.path().join("locked");
    std::fs::create_dir(&sub).unwrap();
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    let mut out = Vec::new();
    let code = run(&["-r".into(), d.path().to_string_lossy().into_owned()], &mut out);
    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code, 2);
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

// --- 위험 점수와 조합 승격 ---

use bookdoctor::cli::Options;
use bookdoctor::scan::scan_file;
use scan_common::{Category, Severity};

fn epub_with_script(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    common::epub_with_chapter(&p, &common::xhtml(&format!("<p>본문</p><script>{script}</script>")));
    (d, p)
}

#[test]
fn scan_file_adds_combined_suspicious_for_obfuscation_plus_network() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    let r = scan_file(&p, &Options::default());
    let f = r.findings.iter().find(|f| f.code == "COMBINED_SUSPICIOUS").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert_eq!(f.detail, "OBFUSCATION + NETWORK_API");
}

#[test]
fn scan_file_does_not_escalate_a_single_medium_signal() {
    let (_d, p) = epub_with_script("var x = atob('aGk=');");
    assert!(!scan_file(&p, &Options::default()).has("COMBINED_SUSPICIOUS"));
}

#[test]
fn scan_file_fills_risk_score_and_a_clean_file_stays_below_high() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    assert!(scan_file(&p, &Options::default()).risk_score >= 70);
    let d = tempfile::tempdir().unwrap();
    let clean = d.path().join("ok.epub");
    common::make_epub(&clean);
    // make_epub의 기본형은 LOW 지표(NAV_MISSING)만 낸다. LOW는 10점이다.
    let r = scan_file(&clean, &Options::default());
    assert!(r.findings.iter().all(|f| f.severity == Severity::Low), "{:?}", r.findings);
    assert_eq!(r.risk_score, if r.findings.is_empty() { 0 } else { 10 });
}

#[test]
fn risk_score_keeps_counting_findings_hidden_by_the_output_filter() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    let mut r = scan_file(&p, &Options::default());
    let before = r.risk_score;
    r.suppress_below(Severity::Critical);
    assert!(r.findings.is_empty());
    assert_eq!(r.risk_score, before);
}

#[test]
fn risk_score_is_computed_before_the_only_filter() {
    let (_d, p) = epub_with_script("var x = atob('aGk='); fetch('/x');");
    let full = scan_file(&p, &Options::default()).risk_score;
    let only_render = scan_file(&p, &Options { only: vec![Category::Render], ..Options::default() });
    assert!(only_render.findings.iter().all(|f| f.category == Category::Render));
    assert_eq!(only_render.risk_score, full);
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

#[test]
fn directory_walk_collects_zip_docx_hwpx_but_not_lookalike_extensions() {
    let d = tempfile::tempdir().unwrap();
    for name in ["a.zip", "b.docx", "c.hwpx", "d.docs", "e.zipx"] {
        std::fs::write(d.path().join(name), b"PK\x03\x04 not a real archive").unwrap();
    }
    let mut out = Vec::new();
    run(&["-r".into(), "--min-severity".into(), "low".into(), "--color".into(), "never".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    for hit in ["a.zip", "b.docx", "c.hwpx"] {
        assert!(out.contains(hit), "{hit}: {out}");
    }
    for miss in ["d.docs", "e.zipx"] {
        assert!(!out.contains(miss), "{miss}: {out}");
    }
}
