#[path = "../ole_common.rs"]
mod ole_common;

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli_parser::{Options, run};
use bookdoctor::format_dispatcher::scan_file;
use ole_common::{Cfb, make_cfb};
use bookdoctor::Format;

/// `WordDocument` 스트림: FIB 32바이트(wIdent, nFib, flags1)를 512바이트로 채우고 본문을 붙인다.
fn word_stream(flags1: u16, body: &[u8]) -> Vec<u8> {
    let mut s = vec![0u8; 512];
    s[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
    s[2..4].copy_from_slice(&0x00C1u16.to_le_bytes());
    s[10..12].copy_from_slice(&flags1.to_le_bytes());
    s.extend_from_slice(body);
    s
}

fn doc_cfb(extra: &[(&str, &[u8])], flags1: u16, body: &[u8]) -> Cfb {
    let word = word_stream(flags1, body);
    let mut streams: Vec<(&str, &[u8])> = vec![("WordDocument", &word)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, bytes).unwrap();
    (d, p)
}

#[test]
fn doc_and_dot_are_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    let bytes = doc_cfb(&[], 0, b"x").bytes;
    for name in ["a.doc", "b.DOT", "c.docs"] {
        fs::write(d.path().join(name), &bytes).unwrap();
    }
    let mut out = Vec::new();
    run(&["-r".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("c.docs"), "{out}");
    let (_d2, p) = write("b.dot", &bytes);
    assert_eq!(scan_file(&p, &Options::default()).format, Some(Format::Doc));
}
