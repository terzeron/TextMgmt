//! RTF 위장 테스트. fixture는 합성이다. 이름은 Word 문서인데 내용이 RTF인 파일과 `.rtf` 파일을 본다.

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli_parser::{Options, run};
use bookdoctor::format_dispatcher::scan_file;
use bookdoctor::FileReport;

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
fn rtf_is_collected_and_other_content_named_rtf_is_a_mismatch() {
    let d = tempfile::tempdir().unwrap();
    // 깨끗한 파일은 출력에 나오지 않으므로 지표가 나오는 내용을 쓴다.
    fs::write(d.path().join("a.RTF"), b"{\\rtf1{\\*\\objclass X}}").unwrap();
    fs::write(d.path().join("b.rtfx"), b"{\\rtf1{\\*\\objclass X}}").unwrap();
    let mut out = Vec::new();
    run(&["scan".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("a.RTF"), "{out}");
    assert!(!out.contains("b.rtfx"), "{out}");
    let r = scan("c.rtf", "그냥 텍스트\n".repeat(50).as_bytes());
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
}
