//! `common::zip_reader` 테스트: ZIP 열기, 항목 이름, 멤버 읽기 상한.

use std::io::{Read, Write};
use std::path::Path;

use common::zip_reader::{MAX_MEMBER_BYTES, MemberError, open_zip, read_member as read_limited, read_member_prefix, zip_entry_name};
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

fn archive(name: &str, body: &[u8]) -> zip::ZipArchive<std::io::Cursor<Vec<u8>>> {
    zip::ZipArchive::new(std::io::Cursor::new(zip_bytes(name, body))).unwrap()
}

#[test]
fn read_member_reads_a_member_up_to_the_limit() {
    let mut zip = archive("a.txt", b"hello");
    assert_eq!(read_limited(&mut zip, "a.txt", 5).unwrap(), b"hello");
}

#[test]
fn read_member_rejects_a_member_over_the_limit() {
    let mut zip = archive("a.txt", b"hello");
    assert!(matches!(read_limited(&mut zip, "a.txt", 4), Err(MemberError::TooLarge)));
}

#[test]
fn read_member_reports_a_missing_member_as_open_error() {
    let mut zip = archive("a.txt", b"hello");
    let error = read_limited(&mut zip, "b.txt", 5).unwrap_err();
    assert!(matches!(error, MemberError::Open(_)), "{error:?}");
    assert!(!error.to_string().is_empty());
    assert_eq!(MemberError::TooLarge.to_string(), "압축을 푼 크기가 상한을 넘습니다");
}

#[test]
fn read_member_prefix_truncates_long_members() {
    let mut zip = archive("a.txt", b"hello");
    assert_eq!(read_member_prefix(&mut zip, "a.txt", 3).unwrap(), b"hel");
    assert_eq!(read_member_prefix(&mut zip, "a.txt", 10).unwrap(), b"hello");
    assert!(matches!(read_member_prefix(&mut zip, "b.txt", 3), Err(MemberError::Open(_))));
}
