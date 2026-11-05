//! `doc_checker::hwp::record_reader` 단위 테스트.

#[path = "../support/mod.rs"]
mod support;

use crate::ole_common::{make_cfb, make_cfb_v4};
use crate::support::shared_fixtures::{HWPTAG_BEGIN, TAG_SHAPE_COMPONENT_OLE, bin_data_link, file_header, finding, raw_deflate, record, scan, valid_stream, write};
use crate::support::shared_fixtures::{hwp_cfb, scan_bindata, scan_cfb};
use doc_checker::CheckOptions;
use doc_checker::FileReport;
use doc_checker::format_dispatcher::scan_file;
use support::ole_common;

fn scan_sections(flags: u32, docinfo: Option<Vec<u8>>, section0: Vec<u8>) -> FileReport {
    let mut streams: Vec<(&str, Vec<u8>)> = vec![("BodyText/Section0", section0)];
    if let Some(d) = docinfo {
        streams.push(("DocInfo", d));
    }
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    scan_cfb("a.hwp", &hwp_cfb(&refs, flags))
}

#[test]
fn a_docinfo_that_cannot_be_decompressed_is_incomplete() {
    let r = scan_bindata(0x1, Some(vec![0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]), &[]);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("DocInfo")), "{:?}", r.reasons);
}

#[test]
fn valid_records_are_clean_plain_and_compressed() {
    let r = scan_sections(0, Some(valid_stream()), valid_stream());
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    let r = scan_sections(0x1, Some(raw_deflate(&valid_stream())), raw_deflate(&valid_stream()));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn size_past_the_end_is_a_high_defect() {
    let mut bad = valid_stream();
    bad.extend_from_slice(&(HWPTAG_BEGIN | 100 << 20).to_le_bytes());
    bad.extend_from_slice(&[0u8; 10]);
    let r = scan_sections(0, None, bad.clone());
    let f = finding(&r, "HWP_BAD_RECORD");
    assert_eq!((f.severity, f.category), (doc_checker::Severity::High, doc_checker::Category::Defect));
    assert!(f.location.contains("Section0"), "{}", f.location);
    // 압축된 본문 안에서도 같다. DocInfo도 본다.
    assert!(scan_sections(0x1, None, raw_deflate(&bad)).has("HWP_BAD_RECORD"));
    assert!(scan_sections(0, Some(bad), valid_stream()).findings.iter().any(|f| f.code == "HWP_BAD_RECORD" && f.location == "DocInfo"));
}

#[test]
fn truncated_extended_length_is_a_high_defect() {
    // 확장 길이(0xFFF)인데 길이 DWORD가 잘렸거나 선언 길이가 남은 바이트를 넘는다.
    let mut cut = valid_stream();
    cut.extend_from_slice(&(HWPTAG_BEGIN | 0xFFF << 20).to_le_bytes());
    cut.extend_from_slice(&[1, 2]);
    assert!(scan_sections(0, None, cut).has("HWP_BAD_RECORD"));
    let mut huge = valid_stream();
    huge.extend_from_slice(&(HWPTAG_BEGIN | 0xFFF << 20).to_le_bytes());
    huge.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(scan_sections(0, None, huge).has("HWP_BAD_RECORD"));
    // 4095바이트 이상의 정상 확장 레코드는 문제없다.
    let ok = [record(HWPTAG_BEGIN + 51, 1, &vec![0u8; 5000]), valid_stream()].concat();
    assert!(!scan_sections(0, None, ok).has("HWP_BAD_RECORD"));
}

#[test]
fn level_jump_is_medium_and_reported_once() {
    let jump = [record(HWPTAG_BEGIN + 50, 0, &[0u8; 4]), record(HWPTAG_BEGIN + 51, 3, &[0u8; 4]), record(HWPTAG_BEGIN + 51, 9, &[0u8; 4])].concat();
    let r = scan_sections(0, None, jump);
    assert_eq!(finding(&r, "HWP_RECORD_LEVEL_JUMP").severity, doc_checker::Severity::Medium);
    assert_eq!(r.findings.iter().filter(|f| f.code == "HWP_RECORD_LEVEL_JUMP").count(), 1);
}

#[test]
fn ole_shape_records_are_high() {
    let body = [record(HWPTAG_BEGIN + 50, 0, &[0u8; 4]), record(TAG_SHAPE_COMPONENT_OLE, 1, &[0u8; 24])].concat();
    assert_eq!(finding(&scan_sections(0, None, body), "HWP_OLE_OBJECT").severity, doc_checker::Severity::High);
}

#[test]
fn a_section_that_cannot_be_decompressed_is_incomplete() {
    let r = scan_sections(0x1, None, vec![0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("Section0")), "{:?}", r.reasons);
}

#[test]
fn record_count_limit_is_incomplete_and_zero_headers_are_linear() {
    // 크기 0 레코드 한 개가 4바이트다. 상한(5,000,000)을 넘는 스트림은 미완료다.
    let many = vec![0u8; 4 * 5_000_100];
    let (_d, p) = write("many.hwp", &make_cfb_v4(&[("FileHeader", &file_header(0)[..]), ("BodyText/Section0", &many[..])]).bytes);
    let started = std::time::Instant::now();
    let r = scan_file(&p, &CheckOptions::default());
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("records")), "{:?}", r.reasons);
    // 1MiB의 0은 레코드 262,144개이고 정상이다.
    let r = scan_sections(0, None, vec![0u8; 1 << 20]);
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn a_failed_decompression_uses_up_the_section_budget() {
    // 둘째 섹션이 상한을 넘게 풀리면 그 작업도 예산을 쓴 것이다. 이후 섹션은 검증하지 않고 미완료로 남긴다.
    let bad = {
        let mut b = valid_stream();
        b.extend_from_slice(&(HWPTAG_BEGIN | 100 << 20).to_le_bytes());
        b
    };
    let big = raw_deflate(&vec![0u8; 70 * 1024 * 1024]);
    let small = raw_deflate(&valid_stream());
    let cfb = make_cfb_v4(&[("FileHeader", &file_header(0x1)[..]), ("BodyText/Section0", &small[..]), ("BodyText/Section1", &big[..]), ("BodyText/Section2", &raw_deflate(&bad)[..])]);
    let r = scan("a.hwp", &cfb.bytes);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("Section1")), "{:?}", r.reasons);
    assert!(r.reasons.iter().any(|m| m.contains("budget") || m.contains("not validated")), "{:?}", r.reasons);
    assert!(!r.has("HWP_BAD_RECORD"), "a section after the exhausted budget must not be decompressed: {:?}", r.findings);
}

