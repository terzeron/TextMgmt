//! HWP 5.x 테스트. fixture는 합성이다: 코퍼스에 정상 HWP5 파일이 없다(`docs/bookdoctor-hwp5-notes.md`).

#[path = "../ole_reader/common.rs"]
mod ole_common;

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli::{Options, run};
use bookdoctor::scan::scan_file;
use ole_common::{Cfb, compress, dir_stream, make_cfb, make_cfb_v4, module_stream};
use scan_common::{FileReport, Format};

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

const HWP3_SIGNATURE: &[u8] = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05";

fn pe_bytes() -> Vec<u8> {
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    pe
}

#[test]
fn hwp_is_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    // 깨끗한 파일은 출력에 나오지 않으므로 이름 불일치가 나오는 내용을 쓴다.
    for name in ["a.hwp", "b.HWP", "c.hwpp"] {
        fs::write(d.path().join(name), "그냥 텍스트\n".repeat(50)).unwrap();
    }
    let mut out = Vec::new();
    run(&["-r".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("a.hwp") && out.contains("b.HWP"), "{out}");
    assert!(!out.contains("c.hwpp"), "{out}");
    assert_eq!(scan("x.hwp", &hwp_cfb(&[], 0).bytes).format, Some(Format::Hwp));
}

#[test]
fn hwp3_signature_is_recognized_and_not_incomplete() {
    let mut bytes = HWP3_SIGNATURE.to_vec();
    bytes.extend(vec![0u8; 200]);
    let r = scan("old.hwp", &bytes);
    assert_eq!(r.format, Some(Format::Hwp));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn hwp_with_other_content_is_a_mismatch() {
    let r = scan("a.hwp", "그냥 텍스트\n".repeat(50).as_bytes());
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    assert!(r.incomplete, "unanalyzed content must not be reported clean: {:?}", r.reasons);
    let r = scan("b.hwp", &pe_bytes());
    assert!(r.has("DISGUISED_EXECUTABLE"), "{:?}", r.findings);
}

#[test]
fn a_zip_named_hwp_is_scanned_as_hwpx() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("renamed.hwp");
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    z.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"hi").unwrap();
    z.finish().unwrap();
    let r = scan_file(&p, &Options::default());
    assert_eq!(r.format, Some(Format::Hwpx));
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
}

#[test]
fn hwpx_named_ole_with_a_fileheader_is_analyzed_as_hwp() {
    let r = scan("old.hwpx", &hwp_cfb(&[], 0).bytes);
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    assert!(!r.incomplete, "{:?}", r.reasons);
    // FileHeader가 없는 OLE는 HWP인지 알 수 없으므로 P1처럼 일반 미완료다.
    let r = scan("odd.hwpx", &make_cfb(&[("Something", &[1u8, 2, 3][..])]).bytes);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("OLE")), "{:?}", r.reasons);
}

