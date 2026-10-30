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
