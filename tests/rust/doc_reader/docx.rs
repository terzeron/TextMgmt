use std::fs;
use std::path::{Path, PathBuf};

use doc_reader::docx::read_from_docx;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/doc_reader/fixtures").join(name)
}

#[test]
fn docx_matches_python_docx() {
    let expected: Value = serde_json::from_str(&fs::read_to_string(fixture("docx_expected.json")).unwrap()).unwrap();
    for (name, want) in expected.as_object().unwrap() {
        let (summary, lines) = read_from_docx(&fixture(name)).unwrap();
        assert_eq!(summary, want[0].as_str().unwrap(), "{name}");
        assert_eq!(lines as i64, want[1].as_i64().unwrap(), "{name}");
    }
}

#[test]
fn a_utf8_bom_on_docx_xml_parts_is_ignored() {
    use std::io::{Read, Write};
    let tmp = tempfile::tempdir().unwrap();
    let with_bom = tmp.path().join("bom.docx");
    let mut source = zip::ZipArchive::new(fs::File::open(fixture("a.docx")).unwrap()).unwrap();
    let mut writer = zip::ZipWriter::new(fs::File::create(&with_bom).unwrap());
    for i in 0..source.len() {
        let mut member = source.by_index(i).unwrap();
        let name = member.name().unwrap().into_owned();
        let mut data = Vec::new();
        member.read_to_end(&mut data).unwrap();
        if name.ends_with(".xml") || name.ends_with(".rels") {
            data.splice(0..0, *b"\xef\xbb\xbf");
        }
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&data).unwrap();
    }
    writer.finish().unwrap();
    assert_eq!(read_from_docx(&with_bom).unwrap(), read_from_docx(&fixture("a.docx")).unwrap());
}

#[test]
fn broken_docx_files_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("bad.docx");
    fs::write(&bad, b"not a zip").unwrap();
    assert!(read_from_docx(&bad).is_err());
    assert!(read_from_docx(&tmp.path().join("none.docx")).is_err());
}