fn scan_cfb(name: &str, cfb: &Cfb) -> FileReport {
    scan(name, &cfb.bytes)
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a scan_common::Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

#[test]
fn plain_and_compressed_headers_are_clean() {
    for flags in [0u32, 0x1, 0x1 | 0x20 | 0x40] {
        let r = scan_cfb("a.hwp", &hwp_cfb(&[], flags));
        assert!(r.findings.is_empty() && !r.incomplete, "flags {flags:#x}: {:?} {:?}", r.findings, r.reasons);
    }
}

#[test]
fn password_drm_and_certificate_flags_are_incomplete_and_marked_encrypted() {
    // 명세 표 3: bit1 암호, bit4 DRM, bit8 공인 인증서 암호화, bit10 공인 인증서 DRM.
    for (bit, what) in [(1u32, "password"), (4, "DRM"), (8, "certificate encryption"), (10, "certificate DRM")] {
        let r = scan_cfb("a.hwp", &hwp_cfb(&[], 1 << bit));
        assert!(r.incomplete, "bit {bit}");
        assert!(r.reasons.iter().any(|m| FileReport::is_encrypted_reason(m) && m.contains(what)), "bit {bit}: {:?}", r.reasons);
        assert!(!r.has("FORMAT_MISMATCH"), "bit {bit}");
    }
    // 같은 비트를 담은 .hwpx 이름도 같다.
    assert!(scan_cfb("b.hwpx", &hwp_cfb(&[], 1 << 1)).reasons.iter().any(|m| FileReport::is_encrypted_reason(m)));
}

#[test]
fn bad_signature_and_short_header_are_defects() {
    let mut bad = file_header(0);
    bad[..17].copy_from_slice(b"Not A Hwp Doc!!!!");
    let short = vec![0u8; 100];
    for (name, header) in [("bad signature", bad), ("short", short)] {
        let r = scan_cfb("a.hwp", &make_cfb(&[("FileHeader", &header[..])]));
        let f = finding(&r, "HWP_BAD_HEADER");
        assert_eq!(f.severity, scan_common::Severity::Medium, "{name}");
        assert_eq!(f.category, scan_common::Category::Defect, "{name}");
    }
}

#[test]
fn distribution_doc_without_bodytext_is_incomplete() {
    let section = [0u8; 16];
    let r = scan_cfb("a.hwp", &hwp_cfb(&[("ViewText/Section0", &section[..])], 0x4));
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("ViewText")), "{:?}", r.reasons);
    assert!(!r.reasons.iter().any(|m| FileReport::is_encrypted_reason(m)), "{:?}", r.reasons);
    // 배포용 비트가 있어도 BodyText가 있으면 평소처럼 읽는다.
    let r = scan_cfb("b.hwp", &hwp_cfb(&[("BodyText/Section0", &section[..])], 0x4));
    assert!(!r.incomplete, "{:?}", r.reasons);
}

#[test]
fn ole_without_a_fileheader_is_a_mismatch_defect_and_not_clean() {
    for name in ["a.hwp", "b.hwpx"] {
        let r = scan_cfb(name, &make_cfb(&[("Something", &[1u8, 2, 3][..])]));
        assert!(r.incomplete, "{name}: unanalyzed OLE must not be reported clean: {:?}", r.reasons);
    }
    let r = scan_cfb("a.hwp", &make_cfb(&[("Something", &[1u8, 2, 3][..])]));
    let f = finding(&r, "FORMAT_MISMATCH");
    assert_eq!(f.location, "FileHeader");
}

#[test]
fn a_word_document_named_hwp_is_analyzed_as_doc() {
    let dir = compress(&dir_stream(&[("Module1", "Module1", 16)]));
    let module = module_stream(16, "Sub AutoOpen()\nShell \"calc\"\nEnd Sub\n");
    let mut word = vec![0u8; 512];
    word[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
    for name in ["renamed.hwp", "renamed.hwpx"] {
        let cfb = make_cfb(&[("WordDocument", &word[..]), ("Macros/VBA/dir", &dir[..]), ("Macros/VBA/Module1", &module[..])]);
        let r = scan_cfb(name, &cfb);
        assert!(r.has("FORMAT_MISMATCH"), "{name}: {:?}", r.findings);
        assert!(r.has("DOC_VBA_MACRO"), "{name}: macro in a renamed Word document was missed: {:?} {:?}", r.findings, r.reasons);
    }
    // 암호화 OOXML 패키지도 같다.
    let enc = make_cfb(&[("EncryptionInfo", &[4u8, 0, 4, 0][..]), ("EncryptedPackage", &[0u8; 64][..])]);
    assert!(scan_cfb("a.hwp", &enc).reasons.iter().any(|m| FileReport::is_encrypted_reason(m)));
}

// --- Scripts (명세 표 8, 9) ---

fn wchars(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// `DefaultJScript`: 헤더·소스·Pre·Post 각각 DWORD 길이(글자 수) + WCHAR 배열, 끝에 DWORD `-1`.
fn default_jscript(parts: [&str; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in parts {
        out.extend_from_slice(&(p.encode_utf16().count() as u32).to_le_bytes());
        out.extend_from_slice(&wchars(p));
    }
    out.extend_from_slice(&u32::MAX.to_le_bytes());
    out
}

const JSCRIPT_VERSION: [u8; 8] = [0u8; 8];

fn scan_scripts(flags: u32, script: &[u8]) -> FileReport {
    scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..]), ("Scripts/DefaultJScript", script)], flags))
}

