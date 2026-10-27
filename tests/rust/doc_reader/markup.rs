use doc_reader::markup::{count_lines, get_body_text, get_text, read_from_html};
use serde_json::Value;
use std::fs;

#[path = "corpus.rs"]
mod corpus;

fn cases() -> Value {
    serde_json::from_str(include_str!("markup_cases.json")).unwrap()
}

#[test]
fn get_text_and_line_counts_match_python_lxml() {
    for case in cases().as_array().unwrap() {
        let html = case["html"].as_str().unwrap();
        assert_eq!(get_text(html), case["text"].as_str().unwrap(), "text of {html:?}");
        assert_eq!(get_body_text(html), case["body_text"].as_str().unwrap(), "body text of {html:?}");
        assert_eq!(count_lines(html) as i64, case["lines"].as_i64().unwrap(), "lines of {html:?}");
    }
}

#[test]
fn get_text_keeps_whitespace_before_an_implicit_head() {
    let html = "<!DOCTYPE html>\n<html>\n<body><p>x</p></body></html>";
    assert_eq!(get_text(html), "\nx");
}

#[test]
fn read_from_html_cleans_symbols_and_counts_source_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.html");
    fs::write(&path, "<html><body><p>Hello, 세계!</p>\r\n<p>둘째 (줄)</p></body></html>").unwrap();
    let (summary, lines, pages) = read_from_html(&path).unwrap();
    assert_eq!(summary, "Hello  세계 \n둘째  줄 ");
    assert_eq!((lines, pages), (2, 0));
}

#[test]
fn read_from_html_truncates_to_4096_chars_and_reports_unreadable_files() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("long.html");
    fs::write(&path, format!("<p>{}</p>", "가".repeat(5000))).unwrap();
    assert_eq!(read_from_html(&path).unwrap().0.chars().count(), 4096);
    let bad = tmp.path().join("bad.html");
    fs::write(&bad, b"<p>\xff\xfe</p>").unwrap();
    assert!(read_from_html(&bad).unwrap_err().contains("utf-8"));
    assert!(read_from_html(&tmp.path().join("none.html")).is_err());
}

#[test]
fn corpus_html_matches_python() {
    let root = corpus::books_root();
    for rec in corpus::records(include_str!("fixtures/corpus_html.jsonl")) {
        let path = rec["path"].as_str().unwrap();
        let got = read_from_html(&root.join(path));
        if rec.get("error").is_some() {
            assert!(got.is_err(), "expected Python reader error for {path}: {got:?}");
            continue;
        }
        let (summary, lines, pages) = got.unwrap_or_else(|error| panic!("Rust reader failed for {path}: {error}"));
        assert_eq!(corpus::fnv1a64(&summary), rec["summary_fnv1a64"].as_str().unwrap(), "summary for {path}");
        assert_eq!(lines as i64, rec["line_count"].as_i64().unwrap(), "line count for {path}");
        assert_eq!(pages as i64, rec["page_count"].as_i64().unwrap(), "page count for {path}");
    }
}
