mod common;
mod corpus;

use std::fs;

use crate::common::shared_fixtures::fixture;
use doc_reader::rtf::{read_from_rtf_text, rtf_to_text};
use serde_json::Value;

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
fn corpus_rtf_matches_the_recorded_summaries() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_rtf.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let got = fs::read(root.join(path)).ok().and_then(|bytes| String::from_utf8(bytes).ok()).and_then(|text| read_from_rtf_text(&text).ok()).unwrap_or_default();
        assert_eq!(corpus::fnv1a64(&got.0), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(got.1 as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
    }
}

#[test]
fn font_charsets_pick_the_codec_for_hex_bytes() {
    let with_charset = |charset: u32, hex: &str| rtf_to_text(&format!(r"{{\rtf1{{\fonttbl{{\f0\fcharset{charset} F;}}}}\f0 {hex}}}")).unwrap();
    assert_eq!(with_charset(128, r"\'82\'a0"), "あ");
    assert_eq!(with_charset(134, r"\'c4\'e3"), "你");
    assert_eq!(with_charset(136, r"\'a4\'a4"), "中");
    assert_eq!(with_charset(222, r"\'a1"), "ก");
    assert_eq!(with_charset(77, r"\'8a"), "ä");
    assert_eq!(with_charset(84, r"\'80"), "А");
    // encoding_rs가 모르는 코덱은 UTF-8로 읽고 깨진 바이트는 버린다.
    assert_eq!(with_charset(80, r"\'41\'ff"), "A");
    // 목록에 없는 코드 페이지는 UTF-8로 본다.
    assert_eq!(rtf_to_text(r"{\rtf1\ansi\ansicpg65001 \'ea\'b0\'80}").unwrap(), "가");
}

#[test]
fn unicode_skips_and_unbalanced_braces() {
    // 인수 없는 `\u`도 대체 글자 하나를 건너뛴다.
    assert_eq!(rtf_to_text(r"{\rtf1\uc1 \u \'41B}").unwrap(), "B");
    // 짝이 없는 닫는 중괄호 뒤의 글은 버린다.
    assert_eq!(rtf_to_text(r"{\rtf1 a}}b").unwrap(), "a");
}
