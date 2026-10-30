//! RTF 위장 테스트. fixture는 합성이다. 이름은 Word 문서인데 내용이 RTF인 파일과 `.rtf` 파일을 본다.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookdoctor::cli::{Options, run};
use bookdoctor::scan::scan_file;
use scan_common::{FileReport, Finding, Format, Severity};

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

fn finding<'a>(r: &'a FileReport, code: &str) -> &'a Finding {
    r.findings.iter().find(|f| f.code == code).unwrap_or_else(|| panic!("{code} missing: {:?} / {:?}", r.findings, r.reasons))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

fn pe_bytes() -> Vec<u8> {
    let mut pe = vec![0u8; 0x100];
    pe[..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    pe[0x40..0x44].copy_from_slice(b"PE\0\0");
    pe
}

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
fn rtf_is_collected_and_other_content_named_rtf_is_a_mismatch() {
    let d = tempfile::tempdir().unwrap();
    // 깨끗한 파일은 출력에 나오지 않으므로 지표가 나오는 내용을 쓴다.
    fs::write(d.path().join("a.RTF"), b"{\\rtf1{\\*\\objclass X}}").unwrap();
    fs::write(d.path().join("b.rtfx"), b"{\\rtf1{\\*\\objclass X}}").unwrap();
    let mut out = Vec::new();
    run(&["-r".into(), "--min-severity".into(), "low".into(), "--json".into(), d.path().to_string_lossy().into_owned()], &mut out);
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("a.RTF"), "{out}");
    assert!(!out.contains("b.rtfx"), "{out}");
    let r = scan("c.rtf", "그냥 텍스트\n".repeat(50).as_bytes());
    assert!(r.has("FORMAT_MISMATCH"), "{:?}", r.findings);
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
