#[path = "../support/mod.rs"]
mod support;

use std::fs;
use std::path::Path;

use bookdoctor::cli::{Options, run};
use bookdoctor::scan::scan_file;
use bookdoctor::FileReport;

const OLE_STUB: &[u8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1 macro";

fn bytes_of(build: impl FnOnce(&Path)) -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("x");
    build(&p);
    fs::read(&p).unwrap()
}

/// 매크로 프로젝트가 든 `.docx`의 바이트(매크로 없는 형식에 든 `vbaProject.bin`이라 Critical).
fn vba_docx() -> Vec<u8> {
    bytes_of(|p| support::docx_with(p, &[], &[("word/vbaProject.bin", OLE_STUB)]))
}

fn zip_bytes(members: &[(&str, &[u8])]) -> Vec<u8> {
    bytes_of(|p| support::zip_of(p, members))
}

fn scan_bytes(name: &str, data: &[u8], opts: &Options) -> FileReport {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, data).unwrap();
    scan_file(&p, opts)
}

fn has_vba_under(r: &FileReport, prefix: &str) -> bool {
    r.findings.iter().any(|f| f.code == "OOXML_VBA_PROJECT" && f.location.starts_with(prefix) && f.location.contains("vbaProject.bin"))
}

#[test]
fn no_recurse_turns_the_nested_scan_off() {
    let outer = zip_bytes(&[("inner.docx", &vba_docx())]);
    let opts = Options { recurse: false, ..Options::default() };
    let r = scan_bytes("a.zip", &outer, &opts);
    assert!(!has_vba_under(&r, "inner.docx!"), "{:?}", r.findings);
    assert!(Options::default().recurse);
    // 명령행 옵션도 받는다.
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a.zip"), &outer).unwrap();
    let mut out = Vec::new();
    run(&["--no-recurse".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().join("a.zip").to_string_lossy().into_owned()], &mut out);
    assert!(!String::from_utf8(out).unwrap().contains("inner.docx!"));
}
