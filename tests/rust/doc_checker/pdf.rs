//! `doc_checker::pdf` 단위 테스트. 하위 검사기로 보내기 전의 읽기·머리 확인 분기를 본다.

use std::path::Path;

use doc_checker::CheckOptions;
use doc_checker::pdf::scan_pdf_file;
use doc_checker::{FileReport, Format};

fn options() -> CheckOptions {
    CheckOptions { no_pdf_worker: true, ..CheckOptions::default() }
}

fn scan(path: &Path, opts: &CheckOptions) -> FileReport {
    let mut report = FileReport::new(path, Some(Format::Pdf));
    scan_pdf_file(path, opts, &mut report);
    report
}

#[test]
fn an_unreadable_path_is_incomplete() {
    let dir = tempfile::tempdir().unwrap();
    let report = scan(&dir.path().join("missing.pdf"), &options());
    assert!(report.incomplete);
    assert!(report.reasons.iter().any(|r| r.starts_with("unreadable")), "{:?}", report.reasons);
}

#[test]
fn a_file_without_the_pdf_header_is_incomplete_but_still_inspected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not.pdf");
    std::fs::write(&path, b"plain text, not a pdf").unwrap();
    let report = scan(&path, &options());
    assert!(report.incomplete);
    assert!(report.reasons.iter().any(|r| r == "no %PDF- header"), "{:?}", report.reasons);
}

#[test]
fn a_file_past_the_read_limit_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.pdf");
    std::fs::write(&path, b"%PDF-1.4\n").unwrap();
    let report = scan(&path, &CheckOptions { max_total_mb: 0, ..options() });
    assert!(report.incomplete);
    assert!(report.reasons.iter().any(|r| r.contains("grew past --max-total-mb")), "{:?}", report.reasons);
}

#[test]
fn a_file_with_the_header_is_parsed_instead_of_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.pdf");
    std::fs::write(&path, b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n").unwrap();
    let report = scan(&path, &options());
    assert!(!report.reasons.iter().any(|r| r == "no %PDF- header"), "{:?}", report.reasons);
}
