
mod support;
mod ole_common;
use std::fs;
use bookdoctor::cli_parser::Options;
use bookdoctor::format_dispatcher::scan_file;
use ole_common::{Cfb, compress, dir_stream, make_cfb, module_stream};
use bookdoctor::{FileReport, Format};
use bookdoctor::hwp::{decompress, decompress_prefix, InflateError};
use crate::support::shared_fixtures::{JSCRIPT_VERSION, TAG_SHAPE_COMPONENT_OLE, bin_data_link, default_jscript, file_header, finding, pe_bytes, raw_deflate, record, scan, valid_stream, zlib};

fn hwp_cfb(extra: &[(&str, &[u8])], flags: u32) -> Cfb {
    let header = file_header(flags);
    let mut streams: Vec<(&str, &[u8])> = vec![("FileHeader", &header)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

const HWP3_SIGNATURE: &[u8] = b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05";

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
        assert_eq!(f.severity, bookdoctor::Severity::Medium, "{name}");
        assert_eq!(f.category, bookdoctor::Category::Defect, "{name}");
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
fn decompress_prefix_stops_after_n_bytes_without_error() {
    let bomb = raw_deflate(&vec![0u8; 64 * 1024 * 1024]);
    assert_eq!(decompress_prefix(&bomb, 4096).map(|v| v.len()), Ok(4096));
    // 끊긴 스트림도 그때까지 풀린 앞부분을 돌려준다.
    let full = raw_deflate(&b"abcdefgh".repeat(4096));
    let cut = &full[..full.len() / 2];
    assert!(decompress_prefix(cut, 16).is_ok_and(|v| v == b"abcdefghabcdefgh"));
    assert_eq!(decompress_prefix(&[0xFF, 0xEE, 0xDD], 16), Err(InflateError::Invalid));
}
