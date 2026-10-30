//! DOCX 구조와 확장자 위장 테스트.

mod common;

use std::fs;
use std::path::Path;

use bookdoctor::cli::Options;
use bookdoctor::scan::scan_file;
use scan_common::{Category, FileReport, Format, Severity};

fn scan(path: &Path) -> FileReport {
    scan_file(path, &Options::default())
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a scan_common::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

fn docx_named(name: &str, remove: &[&str], add: &[(&str, &[u8])]) -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    common::docx_with(&p, remove, add);
    (d, p)
}

#[test]
fn minimal_docx_family_has_no_findings() {
    for name in ["a.docx", "a.docm", "a.dotx", "a.dotm"] {
        let (_d, p) = docx_named(name, &[], &[]);
        let r = scan(&p);
        assert_eq!(r.format, Some(Format::Docx), "{name}");
        assert!(r.findings.is_empty() && !r.incomplete, "{name}: {:?} {:?}", r.findings, r.reasons);
    }
}

#[test]
fn each_missing_required_part_is_reported_with_its_name() {
    for part in ["word/document.xml", "[Content_Types].xml", "_rels/.rels"] {
        let (_d, p) = docx_named("a.docx", &[part], &[]);
        let r = scan(&p);
        let f = r.findings.iter().find(|f| f.code == "OOXML_NOT_DOCX" && f.location == part).unwrap_or_else(|| panic!("{part}: {:?}", r.findings));
        assert_eq!((f.category, f.severity), (Category::Render, Severity::High), "{part}");
    }
}

#[test]
fn broken_xml_in_a_required_part_is_malformed_markup() {
    let (_d, p) = docx_named("a.docx", &[], &[("word/document.xml", b"<w:document><w:body>")]);
    let r = scan(&p);
    let f = r.findings.iter().find(|f| f.code == "MALFORMED_MARKUP" && f.location == "word/document.xml").unwrap_or_else(|| panic!("{:?}", r.findings));
    assert_eq!((f.category, f.severity), (Category::Render, Severity::High));
}

#[test]
fn content_types_declaring_a_missing_part_is_reported() {
    let types = common::DOCX_CONTENT_TYPES.replace("</Types>", r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#);
    let (_d, p) = docx_named("a.docx", &[], &[("[Content_Types].xml", types.as_bytes())]);
    let r = scan(&p);
    let f = finding(&r, "OOXML_PART_MISSING");
    assert_eq!((f.category, f.severity, f.location.as_str()), (Category::Render, Severity::Medium, "word/footer1.xml"));
}

#[test]
fn docx_that_is_plain_text_is_a_mismatch_and_not_a_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.docx");
    fs::write(&p, "그냥 글이다\n".repeat(30)).unwrap();
    let r = scan(&p);
    assert_eq!(finding(&r, "FORMAT_MISMATCH").category, Category::Defect);
    assert!(r.has("NOT_ZIP"));
}

#[test]
fn docx_that_is_a_pdf_is_scanned_as_pdf_and_flagged() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.docx");
    fs::write(&p, b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\n").unwrap();
    let r = scan(&p);
    assert_eq!(r.format, Some(Format::Pdf));
    assert!(r.has("FORMAT_MISMATCH"));
}

#[test]
fn docx_that_is_an_exe_is_a_critical_disguise() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("invoice.docx");
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    fs::write(&p, pe).unwrap();
    let r = scan(&p);
    assert_eq!(finding(&r, "DISGUISED_EXECUTABLE").severity, Severity::Critical);
}

#[test]
fn zip_family_extensions_with_exe_content_are_disguises_too() {
    let d = tempfile::tempdir().unwrap();
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    for name in ["a.zip", "a.docm", "a.hwpx"] {
        let p = d.path().join(name);
        fs::write(&p, &pe).unwrap();
        assert!(scan(&p).has("DISGUISED_EXECUTABLE"), "{name}");
    }
}

#[test]
fn docx_with_ole_content_is_a_mismatch_and_incomplete_but_not_called_encrypted() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("old.docx");
    let mut b = common::OLE_MAGIC.to_vec();
    b.resize(2048, 0);
    fs::write(&p, b).unwrap();
    let r = scan(&p);
    assert!(r.has("FORMAT_MISMATCH"));
    assert!(!r.has("NOT_ZIP"), "OLE content is not reported as a broken zip");
    assert!(r.incomplete);
    assert!(r.reasons.iter().any(|x| x.contains("OLE")), "{:?}", r.reasons);
    // CFB로 읽히지 않는 파일은 암호화인지 구형 .doc인지 알 수 없다. 확인하지 못한 것을 암호화라고 칠하지 않는다.
    assert!(!r.reasons.iter().any(|x| FileReport::is_encrypted_reason(x)), "{:?}", r.reasons);
}

#[test]
fn zip_extension_with_ole_content_is_a_mismatch() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.zip");
    let mut b = common::OLE_MAGIC.to_vec();
    b.resize(2048, 0);
    fs::write(&p, b).unwrap();
    assert!(scan(&p).has("FORMAT_MISMATCH"));
}
