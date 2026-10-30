//! zipnames 테스트: ZIP 멤버 이름 규칙. EPUB 보안 검사가 쓰던 규칙과 같아야 한다(`epub_security.rs`의 기존 단언이 기준이다).

use std::collections::BTreeSet;

use bookdoctor::zipnames::check;
use scan_common::Severity;
use scan_common::Severity::{Critical, High, Low, Medium};

/// 같은 (코드, 심각도)가 두 번 나와도 하나로 본다. 호출자(`Scanner::add`)가 (코드, 위치)로 합친다.
fn got(name: &str) -> BTreeSet<(&'static str, Severity)> {
    check(name).into_iter().map(|f| (f.code, f.severity)).collect()
}

fn want(items: &[(&'static str, Severity)]) -> BTreeSet<(&'static str, Severity)> {
    items.iter().copied().collect()
}

#[test]
fn clean_names_have_no_findings() {
    for n in ["OEBPS/ch1.xhtml", "word/document.xml", "a.txt", "dir/sub/file.bin", "OEBPS/console.txt"] {
        assert!(check(n).is_empty(), "{n}: {:?}", got(n));
    }
}

#[test]
fn traversal_is_critical_including_encoded_and_backslash_forms() {
    assert_eq!(got("../evil.txt"), want(&[("ZIP_PATH_TRAVERSAL", Critical)]));
    assert_eq!(got("OEBPS/../../evil.txt"), want(&[("ZIP_PATH_TRAVERSAL", Critical)]));
    assert_eq!(got("OEBPS/%2e%2e/evil.txt"), want(&[("ZIP_PATH_TRAVERSAL", Critical)]));
    assert_eq!(got("..\\evil.txt"), want(&[("ZIP_PATH_TRAVERSAL", Critical), ("ZIP_BACKSLASH_PATH", Medium)]));
}

#[test]
fn absolute_paths_and_drive_letters_are_high() {
    assert_eq!(got("/etc/passwd"), want(&[("ZIP_ABSOLUTE_PATH", High)]));
    assert_eq!(got("C:/x.txt"), want(&[("ZIP_ABSOLUTE_PATH", High)]));
    assert_eq!(got("\\windows\\x"), want(&[("ZIP_ABSOLUTE_PATH", High), ("ZIP_BACKSLASH_PATH", Medium)]));
    assert_eq!(got("c:\\x.txt"), want(&[("ZIP_ABSOLUTE_PATH", High), ("ZIP_BACKSLASH_PATH", Medium)]));
}

#[test]
fn backslash_control_reserved_and_stream_names_are_medium() {
    assert_eq!(got("OEBPS\\a.txt"), want(&[("ZIP_BACKSLASH_PATH", Medium)]));
    assert_eq!(got("OEBPS/a\u{1b}b.txt"), want(&[("ZIP_CONTROL_CHAR_NAME", Medium)]));
    for n in ["OEBPS/CON.txt", "OEBPS/nul", "OEBPS/Com1.html", "OEBPS/lpt9.x"] {
        assert_eq!(got(n), want(&[("WINDOWS_RESERVED_NAME", Medium)]), "{n}");
    }
    assert_eq!(got("OEBPS/a.xhtml:evil.exe"), want(&[("ZIP_ALT_DATA_STREAM", Medium)]));
}

#[test]
fn long_names_are_low() {
    assert_eq!(got(&format!("OEBPS/{}.txt", "a".repeat(300))), want(&[("ZIP_NAME_TOO_LONG", Low)]));
    assert_eq!(got(&format!("OEBPS/{}.txt", "a".repeat(1_025))), want(&[("ZIP_NAME_TOO_LONG", Low)]));
}

#[test]
fn details_are_the_fixed_english_phrases_the_epub_reports_used() {
    let d = |n: &str, code: &str| check(n).into_iter().find(|f| f.code == code).unwrap().detail;
    assert_eq!(d("../x", "ZIP_PATH_TRAVERSAL"), "Entry name climbs out of the archive");
    assert_eq!(d("/x", "ZIP_ABSOLUTE_PATH"), "Absolute path or drive letter");
    assert_eq!(d("a\\b", "ZIP_BACKSLASH_PATH"), "Backslash in entry name");
    assert_eq!(d("a\u{1}b", "ZIP_CONTROL_CHAR_NAME"), "Control character in entry name");
    assert_eq!(d("con", "WINDOWS_RESERVED_NAME"), "Windows device name");
    assert_eq!(d("dir/a:b", "ZIP_ALT_DATA_STREAM"), "Colon in name (NTFS stream)");
}
