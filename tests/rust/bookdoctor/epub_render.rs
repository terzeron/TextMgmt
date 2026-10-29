//! EPUB 렌더링 검사 테스트. 규칙의 근거는 frontend/src/EpubDiagnose.js와 ViewEPUB.jsx다.

mod common;

use std::fs;
use std::path::Path;

use bookdoctor::cli::Options;
use bookdoctor::epub::render::{ReadErrorKind, Selector, classify_read_error, count_hidden_elements, inspect, inspect_with, zero_size_selectors};
use bookdoctor::epub::security::Limits;
use bookdoctor::scan::scan_file;
use common::*;
use scan_common::{Category, FileReport, Format, Severity};

const OPF_NS_DECL: &str = r#" xmlns:dc="http://purl.org/dc/elements/1.1/""#;
const OPF_PATH: &str = "OEBPS/content.opf";

fn valid_opf() -> String {
    format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"{OPF_NS_DECL} version="3.0" unique-identifier="id"><metadata><dc:title>t</dc:title><dc:language>ko</dc:language><dc:identifier id="id">x</dc:identifier></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest><spine toc="ncx"><itemref idref="c1"/></spine><guide><reference type="text" href="ch1.xhtml#a"/></guide></package>"#
    )
}

fn valid_ncx() -> String {
    r#"<?xml version="1.0"?><ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1"><navMap><navPoint id="n1"><navLabel><text>1</text></navLabel><content src="ch1.xhtml#a"/></navPoint></navMap></ncx>"#.to_string()
}

struct Parts {
    opf: String,
    ncx: Option<String>,
    ch1: Vec<u8>,
    extra: Vec<(String, Vec<u8>)>,
    stored: Vec<String>,
}

fn valid() -> Parts {
    Parts { opf: valid_opf(), ncx: Some(valid_ncx()), ch1: xhtml("<p>본문</p>").into_bytes(), extra: Vec::new(), stored: Vec::new() }
}

fn build(p: &Parts) -> NamedFile {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.epub");
    let nav = xhtml("<nav/>");
    let mut members: Vec<(&str, &[u8])> = vec![(OPF_PATH, p.opf.as_bytes()), ("OEBPS/ch1.xhtml", &p.ch1), ("OEBPS/nav.xhtml", nav.as_bytes())];
    if let Some(n) = &p.ncx {
        members.push(("OEBPS/toc.ncx", n.as_bytes()));
    }
    for (n, d) in &p.extra {
        members.push((n, d));
    }
    let stored: Vec<&str> = p.stored.iter().map(String::as_str).collect();
    epub_members(&path, &members, &stored);
    NamedFile::keep(dir, path)
}

fn report(f: &NamedFile) -> FileReport {
    let mut r = FileReport::new(f.path(), Some(Format::Epub));
    inspect(f.path(), &mut r);
    r
}

fn scan(p: &Parts) -> FileReport {
    report(&build(p))
}

fn flagged(r: &FileReport, code: &str) -> (Category, Severity, String) {
    let f = r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} not flagged: {:?}", r.findings));
    (f.category, f.severity, f.location.clone())
}

fn assert_clean(code: &str) {
    let r = scan(&valid());
    assert!(!r.has(code), "{code} on valid epub: {:?}", r.findings);
}

// ---- UNDECLARED_PREFIX ----

#[test]
fn flags_undeclared_prefix() {
    let mut p = valid();
    p.opf = p.opf.replace(OPF_NS_DECL, "");
    let (c, s, loc) = flagged(&scan(&p), "UNDECLARED_PREFIX");
    assert_eq!((c, s, loc.as_str()), (Category::Render, Severity::Medium, OPF_PATH));
}

#[test]
fn clean_epub_has_no_undeclared_prefix() {
    assert_clean("UNDECLARED_PREFIX");
}

// ---- MANIFEST_EMPTY / SPINE_EMPTY ----

