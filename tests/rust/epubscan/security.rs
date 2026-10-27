//! 스크립트 실행 코드·위험 구조 검출 테스트. EPUB을 직접 만들어 규칙마다 확인한다.

mod common;

use std::fs;
use std::io::Write;
use std::path::Path;

use common::*;
use epubscan::security::{Limits, SecurityReport, Severity, Status, scan_epub};

fn scan(path: &Path) -> SecurityReport {
    scan_epub(path, &Limits::default())
}

fn codes(r: &SecurityReport) -> Vec<&str> {
    r.findings.iter().map(|f| f.code.as_str()).collect()
}

fn has(r: &SecurityReport, code: &str) -> bool {
    r.findings.iter().any(|f| f.code == code)
}

fn severity_of(r: &SecurityReport, code: &str) -> Severity {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", codes(r))).severity
}

/// 본문 한 장짜리 EPUB을 만들어 검사한다. OPF는 scripted 선언이 없는 기본형이다.
fn scan_chapter(body: &str) -> SecurityReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml(body));
    scan(&p)
}

fn scan_extra(name: &str, data: &[u8]) -> SecurityReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml("<p>본문</p>"), &[(name, data)]);
    scan(&p)
}

// ---- 기준선 ----

#[test]
fn clean_epub_has_no_findings() {
    let r = scan_chapter("<p>안녕하세요</p>");
    assert_eq!(r.status, Status::None, "{:?}", r.findings);
    assert!(r.findings.is_empty() && r.notes.is_empty());
    assert_eq!(r.entries, 4);
    assert!(r.inspected_bytes > 0);
}

// ---- HTML 활성 요소 ----

#[test]
fn script_tags_are_flagged_and_marked_as_undeclared_without_scripted_property() {
    let r = scan_chapter("<script>var a = 1;</script>");
    assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::High);
    assert!(has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
    assert_eq!(r.status, Status::Findings);
}

#[test]
fn declared_scripted_property_suppresses_the_undeclared_finding() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &scripted_opf(), &xhtml("<script>1</script>"), &[]);
    let r = scan(&p);
    assert!(has(&r, "SCRIPT_TAG") && !has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
}

