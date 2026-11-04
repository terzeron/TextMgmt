//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]
use super::epub_with_files;
use super::ole_common::{Cfb, make_cfb};
use bookdoctor::cli_parser::Options;
use bookdoctor::cli_parser::{execute, parse};
use bookdoctor::epub::{security_checker::Limits, security_checker::Stats, security_checker::scan as scan_into, security_checker::scan_stats};
use bookdoctor::format_dispatcher::scan_file;
use bookdoctor::{FileReport, Format};
use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

pub const HWPTAG_BEGIN: u32 = 0x010;

pub const JSCRIPT_VERSION: [u8; 8] = [0u8; 8];

pub const OLE_STUB: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 macro";

pub const TAG_BIN_DATA: u32 = HWPTAG_BEGIN + 2;

// --- 레코드 검증 (명세 4.1) ---

pub const TAG_SHAPE_COMPONENT_OLE: u32 = HWPTAG_BEGIN + 68;

/// `HWPTAG_BIN_DATA` LINK(Type 0): 속성, 절대 경로, 상대 경로.
pub fn bin_data_link(absolute: &str, relative: &str) -> Vec<u8> {
    let mut d = 0u16.to_le_bytes().to_vec();
    d.extend_from_slice(&wstr_with_len(absolute));
    d.extend_from_slice(&wstr_with_len(relative));
    record(TAG_BIN_DATA, 1, &d)
}

pub fn binary(len: usize, seed: u8) -> Vec<u8> {
    // UTF-8로 유효하지 않은 바이트가 많이 든 데이터(이미지·폰트처럼).
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) | 0x80).collect()
}

pub fn bytes_of(build: impl FnOnce(&Path)) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("x");
    build(&p);
    fs::read(&p).unwrap()
}

pub fn codes(r: &FileReport) -> Vec<&str> {
    r.findings.iter().map(|f| f.code.as_str()).collect()
}

/// `DefaultJScript`: 헤더·소스·Pre·Post 각각 DWORD 길이(글자 수) + WCHAR 배열, 끝에 DWORD `-1`.
pub fn default_jscript(parts: [&str; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        out.extend_from_slice(&(p.encode_utf16().count() as u32).to_le_bytes());
        out.extend_from_slice(&wchars(p));
    }
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out
}

/// EICAR 표준 시험 문자열(악성 코드가 아니다). 저장소 파일이 백신에 걸리지 않도록 두 조각으로 둔다.
pub fn eicar() -> Vec<u8> {
    [&b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$"[..], &b"EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*"[..]].concat()
}

pub fn epub_bytes(chapter: &str, opf: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("t.epub");
    epub_with_files(&p, opf, chapter, extra);
    fs::read(p).unwrap()
}

/// `FileHeader` 스트림: 서명 32, 버전 4, 속성1 4, 속성2 4, EncryptVersion 4, KOGL 1, 예약 207 = 256바이트(명세 표 3).
pub fn file_header(flags: u32) -> Vec<u8> {
    let mut h = vec![0u8; 256];
    h[..17].copy_from_slice(b"HWP Document File");
    h[32..36].copy_from_slice(&0x0500_0300u32.to_le_bytes());
    h[36..40].copy_from_slice(&flags.to_le_bytes());
    h
}

pub fn finding<'a>(r: &'a FileReport, code: &str) -> &'a bookdoctor::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

/// 중앙 디렉터리와 로컬 헤더의 선언 크기(비압축)를 `declared`로 바꾼다.
pub fn forge_declared_size(path: &Path, member: &str, declared: u32) {
    let mut data = fs::read(path).unwrap();
    let name = member.as_bytes();
    let local = (0..data.len() - 30).find(|&i| &data[i..i + 4] == b"PK\x03\x04" && &data[i + 30..i + 30 + name.len()] == name).unwrap();
    data[local + 22..local + 26].copy_from_slice(&declared.to_le_bytes());
    let central = (0..data.len() - 46).find(|&i| &data[i..i + 4] == b"PK\x01\x02" && &data[i + 46..i + 46 + name.len()] == name).unwrap();
    data[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
    fs::write(path, data).unwrap();
}

pub fn has(r: &FileReport, code: &str) -> bool {
    r.findings.iter().any(|f| f.code == code)
}

pub fn has_vba_under(r: &FileReport, prefix: &str) -> bool {
    r.findings.iter().any(|f| f.code == "OOXML_VBA_PROJECT" && f.location.starts_with(prefix) && f.location.contains("vbaProject.bin"))
}

pub fn member(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
    s
}

pub fn names(bytes: &[u8]) -> Vec<String> {
    zip::ZipArchive::new(Cursor::new(bytes)).unwrap().file_names().map(|n| n.unwrap().into_owned()).collect()
}

pub fn opts() -> Options {
    Options { no_pdf_worker: true, ..Options::default() }
}

// ---- 손상된 ZIP 필드 ----

/// 중앙 디렉토리 항목의 오프셋 필드를 고친다. `field`는 항목 시작 기준 오프셋이다.
pub fn patch_central(path: &Path, member: &str, field: usize, bytes: &[u8]) {
    let mut data = fs::read(path).unwrap();
    let name = member.as_bytes();
    let at = (0..data.len() - 46 - name.len()).find(|&i| &data[i..i + 4] == b"PK\x01\x02" && &data[i + 46..i + 46 + name.len()] == name).expect("central header");
    data[at + field..at + field + bytes.len()].copy_from_slice(bytes);
    fs::write(path, data).unwrap();
}

/// 객체 본문 목록으로 PDF를 만든다. 1번 객체가 Catalog, 2번이 Pages, 3번이 Page다.
pub fn pdf(objects: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).bytes());
    out
}

