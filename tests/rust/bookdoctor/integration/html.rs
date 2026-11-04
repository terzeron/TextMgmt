//! 독립 HTML 테스트. 규칙은 EPUB의 HTML 규칙을 재사용한다.

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli_parser::{Options, run};
use bookdoctor::format_dispatcher::scan_file;
use bookdoctor::{FileReport, Format};

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
fn html_is_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    let page = b"<html><body><script src=\"http://evil.example/a.js\"></script></body></html>";
    for name in ["a.html", "b.HTM", "c.xhtml", "d.htmlx"] {
        fs::write(d.path().join(name), page).unwrap();
    }
    let mut out = Vec::new();
    run(&["scan".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("a.html") && out.contains("b.HTM") && out.contains("c.xhtml"), "{out}");
    assert!(!out.contains("d.htmlx"), "{out}");
    assert_eq!(scan("x.html", page).format, Some(Format::Html));
}