#[test]
fn non_javascript_script_types_are_low() {
    let r = scan_chapter(r#"<script type="application/ld+json">{"a":1}</script>"#);
    assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::Low);
    assert!(!has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
}

#[test]
fn remote_script_is_high_and_reported_separately() {
    let r = scan_chapter(r#"<script src="https://evil.example/x.js"></script>"#);
    assert_eq!(severity_of(&r, "REMOTE_SCRIPT"), Severity::High);
}

#[test]
fn script_body_goes_through_the_javascript_rules() {
    let r = scan_chapter("<script>eval(atob('YQ=='))</script>");
    assert!(has(&r, "DYNAMIC_CODE_EXEC") && has(&r, "OBFUSCATION"));
}

#[test]
fn event_handler_attributes_are_high() {
    let r = scan_chapter(r#"<p onclick="alert(1)">x</p><img src="a.png" onerror=alert(1)>"#);
    assert_eq!(severity_of(&r, "EVENT_HANDLER_ATTR"), Severity::High);
    assert!(has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
}

#[test]
fn javascript_urls_are_found_even_when_obfuscated() {
    for href in ["javascript:alert(1)", "JaVaScRiPt:alert(1)", "jav&#x09;ascript:alert(1)", "  javascript:alert(1)", "java\nscript:alert(1)", "&#106;avascript:alert(1)", "vbscript:msgbox(1)"] {
        let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
        assert!(has(&r, "JS_URL"), "{href}: {:?}", codes(&r));
    }
}

#[test]
fn data_html_urls_and_dangerous_schemes_are_flagged() {
    let r = scan_chapter(r#"<a href="data:text/html;base64,PHNjcmlwdD4=">x</a>"#);
    assert!(has(&r, "JS_URL"));
    for href in ["file:///etc/passwd", "smb://host/share"] {
        let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
        assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::High, "{href}");
    }
    for href in ["ms-msdt:/id", "search-ms:query=x", "ms-appinstaller:?source=x", "MS-MSDT:/id"] {
        let r = scan_chapter(&format!(r#"<a href="{href}">x</a>"#));
        assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::Critical, "{href}");
    }
    let r = scan_chapter(r#"<a href="weird-app://open">x</a>"#);
    assert_eq!(severity_of(&r, "UNKNOWN_URI_SCHEME"), Severity::Low);
}

#[test]
fn ordinary_links_are_clean() {
    let r = scan_chapter(r##"<a href="#top">a</a><a href="ch2.xhtml">b</a><a href="mailto:a@b.c">c</a><a href="https://example.com/">d</a><a href="tel:123">e</a>"##);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn unc_paths_are_flagged() {
    let r = scan_chapter(r#"<img src="\\attacker\share\a.png">"#);
    assert_eq!(severity_of(&r, "UNC_PATH"), Severity::High);
}

#[test]
fn active_elements_are_flagged() {
    for (body, sev) in [
        ("<object data=\"a.swf\"></object>", Severity::High),
        ("<embed src=\"a.swf\">", Severity::High),
        ("<applet code=\"A\"></applet>", Severity::High),
        ("<iframe src=\"a.xhtml\"></iframe>", Severity::Medium),
        ("<iframe srcdoc=\"<b>\"></iframe>", Severity::High),
        ("<iframe src=\"https://x.example/\"></iframe>", Severity::High),
    ] {
        let r = scan_chapter(body);
        assert_eq!(severity_of(&r, "ACTIVE_ELEMENT"), sev, "{body}");
    }
}

#[test]
fn forms_meta_refresh_and_base_are_flagged() {
    let r = scan_chapter(r#"<form action="a.xhtml"><input name="x"></form>"#);
    assert_eq!(severity_of(&r, "FORM_ELEMENT"), Severity::Medium);
    assert!(has(&r, "UNDECLARED_SCRIPTED_CONTENT"));
    let r = scan_chapter(r#"<form action="https://evil.example/post"></form>"#);
    assert_eq!(severity_of(&r, "REMOTE_FORM_ACTION"), Severity::High);

    let r = scan_chapter(r#"<meta http-equiv="refresh" content="0;url=ch2.xhtml">"#);
    assert_eq!(severity_of(&r, "META_REFRESH"), Severity::Medium);
    let r = scan_chapter(r#"<meta http-equiv="Refresh" content="0; URL=https://evil.example/">"#);
    assert_eq!(severity_of(&r, "META_REFRESH"), Severity::High);

    let r = scan_chapter(r#"<base href="https://evil.example/">"#);
    assert_eq!(severity_of(&r, "BASE_HREF"), Severity::Medium);
}

#[test]
fn remote_resources_are_low_and_require_the_opf_declaration() {
    let r = scan_chapter(r#"<img src="https://tracker.example/p.gif"><link rel="stylesheet" href="http://x.example/a.css"><video src="https://x.example/v.mp4"></video><source src="//x.example/a">"#);
    assert_eq!(severity_of(&r, "REMOTE_RESOURCE"), Severity::Low);
    assert!(has(&r, "UNDECLARED_REMOTE_RESOURCES"));

    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &opf(&opf_item("c1", "ch1.xhtml", "application/xhtml+xml", "remote-resources")), &xhtml(r#"<img src="https://tracker.example/p.gif">"#), &[]);
    assert!(!has(&scan(&p), "UNDECLARED_REMOTE_RESOURCES"));
}

#[test]
fn svg_foreign_object_and_href_animation_are_flagged() {
    let r = scan_chapter(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><foreignObject><div/></foreignObject><a><set attributeName="href" to="javascript:alert(1)"/></a><animate attributeName="xlink:href" values="x"/></svg>"#,
    );
    assert_eq!(severity_of(&r, "SVG_FOREIGN_OBJECT"), Severity::Medium);
    assert_eq!(severity_of(&r, "SVG_ANIMATE_HREF"), Severity::Medium);
}

#[test]
fn comments_and_cdata_do_not_hide_or_fake_findings() {
    // 주석 안의 태그는 실행되지 않는다.
    let r = scan_chapter("<!-- <script>alert(1)</script> --><p>x</p>");
    assert!(!has(&r, "SCRIPT_TAG"));
    // 대소문자와 따옴표 없는 속성, 줄바꿈이 낀 태그도 잡는다.
    let r = scan_chapter("<SCRIPT\n type=text/javascript\n>alert(1)</SCRIPT>");
    assert!(has(&r, "SCRIPT_TAG"));
}

#[test]
fn style_blocks_and_attributes_use_the_css_rules() {
    let r = scan_chapter("<style>p { width: expression(alert(1)); }</style>");
    assert_eq!(severity_of(&r, "CSS_EXPRESSION"), Severity::High);
    let r = scan_chapter(r#"<p style="behavior:url(x.htc); -moz-binding:url(http://x/y.xml#z)">x</p>"#);
    assert!(has(&r, "CSS_BEHAVIOR") && has(&r, "CSS_MOZ_BINDING"));
}

#[test]
fn css_files_are_scanned() {
    let r =
        scan_extra("OEBPS/a.css", b"@import url(http://evil.example/a.css); p{background:url(javascript:alert(1))} q{background:url('https://x.example/i.png')} @import 'https://y.example/b.css';");
    assert!(has(&r, "REMOTE_CSS_IMPORT") && has(&r, "JS_URL") && has(&r, "REMOTE_RESOURCE"));
    let clean = scan_extra("OEBPS/a.css", b"p { color: red; background: url(img/a.png); }");
    assert!(clean.findings.is_empty(), "{:?}", clean.findings);
}

// ---- JavaScript 파일 ----

#[test]
fn javascript_members_are_flagged_and_scanned() {
    let r = scan_extra("OEBPS/a.js", b"eval('x'); new Function('return 1'); setTimeout('go()', 10); document.write('x'); var x = new XMLHttpRequest(); fetch('/a'); new WebSocket('ws://x'); navigator.sendBeacon('/b'); window.open('x'); location.href = 'y'; location.replace('z'); new ActiveXObject('WScript.Shell'); require('child_process'); localStorage.setItem('a','b'); document.cookie; importScripts('x'); import('x'); unescape('%41'); String.fromCharCode(1,2,3,4,5,6,7,8,9,10,11,12);");
    assert_eq!(severity_of(&r, "SCRIPT_FILE"), Severity::Medium);
    for code in ["DYNAMIC_CODE_EXEC", "DOCUMENT_WRITE", "NETWORK_API", "NAVIGATION", "HOST_API", "STORAGE_API", "DYNAMIC_IMPORT", "OBFUSCATION"] {
        assert!(has(&r, code), "{code}: {:?}", codes(&r));
    }
    let mjs = scan_extra("OEBPS/a.mjs", b"export const a = 1;");
    assert!(has(&mjs, "SCRIPT_FILE"));
}

#[test]
fn plain_function_names_do_not_trigger_dynamic_code_rules() {
    let r = scan_extra("OEBPS/a.js", b"function evaluate() { return 1; } const retrieval = 2;");
    assert_eq!(codes(&r), ["SCRIPT_FILE"]);
}

#[test]
fn long_hex_escape_runs_count_as_obfuscation() {
    let r = scan_extra("OEBPS/a.js", format!("var s = \"{}\";", "\\x41".repeat(40)).as_bytes());
    assert!(has(&r, "OBFUSCATION"));
}

// ---- ZIP 멤버 내용 ----

#[test]
fn executable_extensions_and_magic_bytes_are_flagged() {
    assert_eq!(severity_of(&scan_extra("OEBPS/x.exe", b"hello"), "EMBEDDED_EXECUTABLE"), Severity::High);
    assert!(has(&scan_extra("OEBPS/run.sh", b"echo"), "EMBEDDED_EXECUTABLE"));
    // 실행 파일 확장자의 PE·ELF·Mach-O는 번들된 설치 파일일 수 있어 High·Medium이고, 다른 확장자로 위장하면 Critical이다.
    assert_eq!(severity_of(&scan_extra("OEBPS/a.exe", b"MZ\x90\x00more"), "PE_EXECUTABLE_CONTENT"), Severity::High);
    assert_eq!(severity_of(&scan_extra("OEBPS/a.so", b"\x7fELF\x02\x01"), "ELF_BINARY_CONTENT"), Severity::Medium);
    assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"MZ\x90\x00more"), "PE_EXECUTABLE_CONTENT"), Severity::Critical);
    assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"\x7fELF\x02\x01"), "ELF_BINARY_CONTENT"), Severity::Critical);
    assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"\xcf\xfa\xed\xfe\x07"), "MACHO_BINARY_CONTENT"), Severity::Critical);
    assert_eq!(severity_of(&scan_extra("OEBPS/a.jpg", b"\xca\xfe\xba\xbe\x00\x00\x00\x34"), "JAVA_CLASS_CONTENT"), Severity::Medium);
    assert_eq!(severity_of(&scan_extra("OEBPS/a.txt", b"#!/bin/sh\nrm -rf /"), "SCRIPT_SHEBANG"), Severity::Medium);
    assert!(scan_extra("OEBPS/a.png", b"\x89PNG\r\n\x1a\nrest").findings.is_empty());
}

#[test]
fn nested_archives_are_low() {
    assert_eq!(severity_of(&scan_extra("OEBPS/inner.zip", b"PK\x03\x04rest"), "NESTED_ARCHIVE"), Severity::Low);
}

#[test]
fn high_compression_ratio_is_flagged_as_a_possible_bomb() {
    let r = scan_extra("OEBPS/zeros.bin", &vec![0u8; 3 * 1024 * 1024]);
    assert_eq!(severity_of(&r, "HIGH_COMPRESSION_RATIO"), Severity::Medium);
    // 작으면 비율이 높아도 무시한다.
    assert!(!has(&scan_extra("OEBPS/zeros.bin", &vec![0u8; 100 * 1024]), "HIGH_COMPRESSION_RATIO"));
}

// ---- ZIP 이름과 구조 ----

fn raw_zip(path: &Path, names: &[&str]) {
    let mut z = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", opts).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.start_file("META-INF/container.xml", opts).unwrap();
    z.write_all(CONTAINER_XML.as_bytes()).unwrap();
    z.start_file("OEBPS/content.opf", opts).unwrap();
    z.write_all(clean_opf().as_bytes()).unwrap();
    z.start_file("OEBPS/ch1.xhtml", opts).unwrap();
    z.write_all(xhtml("<p>x</p>").as_bytes()).unwrap();
    for n in names {
        z.start_file(*n, opts).unwrap();
        z.write_all(b"data").unwrap();
    }
    z.finish().unwrap();
}

fn scan_names(names: &[&str]) -> SecurityReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    raw_zip(&p, names);
    scan(&p)
}

#[test]
fn path_traversal_is_critical_and_absolute_names_are_high() {
    for n in ["../evil.txt", "OEBPS/../../evil.txt", "OEBPS/%2e%2e/evil.txt", "..\\evil.txt"] {
        assert_eq!(severity_of(&scan_names(&[n]), "ZIP_PATH_TRAVERSAL"), Severity::Critical, "{n}");
    }
    for n in ["/etc/passwd", "\\windows\\x", "C:/x.txt", "c:\\x.txt"] {
        assert_eq!(severity_of(&scan_names(&[n]), "ZIP_ABSOLUTE_PATH"), Severity::High, "{n}");
    }
}

#[test]
fn backslash_control_reserved_and_stream_names_are_flagged() {
    assert_eq!(severity_of(&scan_names(&["OEBPS\\a.txt"]), "ZIP_BACKSLASH_PATH"), Severity::Medium);
    assert_eq!(severity_of(&scan_names(&["OEBPS/a\u{1b}b.txt"]), "ZIP_CONTROL_CHAR_NAME"), Severity::Medium);
    for n in ["OEBPS/CON.txt", "OEBPS/nul", "OEBPS/Com1.html", "OEBPS/lpt9.x"] {
        assert!(has(&scan_names(&[n]), "WINDOWS_RESERVED_NAME"), "{n}");
    }
    assert!(!has(&scan_names(&["OEBPS/console.txt"]), "WINDOWS_RESERVED_NAME"));
    assert_eq!(severity_of(&scan_names(&["OEBPS/a.xhtml:evil.exe"]), "ZIP_ALT_DATA_STREAM"), Severity::Medium);
    assert_eq!(severity_of(&scan_names(&[&format!("OEBPS/{}.txt", "a".repeat(300))]), "ZIP_NAME_TOO_LONG"), Severity::Low);
}

#[test]
fn names_that_collide_on_windows_are_flagged() {
    let r = scan_names(&["OEBPS/Page.xhtml", "OEBPS/page.xhtml "]);
    assert_eq!(severity_of(&r, "WINDOWS_FILENAME_COLLISION"), Severity::Medium);
    let r = scan_names(&["OEBPS/a.txt", "OEBPS/A.TXT"]);
    assert!(has(&r, "WINDOWS_FILENAME_COLLISION"));
    assert!(!has(&scan_names(&["OEBPS/a.txt", "OEBPS/b.txt"]), "WINDOWS_FILENAME_COLLISION"));
}

#[test]
fn symlink_entries_are_flagged() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.add_symlink("OEBPS/link", "/etc/passwd", stored).unwrap();
    z.finish().unwrap();
    assert_eq!(severity_of(&scan(&p), "ZIP_SYMLINK"), Severity::Medium);
}

#[test]
fn damaged_local_headers_are_high() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml("<p>x</p>"));
    corrupt_local_header(&p, "OEBPS/ch1.xhtml");
    let r = scan(&p);
    assert_eq!(severity_of(&r, "BAD_LOCAL_HEADER"), Severity::High);

    let q = d.path().join("u.epub");
    epub_with_chapter(&q, &xhtml("<p>x</p>"));
    mangle_local_name(&q, "OEBPS/ch1.xhtml");
    assert_eq!(severity_of(&scan(&q), "ZIP_HEADER_NAME_MISMATCH"), Severity::High);
}

// ---- mimetype / container ----

#[test]
fn epub_basics_are_checked() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    {
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("OEBPS/a.xhtml", o).unwrap();
        z.write_all(b"<p/>").unwrap();
        z.finish().unwrap();
    }
    let r = scan(&p);
    assert!(has(&r, "MISSING_MIMETYPE") && has(&r, "MISSING_CONTAINER_XML"));

    let q = d.path().join("u.epub");
    {
        let mut z = zip::ZipWriter::new(fs::File::create(&q).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(CONTAINER_XML.as_bytes()).unwrap();
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"text/plain").unwrap();
        z.finish().unwrap();
    }
    let r = scan(&q);
    assert_eq!(severity_of(&r, "NONSTANDARD_MIMETYPE"), Severity::Low);
    assert_eq!(severity_of(&r, "BAD_MIMETYPE"), Severity::Medium);
}

// ---- XML / OPF ----

#[test]
fn xml_external_entities_stylesheets_and_xinclude_are_flagged() {
    let r = scan_extra("OEBPS/a.xml", br#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY x SYSTEM "file:///etc/passwd">]><r>&x;</r>"#);
    assert_eq!(severity_of(&r, "XXE_ENTITY"), Severity::Critical);
    let r = scan_extra("OEBPS/a.xml", br#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY x SYSTEM "http://evil.example/e">]><r>&x;</r>"#);
    assert_eq!(severity_of(&r, "XXE_ENTITY"), Severity::High);
    let r = scan_extra("OEBPS/a.xml", br#"<!DOCTYPE r SYSTEM "http://evil.example/a.dtd"><r/>"#);
    assert_eq!(severity_of(&r, "EXTERNAL_DTD"), Severity::Medium);
    let r = scan_extra("OEBPS/a.xml", br#"<!DOCTYPE r [<!ENTITY a "aaaa">]><r>&a;</r>"#);
    assert_eq!(severity_of(&r, "ENTITY_DECLARATION"), Severity::Medium);
    let r = scan_extra("OEBPS/a.xml", br#"<?xml-stylesheet type="text/xsl" href="http://evil.example/a.xsl"?><r/>"#);
    assert_eq!(severity_of(&r, "XML_STYLESHEET"), Severity::Medium);
    let r = scan_extra("OEBPS/a.xml", br#"<r xmlns:xi="http://www.w3.org/2001/XInclude"><xi:include href="/etc/passwd" parse="text"/></r>"#);
    assert_eq!(severity_of(&r, "XINCLUDE"), Severity::Medium);
    assert!(scan_extra("OEBPS/a.xml", b"<r><a>plain</a></r>").findings.is_empty());
}

#[test]
fn standard_public_doctypes_are_low_noise() {
    let r = scan_chapter("<p>x</p>");
    assert!(r.findings.is_empty());
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    let page = format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.1//EN\" \"http://www.w3.org/TR/xhtml11/DTD/xhtml11.dtd\">\n{}",
        xhtml("<p>x</p>").replace("<?xml version=\"1.0\"?>", "")
    );
    epub_with_files(&p, &clean_opf(), &page, &[]);
    let r = scan(&p);
    assert_eq!(severity_of(&r, "EXTERNAL_DTD"), Severity::Low);
}

#[test]
fn opf_manifest_is_checked_for_escapes_remote_items_and_script_media_types() {
    let items = format!(
        "{}{}{}{}{}",
        opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""),
        opf_item("e1", "../../outside.txt", "text/plain", ""),
        opf_item("r1", "https://x.example/a.png", "image/png", ""),
        opf_item("f1", "file:///etc/passwd", "text/plain", ""),
        opf_item("s1", "a.js", "application/javascript", "")
    );
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[("OEBPS/a.js", b"1")]);
    let r = scan(&p);
    assert_eq!(severity_of(&r, "MANIFEST_PATH_ESCAPE"), Severity::High);
    assert_eq!(severity_of(&r, "REMOTE_MANIFEST_ITEM"), Severity::Medium);
    assert_eq!(severity_of(&r, "DANGEROUS_URI_SCHEME"), Severity::High);
    assert_eq!(severity_of(&r, "SCRIPT_MEDIA_TYPE"), Severity::Medium);
}

#[test]
fn malformed_opf_is_noted_without_aborting() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, "<package", &xhtml("<p>x</p>"), &[]);
    let r = scan(&p);
    assert!(r.notes.iter().any(|n| n.contains("OEBPS/content.opf")), "{:?}", r.notes);
    assert_eq!(r.status, Status::Incomplete);
}

// ---- 한도와 불완전 ----

#[test]
fn limits_make_the_scan_incomplete_instead_of_failing() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/big.bin", &vec![7u8; 4096])]);
    let r = scan_epub(&p, &Limits { max_member: 1024, max_total: 200 * 1024 * 1024, max_files: 10_000 });
    assert_eq!(r.status, Status::Incomplete);
    assert!(r.notes.iter().any(|n| n.contains("OEBPS/big.bin") && n.contains("max inspection size")), "{:?}", r.notes);

    let r = scan_epub(&p, &Limits { max_member: 1 << 20, max_total: 100, max_files: 10_000 });
    assert!(r.notes.iter().any(|n| n.contains("Total decompressed-byte scan limit")), "{:?}", r.notes);

    let r = scan_epub(&p, &Limits { max_member: 1 << 20, max_total: 1 << 28, max_files: 2 });
    assert!(r.notes.iter().any(|n| n.contains("Too many entries")), "{:?}", r.notes);
    assert!(r.entries >= 5);
}

#[test]
fn unreadable_members_are_noted_when_the_data_is_corrupt() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml(&"<p>안녕하세요</p>".repeat(50)));
    // 압축 데이터 한가운데를 망가뜨려 CRC·해제 오류를 낸다.
    let mut data = fs::read(&p).unwrap();
    let marker = b"OEBPS/ch1.xhtml";
    let off = data.windows(marker.len()).position(|w| w == marker).unwrap() + marker.len() + 8;
    for b in &mut data[off..off + 16] {
        *b ^= 0xff;
    }
    fs::write(&p, data).unwrap();
    let r = scan(&p);
    assert!(r.notes.iter().any(|n| n.contains("Could not decompress")), "{:?} {:?}", r.notes, r.findings);
}

