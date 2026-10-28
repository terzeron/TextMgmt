use std::fs;
use std::path::Path;

use doc_reader::mojibake::{count_mojibake_runs, is_usable_text};
use doc_reader::pytext::{collapse_spaces, head_chars, is_space, keep_ascii_hangul, keep_word_chars, universal_newlines};
use doc_reader::text::{Enc, decode, detect_hint, high_byte_paired_ratio, read_from_text, reencode_text_file_to_utf8, summarize};

#[path = "corpus.rs"]
mod corpus;

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn python_string_helpers() {
    assert!(is_space('\u{1c}') && is_space('\u{a0}') && is_space('\u{3000}') && !is_space('a'));
    assert_eq!(head_chars("가나다라", 2), "가나");
    assert_eq!(head_chars("ab", 5), "ab");
    assert_eq!(collapse_spaces("  a \t\n b\u{a0}c  "), "a b c");
    assert_eq!(keep_ascii_hangul("a,b가_나\u{3131}é"), "a b가 나\u{3131} ");
    assert_eq!(keep_word_chars("a,b가_나é!"), "a b가_나é ");
    assert_eq!(universal_newlines("a\r\nb\rc\nd"), "a\nb\nc\nd");
}

#[test]
fn python_313_isspace_matches_unicode_151_for_all_codepoints() {
    // Python 3.13.7 / Unicode 15.1.0 `chr(cp).isspace()` oracle.
    const RANGES: [(u32, u32); 10] =
        [(0x0009, 0x000d), (0x001c, 0x0020), (0x0085, 0x0085), (0x00a0, 0x00a0), (0x1680, 0x1680), (0x2000, 0x200a), (0x2028, 0x2029), (0x202f, 0x202f), (0x205f, 0x205f), (0x3000, 0x3000)];
    for codepoint in 0..=0x10ffff {
        let expected = RANGES.iter().any(|(start, end)| codepoint >= *start && codepoint <= *end);
        let actual = char::from_u32(codepoint).is_some_and(is_space);
        assert_eq!(actual, expected, "U+{codepoint:04X}");
    }
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
fn decode_follows_python_codec_semantics() {
    assert_eq!(decode(Enc::Utf16, b"\xff\xfea\0", false).as_deref(), Some("a"));
    assert_eq!(decode(Enc::Utf16, b"\xfe\xff\0a", false).as_deref(), Some("a"));
    assert_eq!(decode(Enc::Utf16, b"a\0", false).as_deref(), Some("a"), "BOM이 없으면 리틀엔디언");
    assert_eq!(decode(Enc::Utf16Le, b"\xff\xfea\0", false).as_deref(), Some("\u{feff}a"), "utf-16-le는 BOM을 지우지 않는다");
    assert_eq!(decode(Enc::Utf16Le, b"a", false), None, "홀수 바이트는 오류");
    assert_eq!(decode(Enc::Utf8, b"\xef\xbb\xbfa", false).as_deref(), Some("\u{feff}a"));
    assert_eq!(decode(Enc::Utf8Sig, b"\xef\xbb\xbfa", false).as_deref(), Some("a"));
    assert_eq!(decode(Enc::Utf8, b"a\xff", false), None);
    assert_eq!(decode(Enc::Utf8, b"a\xff", true).as_deref(), Some("a\u{fffd}"));
    assert_eq!(decode(Enc::Cp1252, b"caf\xe9", false).as_deref(), Some("café"));
    assert_eq!(Enc::Cp949.name(), "cp949");
}

#[test]
fn detect_hint_uses_bom_and_high_byte_pairing() {
    assert_eq!(detect_hint(b"\xef\xbb\xbfx"), Some(Enc::Utf8Sig));
    assert_eq!(detect_hint(b"\xff\xfex\0"), Some(Enc::Utf16));
    assert_eq!(detect_hint(b"plain ascii"), None);
    let (korean, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    assert_eq!(detect_hint(&korean), Some(Enc::EucKr));
    assert_eq!(detect_hint(b"caf\xe9 au lait"), Some(Enc::Cp1252));
    assert_eq!(high_byte_paired_ratio(b"abc"), None);
    assert_eq!(high_byte_paired_ratio(b"a\xe9b\xe9\xe9"), Some(2.0 / 3.0));
    assert_eq!(high_byte_paired_ratio(b"\xe9"), Some(0.0), "끝에 오는 구간도 센다");
}

#[test]
fn mojibake_runs_count_latin1_misreads_of_euc_kr() {
    // "안녕하세요"를 EUC-KR 바이트로 만든 뒤 latin-1로 잘못 읽은 글자들
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    let mojibake: String = bytes.iter().map(|&b| b as char).collect();
    assert_eq!(count_mojibake_runs(&mojibake), 1);
    assert_eq!(count_mojibake_runs(&format!("{mojibake} 중간 {mojibake}")), 2);
    assert_eq!(count_mojibake_runs("café résumé"), 0);
    assert_eq!(count_mojibake_runs("정상 한국어"), 0);
    assert!(is_usable_text("정상 한국어", None));
    assert!(!is_usable_text("   ", None));
    assert!(!is_usable_text("x", Some(3)));
    assert!(is_usable_text("x", Some(2)));
}

#[test]
fn reencode_converts_cp949_in_place_and_leaves_utf8_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요 반갑습니다\n둘째 줄입니다\n");
    let path = write(tmp.path(), "k.txt", &bytes);
    let inode = std::os::unix::fs::MetadataExt::ino(&fs::metadata(&path).unwrap());

    let dry = reencode_text_file_to_utf8(&path, true, None, Path::new("k.txt"));
    assert!(dry.changed && dry.reason.contains("[dry-run]") && dry.reason.contains("euc-kr"), "{dry:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes.as_ref(), "dry-run은 파일을 바꾸지 않는다");

    let backup = tmp.path().join("backup");
    let done = reencode_text_file_to_utf8(&path, false, Some(&backup), Path::new("sub/k.txt"));
    assert!(done.changed, "{done:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "안녕하세요 반갑습니다\n둘째 줄입니다\n");
    assert_eq!(fs::read(backup.join("sub/k.txt")).unwrap(), bytes.as_ref());
    assert_eq!(std::os::unix::fs::MetadataExt::ino(&fs::metadata(&path).unwrap()), inode, "같은 inode에 덮어쓴다");
    assert!(!tmp.path().join("k.txt.utf8.tmp").exists());

    let again = reencode_text_file_to_utf8(&path, false, None, Path::new("k.txt"));
    assert_eq!((again.changed, again.reason.as_str()), (false, "이미 UTF-8"));
    let empty = write(tmp.path(), "e.txt", b"");
    assert_eq!(reencode_text_file_to_utf8(&empty, false, None, Path::new("e.txt")).reason, "빈 파일");
    assert!(reencode_text_file_to_utf8(&tmp.path().join("none"), false, None, Path::new("n")).reason.starts_with("읽기 실패"));
}

#[test]
fn reencode_refuses_when_nothing_decodes_losslessly_or_the_backup_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let garbage = write(tmp.path(), "g.txt", b"\x80\x81\x82\xff\xfe\x00\x01 abc \xfd");
    let r = reencode_text_file_to_utf8(&garbage, false, None, Path::new("g.txt"));
    assert!(!r.changed, "{r:?}");

    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요 반갑습니다\n");
    let path = write(tmp.path(), "k.txt", &bytes);
    let blocker = write(tmp.path(), "blocker", b"x");
    let r = reencode_text_file_to_utf8(&path, false, Some(&blocker), Path::new("k.txt"));
    assert!(!r.changed && r.reason.starts_with("백업 실패"), "{r:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes.as_ref(), "백업이 실패하면 변환하지 않는다");
}

#[test]
fn western_single_byte_text_is_converted_via_cp1252_not_cp949() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write(tmp.path(), "w.txt", b"Don Sabas\x97a man called caf\xe9 and more text here\n");
    let r = reencode_text_file_to_utf8(&path, false, None, Path::new("w.txt"));
    assert!(r.changed && r.reason.contains("cp1252"), "{r:?}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "Don Sabas\u{2014}a man called café and more text here\n");
}

#[test]
fn corpus_txt_matches_python() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_txt.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let read = read_from_text(&root.join(path)).unwrap_or_default();
        assert_eq!(corpus::fnv1a64(&read.summary), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(read.line_count as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
        assert_eq!(read.raw_content.chars().count() as i64, rec["raw_len"].as_i64().unwrap(), "raw length for {path}");
    }
}
