//! 분기 커버리지를 채우는 테스트: 토크나이저 가장자리, 엔티티, 손상된 ZIP 필드, CLI 오류 경로.

mod common;

use std::fs;
use std::io::Write;
use std::path::Path;

use common::*;
use bookdoctor::epub::security::{Limits, scan as scan_into};
use scan_common::{FileReport, Format, Severity};

fn scan(path: &Path) -> FileReport {
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan_into(path, &Limits::default(), &mut r);
    r
}

fn scan_chapter(body: &str) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml(body));
    scan(&p)
}

fn has(r: &FileReport, code: &str) -> bool {
    r.findings.iter().any(|f| f.code == code)
}

// ---- 토크나이저 가장자리 ----

#[test]
fn truncated_and_odd_markup_never_panics_and_still_finds_what_is_there() {
    for body in [
        "<a href=\"x",
        "<p class=a/>",
        "<img src=x onerror=1",
        "<",
        "<1>",
        "x < y",
        "</",
        "<a",
        "<!doctype",
        "<?xml",
        "<p =x>",
        "<p a= >",
        "<p a='b>",
        "<p a=b c>",
        "&",
        "&#",
        "&#x",
        "&unknown; &#xZZ; &#99999999;",
    ] {
        let _ = scan_chapter(body);
    }
    // 따옴표 없는 속성값과 값 없는 속성이 섞여도 핸들러를 잡는다.
    assert!(has(&scan_chapter("<p hidden onclick=alert(1)>x</p>"), "EVENT_HANDLER_ATTR"));
    assert!(has(&scan_chapter("<script></script"), "SCRIPT_TAG"));
    assert!(has(&scan_chapter("<script>alert(1)"), "SCRIPT_TAG"));
}

#[test]
fn named_and_numeric_entities_are_decoded_before_scheme_checks() {
    for href in ["javascript&colon;alert(1)", "java&Tab;script:alert(1)", "java&NewLine;script:alert(1)", "javascript&#58;alert(1)", "&#x6A;avascript:alert(1)", "&bsol;&bsol;host\\share"] {
        let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
        assert!(has(&r, "JS_URL") || has(&r, "UNC_PATH") || has(&r, "UNKNOWN_URI_SCHEME"), "{href}");
    }
    // 괄호 엔티티는 스킴으로 이어지지 않는다(`javascript(1):x`는 JS URL이 아니다).
    assert!(!has(&scan_chapter(r#"<a href="javascript&lpar;1&rpar;:x">x</a>"#), "JS_URL"));
    // 그 밖의 이름 있는 엔티티는 그대로 두되 스킴 판정에는 영향이 없다.
    for ent in ["amp", "lt", "gt", "quot", "apos", "sol", "nbsp"] {
        let r = scan_chapter(&format!(r#"<a href="a&{ent};b">x</a>"#));
        assert!(!has(&r, "JS_URL"), "{ent}");
    }
}

#[test]
fn script_src_with_a_local_or_dangerous_url_is_classified() {
    assert!(has(&scan_chapter(r#"<script src="javascript:alert(1)"></script>"#), "JS_URL"));
    assert!(!has(&scan_chapter(r#"<script src="a.js"></script>"#), "REMOTE_SCRIPT"));
}

#[test]
fn svg_animation_values_with_script_urls_are_flagged() {
    let r = scan_chapter(r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="href" values="javascript:alert(1)"/><set attributeName="xlink:href" from="vbscript:x" to="ok"/></svg>"#);
    assert!(has(&r, "JS_URL") && has(&r, "SVG_ANIMATE_HREF"));
    // href가 아닌 속성을 바꾸는 애니메이션은 무시한다.
    assert!(!has(&scan_chapter(r#"<svg xmlns="http://www.w3.org/2000/svg"><animate attributeName="opacity" values="0;1"/></svg>"#), "SVG_ANIMATE_HREF"));
}

#[test]
fn meta_without_refresh_and_base_without_href_are_ignored() {
    let r = scan_chapter(r#"<meta name="viewport" content="x"><base target="_blank"><meta http-equiv="refresh" content="5">"#);
    assert!(!has(&r, "BASE_HREF"));
    assert_eq!(r.findings.iter().find(|f| f.code == "META_REFRESH").map(|f| f.severity), Some(Severity::Medium));
}

// ---- 매니페스트와 매직 ----

#[test]
fn manifest_items_with_script_unc_and_unknown_schemes_are_flagged() {
    let items = [
        opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""),
        opf_item("a", "javascript:alert(1)", "text/plain", ""),
        opf_item("b", "\\\\host\\share\\x", "text/plain", ""),
        opf_item("c", "weird-app://x", "text/plain", ""),
    ]
    .concat();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[]);
    let r = scan(&p);
    for code in ["JS_URL", "UNC_PATH", "UNKNOWN_URI_SCHEME"] {
        assert!(has(&r, code), "{code}");
    }
}

#[test]
fn mach_o_universal_binaries_are_told_apart_from_java_classes() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/a.bin2", b"\xca\xfe\xba\xbe\x00\x00\x00\x02rest")]);
    assert!(has(&scan(&p), "MACHO_BINARY_CONTENT"));
}

#[test]
fn directory_entries_and_very_long_names_are_handled() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    {
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.add_directory("OEBPS/", o).unwrap();
        let long = format!("OEBPS/{}", ["dd"; 600].join("/"));
        z.start_file(format!("{long}/f.txt"), o).unwrap();
        z.write_all(b"x").unwrap();
        z.finish().unwrap();
    }
    let r = scan(&p);
    assert!(has(&r, "ZIP_NAME_TOO_LONG"), "{:?}", r.findings);
}

// ---- 손상된 ZIP 필드 ----

/// 중앙 디렉토리 항목의 오프셋 필드를 고친다. `field`는 항목 시작 기준 오프셋이다.
fn patch_central(path: &Path, member: &str, field: usize, bytes: &[u8]) {
    let mut data = fs::read(path).unwrap();
    let name = member.as_bytes();
    let at = (0..data.len() - 46 - name.len()).find(|&i| &data[i..i + 4] == b"PK\x01\x02" && &data[i + 46..i + 46 + name.len()] == name).expect("central header");
    data[at + field..at + field + bytes.len()].copy_from_slice(bytes);
    fs::write(path, data).unwrap();
}

#[test]
fn encrypted_and_unsupported_compression_entries_are_noted_not_read() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml("<p>x</p>"));
    patch_central(&p, "OEBPS/ch1.xhtml", 8, &[0x01, 0x00]); // 일반 목적 비트 0: 암호화
    let r = scan(&p);
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|n| n.contains("ZIP-encrypted")), "{:?}", r.reasons);

    let q = d.path().join("u.epub");
    epub_with_chapter(&q, &xhtml("<p>x</p>"));
    patch_central(&q, "OEBPS/ch1.xhtml", 10, &[12, 0]); // bzip2
    let r = scan(&q);
    assert!(r.reasons.iter().any(|n| n.contains("unsupported ZIP compression")), "{:?}", r.reasons);
}