#[test]
fn non_zip_and_missing_files_are_incomplete() {
    let d = tempfile::tempdir().unwrap();
    let bad = d.path().join("bad.epub");
    fs::write(&bad, b"not a zip").unwrap();
    for p in [bad, d.path().join("none.epub")] {
        let r = scan(&p);
        assert_eq!(r.status, Status::Incomplete);
        assert!(r.notes[0].contains("Cannot open as ZIP EPUB"));
        assert_eq!(r.entries, 0);
    }
}

// ---- 정렬과 출력 안전 ----

#[test]
fn findings_are_sorted_by_severity_then_code_then_location() {
    let r = scan_chapter(r#"<script>eval(1)</script><img src="https://x.example/a.gif"><base href="x">"#);
    let sev: Vec<Severity> = r.findings.iter().map(|f| f.severity).collect();
    let mut sorted = sev.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(sev, sorted);
}

#[test]
fn report_strings_never_carry_control_characters() {
    let r = scan_names(&["OEBPS/a\u{1b}[2J\u{85}b.exe"]);
    for f in &r.findings {
        assert!(!f.location.chars().any(|c| c.is_control()) && !f.detail.chars().any(|c| c.is_control()), "{f:?}");
    }
    assert!(r.findings.iter().all(|f| f.location.chars().count() <= 160 && f.detail.chars().count() <= 160));
}

#[test]
fn report_serializes_with_python_compatible_status_labels() {
    let r = scan_chapter("<script>1</script>");
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["status"], "FINDINGS");
    assert_eq!(v["findings"][0]["severity"], "HIGH");
    assert!(v["findings"][0]["code"].is_string());
}

