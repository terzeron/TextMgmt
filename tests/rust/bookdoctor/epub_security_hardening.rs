//! EPUB 보안 검사 하드닝(Task 9b) 테스트. 자동 보안 리뷰가 찾은 네 빈틈을 닫는다.
//! 기존 규칙의 결과는 그대로이고 검출·상한만 늘어난다. 기존 테스트는 epub_security.rs, epub_gaps.rs에 있다.

mod common;

use std::fs;
use std::path::Path;

use bookdoctor::epub::security::{Limits, Stats, scan as scan_into, scan_stats};
use common::*;
use scan_common::{FileReport, Format, Severity};

fn scan_with(path: &Path, limits: &Limits) -> FileReport {
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan_into(path, limits, &mut r);
    r
}

fn scan_with_stats(path: &Path, limits: &Limits) -> (FileReport, Stats) {
    let mut r = FileReport::new(path, Some(Format::Epub));
    let stats = scan_stats(path, limits, &mut r);
    (r, stats)
}

fn codes(r: &FileReport) -> Vec<&str> {
    r.findings.iter().map(|f| f.code.as_str()).collect()
}

fn has(r: &FileReport, code: &str) -> bool {
    r.findings.iter().any(|f| f.code == code)
}

fn sev(r: &FileReport, code: &str) -> Severity {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} 없음: {:?}", codes(r))).severity
}

fn scan_chapter(body: &str) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_chapter(&p, &xhtml(body));
    scan_with(&p, &Limits::default())
}

fn scan_extra(name: &str, data: &[u8]) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml("<p>본문</p>"), &[(name, data)]);
    scan_with(&p, &Limits::default())
}

// ---- 1. 읽기 상한은 선언 크기를 믿지 않는다 ----

const REMOTE_ITEM: &str = r#"<item id="r" href="http://evil.example/a.css" media-type="text/css"/>"#;

fn padded_opf() -> String {
    // 매니페스트 앞쪽을 큰 주석으로 부풀리고, 원격 항목을 맨 뒤에 둔다.
    opf(&format!("<!--{}-->{}{}", "x".repeat(6000), opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), REMOTE_ITEM))
}

#[test]
fn opf_prepass_reads_are_charged_to_the_total_budget() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &padded_opf(), &xhtml("<p>x</p>"), &[]);
    // 예산(3000)보다 큰 OPF를 선검사가 통째로 읽으면 예산 밖에서 항목이 검사된다.
    let (r, stats) = scan_with_stats(&p, &Limits { max_member: 1 << 20, max_total: 3000, max_files: 10_000 });
    assert!(!has(&r, "REMOTE_MANIFEST_ITEM"), "예산을 넘겨 읽었다: {:?}", codes(&r));
    assert!(r.incomplete && r.reasons.iter().any(|n| n.contains("Total decompressed-byte scan limit")), "{:?}", r.reasons);
    assert!(stats.inspected_bytes <= 3000, "{stats:?}");
    // 음성: 예산이 충분하면 같은 OPF에서 항목이 그대로 검출된다.
    let r = scan_with(&p, &Limits::default());
    assert!(has(&r, "REMOTE_MANIFEST_ITEM"), "{:?}", codes(&r));
}

#[test]
fn opf_prepass_is_capped_at_max_files() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    let extra_opf = opf(&format!("{}{}", opf_item("c1", "ch1.xhtml", "application/xhtml+xml", ""), REMOTE_ITEM));
    epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/extra.opf", extra_opf.as_bytes())]);
    // 항목은 mimetype, container, content.opf, ch1, extra.opf 순이라 max_files=4면 extra.opf는 범위 밖이다.
    let r = scan_with(&p, &Limits { max_files: 4, ..Limits::default() });
    assert!(!has(&r, "REMOTE_MANIFEST_ITEM"), "{:?}", codes(&r));
    assert!(r.reasons.iter().any(|n| n.contains("Too many entries")), "{:?}", r.reasons);
    // 음성: 범위 안이면 검출된다.
    let r = scan_with(&p, &Limits { max_files: 5, ..Limits::default() });
    assert!(has(&r, "REMOTE_MANIFEST_ITEM"), "{:?}", codes(&r));
}