#[test]
fn no_scripts_storage_and_no_flag_is_clean() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[], 0));
    assert!(!r.has("HWP_SCRIPT") && !r.has("HWP_SCRIPT_FLAG_MISMATCH"), "{:?}", r.findings);
}

#[test]
fn an_empty_scripts_storage_of_an_ordinary_document_is_clean() {
    // 한글은 스크립트가 없어도 `Scripts` 저장소(빈 DefaultJScript)를 항상 쓴다. 실제 건강한 HWP5(EPUB 안에서 발견)의 두 스트림을 그대로 쓴다.
    const REAL_JSCRIPT_VERSION: [u8; 13] = [0x63, 0x64, 0x80, 0x00, 0x00, 0xf7, 0xdf, 0x88, 0xa9, 0x08, 0x00, 0x00, 0x00];
    const REAL_DEFAULT_JSCRIPT: [u8; 16] = [0x63, 0x60, 0x40, 0x05, 0xff, 0x81, 0x00, 0x00, 0x6e, 0xbb, 0x6e, 0xd1, 0x14, 0x00, 0x00, 0x00];
    let real = scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &REAL_JSCRIPT_VERSION[..]), ("Scripts/DefaultJScript", &REAL_DEFAULT_JSCRIPT[..])], 0x1));
    assert!(real.findings.is_empty() && !real.incomplete, "{:?} {:?}", real.findings, real.reasons);
    // 압축하지 않은 빈 표와 압축한 빈 표도 같다.
    let empty = default_jscript(["", "", "", ""]);
    for (flags, bytes) in [(0u32, empty.clone()), (0x1, raw_deflate(&empty)), (0x1, zlib(&empty))] {
        let r = scan_scripts(flags, &bytes);
        assert!(r.findings.is_empty() && !r.incomplete, "flags {flags:#x}: {:?} {:?}", r.findings, r.reasons);
    }
    // 공백뿐인 스크립트도 스크립트가 아니다.
    assert!(scan_scripts(0, &default_jscript(["  ", "\n\t", "", ""])).findings.is_empty());
}

#[test]
fn the_script_flag_makes_the_storage_high_even_when_empty() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..])], 0x8));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, scan_common::Severity::High);
    assert!(!r.has("HWP_SCRIPT_FLAG_MISMATCH"), "{:?}", r.findings);
    // 실제 스크립트 소스가 있으면 비트가 없어도 High다.
    let r = scan_scripts(0, &default_jscript(["", "var a = 1;", "", ""]));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, scan_common::Severity::High);
    assert!(!r.has("HWP_SCRIPT_FLAG_MISMATCH"), "storage without the flag is what Hangul writes: {:?}", r.findings);
}

#[test]
fn a_script_bit_without_a_storage_is_a_mismatch() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[], 0x8));
    let f = finding(&r, "HWP_SCRIPT_FLAG_MISMATCH");
    assert_eq!((f.severity, f.category), (scan_common::Severity::Medium, scan_common::Category::Defect));
    assert!(!r.has("HWP_SCRIPT"));
}

#[test]
fn suspicious_script_is_critical_in_any_part() {
    let evil = r#"var s = new ActiveXObject("WScript.Shell"); s.Run("cmd.exe /c calc");"#;
    for parts in [["", evil, "", ""], ["", "", evil, ""], ["", "", "", evil], [evil, "", "", ""]] {
        let r = scan_scripts(0x8, &default_jscript(parts));
        assert_eq!(finding(&r, "HWP_SCRIPT_SUSPICIOUS").severity, scan_common::Severity::Critical, "{parts:?}");
    }
    let r = scan_scripts(0x8, &default_jscript(["", "var x = EVAL ( payload ) ; new Function('a')", "", ""]));
    assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "{:?}", r.findings);
}