// ---- 파서 차이를 이용한 우회 ----

#[test]
fn declared_media_type_decides_which_scanner_runs() {
    let items = format!(
        "{}{}{}{}",
        opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""),
        opf_item("p1", "payload.dat", "application/xhtml+xml", ""),
        opf_item("p2", "pic.png", "text/javascript", ""),
        opf_item("p3", "style.txt", "text/css", "")
    );
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(
        &p,
        &opf(&items),
        &xhtml("<p>x</p>"),
        &[("OEBPS/payload.dat", b"<html><script>alert(1)</script></html>"), ("OEBPS/pic.png", b"eval(document.cookie)"), ("OEBPS/style.txt", b"p { width: expression(alert(1)); }")],
    );
    let r = scan(&p);
    for code in ["SCRIPT_TAG", "DYNAMIC_CODE_EXEC", "CSS_EXPRESSION"] {
        assert!(has(&r, code), "{code}: {:?}", codes(&r));
    }
    let mismatches: Vec<&str> = r.findings.iter().filter(|f| f.code == "MEDIA_TYPE_MISMATCH").map(|f| f.location.as_str()).collect();
    assert_eq!(severity_of(&r, "MEDIA_TYPE_MISMATCH"), Severity::Medium);
    assert_eq!(mismatches.len(), 3, "{mismatches:?}");
}

