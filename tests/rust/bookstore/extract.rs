//! 추출 함수 테스트. 기대값은 Python 원본(BeautifulSoup `html.parser`)이 같은 HTML에서 만든 `expected.json`이다.

use bookstore::{Store, strip_trailing_number, truncate_title};
use serde_json::Value;

fn expected() -> Value {
    serde_json::from_str(include_str!("expected.json")).unwrap()
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/../../tests/rust/bookstore/fixtures/{name}.html", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn store(name: &str) -> Store {
    Store::from_name(name).unwrap()
}

#[test]
fn search_links_match_python() {
    for (name, case) in expected()["links"].as_object().unwrap() {
        let links = store(case[0].as_str().unwrap()).extract_search_links(&fixture(name));
        let want: Vec<&str> = case[1].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(links, want, "{name}");
    }
}

#[test]
fn book_info_matches_python() {
    for (name, case) in expected()["info"].as_object().unwrap() {
        let info = store(case[0].as_str().unwrap()).extract_book_info(&fixture(name));
        let want = &case[1];
        assert_eq!(
            (info.title.as_str(), info.author.as_str(), info.category.as_str(), info.isbn.as_str()),
            (want["title"].as_str().unwrap(), want["author"].as_str().unwrap(), want["category"].as_str().unwrap(), want["isbn"].as_str().unwrap()),
            "{name}"
        );
    }
}

#[test]
fn search_urls_match_python() {
    for (name, urls) in expected()["urls"].as_object().unwrap() {
        let s = store(name);
        assert_eq!(s.build_search_url("원피스 1 & 저자"), urls[0].as_str().unwrap(), "{name}");
        assert_eq!(s.build_isbn_search_url("9788900000001"), urls[1].as_str().unwrap(), "{name}");
    }
}

#[test]
fn title_helpers_match_python() {
    for case in expected()["truncate"].as_array().unwrap() {
        assert_eq!(truncate_title(case[0].as_str().unwrap()), case[1].as_str().unwrap());
    }
    for case in expected()["strip_num"].as_array().unwrap() {
        assert_eq!(strip_trailing_number(case[0].as_str().unwrap()), case[1].as_str().unwrap());
    }
}

#[test]
fn stores_know_their_names_and_capabilities() {
    assert_eq!(Store::ALL.map(|s| s.name()), ["yes24", "aladin", "kyobo"]);
    assert!(Store::from_name("nope").is_none());
    assert!(Store::Yes24.supports_isbn_search() && Store::Aladin.supports_isbn_search() && Store::Kyobo.supports_isbn_search());
    assert!(Store::Aladin.author_first() && !Store::Yes24.author_first());
    assert!(Store::Kyobo.user_agent().contains("Mozilla") && Store::Yes24.user_agent().starts_with("curl/"));
}

#[test]
fn malformed_html_yields_empty_results_not_a_panic() {
    for s in Store::ALL {
        assert!(s.extract_search_links("").is_empty());
        let info = s.extract_book_info("<<<not html");
        assert_eq!(info.title, "");
    }
}