#[test]
fn benign_script_is_high_not_critical() {
    let r = scan_scripts(0x8, &default_jscript(["", "function f() { return 1 + 1; } // a shell of a function", "", ""]));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, scan_common::Severity::High);
    assert!(!r.has("HWP_SCRIPT_SUSPICIOUS") && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn malformed_script_is_still_high_incomplete_and_never_panics() {
    let mut huge = Vec::new();
    huge.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    huge.extend_from_slice(&wchars("short"));
    let mut missing_end = default_jscript(["", "x", "", ""]);
    missing_end.truncate(missing_end.len() - 4);
    let mut wrong_end = default_jscript(["", "x", "", ""]);
    let n = wrong_end.len();
    wrong_end[n - 4..].copy_from_slice(&7u32.to_le_bytes());
    for (name, bytes) in [("huge length", huge), ("missing end flag", missing_end), ("wrong end flag", wrong_end), ("empty", Vec::new()), ("three bytes", vec![1, 2, 3])] {
        let r = scan_scripts(0x8, &bytes);
        assert!(r.has("HWP_SCRIPT") && r.incomplete, "{name}: {:?} {:?}", r.findings, r.reasons);
    }
    // 구조가 깨져도 원시 바이트에서 키워드를 찾는다.
    let mut evil = Vec::new();
    evil.extend_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    evil.extend_from_slice(&wchars("WScript.Shell"));
    assert!(scan_scripts(0x8, &evil).has("HWP_SCRIPT_SUSPICIOUS"));
    // 임의 바이트로 열어도 panic하지 않는다.
    let mut seed = 0x1234_5678u32;
    for _ in 0..200 {
        let bytes: Vec<u8> = (0..64).map(|_| { seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (seed >> 24) as u8 }).collect();
        let r = scan_scripts(0x8, &bytes);
        assert!(!r.reasons.iter().any(|m| m.contains("panic")), "{:?}", r.reasons);
    }
}

#[test]
fn a_script_stream_over_the_limit_is_incomplete() {
    let big = default_jscript(["", &"a".repeat(3 * 1024 * 1024), "", ""]);
    assert!(big.len() > 4 * 1024 * 1024);
    let r = scan_scripts(0x8, &big);
    assert!(r.has("HWP_SCRIPT") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

// --- 압축 해제 ---

use bookdoctor::hwp::inflate::{InflateError, decompress};
use flate2::Compression;
use flate2::write::{DeflateEncoder, ZlibEncoder};
use std::io::Write;

fn raw_deflate(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

#[test]
fn zlib_and_raw_deflate_both_decode() {
    let text = b"hello hwp ".repeat(50);
    assert_eq!(decompress(&raw_deflate(&text), 1 << 20), Ok(text.clone()));
    assert_eq!(decompress(&zlib(&text), 1 << 20), Ok(text));
}

#[test]
fn output_over_the_limit_is_too_large_without_allocating_it() {
    // 1GiB의 0을 압축하면 1MiB 남짓이다. 상한 1MiB로 풀면 TooLarge이고 메모리를 쓰지 않는다.
    let bomb = raw_deflate(&vec![0u8; 64 * 1024 * 1024]);
    assert!(bomb.len() < 128 * 1024, "fixture sanity: {}", bomb.len());
    assert_eq!(decompress(&bomb, 1 << 20), Err(InflateError::TooLarge));
    assert_eq!(decompress(&zlib(&vec![0u8; 8 << 20]), 1 << 20), Err(InflateError::TooLarge));
    // 상한과 같은 크기는 허용한다.
    assert_eq!(decompress(&raw_deflate(&vec![1u8; 1 << 20]), 1 << 20).map(|v| v.len()), Ok(1 << 20));
}

#[test]
fn truncated_and_garbage_are_invalid() {
    let full = raw_deflate(&b"some text that compresses ".repeat(100));
    assert_eq!(decompress(&full[..full.len() / 2], 1 << 20), Err(InflateError::Invalid));
    assert_eq!(decompress(&[0xFF, 0xEE, 0xDD, 0xCC, 0xBB], 1 << 20), Err(InflateError::Invalid));
    let mut seed = 99u32;
    for _ in 0..200 {
        let bytes: Vec<u8> = (0..128).map(|_| { seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (seed >> 24) as u8 }).collect();
        let _ = decompress(&bytes, 1 << 16); // panic하지 않는다.
    }
}

#[test]
fn empty_input_is_an_empty_output() {
    assert_eq!(decompress(&[], 1 << 20), Ok(Vec::new()));
}

#[test]
fn compressed_script_is_read() {
    let evil = r#"new ActiveXObject("WScript.Shell").Run("cmd.exe");"#;
    let plain = default_jscript(["", evil, "", ""]);
    // 압축 비트(0x1)가 서 있고 raw deflate나 zlib으로 압축된 스크립트다.
    for (name, bytes) in [("raw", raw_deflate(&plain)), ("zlib", zlib(&plain))] {
        let r = scan_scripts(0x1 | 0x8, &bytes);
        assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "{name}: {:?} {:?}", r.findings, r.reasons);
        assert!(!r.incomplete, "{name}: {:?}", r.reasons);
    }
    // 압축 비트가 서 있어도 실제로는 압축하지 않은 스크립트(표 2는 Scripts를 압축 대상으로 표시하지 않는다).
    assert!(scan_scripts(0x1 | 0x8, &plain).has("HWP_SCRIPT_SUSPICIOUS"));
    // 압축 비트가 없는데 압축된 스크립트는 구조를 못 읽으므로 원시 검사와 미완료다.
    assert!(scan_scripts(0x8, &raw_deflate(&plain)).incomplete);
}

#[test]
fn a_script_that_decompresses_past_the_limit_is_incomplete() {
    let bomb = raw_deflate(&default_jscript(["", &"a".repeat(6 * 1024 * 1024), "", ""]));
    assert!(bomb.len() < 64 * 1024);
    let r = scan_scripts(0x1 | 0x8, &bomb);
    assert!(r.has("HWP_SCRIPT") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    assert!(r.reasons.iter().any(|m| m.contains("decompresses beyond")), "{:?}", r.reasons);
}

#[test]
fn every_stream_under_scripts_is_scanned_not_only_defaultjscript() {
    let evil = wchars("var s = new ActiveXObject('WScript.Shell');");
    let cfb = hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..]), ("Scripts/Extra", &evil[..])], 0x8);
    let r = scan_cfb("a.hwp", &cfb);
    let f = finding(&r, "HWP_SCRIPT_SUSPICIOUS");
    assert!(f.location.contains("Extra"), "{}", f.location);
    // 압축되어 있어도 같다.
    let cfb = hwp_cfb(&[("Scripts/Extra", &raw_deflate(&evil)[..])], 0x1 | 0x8);
    assert!(scan_cfb("b.hwp", &cfb).has("HWP_SCRIPT_SUSPICIOUS"));
}