/// 중앙 디렉터리와 로컬 헤더의 선언 크기(비압축)를 `declared`로 바꾼다.
fn forge_declared_size(path: &Path, member: &str, declared: u32) {
    let mut data = fs::read(path).unwrap();
    let name = member.as_bytes();
    let local = (0..data.len() - 30).find(|&i| &data[i..i + 4] == b"PK\x03\x04" && &data[i + 30..i + 30 + name.len()] == name).unwrap();
    data[local + 22..local + 26].copy_from_slice(&declared.to_le_bytes());
    let central = (0..data.len() - 46).find(|&i| &data[i..i + 4] == b"PK\x01\x02" && &data[i + 46..i + 46 + name.len()] == name).unwrap();
    data[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
    fs::write(path, data).unwrap();
}

#[test]
fn forged_small_declared_size_cannot_exceed_the_total_budget() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/bomb.bin", &vec![0u8; 4 << 20])]);
    forge_declared_size(&p, "OEBPS/bomb.bin", 10);
    let limits = Limits { max_member: 8 << 20, max_total: 100_000, max_files: 10_000 };
    let (r, stats) = scan_with_stats(&p, &limits);
    assert!(stats.inspected_bytes <= limits.max_total, "{stats:?}");
    assert!(!r.reasons.iter().any(|n| n.contains("panic")), "{:?}", r.reasons);
    // 음성: 속이지 않은 같은 멤버는 선언 크기가 예산을 넘으므로 incomplete 한 건으로 건너뛴다.
    let q = d.path().join("honest.epub");
    epub_with_files(&q, &clean_opf(), &xhtml("<p>x</p>"), &[("OEBPS/bomb.bin", &vec![0u8; 4 << 20])]);
    let (r, stats) = scan_with_stats(&q, &limits);
    assert!(r.incomplete && stats.inspected_bytes <= limits.max_total);
}

#[test]
fn honest_zip_bomb_is_still_flagged_with_the_old_code_and_severity() {
    let r = scan_extra("OEBPS/bomb.bin", &vec![0u8; 4 << 20]);
    assert_eq!(sev(&r, "HIGH_COMPRESSION_RATIO"), Severity::Medium);
}

// ---- 2. URL 속성은 접두사와 무관하게 로컬 이름으로 본다 ----

