#[path = "../ole_common.rs"]
mod ole_common;

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli_parser::{Options, run};
use bookdoctor::format_dispatcher::scan_file;
use ole_common::{Cfb, make_cfb};
use bookdoctor::{FileReport, Format};

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

#[test]
fn hwp_is_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    // 깨끗한 파일은 출력에 나오지 않으므로 이름 불일치가 나오는 내용을 쓴다.
    for name in ["a.hwp", "b.HWP", "c.hwpp"] {
        fs::write(d.path().join(name), "그냥 텍스트\n".repeat(50)).unwrap();
    }
    let mut out = Vec::new();
    run(&["scan".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("a.hwp") && out.contains("b.HWP"), "{out}");
    assert!(!out.contains("c.hwpp"), "{out}");
    assert_eq!(scan("x.hwp", &hwp_cfb(&[], 0).bytes).format, Some(Format::Hwp));
}

// --- 압축 해제 ---

