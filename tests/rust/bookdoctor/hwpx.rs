//! HWPX 테스트: 구조, 내장 실행 파일·OLE, XML 공격. 스크립트 탐지는 구현하지 않는다(docs/bookdoctor-hwpx-notes.md).

mod common;

use std::fs;

use bookdoctor::cli::Options;
use bookdoctor::scan::scan_file;
use scan_common::{Category, FileReport, Finding, Format, Severity};

fn hwpx(remove: &[&str], add: &[(&str, &[u8])]) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.hwpx");
    common::hwpx_with(&p, remove, add);
    scan_file(&p, &Options::default())
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
const OLE: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 payload";

#[test]
fn minimal_hwpx_has_no_findings() {
    let r = hwpx(&[], &[]);
    assert_eq!(r.format, Some(Format::Hwpx));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn missing_manifest_is_high() {
    let r = hwpx(&["Contents/content.hpf"], &[]);
    let f = finding(&r, "HWPX_NO_MANIFEST");
    assert_eq!((f.category, f.severity), (Category::Render, Severity::High));
}

#[test]
fn manifest_pointing_to_a_missing_section_is_high() {
    let r = hwpx(&["Contents/section0.xml"], &[]);
    let f = finding(&r, "HWPX_SECTION_MISSING");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Render, Severity::High, "Contents/section0.xml"));
}

#[test]
fn bad_mimetype_is_medium_and_missing_mimetype_is_allowed() {
    let r = hwpx(&[], &[("mimetype", b"text/plain")]);
    let f = finding(&r, "HWPX_BAD_MIMETYPE");
    assert_eq!((f.category, f.severity), (Category::Render, Severity::Medium));
    assert!(hwpx(&["mimetype"], &[]).findings.is_empty(), "일부 파서는 mimetype이 없는 파일을 허용한다");
    assert!(hwpx(&[], &[("mimetype", b"application/owpml")]).findings.is_empty());
}

#[test]
fn broken_xml_in_a_content_part_is_malformed_markup() {
    let r = hwpx(&[], &[("Contents/section0.xml", b"<hs:sec><hp:p>")]);
    let f = r.findings.iter().find(|f| f.code == "MALFORMED_MARKUP" && f.location == "Contents/section0.xml").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!((f.category, f.severity), (Category::Render, Severity::High));
}

#[test]
fn bindata_executables_by_name_and_by_magic_are_high() {
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let r = hwpx(&[], &[("BinData/setup.exe", b"not really"), ("BinData/image1.dat", &pe), ("BinData/ok.png", PNG)]);
    let locs: Vec<&str> = r.findings.iter().filter(|f| f.code == "EMBEDDED_EXECUTABLE").map(|f| f.location.as_str()).collect();
    assert!(locs.contains(&"BinData/setup.exe") && locs.contains(&"BinData/image1.dat") && !locs.contains(&"BinData/ok.png"), "{locs:?}");
    assert_eq!(finding(&r, "EMBEDDED_EXECUTABLE").severity, Severity::High);
}

#[test]
fn bindata_ole_objects_are_high_and_plain_images_are_clean() {
    let r = hwpx(&[], &[("BinData/BIN0001.ole", OLE), ("BinData/BIN0002.bin", OLE), ("BinData/image1.png", PNG)]);
    let ole: Vec<&str> = r.findings.iter().filter(|f| f.code == "HWPX_OLE_OBJECT").map(|f| f.location.as_str()).collect();
    assert!(ole.contains(&"BinData/BIN0001.ole") && ole.contains(&"BinData/BIN0002.bin") && !ole.contains(&"BinData/image1.png"), "{ole:?}");
    let f = finding(&r, "HWPX_OLE_OBJECT");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(hwpx(&[], &[("BinData/image1.png", PNG)]).findings.is_empty());
}

#[test]
fn xml_entity_attack_in_a_section_is_reported_with_the_shared_codes() {
    let xxe = br#"<?xml version="1.0"?><!DOCTYPE d [<!ENTITY x SYSTEM "file:///etc/passwd">]><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section">&x;</hs:sec>"#;
    let r = hwpx(&[], &[("Contents/section0.xml", xxe)]);
    let f = finding(&r, "XXE_ENTITY");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::Critical));
}

#[test]
fn external_hyperlink_style_content_is_not_flagged() {
    let sec = br#"<?xml version="1.0"?><hs:sec xmlns:hs="http://www.hancom.co.kr/hwpml/2011/section"><hp:p xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph"><hp:run><hp:ctrl><hp:fieldBegin type="HYPERLINK"><hp:parameters><hp:stringParam name="Command">https://example.com;1;0;0;</hp:stringParam></hp:parameters></hp:fieldBegin></hp:ctrl></hp:run></hp:p></hs:sec>"#;
    assert!(hwpx(&[], &[("Contents/section0.xml", sec)]).findings.is_empty());
}

#[test]
fn plain_zip_with_hwpx_extension_has_no_manifest() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.hwpx");
    common::zip_of(&p, &[("hello.txt", b"hi")]);
    assert!(scan_file(&p, &Options::default()).has("HWPX_NO_MANIFEST"));
}

#[test]
fn hwpx_extension_with_ole_content_is_a_mismatch_and_incomplete_without_claiming_encryption() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("old.hwpx");
    let mut b = common::OLE_MAGIC.to_vec();
    b.resize(2048, 0);
    fs::write(&p, b).unwrap();
    let r = scan_file(&p, &Options::default());
    assert!(r.has("FORMAT_MISMATCH") && !r.has("NOT_ZIP"));
    assert!(r.incomplete && r.reasons.iter().any(|x| x.contains("HWP 5")), "{:?}", r.reasons);
    assert!(!r.reasons.iter().any(|x| FileReport::is_encrypted_reason(x)), "{:?}", r.reasons);
}
