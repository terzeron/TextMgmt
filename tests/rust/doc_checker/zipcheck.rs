//! `doc_checker::zipcheck` 단위 테스트. ZIP·DOCX·HWPX 진입 함수가 ZIP 검사를 거치는지 본다.

use std::io::Write;
use std::path::Path;

use doc_checker::CheckOptions;
use doc_checker::zipcheck::{scan_docx_file, scan_hwpx_file, scan_zip_file};
use doc_checker::{FileReport, Format};

fn write_zip(path: &Path, members: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in members {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn a_plain_zip_is_checked_without_findings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.zip");
    write_zip(&path, &[("readme.txt", b"hello")]);
    let mut report = FileReport::new(&path, Some(Format::Zip));
    scan_zip_file(&path, &CheckOptions::default(), &mut report);
    assert!(!report.incomplete, "{:?}", report.reasons);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn a_docx_that_is_not_a_zip_stops_before_the_ooxml_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.docx");
    std::fs::write(&path, b"not a zip archive").unwrap();
    let mut report = FileReport::new(&path, Some(Format::Docx));
    scan_docx_file(&path, &CheckOptions::default(), &mut report);
    assert!(report.incomplete || !report.findings.is_empty(), "열 수 없는 ZIP을 정상으로 보고하지 않는다");
    assert!(!report.findings.iter().any(|f| f.code.starts_with("OOXML_")), "{:?}", report.findings);
}

#[test]
fn a_hwpx_zip_goes_through_the_package_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.hwpx");
    write_zip(&path, &[("mimetype", b"application/hwp+zip")]);
    let mut report = FileReport::new(&path, Some(Format::Hwpx));
    scan_hwpx_file(&path, &CheckOptions::default(), &mut report);
    assert!(report.incomplete || !report.findings.is_empty(), "필수 부분이 빠진 HWPX를 정상으로 보고하지 않는다");
}
