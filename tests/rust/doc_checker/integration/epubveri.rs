//! `epubveri` CLI(`libs/doc_checker/src/bin/epubveri.rs`) 통합 테스트. 실행 파일을 하위 프로세스로 돌려 JSON 리포트와 종료 코드를 본다.

#[path = "../../shared/epub.rs"]
mod epub;
#[path = "../../shared/image.rs"]
mod image;
#[path = "../../shared/zipx.rs"]
mod zipx;

use std::process::Command;

use epub::{EpubSpec, make_epub};
use serde_json::Value;

#[test]
fn test_epubveri_cli_writes_epubcheck_compatible_json_for_valid_and_invalid_books() {
    let tmp = tempfile::tempdir().unwrap();
    let valid_path = tmp.path().join("valid.epub");
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">urn:uuid:123e4567-e89b-12d3-a456-426614174000</dc:identifier><dc:title>Rust wrapper</dc:title><dc:language>en</dc:language>
    <meta property="dcterms:modified">2024-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="ch1"/></spine>
</package>"#;
    let nav = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head><body><nav epub:type="toc"><ol><li><a href="ch1.xhtml">Chapter</a></li></ol></nav></body></html>"#;
    let mut valid_spec = EpubSpec::minimal().with_opf(opf).with_extra("nav.xhtml", nav);
    valid_spec.chapters = vec![("ch1.xhtml".into(), "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Text</title></head><body><p>Text</p></body></html>".into())];
    make_epub(&valid_path, &valid_spec);
    let valid_json = tmp.path().join("valid.json");
    let valid = Command::new(env!("CARGO_BIN_EXE_epubveri")).arg(&valid_path).arg("--json").arg(&valid_json).output().unwrap();
    let valid_report: Value = serde_json::from_slice(&std::fs::read(valid_json).unwrap()).unwrap();
    assert_eq!(valid.status.code(), Some(0), "{}", valid_report);
    assert_eq!(valid_report["checker"]["nFatal"], 0);
    assert_eq!(valid_report["publication"]["title"], "Rust wrapper");

    let invalid_path = tmp.path().join("invalid.epub");
    make_epub(&invalid_path, &EpubSpec::minimal().without_mimetype());
    let invalid_json = tmp.path().join("invalid.json");
    let invalid = Command::new(env!("CARGO_BIN_EXE_epubveri")).arg(&invalid_path).arg("--json").arg(&invalid_json).output().unwrap();
    assert_eq!(invalid.status.code(), Some(1), "{}", String::from_utf8_lossy(&invalid.stderr));
    let invalid_report: Value = serde_json::from_slice(&std::fs::read(invalid_json).unwrap()).unwrap();
    let messages = invalid_report["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    assert!(messages.iter().all(|message| message.get("ID").is_some()));
}

#[test]
fn test_epubveri_cli_exits_2_on_bad_arguments_unreadable_books_and_unwritable_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let book = tmp.path().join("book.epub");
    make_epub(&book, &EpubSpec::minimal());
    let run = |args: &[&std::ffi::OsStr]| Command::new(env!("CARGO_BIN_EXE_epubveri")).args(args).output().unwrap();
    let (b, j) = (book.as_os_str(), std::ffi::OsStr::new("--json"));
    let out = tmp.path().join("r.json");
    for args in [vec![], vec![b], vec![b, std::ffi::OsStr::new("--yaml"), out.as_os_str()], vec![b, j], vec![b, j, out.as_os_str(), b]] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&o.stderr).contains("usage: epubveri"), "{args:?}");
    }
    let o = run(&[tmp.path().join("none.epub").as_os_str(), j, out.as_os_str()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("epubveri validation failed"));
    // 리포트 경로가 디렉터리면 쓰지 못한다.
    let o = run(&[b, j, tmp.path().as_os_str()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("failed to write JSON report"));
}