#[test]
fn xlink_href_literal_spelling_is_still_flagged() {
    let r = scan_chapter(r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><a xlink:href="javascript:alert(1)"><text>x</text></a></svg>"#);
    assert_eq!(sev(&r, "JS_URL"), Severity::High);
}

#[test]
fn url_attributes_with_any_namespace_prefix_are_checked() {
    for body in [
        r#"<a x:href="javascript:alert(1)">x</a>"#,
        r#"<img foo:src="javascript:alert(1)"/>"#,
        r#"<form bar:action="javascript:alert(1)"></form>"#,
    ] {
        let r = scan_chapter(body);
        assert_eq!(sev(&r, "JS_URL"), Severity::High, "{body}: {:?}", codes(&r));
    }
    // 접두사 없는 같은 값과 같은 코드·심각도다.
    assert_eq!(sev(&scan_chapter(r#"<a href="javascript:alert(1)">x</a>"#), "JS_URL"), Severity::High);
}

#[test]
fn prefixed_url_attribute_with_a_safe_value_or_other_local_name_is_not_flagged() {
    assert!(!has(&scan_chapter(r#"<a x:href="chapter2.xhtml">x</a>"#), "JS_URL"));
    assert!(!has(&scan_chapter(r#"<a x:myhref="javascript:alert(1)">x</a>"#), "JS_URL"));
    assert!(!has(&scan_chapter(r#"<a x:href="https://example.com/">x</a>"#), "JS_URL"));
}

#[test]
fn animate_attribute_name_is_matched_by_local_name() {
    let r = scan_chapter(r#"<svg><a><animate attributeName="x:href" to="javascript:alert(1)"/></a></svg>"#);
    assert_eq!(sev(&r, "SVG_ANIMATE_HREF"), Severity::Medium);
    assert_eq!(sev(&r, "JS_URL"), Severity::High);
    // 음성: href가 아닌 속성을 바꾸는 애니메이션은 그대로 통과한다.
    let r = scan_chapter(r#"<svg><circle><animate attributeName="x:fill" to="javascript:alert(1)"/></circle></svg>"#);
    assert!(!has(&r, "SVG_ANIMATE_HREF") && !has(&r, "JS_URL"), "{:?}", codes(&r));
    // 기존 철자는 그대로다.
    let r = scan_chapter(r#"<svg><a><set attributeName="xlink:href" to="javascript:alert(1)"/></a></svg>"#);
    assert_eq!(sev(&r, "SVG_ANIMATE_HREF"), Severity::Medium);
}

// ---- 3. UTF-16/32 마크업 ----

fn utf16(text: &str, le: bool, bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bom {
        out.extend(if le { [0xFF, 0xFE] } else { [0xFE, 0xFF] });
    }
    for u in text.encode_utf16() {
        out.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
    }
    out
}

const SCRIPT_DOC: &str = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><script>alert(1)</script></body></html>";

#[test]
fn utf16_markup_with_bom_is_transcoded_and_scanned() {
    for le in [true, false] {
        let r = scan_extra("OEBPS/u16.xhtml", &utf16(SCRIPT_DOC, le, true));
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High, "le={le}: {:?}", codes(&r));
        assert!(!has(&r, "MARKUP_UNDECODABLE"));
    }
}

#[test]
fn utf16_markup_without_bom_is_detected_by_nul_interleave() {
    for le in [true, false] {
        let r = scan_extra("OEBPS/u16.xhtml", &utf16(SCRIPT_DOC, le, false));
        assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High, "le={le}: {:?}", codes(&r));
    }
}

#[test]
fn utf32_markup_is_reported_as_undecodable() {
    for bytes in [[&[0xFF, 0xFE, 0, 0][..], b"<\0\0\0s\0\0\0"].concat(), [&[0, 0, 0xFE, 0xFF][..], b"\0\0\0<\0\0\0s"].concat()] {
        let r = scan_extra("OEBPS/u32.xhtml", &bytes);
        assert_eq!(sev(&r, "MARKUP_UNDECODABLE"), Severity::Medium, "{:?}", codes(&r));
        assert!(r.incomplete && r.reasons.iter().any(|n| n.contains("unsupported encoding utf-32") && n.contains("OEBPS/u32.xhtml")), "{:?}", r.reasons);
    }
}

#[test]
fn utf8_and_utf8_bom_markup_behave_as_before() {
    let doc = xhtml("<script>alert(1)</script>");
    let r = scan_extra("OEBPS/a.xhtml", doc.as_bytes());
    assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
    assert!(!r.reasons.iter().any(|n| n.contains("non-utf8")), "{:?}", r.reasons);
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend(doc.as_bytes());
    let r = scan_extra("OEBPS/b.xhtml", &bom);
    assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
    assert!(!r.incomplete && !has(&r, "MARKUP_UNDECODABLE"), "{:?}", r.reasons);
    let r = scan_extra("OEBPS/c.xhtml", xhtml("<p>한글 본문</p>").as_bytes());
    assert!(!r.incomplete && r.findings.is_empty(), "{:?}", r.reasons);
}

#[test]
fn invalid_utf8_markup_makes_the_scan_incomplete_once_per_member() {
    let mut bad = xhtml("<p>caf").into_bytes();
    bad.push(0xE9);
    bad.extend(b"</p><script>1</script>");
    let r = scan_extra("OEBPS/latin.xhtml", &bad);
    let notes: Vec<_> = r.reasons.iter().filter(|n| n.contains("non-utf8 markup: OEBPS/latin.xhtml")).collect();
    assert_eq!(notes.len(), 1, "{:?}", r.reasons);
    assert!(r.incomplete);
    assert_eq!(sev(&r, "SCRIPT_TAG"), Severity::High);
}

// ---- 4. data: URL ----

#[test]
fn active_data_url_mime_types_are_flagged_like_data_text_html() {
    for url in [
        "data:image/svg+xml;base64,PHN2Zy8+",
        "data:application/xml,%3Cx/%3E",
        "data:text/xml,%3Cx/%3E",
        "data:application/atom+xml,x",
        "data:text/javascript,alert(1)",
        "data:application/javascript,alert(1)",
        "data:application/x-javascript,alert(1)",
        "data:application/xhtml+xml,x",
        "  DATA:Image/SVG+XML;base64,PHN2Zy8+",
        "data: text/xml ,x",
    ] {
        let r = scan_chapter(&format!(r#"<img src="{url}"/>"#));
        assert_eq!(sev(&r, "JS_URL"), Severity::High, "{url}: {:?}", codes(&r));
    }
    let r = scan_chapter(r#"<a href="data:text/html;base64,PGI+">x</a>"#);
    assert_eq!(sev(&r, "JS_URL"), Severity::High);
}

#[test]
fn benign_data_urls_in_img_are_not_flagged() {
    for url in ["data:image/png;base64,iVBORw0KGgo=", "data:image/jpeg;base64,/9j/", "data:image/gif;base64,R0lGOD", "data:text/plain,hello"] {
        let r = scan_chapter(&format!(r#"<img src="{url}"/>"#));
        assert!(!has(&r, "JS_URL"), "{url}: {:?}", codes(&r));
    }
}

#[test]
fn data_urls_in_embedding_elements_must_be_images_or_fonts() {
    for body in [
        r#"<iframe src="data:text/plain,hi"></iframe>"#,
        r#"<iframe src="data:,hi"></iframe>"#,
        r#"<object data="data:application/pdf;base64,AAAA"></object>"#,
        r#"<embed src="data:application/octet-stream;base64,AAAA"/>"#,
        r#"<frame src="data:image/svg+xml;base64,PHN2Zy8+"/>"#,
    ] {
        let r = scan_chapter(body);
        assert_eq!(sev(&r, "JS_URL"), Severity::High, "{body}: {:?}", codes(&r));
    }
    for body in [
        r#"<iframe src="data:image/png;base64,iVBORw0KGgo="></iframe>"#,
        r#"<object data="data:image/webp;base64,AAAA"></object>"#,
        r#"<embed src="data:font/woff2;base64,AAAA"/>"#,
        r#"<object data="data:application/font-woff;base64,AAAA"></object>"#,
    ] {
        let r = scan_chapter(body);
        assert!(!has(&r, "JS_URL"), "{body}: {:?}", codes(&r));
    }
}