// --- BinData (명세 표 17, 18) ---

use bookdoctor::hwp::inflate::decompress_prefix;

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

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR some png bytes";

#[test]
fn decompress_prefix_stops_after_n_bytes_without_error() {
    let bomb = raw_deflate(&vec![0u8; 64 * 1024 * 1024]);
    assert_eq!(decompress_prefix(&bomb, 4096).map(|v| v.len()), Ok(4096));
    // 끊긴 스트림도 그때까지 풀린 앞부분을 돌려준다.
    let full = raw_deflate(&b"abcdefgh".repeat(4096));
    let cut = &full[..full.len() / 2];
    assert!(decompress_prefix(cut, 16).is_ok_and(|v| v == b"abcdefghabcdefgh"));
    assert_eq!(decompress_prefix(&[0xFF, 0xEE, 0xDD], 16), Err(InflateError::Invalid));
}

#[test]
fn linked_file_unc_is_high_and_local_is_medium() {
    for (path, severity) in [("\\\\evil\\share\\a.exe", scan_common::Severity::High), ("http://evil.example/a", scan_common::Severity::High), ("C:\\docs\\a.png", scan_common::Severity::Medium)] {
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
    assert!(r.findings.iter().all(|f| f.severity != scan_common::Severity::Low));
}

#[test]
fn pe_with_a_png_name_is_still_flagged_and_an_exe_name_is_flagged_by_name() {
    // 압축 비트가 없는 문서, 압축 비트가 있지만 스트림은 압축하지 않은 경우(속성 0x20), 이름만 png인 PE.
    assert!(scan_bindata(0, None, &[("BIN0001.png", pe_bytes())]).has("EMBEDDED_EXECUTABLE"));
    assert!(scan_bindata(0x1, None, &[("BIN0001.png", pe_bytes())]).has("EMBEDDED_EXECUTABLE"));
    // 이름이 exe면 내용이 PNG여도 High다.
    let r = scan_bindata(0, None, &[("BIN0001.exe", PNG.to_vec())]);
    assert_eq!(finding(&r, "EMBEDDED_EXECUTABLE").severity, scan_common::Severity::High);
}

#[test]
fn ole_and_eps_payloads() {
    let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.extend_from_slice(&[0u8; 64]);
    let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n".to_vec();
    let mut dos_eps = vec![0xC5, 0xD0, 0xD3, 0xC6];
    dos_eps.extend_from_slice(&[0u8; 60]);
    let r = scan_bindata(0x1, None, &[("BIN0001.ole", raw_deflate(&ole)), ("BIN0002.eps", raw_deflate(&eps)), ("BIN0003.eps", dos_eps)]);
    assert_eq!(finding(&r, "HWP_OLE_OBJECT").severity, scan_common::Severity::High);
    assert_eq!(finding(&r, "HWP_EPS_OBJECT").severity, scan_common::Severity::Medium);
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

#[test]
fn a_docinfo_that_cannot_be_decompressed_is_incomplete() {
    let r = scan_bindata(0x1, Some(vec![0xFF, 0xEE, 0xDD, 0xCC, 0xBB, 0xAA]), &[]);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("DocInfo")), "{:?}", r.reasons);
}

