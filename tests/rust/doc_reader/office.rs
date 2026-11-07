mod common;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::runner::{FakeRunner, output};
use doc_reader::office::{
    ConvertFailure, convert_with_libreoffice, convert_with_libreoffice_using, find_libreoffice, find_libreoffice_with, read_from_doc, read_from_hwp, read_from_hwp_with, read_from_hwp_with_fallback,
    read_from_image,
};
use sysdeps::CommandOutput;

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
fn injected_hwp_readers_use_native_text_first_and_report_conversion_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let hwp3 = tmp.path().join("v3.hwp");
    fs::write(&hwp3, b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05").unwrap();
    let convert = |_: &Path| -> Result<String, String> { panic!("네이티브 결과가 있으면 변환하지 않는다") };
    let native = |_: &Path| -> Result<String, String> { Ok("네이티브 본문\n둘째 줄".into()) };
    let (summary, lines, errors) = read_from_hwp_with_fallback(&hwp3, Path::new("unused"), &convert, &native);
    assert_eq!((summary.as_str(), lines), ("네이티브 본문\n둘째 줄", 2));
    assert!(errors.is_empty());

    let hwp5 = tmp.path().join("v5.hwp");
    fs::write(&hwp5, b"not hwp3").unwrap();
    let failing = |_: &Path| -> Result<String, String> { Err("변환 실패".into()) };
    assert_eq!(read_from_hwp_with(&hwp5, Path::new("unused"), &failing), (String::new(), 0, vec!["변환 실패".to_owned()]));
}

#[test]
fn images_have_no_text() {
    assert_eq!(read_from_image(), (String::new(), 0, 0));
    assert!(!find_libreoffice().as_os_str().is_empty());
}

mod docx_reader {
    use crate::common::shared_fixtures::fixture;
    use std::fs;

    use doc_reader::office::read_from_docx;
    use serde_json::Value;

    #[test]
    fn docx_reads_paragraph_text() {
        let expected: Value = serde_json::from_str(&fs::read_to_string(fixture("docx_expected.json")).unwrap()).unwrap();
        for (name, want) in expected.as_object().unwrap() {
            let (summary, lines) = read_from_docx(&fixture(name)).unwrap();
            assert_eq!(summary, want[0].as_str().unwrap(), "{name}");
            assert_eq!(lines as i64, want[1].as_i64().unwrap(), "{name}");
        }
    }

    #[test]
    fn a_utf8_bom_on_docx_xml_parts_is_ignored() {
        use crate::common::zipx::{ZipMode, member_names, read_zip_entry, write_zip_owned};
        let tmp = tempfile::tempdir().unwrap();
        let with_bom = tmp.path().join("bom.docx");
        let source = fixture("a.docx");
        let members: Vec<(String, Vec<u8>)> = member_names(&source)
            .into_iter()
            .map(|name| {
                let mut data = read_zip_entry(&source, &name);
                if name.ends_with(".xml") || name.ends_with(".rels") {
                    data.splice(0..0, *b"\xef\xbb\xbf");
                }
                (name, data)
            })
            .collect();
        write_zip_owned(&with_bom, &members, ZipMode::Deflated);
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

    #[test]
    fn paragraph_properties_are_skipped_and_a_missing_relationship_part_is_an_error() {
        use crate::common::zipx::write_zip_text;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("p.docx");
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;
        let document = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr/><w:r><w:t>글</w:t></w:r></w:p></w:body></w:document>"#;
        write_zip_text(&path, &[("_rels/.rels", rels), ("word/document.xml", document)], true);
        assert_eq!(read_from_docx(&path).unwrap(), ("글".to_owned(), 1));
        write_zip_text(&path, &[("word/document.xml", document)], true);
        assert_eq!(read_from_docx(&path).unwrap_err(), "ZIP 항목이 없다: _rels/.rels");
    }

    #[test]
    fn a_corrupt_document_part_is_an_io_error() {
        use crate::common::zipx::{ZipMode, bytes_replace, member_names, read_zip_entry, write_zip_owned};
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture("a.docx");
        let members: Vec<(String, Vec<u8>)> = member_names(&source).into_iter().map(|name| (name.clone(), read_zip_entry(&source, &name))).collect();
        let corrupt = tmp.path().join("crc.docx");
        write_zip_owned(&corrupt, &members, ZipMode::Stored);
        // 저장 방식 멤버의 내용을 같은 길이로 바꾸면 CRC 검사에서 읽기가 실패한다.
        let bytes = bytes_replace(&fs::read(&corrupt).unwrap(), b"<w:body>", b"<w:bodx>");
        fs::write(&corrupt, bytes).unwrap();
        let error = read_from_docx(&corrupt).unwrap_err();
        assert!(!error.contains("ZIP 항목이 없다") && !error.contains("XML"), "{error}");
    }
}

// ---- 실행기를 주입하는 변환과 실행 파일 찾기 (book_manager 테스트에서 옮김) ----

fn no_output_message(result: Result<String, ConvertFailure>) -> String {
    match result {
        Err(ConvertFailure::NoOutput(message)) => message,
        other => panic!("결과 파일 없음이어야 한다: {other:?}"),
    }
}

#[test]
fn libreoffice_without_an_output_file_reports_the_directory_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("unrelated.log"), "x").unwrap();
    let runner = FakeRunner::returning(Ok(output(0, "", "warn: nothing written")));

    let message = no_output_message(convert_with_libreoffice_using(&*runner, Path::new("soffice"), Some(dir.path()), Path::new("/books/book.hwp"), "txt:Text"));

    assert!(message.contains("LibreOffice produced no output") && message.contains("format='txt:Text'"), "{message}");
    assert!(message.contains("unrelated.log") && message.contains("warn: nothing written"), "{message}");
}

