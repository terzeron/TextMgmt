//! HWP 5.x 테스트. fixture는 합성이다: 코퍼스에 정상 HWP5 파일이 없다(`docs/bookdoctor-hwp5-notes.md`).

#[path = "../ole_reader/common.rs"]
mod ole_common;

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli::{Options, run};
use bookdoctor::scan::scan_file;
use ole_common::{Cfb, compress, dir_stream, make_cfb, module_stream};
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
fn scripts_storage_is_high_even_when_empty() {
    let r = scan_cfb("a.hwp", &hwp_cfb(&[("Scripts/JScriptVersion", &JSCRIPT_VERSION[..])], 0x8));
    assert_eq!(finding(&r, "HWP_SCRIPT").severity, scan_common::Severity::High);
    assert!(!r.has("HWP_SCRIPT_FLAG_MISMATCH"), "{:?}", r.findings);
}

#[test]
fn the_script_flag_and_the_storage_must_agree() {
    // 비트는 있는데 스토리지가 없다.
    let r = scan_cfb("a.hwp", &hwp_cfb(&[], 0x8));
    let f = finding(&r, "HWP_SCRIPT_FLAG_MISMATCH");
    assert_eq!((f.severity, f.category), (scan_common::Severity::Medium, scan_common::Category::Defect));
    assert!(!r.has("HWP_SCRIPT"));
    // 스토리지는 있는데 비트가 없다. 스크립트는 그대로 High다.
    let r = scan_scripts(0, &default_jscript(["", "var a = 1;", "", ""]));
    assert!(r.has("HWP_SCRIPT_FLAG_MISMATCH") && r.has("HWP_SCRIPT"), "{:?}", r.findings);
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