#[test]
fn defaultjscript_is_scanned_even_when_many_other_streams_come_first() {
    let evil = default_jscript(["", r#"new ActiveXObject("WScript.Shell")"#, "", ""]);
    let junk = [1u8, 2, 3];
    let names: Vec<String> = (0..60).map(|i| format!("Scripts/S{i:02}")).collect();
    let mut streams: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &junk[..])).collect();
    streams.push(("Scripts/DefaultJScript", &evil[..]));
    let r = scan_cfb("a.hwp", &hwp_cfb(&streams, 0x8));
    assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "DefaultJScript past the stream cap was skipped: {:?} {:?}", r.findings, r.reasons);
    // 상한 때문에 건너뛴 스트림이 있으면 깨끗하다고 하지 않는다.
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("Scripts")), "{:?}", r.reasons);
}

#[test]
fn data_after_a_valid_script_table_is_still_scanned() {
    // 표 9 구조를 지킨 앞부분 뒤에 붙은 바이트도 키워드를 찾는다. 읽는 쪽이 어디까지 읽는지 알 수 없다.
    let mut bytes = default_jscript(["", "var a = 1;", "", ""]);
    bytes.extend_from_slice(&wchars("new ActiveXObject('WScript.Shell')"));
    let r = scan_scripts(0x8, &bytes);
    assert!(r.has("HWP_SCRIPT_SUSPICIOUS"), "{:?} {:?}", r.findings, r.reasons);
    // 압축 비트가 서 있어도 같다: 원본의 뒷부분과 푼 내용을 모두 본다.
    let mut compressed_flag = default_jscript(["", "var a = 1;", "", ""]);
    compressed_flag.extend_from_slice(&wchars("WScript.Shell"));
    assert!(scan_scripts(0x1 | 0x8, &compressed_flag).has("HWP_SCRIPT_SUSPICIOUS"));
}

