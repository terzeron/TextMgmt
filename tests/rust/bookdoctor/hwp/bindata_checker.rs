//! `bookdoctor::hwp::bindata_checker` 단위 테스트.

#[path = "../support/mod.rs"]
mod support;

use crate::support::shared_fixtures::{HWPTAG_BEGIN, TAG_BIN_DATA, bin_data_link, finding, pe_bytes, raw_deflate, record, wstr_with_len, zlib};
use crate::support::shared_fixtures::scan_bindata;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR some png bytes";

#[test]
fn linked_file_unc_is_high_and_local_is_medium() {
    for (path, severity) in [("\\\\evil\\share\\a.exe", bookdoctor::Severity::High), ("http://evil.example/a", bookdoctor::Severity::High), ("C:\\docs\\a.png", bookdoctor::Severity::Medium)] {
        let r = scan_bindata(0, Some(bin_data_link(path, "a")), &[]);
        let f = finding(&r, "HWP_LINKED_FILE");
        assert_eq!(f.severity, severity, "{path}");
    }
    // 연결 문서(`Type`이 EMBEDDING)는 외부 경로가 아니다.
    let mut embedding = 1u16.to_le_bytes().to_vec();
    embedding.extend_from_slice(&1u16.to_le_bytes());
    embedding.extend_from_slice(&wstr_with_len("png"));
    assert!(!scan_bindata(0, Some(record(TAG_BIN_DATA, 1, &embedding)), &[]).has("HWP_LINKED_FILE"));
}

#[test]
fn linked_file_in_a_compressed_docinfo_is_found() {
    let r = scan_bindata(0x1, Some(raw_deflate(&bin_data_link("\\\\evil\\s\\a", "a"))), &[]);
    assert!(r.has("HWP_LINKED_FILE") && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn pe_in_a_compressed_bindata_is_flagged_by_content() {
    let r = scan_bindata(0x1, None, &[("BIN0001.jpg", raw_deflate(&pe_bytes())), ("BIN0002.jpg", zlib(&pe_bytes()))]);
    assert_eq!(r.findings.iter().filter(|f| f.code == "EMBEDDED_EXECUTABLE").count(), 2, "{:?}", r.findings);
    assert!(r.findings.iter().all(|f| f.severity != bookdoctor::Severity::Low));
}

#[test]
fn pe_with_a_png_name_is_still_flagged_and_an_exe_name_is_flagged_by_name() {
    // 압축 비트가 없는 문서, 압축 비트가 있지만 스트림은 압축하지 않은 경우(속성 0x20), 이름만 png인 PE.
    assert!(scan_bindata(0, None, &[("BIN0001.png", pe_bytes())]).has("EMBEDDED_EXECUTABLE"));
    assert!(scan_bindata(0x1, None, &[("BIN0001.png", pe_bytes())]).has("EMBEDDED_EXECUTABLE"));
    // 이름이 exe면 내용이 PNG여도 High다.
    let r = scan_bindata(0, None, &[("BIN0001.exe", PNG.to_vec())]);
    assert_eq!(finding(&r, "EMBEDDED_EXECUTABLE").severity, bookdoctor::Severity::Critical);
}

#[test]
fn ole_and_eps_payloads() {
    let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.extend_from_slice(&[0u8; 64]);
    let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n".to_vec();
    let mut dos_eps = vec![0xC5, 0xD0, 0xD3, 0xC6];
    dos_eps.extend_from_slice(&[0u8; 60]);
    let r = scan_bindata(0x1, None, &[("BIN0001.ole", raw_deflate(&ole)), ("BIN0002.eps", raw_deflate(&eps)), ("BIN0003.eps", dos_eps)]);
    assert_eq!(finding(&r, "HWP_OLE_OBJECT").severity, bookdoctor::Severity::High);
    assert_eq!(finding(&r, "HWP_EPS_OBJECT").severity, bookdoctor::Severity::Medium);
    assert_eq!(r.findings.iter().filter(|f| f.code == "HWP_EPS_OBJECT").count(), 2, "{:?}", r.findings);
}

#[test]
fn plain_png_bindata_is_clean() {
    let r = scan_bindata(0x1, Some(raw_deflate(&record(HWPTAG_BEGIN, 0, &[0u8; 8]))), &[("BIN0001.png", PNG.to_vec()), ("BIN0002.png", raw_deflate(PNG))]);
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn decompression_bombs_are_bounded() {
    // BinData는 앞 4KiB만 풀기 때문에 폭탄이 있어도 빠르고 미완료도 아니다.
    let started = std::time::Instant::now();
    let r = scan_bindata(0x1, None, &[("BIN0001.bin", raw_deflate(&vec![0u8; 64 * 1024 * 1024]))]);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    assert!(!r.has("EMBEDDED_EXECUTABLE") && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    // DocInfo는 전부 풀어야 하므로 상한을 넘으면 미완료다.
    let r = scan_bindata(0x1, Some(raw_deflate(&vec![0u8; 20 * 1024 * 1024])), &[]);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("DocInfo")), "{:?}", r.reasons);
}