#[test]
fn compatible_declarations_are_not_mismatches() {
    let items = format!("{}{}{}", opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), opf_item("n1", "toc.ncx", "application/x-dtbncx+xml", ""), opf_item("i1", "a.png", "image/png", ""));
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &opf(&items), &xhtml("<p>x</p>"), &[("OEBPS/toc.ncx", b"<ncx/>"), ("OEBPS/a.png", b"\x89PNG\r\n\x1a\n")]);
    assert!(!has(&scan(&p), "MEDIA_TYPE_MISMATCH"));
}

#[test]
fn the_package_document_named_by_container_xml_is_found_without_an_opf_extension() {
    // rootfile이 .opf가 아닌 이름을 가리켜도 매니페스트를 읽어 선언 검사를 한다.
    let container = CONTAINER_XML.replace("OEBPS/content.opf", "OEBPS/package.xml");
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    {
        let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(container.as_bytes()).unwrap();
        z.start_file("OEBPS/package.xml", o).unwrap();
        z.write_all(scripted_opf().replace("scripted", "").as_bytes()).unwrap();
        z.start_file("OEBPS/ch1.xhtml", o).unwrap();
        z.write_all(xhtml("<script>1</script>").as_bytes()).unwrap();
        z.finish().unwrap();
    }
    assert!(has(&scan(&p), "UNDECLARED_SCRIPTED_CONTENT"));
}