// --- 레코드 검증 (명세 4.1) ---

const TAG_SHAPE_COMPONENT_OLE: u32 = HWPTAG_BEGIN + 68;

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
    assert_eq!((f.severity, f.category), (scan_common::Severity::High, scan_common::Category::Defect));
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
    assert_eq!(finding(&r, "HWP_RECORD_LEVEL_JUMP").severity, scan_common::Severity::Medium);
    assert_eq!(r.findings.iter().filter(|f| f.code == "HWP_RECORD_LEVEL_JUMP").count(), 1);
}

#[test]
fn ole_shape_records_are_high() {
    let body = [record(HWPTAG_BEGIN + 50, 0, &[0u8; 4]), record(TAG_SHAPE_COMPONENT_OLE, 1, &[0u8; 24])].concat();
    assert_eq!(finding(&scan_sections(0, None, body), "HWP_OLE_OBJECT").severity, scan_common::Severity::High);
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

#[test]
fn byte_mutations_of_a_full_hwp_never_panic_or_stall() {
    // 모든 검사 경로(헤더, 압축 DocInfo·본문, 스크립트, BinData)를 타는 HWP를 만들어 바이트를 바꿔 가며 돌린다.
    let docinfo = raw_deflate(&[bin_data_link("\\\\evil\\s\\a", "a"), valid_stream()].concat());
    let section = raw_deflate(&[valid_stream(), record(TAG_SHAPE_COMPONENT_OLE, 1, &[0u8; 24])].concat());
    let script = raw_deflate(&default_jscript(["", r#"new ActiveXObject("WScript.Shell")"#, "", ""]));
    let pe = raw_deflate(&pe_bytes());
    let base = make_cfb(&[
        ("FileHeader", &file_header(0x1 | 0x8)[..]),
        ("DocInfo", &docinfo[..]),
        ("BodyText/Section0", &section[..]),
        ("Scripts/DefaultJScript", &script[..]),
        ("Scripts/JScriptVersion", &JSCRIPT_VERSION[..]),
        ("BinData/BIN0001.jpg", &pe[..]),
    ])
    .bytes;
    let clean = scan("base.hwp", &base);
    assert!(clean.has("HWP_LINKED_FILE") && clean.has("HWP_SCRIPT_SUSPICIOUS") && clean.has("EMBEDDED_EXECUTABLE") && clean.has("HWP_OLE_OBJECT"), "the unmutated file must exercise every path: {:?} {:?}", clean.findings, clean.reasons);
    let mut seed = 0xC0FF_EE11u32;
    let mut next = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as usize
    };
    let started = std::time::Instant::now();
    for round in 0..600 {
        let mut m = base.clone();
        match round % 3 {
            0 => (0..1 + next() % 8).for_each(|_| {
                let at = next() % m.len();
                m[at] = (next() & 0xFF) as u8;
            }),
            1 => (0..1 + next() % 4).for_each(|_| {
                // 헤더와 디렉터리 영역(앞 1.5KiB)을 집중 변조한다.
                let at = next() % m.len().min(1536);
                m[at] = (next() & 0xFF) as u8;
            }),
            _ => m.truncate(8 + next() % (m.len() - 8)),
        }
        let r = scan("m.hwp", &m);
        assert!(!r.reasons.iter().any(|x| x.contains("panic")), "round {round}: {:?}", r.reasons);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(60), "{:?}", started.elapsed());
}
