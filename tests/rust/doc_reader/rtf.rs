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
fn corpus_rtf_matches_python() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_rtf.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let got = fs::read(root.join(path)).ok().and_then(|bytes| String::from_utf8(bytes).ok()).and_then(|text| read_from_rtf_text(&text).ok()).unwrap_or_default();
        assert_eq!(corpus::fnv1a64(&got.0), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(got.1 as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
    }
}