#[test]
fn sections_and_docinfo_with_duplicate_or_case_colliding_names_are_all_scanned() {
    let bad = {
        let mut b = valid_stream();
        b.extend_from_slice(&(HWPTAG_BEGIN | 100 << 20).to_le_bytes());
        b
    };
    let ok = valid_stream();
    // 같은 이름의 섹션이 둘이면 둘 다 본다(경로 조회는 첫 항목만 돌려준다).
    let cfb = make_cfb(&[("FileHeader", &file_header(0)[..]), ("BodyText/Section0", &ok[..]), ("BodyText/Section0", &bad[..])]);
    assert!(scan("a.hwp", &cfb.bytes).has("HWP_BAD_RECORD"));
    let cfb = make_cfb(&[("FileHeader", &file_header(0)[..]), ("BodyText/Section0", &ok[..]), ("BodyText/SECTION0", &bad[..])]);
    assert!(scan("b.hwp", &cfb.bytes).has("HWP_BAD_RECORD"));
    // 같은 이름의 DocInfo가 둘이어도 연결 파일을 놓치지 않는다.
    let link = bin_data_link("\\\\evil\\s\\a", "a");
    let cfb = make_cfb(&[("FileHeader", &file_header(0)[..]), ("DocInfo", &ok[..]), ("DocInfo", &link[..])]);
    assert!(scan("c.hwp", &cfb.bytes).has("HWP_LINKED_FILE"));
}
