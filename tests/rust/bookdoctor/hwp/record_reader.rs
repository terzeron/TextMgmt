//! `bookdoctor::hwp::record_reader` 단위 테스트.

#[path = "../ole_common.rs"]
mod ole_common;
use std::fs;
use std::path::PathBuf;
use bookdoctor::cli_parser::Options;
use bookdoctor::format_dispatcher::scan_file;
use crate::ole_common::{Cfb, make_cfb, make_cfb_v4};
use bookdoctor::{FileReport};
use flate2::Compression;
use flate2::write::{DeflateEncoder};
use std::io::Write;

/// `FileHeader` 스트림: 서명 32, 버전 4, 속성1 4, 속성2 4, EncryptVersion 4, KOGL 1, 예약 207 = 256바이트(명세 표 3).
fn file_header(flags: u32) -> Vec<u8> {
    let mut h = vec![0u8; 256];
    h[..17].copy_from_slice(b"HWP Document File");
    h[32..36].copy_from_slice(&0x0500_0300u32.to_le_bytes());
    h[36..40].copy_from_slice(&flags.to_le_bytes());
    h
}

fn hwp_cfb(extra: &[(&str, &[u8])], flags: u32) -> Cfb {
    let header = file_header(flags);
    let mut streams: Vec<(&str, &[u8])> = vec![("FileHeader", &header)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, bytes).unwrap();
    (d, p)
}

fn scan(name: &str, bytes: &[u8]) -> FileReport {
    let (_d, p) = write(name, bytes);
    scan_file(&p, &Options::default())
}

fn scan_cfb(name: &str, cfb: &Cfb) -> FileReport {
    scan(name, &cfb.bytes)
}

fn scan_bindata(flags: u32, docinfo: Option<Vec<u8>>, bins: &[(&str, Vec<u8>)]) -> FileReport {
    let mut streams: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(d) = docinfo {
        streams.push(("DocInfo".to_string(), d));
    }
    for (name, data) in bins {
        streams.push((format!("BinData/{name}"), data.clone()));
    }
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    scan_cfb("a.hwp", &hwp_cfb(&refs, flags))
}

fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

// --- BinData (명세 표 17, 18) ---

/// 레코드 하나: 헤더 32비트(TagID 10, Level 10, Size 12), Size가 0xFFF 이상이면 뒤에 DWORD 길이(명세 4.1).
fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    if data.len() >= 0xFFF {
        out.extend_from_slice(&(tag | level << 10 | 0xFFF << 20).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    } else {
        out.extend_from_slice(&(tag | level << 10 | (data.len() as u32) << 20).to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

const HWPTAG_BEGIN: u32 = 0x010;

fn scan_sections(flags: u32, docinfo: Option<Vec<u8>>, section0: Vec<u8>) -> FileReport {
    let mut streams: Vec<(&str, Vec<u8>)> = vec![("BodyText/Section0", section0)];
    if let Some(d) = docinfo {
        streams.push(("DocInfo", d));
    }
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    scan_cfb("a.hwp", &hwp_cfb(&refs, flags))
}

fn valid_stream() -> Vec<u8> {
    [record(HWPTAG_BEGIN + 50, 0, &[0u8; 22]), record(HWPTAG_BEGIN + 51, 1, &[0u8; 8]), record(HWPTAG_BEGIN + 55, 1, &[0u8; 8]), record(HWPTAG_BEGIN + 56, 2, &[0u8; 4]), record(HWPTAG_BEGIN + 50, 0, &[0u8; 22])].concat()
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a bookdoctor::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

// --- 레코드 검증 (명세 4.1) ---

const TAG_SHAPE_COMPONENT_OLE: u32 = HWPTAG_BEGIN + 68;

// --- Scripts (명세 표 8, 9) ---

fn wchars(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

const TAG_BIN_DATA: u32 = HWPTAG_BEGIN + 2;

fn wstr_with_len(s: &str) -> Vec<u8> {
    let mut out = (s.encode_utf16().count() as u16).to_le_bytes().to_vec();
    out.extend_from_slice(&wchars(s));
    out
}

/// `HWPTAG_BIN_DATA` LINK(Type 0): 속성, 절대 경로, 상대 경로.
fn bin_data_link(absolute: &str, relative: &str) -> Vec<u8> {
    let mut d = 0u16.to_le_bytes().to_vec();
    d.extend_from_slice(&wstr_with_len(absolute));
    d.extend_from_slice(&wstr_with_len(relative));
    record(TAG_BIN_DATA, 1, &d)
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
    assert_eq!((f.severity, f.category), (bookdoctor::Severity::High, bookdoctor::Category::Defect));
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
    assert_eq!(finding(&r, "HWP_RECORD_LEVEL_JUMP").severity, bookdoctor::Severity::Medium);
    assert_eq!(r.findings.iter().filter(|f| f.code == "HWP_RECORD_LEVEL_JUMP").count(), 1);
}

#[test]
fn ole_shape_records_are_high() {
    let body = [record(HWPTAG_BEGIN + 50, 0, &[0u8; 4]), record(TAG_SHAPE_COMPONENT_OLE, 1, &[0u8; 24])].concat();
    assert_eq!(finding(&scan_sections(0, None, body), "HWP_OLE_OBJECT").severity, bookdoctor::Severity::High);
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
    let r = scan_file(&p, &Options::default());
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
