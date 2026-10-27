use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use doc_reader::docx::read_from_docx;
use doc_reader::office::{convert_with_libreoffice, find_libreoffice, read_from_doc, read_from_hwp, read_from_image};
use doc_reader::rtf::{read_from_rtf_text, rtf_to_text};
use serde_json::Value;

#[path = "corpus.rs"]
mod corpus;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/rust/doc_reader/fixtures").join(name)
}

#[test]
fn rtf_matches_striprtf() {
    let cases: Value = serde_json::from_str(&fs::read_to_string(fixture("rtf_cases.json")).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let rtf = case["rtf"].as_str().unwrap();
        let got = rtf_to_text(rtf);
        match (case.get("text"), &got) {
            (Some(want), Ok(text)) => assert_eq!(text, want.as_str().unwrap(), "{rtf}"),
            (None, Err(_)) => {}
            other => panic!("{rtf}: {other:?}"),
        }
    }
}

#[test]
fn read_from_rtf_text_cleans_and_counts() {
    let (summary, lines) = read_from_rtf_text(r"{\rtf1\ansi Hello, world\par Second}").unwrap();
    assert_eq!((summary.as_str(), lines), ("Hello  world\nSecond", 2));
    assert_eq!(read_from_rtf_text("{\\rtf1 {\\*\\x hidden}}").unwrap(), (String::new(), 0));
    assert!(read_from_rtf_text(r"{\rtf1 \uc}").is_err());
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
fn broken_docx_files_are_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = tmp.path().join("bad.docx");
    fs::write(&bad, b"not a zip").unwrap();
    assert!(read_from_docx(&bad).is_err());
    assert!(read_from_docx(&tmp.path().join("none.docx")).is_err());
}

/// 변환기를 흉내 내는 셸 스크립트. 마지막 인자(원본)의 stem으로 `--outdir` 아래에 결과를 쓴다.
fn fake_office(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("fake-office");
    fs::write(
        &path,
        format!("#!/bin/sh\nout=\"\"\nwhile [ $# -gt 1 ]; do if [ \"$1\" = \"--outdir\" ]; then out=\"$2\"; fi; shift; done\nsrc=\"$1\"\nname=$(basename \"$src\")\nstem=\"${{name%.*}}\"\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn office_conversion_reads_the_output_file_and_reports_a_missing_one() {
    let tmp = tempfile::tempdir().unwrap();
    let ok = fake_office(tmp.path(), "printf 'line one\\nline, two' > \"$out/$stem.txt\"");
    let src = tmp.path().join("doc one.doc");
    fs::write(&src, b"x").unwrap();
    let mut errors = Vec::new();
    assert_eq!(convert_with_libreoffice(&src, "txt:Text", &ok, &mut errors), "line one\nline, two");
    assert!(errors.is_empty());

    let (summary, lines, errors) = read_from_doc(&src, &ok);
    assert_eq!((summary.as_str(), lines), ("line one\nline  two", 2));
    assert!(errors.is_empty());

    // 이름이 달라도 확장자로 찾는다.
    let renamed = fake_office(tmp.path(), "printf 'other name' > \"$out/whatever.txt\"");
    assert_eq!(convert_with_libreoffice(&src, "txt:Text", &renamed, &mut Vec::new()), "other name");

    let none = fake_office(tmp.path(), "echo oops >&2; exit 3");
    let mut errors = Vec::new();
    assert_eq!(convert_with_libreoffice(&src, "txt:Text", &none, &mut errors), "");
    assert!(errors[0].contains("LibreOffice produced no output") && errors[0].contains("returncode=3") && errors[0].contains("oops"), "{errors:?}");

    let missing = tmp.path().join("no-such-binary");
    let mut errors = Vec::new();
    assert_eq!(convert_with_libreoffice(&src, "txt:Text", &missing, &mut errors), "");
    assert!(errors[0].contains("timeout"), "{errors:?}");
}

#[test]
fn hwp_falls_back_to_the_native_hwp3_parser_when_conversion_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let empty = fake_office(tmp.path(), ": > \"$out/$stem.txt\"");
    let src = tmp.path().join("a.hwp");
    fs::write(&src, b"not a real hwp").unwrap();
    let (summary, lines, errors) = read_from_hwp(&src, &empty);
    assert!(errors.is_empty());
    // HWP3 서명이 아니므로 파서도 빈 글을 준다.
    assert_eq!((summary.as_str(), lines), ("", 0));

    let filled = fake_office(tmp.path(), "printf '한글 문서' > \"$out/$stem.txt\"");
    let (summary, lines, _) = read_from_hwp(&src, &filled);
    assert_eq!((summary.as_str(), lines), ("한글 문서", 1));
}

#[test]
fn images_have_no_text() {
    assert_eq!(read_from_image(), (String::new(), 0, 0));
    assert!(!find_libreoffice().as_os_str().is_empty());
}

#[test]
fn corpus_rtf_matches_python() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_rtf.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let got = fs::read(root.join(path)).ok().and_then(|bytes| String::from_utf8(bytes).ok()).and_then(|text| read_from_rtf_text(&text).ok()).unwrap_or_default();
        assert_eq!(corpus::fnv1a64(&got.0), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(got.1 as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
    }
}