#[test]
fn flags_manifest_empty() {
    let mut p = valid();
    p.opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest></manifest><spine><itemref idref="c1"/></spine></package>"#.into();
    let (c, s, _) = flagged(&scan(&p), "MANIFEST_EMPTY");
    assert_eq!((c, s), (Category::Render, Severity::High));
}

#[test]
fn clean_epub_has_no_manifest_empty() {
    assert_clean("MANIFEST_EMPTY");
}

#[test]
fn flags_spine_empty() {
    let mut p = valid();
    p.opf = p.opf.replace(r#"<itemref idref="c1"/>"#, "");
    let (c, s, _) = flagged(&scan(&p), "SPINE_EMPTY");
    assert_eq!((c, s), (Category::Render, Severity::High));
}

#[test]
fn clean_epub_has_no_spine_empty() {
    assert_clean("SPINE_EMPTY");
}

// ---- SPINE_IDREF_MISSING ----

#[test]
fn flags_spine_idref_missing() {
    let mut p = valid();
    p.opf = p.opf.replace(r#"idref="c1""#, r#"idref="zz""#);
    let (c, s, loc) = flagged(&scan(&p), "SPINE_IDREF_MISSING");
    assert_eq!((c, s, loc.as_str()), (Category::Render, Severity::High, "zz"));
}

#[test]
fn clean_epub_has_no_spine_idref_missing() {
    assert_clean("SPINE_IDREF_MISSING");
}

#[test]
fn spine_member_missing_stays_in_structure_not_render() {
    // manifest에는 있지만 ZIP에 파일이 없는 spine 항목은 structure의 SPINE_MEMBER_MISSING이다.
    let mut p = valid();
    p.opf = p.opf.replace("ch1.xhtml\" media", "gone.xhtml\" media");
    assert!(!scan(&p).has("SPINE_IDREF_MISSING"));
}

// ---- NAV_MISSING ----

#[test]
fn flags_nav_missing() {
    let mut p = valid();
    p.opf = p.opf.replace(r#" properties="nav""#, "");
    let (c, s, _) = flagged(&scan(&p), "NAV_MISSING");
    assert_eq!((c, s), (Category::Render, Severity::Low));
}

#[test]
fn clean_epub_has_no_nav_missing() {
    assert_clean("NAV_MISSING");
}

#[test]
fn epub2_without_nav_is_not_flagged() {
    let mut p = valid();
    p.opf = p.opf.replace(r#" properties="nav""#, "").replace(r#"version="3.0""#, r#"version="2.0""#);
    assert!(!scan(&p).has("NAV_MISSING"));
}

// ---- NCX_REF_MISSING ----

#[test]
fn flags_ncx_ref_missing() {
    let mut p = valid();
    p.ncx = Some(valid_ncx().replace("ch1.xhtml#a", "none.xhtml#a"));
    let (c, s, loc) = flagged(&scan(&p), "NCX_REF_MISSING");
    assert_eq!((c, s, loc.as_str()), (Category::Render, Severity::Medium, "OEBPS/toc.ncx"));
}

#[test]
fn flags_ncx_ref_missing_when_toc_id_not_in_manifest() {
    let mut p = valid();
    p.opf = p.opf.replace(r#"toc="ncx""#, r#"toc="nope""#);
    assert!(scan(&p).has("NCX_REF_MISSING"));
}

#[test]
fn flags_ncx_ref_missing_when_ncx_file_absent() {
    let mut p = valid();
    p.ncx = None;
    assert!(scan(&p).has("NCX_REF_MISSING"));
}

#[test]
fn clean_epub_has_no_ncx_ref_missing() {
    assert_clean("NCX_REF_MISSING");
}

// ---- GUIDE_REF_MISSING ----

#[test]
fn flags_guide_ref_missing() {
    let mut p = valid();
    p.opf = p.opf.replace(r#"href="ch1.xhtml#a""#, r#"href="gone.xhtml""#);
    let (c, s, _) = flagged(&scan(&p), "GUIDE_REF_MISSING");
    assert_eq!((c, s), (Category::Render, Severity::Low));
}

#[test]
fn clean_epub_has_no_guide_ref_missing() {
    assert_clean("GUIDE_REF_MISSING");
}

// ---- XHTML_PARSE_FAILED / HEAD_MISSING ----

#[test]
fn flags_xhtml_parse_failed() {
    let mut p = valid();
    p.ch1 = b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>t</title></head><body><p>x</body></html>".to_vec();
    let (c, s, loc) = flagged(&scan(&p), "XHTML_PARSE_FAILED");
    assert_eq!((c, s, loc.as_str()), (Category::Render, Severity::High, "OEBPS/ch1.xhtml"));
}

#[test]
fn clean_epub_has_no_xhtml_parse_failed() {
    assert_clean("XHTML_PARSE_FAILED");
}

#[test]
fn flags_head_missing_for_body_without_head() {
    let mut p = valid();
    p.ch1 = b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>x</p></body></html>".to_vec();
    let r = scan(&p);
    let h = r.findings.iter().find(|x| x.code == "HEAD_MISSING").unwrap();
    assert_eq!((h.category, h.severity), (Category::Render, Severity::High));
    assert_eq!(h.location, "OEBPS/ch1.xhtml");
}

#[test]
fn clean_epub_has_no_head_missing() {
    assert_clean("HEAD_MISSING");
}

#[test]
fn prefixed_head_counts_as_head_present() {
    // ViewEPUB.jsx secureEpubDocument는 head를 찾기 전에 stripElementPrefixes를 부른다.
    let mut p = valid();
    p.ch1 = prefixed_chapter().into_bytes();
    assert!(!scan(&p).has("HEAD_MISSING"));
}

#[test]
fn prefixed_document_without_head_is_still_flagged() {
    let mut p = valid();
    p.ch1 = format!(r#"<html:html xmlns:html="{XHTML_NS}"><html:body><html:p>x</html:p></html:body></html:html>"#).into_bytes();
    assert!(scan(&p).has("HEAD_MISSING"));
}

#[test]
fn non_markup_spine_member_is_not_parsed() {
    let mut p = valid();
    p.opf = p.opf.replace(r#"href="ch1.xhtml" media-type="application/xhtml+xml""#, r#"href="ch1.xhtml" media-type="audio/mpeg""#);
    p.ch1 = vec![0xff, 0xfb, 0x90, 0x00, 0x01];
    let r = scan(&p);
    assert!(!r.has("XHTML_PARSE_FAILED") && !r.has("HEAD_MISSING"));
}

// ---- DC_METADATA_MISSING ----

#[test]
fn flags_dc_metadata_missing() {
    let mut p = valid();
    p.opf = p.opf.replace("<dc:language>ko</dc:language>", "");
    let r = scan(&p);
    let (c, s, _) = flagged(&r, "DC_METADATA_MISSING");
    assert_eq!((c, s), (Category::Render, Severity::Low));
    let detail = &r.findings.iter().find(|f| f.code == "DC_METADATA_MISSING").unwrap().detail;
    assert!(detail.contains("dc:language") && !detail.contains("dc:title"), "{detail}");
}

#[test]
fn clean_epub_has_no_dc_metadata_missing() {
    assert_clean("DC_METADATA_MISSING");
}

// ---- ZIP_CRC_MISMATCH ----

const MARKER: &[u8] = b"CRC-MARKER-UNIQUE-PAYLOAD-0123456789";

fn flip_payload_byte(path: &Path) {
    let mut data = fs::read(path).unwrap();
    let at = data.windows(MARKER.len()).position(|w| w == MARKER).expect("marker");
    data[at + 3] ^= 0x01;
    fs::write(path, data).unwrap();
}

fn stored_payload_parts(len: usize) -> Parts {
    let mut p = valid();
    let mut payload = MARKER.to_vec();
    payload.resize(len, b'.');
    p.extra.push(("OEBPS/data.bin".into(), payload));
    p.stored.push("OEBPS/data.bin".into());
    p
}

#[test]
fn flags_zip_crc_mismatch() {
    let f = build(&stored_payload_parts(200));
    flip_payload_byte(f.path());
    let (c, s, loc) = flagged(&report(&f), "ZIP_CRC_MISMATCH");
    assert_eq!((c, s, loc.as_str()), (Category::Defect, Severity::High, "OEBPS/data.bin"));
}

#[test]
fn clean_epub_has_no_zip_crc_mismatch() {
    let f = build(&stored_payload_parts(200));
    assert!(!report(&f).has("ZIP_CRC_MISMATCH"));
    assert_clean("ZIP_CRC_MISMATCH");
}

// ---- FONT_TOO_LARGE ----

fn with_font(name: &str, len: usize) -> Parts {
    let mut p = valid();
    p.extra.push((name.into(), vec![0u8; len]));
    p
}

#[test]
fn flags_font_too_large() {
    // 근거: backend/src/book_manager/preview.rs:26 FONT_SIZE_LIMIT = 500 * 1024 (초과만 뺀다).
    let r = scan(&with_font("OEBPS/f.TTF", 500 * 1024 + 1));
    let (c, s, loc) = flagged(&r, "FONT_TOO_LARGE");
    assert_eq!((c, s, loc.as_str()), (Category::Render, Severity::Low, "OEBPS/f.TTF"));
}

#[test]
fn clean_epub_has_no_font_too_large() {
    assert_clean("FONT_TOO_LARGE");
    assert!(!scan(&with_font("OEBPS/f.woff2", 500 * 1024)).has("FONT_TOO_LARGE"));
}

#[test]
fn font_declared_by_media_type_is_checked() {
    let mut p = with_font("OEBPS/f.dat", 500 * 1024 + 1);
    p.opf = p.opf.replace("</manifest>", r#"<item id="f" href="f.dat" media-type="font/ttf"/></manifest>"#);
    assert!(scan(&p).has("FONT_TOO_LARGE"));
}

// ---- OBFUSCATED_TEXT ----

fn spans(n: usize, style: &str) -> String {
    format!(r#"<span style="{style}">x</span>"#).repeat(n)
}

#[test]
fn flags_obfuscated_text() {
    let mut p = valid();
    p.ch1 = xhtml(&spans(20, "font-size:0%")).into_bytes();
    let (c, s, loc) = flagged(&scan(&p), "OBFUSCATED_TEXT");
    assert_eq!((c, s, loc.as_str()), (Category::Render, Severity::Medium, "OEBPS/ch1.xhtml"));
}

#[test]
fn clean_epub_has_no_obfuscated_text() {
    assert_clean("OBFUSCATED_TEXT");
    let mut p = valid();
    p.ch1 = xhtml(&format!("{}{}", spans(19, "font-size:0px"), spans(30, "font-size:0.5em;color:red"))).into_bytes();
    assert!(!scan(&p).has("OBFUSCATED_TEXT"));
}

#[test]
fn flags_obfuscated_text_from_style_block_class() {
    let mut p = valid();
    let body = r#"<span class="h">x</span>"#.repeat(20);
    p.ch1 = format!(r#"<?xml version="1.0"?><html xmlns="{XHTML_NS}"><head><title>t</title><style>.h {{ font-size: 0%; }}</style></head><body>{body}</body></html>"#).into_bytes();
    assert!(scan(&p).has("OBFUSCATED_TEXT"));
}

#[test]
fn flags_obfuscated_text_from_linked_stylesheet_tag() {
    let mut p = valid();
    let body = "<eeḕ>x</eeḕ>".repeat(20);
    p.ch1 = format!(r#"<?xml version="1.0"?><html xmlns="{XHTML_NS}"><head><title>t</title><link rel="stylesheet" href="s.css"/></head><body>{body}</body></html>"#).into_bytes();
    p.extra.push(("OEBPS/s.css".into(), "eeh, eeḕ{font-size:0%}".as_bytes().to_vec()));
    assert!(scan(&p).has("OBFUSCATED_TEXT"));
}

#[test]
fn flags_obfuscated_text_from_rtl_overrides() {
    let mut p = valid();
    p.ch1 = xhtml(&format!("<p>{}</p>", "\u{202e}a".repeat(20))).into_bytes();
    assert!(scan(&p).has("OBFUSCATED_TEXT"));
    let mut q = valid();
    q.ch1 = xhtml(&format!("<p>{}</p>", "\u{202e}a".repeat(19))).into_bytes();
    assert!(!scan(&q).has("OBFUSCATED_TEXT"));
}

// ---- 한도 ----

fn scan_with(p: &Parts, limits: &Limits) -> FileReport {
    let f = build(p);
    let mut r = FileReport::new(f.path(), Some(Format::Epub));
    inspect_with(f.path(), &mut r, limits);
    r
}

#[test]
fn member_over_max_member_marks_incomplete_not_a_finding() {
    let limits = Limits { max_member: 4096, ..Limits::default() };
    let f = build(&stored_payload_parts(8192));
    flip_payload_byte(f.path());
    let mut r = FileReport::new(f.path(), Some(Format::Epub));
    inspect_with(f.path(), &mut r, &limits);
    assert!(!r.has("ZIP_CRC_MISMATCH"), "{:?}", r.findings);
    assert!(r.incomplete);
    assert_eq!(r.reasons.len(), 1, "{:?}", r.reasons);
    assert!(r.reasons[0].contains("max_member") && r.reasons[0].contains('1'), "{:?}", r.reasons);
}

#[test]
fn total_budget_marks_incomplete_when_sum_exceeds() {
    let limits = Limits { max_member: 1 << 20, max_total: 10_000, ..Limits::default() };
    let mut p = valid();
    for i in 0..5 {
        p.extra.push((format!("OEBPS/b{i}.bin"), vec![b'a'; 4000]));
    }
    let r = scan_with(&p, &limits);
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|m| m.contains("budget")), "{:?}", r.reasons);
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
fn total_budget_is_enforced_by_actual_bytes_not_declared_size() {
    // 예산은 선언 크기의 합이 아니라 읽은 바이트로 센다. 선언 크기는 각 멤버가 max_member 이하인지 거르는 데만 쓴다.
    // (a) 멤버마다 선언은 한도 이하여도 읽은 합이 예산을 넘으면, 넘는 멤버의 읽기를 남은 예산+1바이트에서 자르고 incomplete다.
    let limits = Limits { max_member: 4000, max_total: 7000, ..Limits::default() };
    let mut p = valid();
    for i in 0..3 {
        p.extra.push((format!("OEBPS/b{i}.bin"), vec![b'a'; 3000]));
    }
    let r = scan_with(&p, &limits);
    assert!(r.reasons.iter().any(|m| m.contains("budget")), "{:?}", r.reasons);
    // (b) 선언 크기를 10으로 속인 4MiB 멤버: zip 라이브러리가 선언 크기에서 읽기를 끊으므로 실제로 읽은 10바이트만 예산에서 빠진다.
    let limits = Limits { max_member: 8 << 20, max_total: 100_000, ..Limits::default() };
    let mut p = valid();
    p.extra.push(("OEBPS/bomb.bin".into(), vec![0u8; 4 << 20]));
    let f = build(&p);
    forge_declared_size(f.path(), "OEBPS/bomb.bin", 10);
    let mut r = FileReport::new(f.path(), Some(Format::Epub));
    inspect_with(f.path(), &mut r, &limits);
    assert!(!r.incomplete, "{:?}", r.reasons);
    assert!(r.findings.is_empty(), "속인 선언 크기가 거짓 지표를 만들면 안 된다: {:?}", r.findings);
}

// ---- 읽기 예산·CSS·오류 분류 (fix round 1) ----

fn chapter_with(head_extra: &str, body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0"?><html xmlns="{XHTML_NS}"><head><title>t</title>{head_extra}</head><body>{body}</body></html>"#).into_bytes()
}

fn link(name: &str) -> String {
    format!(r#"<link rel="stylesheet" href="{name}"/>"#)
}

fn rules(n: usize) -> Vec<u8> {
    (0..n).map(|i| format!(".c{i}{{font-size:0}}")).collect::<String>().into_bytes()
}

fn padded(rule: &str, len: usize) -> Vec<u8> {
    let mut v = rule.as_bytes().to_vec();
    v.resize(len, b' ');
    v
}

fn reasons_containing(r: &FileReport, word: &str) -> usize {
    r.reasons.iter().filter(|m| m.contains(word)).count()
}

#[test]
fn css_reads_are_bounded_by_remaining_budget() {
    // 서로 다른 스타일시트 5개(각 3000바이트). 예산은 앞의 둘쯤에서 바닥나고, 숨김 규칙이 든 마지막 CSS는 읽지 못한다.
    let limits = Limits { max_member: 4000, max_total: 8000, ..Limits::default() };
    let mut p = valid();
    let links: String = (0..5).map(|i| link(&format!("s{i}.css"))).collect();
    p.ch1 = chapter_with(&links, &r#"<span class="h">x</span>"#.repeat(20));
    for i in 0..4 {
        p.extra.push((format!("OEBPS/s{i}.css"), padded(".z{color:red}", 3000)));
    }
    p.extra.push(("OEBPS/s4.css".into(), padded(".h{font-size:0}", 3000)));
    let r = scan_with(&p, &limits);
    assert!(r.incomplete);
    assert!(reasons_containing(&r, "budget") >= 1, "{:?}", r.reasons);
    assert!(!r.has("OBFUSCATED_TEXT"), "예산 밖 CSS를 일부만 읽고 판정하면 안 된다");
}

#[test]
fn css_over_the_read_cap_is_incomplete_not_silent() {
    // 6MiB CSS는 작은 멤버 상한(5MiB)을 넘는다. 빈 선택자로 조용히 넘기면 난독화를 놓친다.
    let mut p = valid();
    p.ch1 = chapter_with(&link("big.css"), &r#"<span class="h">x</span>"#.repeat(20));
    p.extra.push(("OEBPS/big.css".into(), padded(".h{font-size:0}", 6 << 20)));
    let r = scan(&p);
    assert!(r.incomplete, "{:?}", r.findings);
    assert_eq!(reasons_containing(&r, "stylesheet"), 1, "{:?}", r.reasons);
    assert!(!r.has("OBFUSCATED_TEXT"));
}

#[test]
fn duplicate_link_to_one_stylesheet_adds_selectors_once() {
    let mut p = valid();
    p.ch1 = chapter_with(&link("a.css").repeat(3), "<p>x</p>");
    p.extra.push(("OEBPS/a.css".into(), rules(5000)));
    let r = scan(&p);
    assert!(!r.incomplete, "한 번만 더하면 5000개라 상한(10000) 안이다: {:?}", r.reasons);
    // 대조: 서로 다른 스타일시트 3개면 15000개라 상한을 넘는다.
    let mut q = valid();
    q.ch1 = chapter_with(&format!("{}{}{}", link("a.css"), link("b.css"), link("c.css")), "<p>x</p>");
    for n in ["a", "b", "c"] {
        q.extra.push((format!("OEBPS/{n}.css"), rules(5000)));
    }
    assert!(scan(&q).incomplete);
}

#[test]
fn selector_cap_hit_marks_incomplete_once() {
    let mut p = valid();
    let css = String::from_utf8(rules(10_100)).unwrap();
    p.ch1 = chapter_with(&format!("<style>{css}</style>"), "<p>x</p>");
    let r = scan(&p);
    assert!(r.incomplete);
    assert_eq!(reasons_containing(&r, "selector"), 1, "{:?}", r.reasons);
}

#[test]
fn stylesheet_cap_per_document_marks_incomplete() {
    let mut p = valid();
    let links: String = (0..65).map(|i| link(&format!("s{i}.css"))).collect();
    p.ch1 = chapter_with(&links, "<p>x</p>");
    for i in 0..65 {
        p.extra.push((format!("OEBPS/s{i}.css"), b".z{color:red}".to_vec()));
    }
    let r = scan(&p);
    assert!(r.incomplete);
    assert_eq!(reasons_containing(&r, "stylesheet"), 1, "{:?}", r.reasons);
}

#[test]
fn counting_stops_at_the_threshold() {
    let body = r#"<span class="h">x</span>"#.repeat(100);
    let xml = format!("<r>{body}</r>");
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let sels = zero_size_selectors(".h{font-size:0}");
    assert_eq!(sels.len(), 1);
    assert_eq!(count_hidden_elements(&doc, &sels, 20), 20);
    assert_eq!(count_hidden_elements(&doc, &sels, 1000), 100);
}

#[test]
fn read_errors_are_classified() {
    use std::io::{Error, ErrorKind};
    assert_eq!(classify_read_error(&Error::new(ErrorKind::InvalidData, "Invalid checksum")), ReadErrorKind::Checksum);
    assert_eq!(classify_read_error(&Error::new(ErrorKind::InvalidData, "deflate")), ReadErrorKind::Corrupt);
    assert_eq!(classify_read_error(&Error::new(ErrorKind::UnexpectedEof, "eof")), ReadErrorKind::Corrupt);
    assert_eq!(classify_read_error(&Error::new(ErrorKind::PermissionDenied, "io")), ReadErrorKind::Io);
    assert_eq!(classify_read_error(&Error::other("io")), ReadErrorKind::Io);
}

#[test]
fn zero_font_size_units_are_flagged_and_nonzero_are_not() {
    for unit in ["0em", "0pt", "0rem", "0%", "0px", "0"] {
        assert_eq!(zero_size_selectors(&format!(".h{{font-size:{unit}}}")).len(), 1, "{unit}");
    }
    for value in ["0.5em", "10px", "100%", "1em"] {
        assert!(zero_size_selectors(&format!(".h{{font-size:{value}}}")).is_empty(), "{value}");
    }
    let _: Option<Selector> = None;
}

#[test]
fn container_and_opf_are_charged_to_the_budget_once() {
    // 예산이 container+opf(+mimetype 등) 합보다 약간 넉넉하면 되풀이해 읽지 않으므로 incomplete가 아니다.
    let base = build(&valid());
    let total: u64 = {
        let mut z = zip::ZipArchive::new(std::fs::File::open(base.path()).unwrap()).unwrap();
        (0..z.len()).map(|i| z.by_index(i).unwrap().size()).sum()
    };
    let limits = Limits { max_total: total + 16, ..Limits::default() };
    let r = scan_with(&valid(), &limits);
    assert!(!r.incomplete, "{:?}", r.reasons);
}

// ---- 종합 ----

#[test]
fn clean_epub_has_no_render_findings() {
    let r = scan(&valid());
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert!(!r.incomplete, "{:?}", r.reasons);
}

#[test]
fn not_a_zip_is_silent() {
    let f = file_with("a.epub", b"not a zip at all");
    let r = report(&f);
    assert!(r.findings.is_empty() && !r.incomplete);
}

// ---- 전체 파이프라인 ----

#[test]
fn scan_file_reports_head_missing_through_pipeline() {
    let mut p = valid();
    p.ch1 = b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>x</p></body></html>".to_vec();
    let f = build(&p);
    let r = scan_file(f.path(), &Options::default());
    assert!(r.has("HEAD_MISSING"), "{:?}", r.findings);
}

#[test]
fn scan_file_skips_render_when_not_a_zip() {
    let f = file_with("a.epub", b"PK\x03\x04 truncated");
    let r = scan_file(f.path(), &Options::default());
    assert!(!r.findings.iter().any(|x| x.code == "HEAD_MISSING" || x.code == "MANIFEST_EMPTY"), "{:?}", r.findings);
}