#[test]
fn test_convert_with_libreoffice_success_and_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    let fake_bin = tmp.path().join("libreoffice");
    fs::write(&fake_bin, "").unwrap();

    // 출력 파일 이름(stem)이 달라서 확장자로 찾는 대체 경로를 탄다
    let runner = FakeRunner::new(|argv| {
        let idx = argv.iter().position(|a| a == "--outdir").unwrap();
        let outdir = PathBuf::from(&argv[idx + 1]);
        fs::create_dir_all(&outdir).unwrap();
        fs::write(outdir.join("other.txt"), "ok").unwrap();
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    });
    assert_eq!(convert_with_libreoffice_using(&*runner, &fake_bin, None, &tmp.path().join("x.doc"), "txt").unwrap(), "ok");

    let runner_empty = FakeRunner::new(|_argv| Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] }));
    no_output_message(convert_with_libreoffice_using(&*runner_empty, &fake_bin, None, &tmp.path().join("x.doc"), "txt"));
}

#[test]
fn test_find_libreoffice_mac_path() {
    let mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
    let found = find_libreoffice_with(&|_cmd: &str| None, &|p: &Path| p.to_str() == Some(mac_path));
    assert_eq!(found, PathBuf::from(mac_path));
}

#[test]
fn test_convert_with_libreoffice_direct_output() {
    let tmp = tempfile::tempdir().unwrap();
    let sample = tmp.path().join("sample.docx");
    fs::write(&sample, "doc").unwrap();

    // 임시 디렉터리 대신 tmp를 작업 디렉터리로 쓴다
    let work_dir = tmp.path().to_path_buf();
    let runner = FakeRunner::new(move |_argv| {
        fs::write(work_dir.join("sample.html"), "ok").unwrap();
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    });
    assert_eq!(convert_with_libreoffice_using(&*runner, Path::new("lo"), Some(tmp.path()), &sample, "html").unwrap(), "ok");
}

#[test]
fn test_convert_with_libreoffice_no_output() {
    let tmp = tempfile::tempdir().unwrap();
    let sample = tmp.path().join("sample.docx");
    fs::write(&sample, "doc").unwrap();

    let work_dir = tmp.path().to_path_buf();
    let runner = FakeRunner::new(move |_argv| {
        for entry in fs::read_dir(&work_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "html") {
                fs::remove_file(path).unwrap();
            }
        }
        Ok(CommandOutput { returncode: 1, stdout: vec![], stderr: b"err".to_vec() })
    });
    let message = no_output_message(convert_with_libreoffice_using(&*runner, Path::new("lo"), Some(tmp.path()), &sample, "html"));
    assert!(message.contains("returncode=1") && message.contains("stderr=err"), "{message}");
}

#[test]
fn test_find_libreoffice_which_found() {
    let found = find_libreoffice_with(&|cmd: &str| if cmd == "libreoffice" { Some(PathBuf::from("/usr/bin/libreoffice")) } else { None }, &|p: &Path| p.exists());
    assert_eq!(found, PathBuf::from("/usr/bin/libreoffice"));
}

#[test]
fn test_find_libreoffice_fallback() {
    let found = find_libreoffice_with(&|_cmd: &str| None, &|_p: &Path| false);
    assert_eq!(found, PathBuf::from("libreoffice"));
}