pub fn pe_bytes() -> Vec<u8> {
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    pe
}

pub fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

// --- BinData (명세 표 17, 18) ---

/// 레코드 하나: 헤더 32비트(TagID 10, Level 10, Size 12), Size가 0xFFF 이상이면 뒤에 DWORD 길이(명세 4.1).
pub fn record(tag: u32, level: u32, data: &[u8]) -> Vec<u8> {
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

// ---- 실행: 적용, 휴지통 ----

/// `root`를 코퍼스 루트로 삼아 명령을 실행한다. 휴지통은 `root/0_trash`다.
pub fn run_in(root: &Path, args: &[&str]) -> (i32, String) {
    let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let code = match parse(&argv) {
        Ok(o) => execute(&Options { library_root: root.to_path_buf(), ..o }, &mut out, false, true),
        Err(_) => 2,
    };
    (code, String::from_utf8_lossy(&out).into_owned())
}

/// 경로의 파일을 기본 옵션으로 검사한다.
pub fn scan_path(path: &Path) -> FileReport {
    scan_file(path, &Options::default())
}

pub fn scan(name: &str, bytes: &[u8]) -> FileReport {
    let (_d, p) = write(name, bytes);
    scan_file(&p, &Options::default())
}

pub fn scan_bytes(name: &str, data: &[u8], opts: &Options) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, data).unwrap();
    scan_file(&p, opts)
}

pub fn scan_with(path: &Path, limits: &Limits) -> FileReport {
    let mut r = FileReport::new(path, Some(Format::Epub));
    scan_into(path, limits, &mut r);
    r
}

pub fn scan_with_stats(path: &Path, limits: &Limits) -> (FileReport, Stats) {
    let mut r = FileReport::new(path, Some(Format::Epub));
    let stats = scan_stats(path, limits, &mut r);
    (r, stats)
}

pub fn valid_stream() -> Vec<u8> {
    [
        record(HWPTAG_BEGIN + 50, 0, &[0u8; 22]),
        record(HWPTAG_BEGIN + 51, 1, &[0u8; 8]),
        record(HWPTAG_BEGIN + 55, 1, &[0u8; 8]),
        record(HWPTAG_BEGIN + 56, 2, &[0u8; 4]),
        record(HWPTAG_BEGIN + 50, 0, &[0u8; 22]),
    ]
    .concat()
}

/// 매크로 프로젝트가 든 `.docx`의 바이트(매크로 없는 형식에 든 `vbaProject.bin`이라 Critical).
pub fn vba_docx() -> Vec<u8> {
    bytes_of(|p| super::docx_with(p, &[], &[("word/vbaProject.bin", OLE_STUB)]))
}

// --- Scripts (명세 표 8, 9) ---

pub fn wchars(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// `WordDocument` 스트림: FIB 32바이트(wIdent, nFib, flags1)를 512바이트로 채우고 본문을 붙인다.
pub fn word_stream(flags1: u16, body: &[u8]) -> Vec<u8> {
    let mut s = vec![0u8; 512];
    s[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
    s[2..4].copy_from_slice(&0x00C1u16.to_le_bytes());
    s[10..12].copy_from_slice(&flags1.to_le_bytes());
    s.extend_from_slice(body);
    s
}

pub fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, bytes).unwrap();
    (d, p)
}

pub fn wstr_with_len(s: &str) -> Vec<u8> {
    let mut out = (s.encode_utf16().count() as u16).to_le_bytes().to_vec();
    out.extend_from_slice(&wchars(s));
    out
}

pub fn zip_bytes(members: &[(&str, &[u8])]) -> Vec<u8> {
    bytes_of(|p| super::zip_of(p, members))
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

pub fn doc_cfb(extra: &[(&str, &[u8])], flags1: u16, body: &[u8]) -> Cfb {
    let word = word_stream(flags1, body);
    let mut streams: Vec<(&str, &[u8])> = vec![("WordDocument", &word)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

pub fn hwp_cfb(extra: &[(&str, &[u8])], flags: u32) -> Cfb {
    let header = file_header(flags);
    let mut streams: Vec<(&str, &[u8])> = vec![("FileHeader", &header)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

pub fn scan_bindata(flags: u32, docinfo: Option<Vec<u8>>, bins: &[(&str, Vec<u8>)]) -> FileReport {
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

pub fn scan_cfb(name: &str, cfb: &Cfb) -> FileReport {
    scan(name, &cfb.bytes)
}
