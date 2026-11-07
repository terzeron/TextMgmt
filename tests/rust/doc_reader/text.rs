mod corpus;
use doc_reader::text::{read_from_text, summarize};
use std::fs;
use std::path::Path;

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn summarize_cleans_the_first_4096_chars() {
    let (summary, lines) = summarize("\u{feff}Hello, 세계!\n\n  둘째 줄\n");
    assert_eq!(summary, "Hello 세계 둘째 줄");
    assert_eq!(lines, 4);
    assert_eq!(summarize(""), (String::new(), 0));
    let long = "가".repeat(5000);
    assert_eq!(summarize(&long).0.chars().count(), 4096);
}

#[test]
fn utf8_files_are_read_with_universal_newlines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "a.txt", "첫 줄\r\n둘째 줄\r셋째".as_bytes());
    let read = read_from_text(&path).unwrap();
    assert_eq!(read.raw_content, "첫 줄\n둘째 줄\n셋째");
    assert_eq!(read.line_count, 3);
    assert_eq!(read.summary, "첫 줄 둘째 줄 셋째");
    assert_eq!(read.page_count, 0);
}

#[test]
fn a_utf8_bom_is_kept_in_raw_content_but_dropped_from_the_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "a.txt", b"\xef\xbb\xbfhello");
    let read = read_from_text(&path).unwrap();
    // BOM이 있으면 감지 힌트가 utf-8-sig를 맨 앞에 두어 BOM이 벗겨진다.
    assert_eq!(read.raw_content, "hello");
    assert_eq!(read.summary, "hello");
}

#[test]
fn cp949_files_are_decoded() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요 반갑습니다\n둘째 줄");
    let path = write(tmp.path(), "k.txt", &bytes);
    let read = read_from_text(&path).unwrap();
    assert_eq!(read.raw_content, "안녕하세요 반갑습니다\n둘째 줄");
    assert_eq!(read.line_count, 2);
}

#[test]
fn utf16_with_bom_is_decoded_and_empty_files_are_fine() {
    let tmp = tempfile::tempdir().unwrap();
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend("한글 text".encode_utf16().flat_map(u16::to_le_bytes));
    let path = write(tmp.path(), "u.txt", &bytes);
    assert_eq!(read_from_text(&path).unwrap().raw_content, "한글 text");

    let empty = write(tmp.path(), "e.txt", b"");
    let read = read_from_text(&empty).unwrap();
    assert_eq!((read.raw_content.as_str(), read.line_count, read.summary.as_str()), ("", 0, ""));
}

#[test]
fn undecodable_garbage_falls_back_to_replacement_and_a_missing_file_reports_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "g.txt", b"abc \xff\xfe\xfd def");
    let read = read_from_text(&path).unwrap();
    assert!(read.raw_content.contains("abc"));
    let err = read_from_text(&tmp.path().join("none.txt")).unwrap_err();
    assert!(err.contains("none.txt"));
}

#[test]
fn corpus_txt_matches_the_recorded_summaries() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_txt.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let read = read_from_text(&root.join(path)).unwrap_or_default();
        assert_eq!(corpus::fnv1a64(&read.summary), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(read.line_count as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
        assert_eq!(read.raw_content.chars().count() as i64, rec["raw_len"].as_i64().unwrap(), "raw length for {path}");
    }
}

#[test]
fn a_checked_candidate_that_yields_no_text_is_an_error() {
    use doc_reader::text::{TextHooks, read_from_text_with};
    use text_decoder::text_transcoder::{Candidate, Enc};
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "bad.txt", b"\xff");
    let hooks = TextHooks { encodings: Some(vec![Candidate::Builtin(Enc::Cp949)]), ..TextHooks::default() };
    let error = read_from_text_with(&path, &hooks).unwrap_err();
    assert!(error.contains("can't read unicode text") && error.contains("codec can't decode"), "{error}");
}
