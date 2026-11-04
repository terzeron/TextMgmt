//! `bookdoctor::hwp::bindata_checker` 단위 테스트.

#[path = "../ole_common.rs"]
mod ole_common;
use std::fs;
use std::path::PathBuf;
use bookdoctor::cli_parser::Options;
use bookdoctor::format_dispatcher::scan_file;
use crate::ole_common::{Cfb, make_cfb};
use bookdoctor::{FileReport};
use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};
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

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a bookdoctor::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

// --- Scripts (명세 표 8, 9) ---

fn wchars(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
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

fn pe_bytes() -> Vec<u8> {
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    pe
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

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
