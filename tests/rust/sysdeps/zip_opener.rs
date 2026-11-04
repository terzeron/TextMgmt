//! ZIP 열기 테스트 (`zip_opener` 모듈).

use std::io::{Read, Write};
use std::path::Path;

use sysdeps::zip_opener::{MAX_MEMBER_BYTES, open_zip, zip_entry_name};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn zip_bytes(name: &str, body: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer.start_file(name, SimpleFileOptions::default()).unwrap();
    writer.write_all(body).unwrap();
    writer.finish().unwrap().into_inner()
}

fn read_member(path: &Path, name: &str) -> Option<String> {
    let mut zip = open_zip(path)?;
    let mut text = String::new();
    zip.by_name(name).ok()?.take(MAX_MEMBER_BYTES).read_to_string(&mut text).ok()?;
    Some(text)
}

#[test]
fn opens_a_plain_zip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.zip");
    std::fs::write(&path, zip_bytes("a.txt", b"hello")).unwrap();
    assert_eq!(read_member(&path, "a.txt").as_deref(), Some("hello"));
}

#[test]
fn opens_a_zip_with_data_before_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefixed.zip");
    let mut data = b"PREFIX-DATA-".repeat(10);
    data.extend(zip_bytes("a.txt", b"after prefix"));
    std::fs::write(&path, data).unwrap();
    assert_eq!(read_member(&path, "a.txt").as_deref(), Some("after prefix"));
}

#[test]
fn returns_none_for_missing_or_non_zip_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not.zip");
    std::fs::write(&path, b"not a zip archive").unwrap();
    assert!(open_zip(&path).is_none());
    assert!(open_zip(&dir.path().join("missing.zip")).is_none());
}

#[test]
fn zip_entry_names_follow_the_zip_crate_rules() {
    assert_eq!(zip_entry_name("OEBPS/./Text//ch1.xhtml").as_deref(), Some("OEBPS/Text/ch1.xhtml"));
    assert_eq!(zip_entry_name(Path::new("OEBPS/Text").join("../Images/a.jpg")).as_deref(), Some("OEBPS/Images/a.jpg"));
    // 맨 앞 `/`는 압축 파일 루트다.
    assert_eq!(zip_entry_name("/OEBPS/a.xhtml").as_deref(), Some("OEBPS/a.xhtml"));
    assert_eq!(zip_entry_name("OEBPS/").as_deref(), Some("OEBPS"));
    assert_eq!(zip_entry_name("").as_deref(), Some(""));
    assert_eq!(zip_entry_name("a/..").as_deref(), Some(""));
}

#[test]
fn zip_entry_names_reject_paths_leaving_the_archive() {
    assert_eq!(zip_entry_name("../content.opf"), None);
    assert_eq!(zip_entry_name("OEBPS/../../x"), None);
    assert_eq!(zip_entry_name(Path::new("OEBPS").join("../../etc/passwd")), None);
}
