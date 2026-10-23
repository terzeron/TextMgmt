//! backend/classifier/reader.py 중 detect_mojibake가 쓰는 부분의 테스트(tests/test_classifier_reader.py 이식).

mod common;

use common::*;
use corpus_common::reader::*;
use std::fs;

const KOREAN: &str = "그는 아무 말도 하지 않았다 그리고 다시 길을 걸었다 바람이 불었다 ";

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    text.encode_utf16().flat_map(|u| if big_endian { u.to_be_bytes() } else { u.to_le_bytes() }).collect()
}

fn cp949(text: &str) -> Vec<u8> {
    let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode(text);
    assert!(!had_errors);
    bytes.into_owned()
}

#[test]
fn korean_likeness_needs_enough_syllables() {
    assert_eq!(korean_likeness("짧은 글"), None);
    assert_eq!(korean_likeness(""), None);
    let text = KOREAN.repeat(10);
    assert!(text.chars().filter(|c| ('가'..='힣').contains(c)).count() >= MIN_SYLLABLES_FOR_LIKENESS);
    let score = korean_likeness(&text).unwrap();
    assert!(score > 0.0 && score <= 1.0);
}

#[test]
fn korean_likeness_is_low_for_uncommon_syllables() {
    assert_eq!(korean_likeness(&"쀍".repeat(60)), Some(0.0));
}

#[test]
fn decode_best_empty() {
    assert_eq!(decode_best(b""), "");
}

#[test]
fn decode_best_utf16_by_bom() {
    let mut le = vec![0xff, 0xfe];
    le.extend(utf16(KOREAN, false));
    assert!(decode_best(&le).contains("바람이"));
    let mut be = vec![0xfe, 0xff];
    be.extend(utf16(KOREAN, true));
    assert!(decode_best(&be).contains("바람이"));
}

#[test]
fn decode_best_utf16_by_null_density() {
    for big in [false, true] {
        let raw = utf16(&"hello world ".repeat(40), big);
        let nulls = raw.iter().filter(|&&b| b == 0).count();
        assert!(raw.len() >= 200 && nulls * 4 > raw.len());
        assert!(decode_best(&raw).contains("hello world"));
    }
}

#[test]
fn decode_best_plain_utf8() {
    assert_eq!(decode_best(KOREAN.as_bytes()), KOREAN);
}

#[test]
fn decode_best_trims_truncated_utf8_tail() {
    let raw = KOREAN.repeat(5).into_bytes();
    assert!(decode_best(&raw[..raw.len() - 1]).contains("바람이"));
}

#[test]
fn decode_best_cp949() {
    let raw = cp949(&KOREAN.repeat(5));
    assert!(std::str::from_utf8(&raw).is_err());
    assert!(decode_best(&raw).contains("바람이"));
}

#[test]
fn decode_best_cp949_with_truncated_tail() {
    let raw = cp949(&KOREAN.repeat(5));
    assert!(decode_best(&raw[..raw.len() - 1]).contains("바람이"));
}

#[test]
fn decode_best_falls_back_to_most_korean_reading() {
    let encoded = cp949(&KOREAN.repeat(5));
    let mut raw = encoded[..40].to_vec();
    raw.push(0xff);
    raw.extend(&encoded[40..]);
    assert!(std::str::from_utf8(&raw).is_err());
    assert!(encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(&raw).is_none());
    assert!(decode_best(&raw).contains("바람이"));
}

#[test]
fn decode_best_all_garbage_returns_something() {
    assert_eq!(decode_best(&[0xff; 300]), "");
}

#[test]
fn read_txt_text_missing_file() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(read_txt_text(&d.path().join("없는파일.txt"), 100, 10), "");
}

#[test]
fn read_txt_text_head_only() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.txt");
    fs::write(&f, KOREAN).unwrap();
    assert_eq!(read_txt_text(&f, 1000, 100).trim(), KOREAN.trim());
}

#[test]
fn read_txt_text_reads_head_and_tail() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("b.txt");
    fs::write(&f, format!("시작{}끝부분", "가".repeat(5000))).unwrap();
    let text = read_txt_text(&f, 50, 20);
    assert!(text.contains("시작"));
    assert!(text.contains("끝부분"));
}

#[test]
fn read_txt_text_tail_disabled() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("c.txt");
    fs::write(&f, format!("시작{}끝부분", "가".repeat(5000))).unwrap();
    assert!(!read_txt_text(&f, 50, 0).contains("끝부분"));
}

#[test]
fn read_txt_text_unopenable_path_is_empty() {
    let d = tempfile::tempdir().unwrap();
    // 디렉토리는 stat은 되지만 파일로 열어 읽을 수 없다.
    assert_eq!(read_txt_text(d.path(), 100, 10), "");
}

#[test]
fn read_epub_text_without_html_documents() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.epub");
    make_zip(&p, &[("mimetype", "application/epub+zip")]);
    assert_eq!(read_epub_text(&p, 100, 10), "");
}

#[test]
fn read_epub_text_strips_tags_and_skips_front_matter() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("b.epub");
    let cover = format!("<html><body>{}</body></html>", "표지".repeat(100));
    let body = format!("<html><body>{}</body></html>", KOREAN.repeat(5));
    make_zip(&p, &[("cover.xhtml", &cover), ("body1.xhtml", &body)]);
    let text = read_epub_text(&p, 2000, 0);
    assert!(!text.contains("표지"));
    assert!(text.contains("바람이"));
    assert!(!text.contains('<'));
}

#[test]
fn read_epub_text_rotates_when_many_documents() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("c.epub");
    let docs: Vec<(String, String)> = (0..6).map(|i| (format!("ch{i}.xhtml"), format!("<p>장면{i} {}</p>", KOREAN.repeat(3)))).collect();
    let refs: Vec<(&str, &str)> = docs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    make_zip(&p, &refs);
    let text = read_epub_text(&p, 400, 100);
    assert!(text.contains("장면2"));
    let head: String = text.chars().take(400).collect();
    assert!(!head.contains("장면0"));
}

#[test]
fn read_epub_text_skips_short_documents() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("d.epub");
    let long = format!("<p>{}</p>", KOREAN.repeat(5));
    make_zip(&p, &[("a.xhtml", "<p>짧음</p>"), ("b.xhtml", &long)]);
    assert!(!read_epub_text(&p, 2000, 0).contains("짧음"));
}

#[test]
fn read_epub_text_on_broken_zip() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("broken.epub");
    fs::write(&p, b"not a zip at all").unwrap();
    assert_eq!(read_epub_text(&p, 100, 10), "");
}

#[test]
fn read_epub_text_ignores_unreadable_member() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("e.epub");
    let body = format!("<p>{}</p>", KOREAN.repeat(5));
    make_zip(&p, &[("a.xhtml", &body)]);
    corrupt_local_header(&p, "a.xhtml");
    assert_eq!(read_epub_text(&p, 2000, 0), "");
}

#[test]
fn read_document_text_dispatches_by_extension() {
    let d = tempfile::tempdir().unwrap();
    let txt = d.path().join("a.txt");
    fs::write(&txt, KOREAN).unwrap();
    assert!(read_document_text(&txt, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS).contains("바람이"));

    let epub = d.path().join("a.epub");
    let body = format!("<p>{}</p>", KOREAN.repeat(5));
    make_zip(&epub, &[("body.xhtml", &body)]);
    assert!(read_document_text(&epub, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS).contains("바람이"));

    let other = d.path().join("a.pdf");
    fs::write(&other, b"%PDF-1.4").unwrap();
    assert_eq!(read_document_text(&other, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS), "");
}