#[test]
fn every_javascript_mime_type_and_unknown_types_count_as_executable() {
    for ty in [
        "text/jscript.encode",
        "TEXT/JAVASCRIPT",
        "text/javascript1.2",
        "application/x-ecmascript",
        "text/livescript",
        "text/x-javascript",
        "module",
        "application/javascript; charset=utf-8",
        "  text/javascript  ",
        "text/vbscript",
        "text/x-custom",
    ] {
        let r = scan_chapter(&format!(r#"<script type="{ty}">alert(1)</script>"#));
        assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::High, "{ty:?}");
    }
    for ty in ["application/json", "application/ld+json", "text/plain", "text/x-handlebars-template", "importmap", "speculationrules"] {
        let r = scan_chapter(&format!(r#"<script type="{ty}">{{"a":1}}</script>"#));
        assert_eq!(severity_of(&r, "SCRIPT_TAG"), Severity::Low, "{ty:?}");
    }
}

#[test]
fn markup_hidden_after_cdata_or_odd_comments_is_still_scanned() {
    for body in [
        "<![CDATA[ <script>alert(1)</script> ]]>",
        "<!--> <script>alert(1)</script>",
        "<!---> <script>alert(1)</script>",
        "<!-- x --!> <script>alert(1)</script>",
        "<p>x</p><!-- <script>alert(1)</script>",
    ] {
        let r = scan_chapter(body);
        assert!(has(&r, "SCRIPT_TAG"), "{body}: {:?}", codes(&r));
    }
    // 닫히지 않은 주석은 파서마다 해석이 달라 따로 알린다.
    let r = scan_chapter("<p>x</p><!-- never closed");
    assert_eq!(severity_of(&r, "MALFORMED_MARKUP"), Severity::Low);
    // 정상적으로 닫힌 주석은 여전히 숨긴다.
    assert!(!has(&scan_chapter("<!-- <script>alert(1)</script> --><p>x</p>"), "SCRIPT_TAG"));
    assert!(!has(&scan_chapter("<!-- a --><p>x</p>"), "MALFORMED_MARKUP"));
}

// ---- CRITICAL: 정상 문서에 있을 이유가 없는 수법 ----

#[test]
fn host_command_and_obfuscated_loader_are_critical_but_library_activex_is_not() {
    let r = scan_extra("OEBPS/a.js", b"var s = new ActiveXObject('WScript.Shell'); s.Run('calc');");
    assert_eq!(severity_of(&r, "HOST_COMMAND_EXEC"), Severity::Critical);
    let r = scan_extra("OEBPS/a.js", b"var cp = require('child_process');");
    assert_eq!(severity_of(&r, "HOST_COMMAND_EXEC"), Severity::Critical);
    for payload in ["eval(atob('YQ=='))", "eval(unescape('%61'))", "new Function(String.fromCharCode(97))()"] {
        let r = scan_extra("OEBPS/a.js", payload.as_bytes());
        assert_eq!(severity_of(&r, "OBFUSCATED_LOADER"), Severity::Critical, "{payload}");
    }
    // jQuery 1.x가 IE에서 XML을 파싱할 때 쓰는 ActiveXObject와 `new Function("return " + data)`는 정상이다.
    let jq = scan_extra("OEBPS/a.js", br#"xml = new ActiveXObject("Microsoft.XMLDOM"); return (new Function("return " + b))();"#);
    assert!(has(&jq, "DYNAMIC_CODE_EXEC") && !has(&jq, "HOST_API"), "{:?}", codes(&jq));
    assert!(!has(&jq, "HOST_COMMAND_EXEC") && !has(&jq, "OBFUSCATED_LOADER"), "{:?}", codes(&jq));
    assert!(jq.findings.iter().all(|f| f.severity != Severity::Critical));
}

#[test]
fn script_tag_without_malicious_pattern_never_reaches_critical() {
    let r = scan_chapter("<script>var n = eval('1+1'); document.getElementById('a').onclick = function () {};</script><button onclick=\"go()\">x</button>");
    assert!(r.findings.iter().all(|f| f.severity != Severity::Critical), "{:?}", r.findings);
}

#[test]
fn host_api_ignores_library_activex_fallbacks_and_feature_detection_but_keeps_the_rest() {
    let benign: [&[u8]; 5] = [
        br#"var x = window.ActiveXObject ? new ActiveXObject("Microsoft.XMLHTTP") : new XMLHttpRequest();"#,
        br#"xml = new window.ActiveXObject( 'Msxml2.XMLHTTP.6.0' );"#,
        br#"var d = new ActiveXObject("MSXML2.DOMDocument");"#,
        br#"if ("ActiveXObject" in window) { var ie = true; }"#,
        br#"var a = typeof ActiveXObject !== 'undefined';"#,
    ];
    for js in benign {
        let r = scan_extra("OEBPS/a.js", js);
        assert!(!has(&r, "HOST_API") && !has(&r, "HOST_COMMAND_EXEC"), "{}: {:?}", String::from_utf8_lossy(js), codes(&r));
    }
    // 모르는 ProgID, 동적 인자, 호스트 객체는 계속 High다.
    let risky: [&[u8]; 6] = [
        br#"new ActiveXObject("Some.Unknown.Control");"#,
        br#"new ActiveXObject(prog);"#,
        br#"new ActiveXObject ( "Microsoft.XMLDOM" + x );"#,
        b"WScript.Echo(1);",
        b"var fs = require('fs');",
        b"process.binding('natives');",
    ];
    for js in risky {
        let r = scan_extra("OEBPS/a.js", js);
        assert_eq!(severity_of(&r, "HOST_API"), Severity::High, "{}", String::from_utf8_lossy(js));
    }
    // 레거시 AJAX 라이브러리가 쓰는 MSXML2.XMLHTTP는 Critical이 아니고, 서버측 다운로더만 Critical이다.
    let r = scan_extra("OEBPS/a.js", br#"new ActiveXObject("MSXML2.ServerXMLHTTP");"#);
    assert_eq!(severity_of(&r, "HOST_COMMAND_EXEC"), Severity::Critical);
}

#[test]
fn activex_aliasing_and_comment_tricks_do_not_hide_host_calls() {
    let evasions: [&[u8]; 5] = [
        br#"var A = window.ActiveXObject; var s = new A("WScript.Shell");"#,
        br#"var A = ActiveXObject; new A(prog);"#,
        br#"var A = window["ActiveXObject"]; new A(prog);"#,
        br#"new ActiveXObject/* x */("Some.Unknown.Control");"#,
        br#"var s = new (window.ActiveXObject)('Scripting.FileSystemObject');"#,
    ];
    for js in evasions {
        let r = scan_extra("OEBPS/a.js", js);
        assert!(has(&r, "HOST_API") || has(&r, "HOST_COMMAND_EXEC"), "{}: {:?}", String::from_utf8_lossy(js), codes(&r));
    }
    // 위험 ProgID는 생성자 모양과 무관하게 Critical이다.
    for js in [&br#"var A = window.ActiveXObject; new A("WScript.Shell");"#[..], &br#"new (window.ActiveXObject)('Scripting.FileSystemObject');"#[..]] {
        assert_eq!(severity_of(&scan_extra("OEBPS/a.js", js), "HOST_COMMAND_EXEC"), Severity::Critical, "{}", String::from_utf8_lossy(js));
    }
}
