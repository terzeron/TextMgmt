//! 원본: tests/test_auth_bookstore_gaps.py
//!
//! `auth::observation_hash`의 빈 입력과 Kyobo 링크 상한 분기를 확인한다.
//!
//! 가정한 API:
//! - `backend::auth::observation_hash(value: &str) -> String`
//! - `backend::bookstore::KyoboBookstore::new(verbose: bool) -> Self` (pub 필드 `max_results: usize`)
//! - `backend::bookstore::AbstractBookstore::extract_search_links(&self, soup: &str) -> Vec<String>` (BeautifulSoup 대신 HTML 문자열)
//! - `backend::bookstore::MAX_RESULTS: usize` (서점 하나가 돌려주는 결과 수 기본 상한)

mod common;

use backend::auth;
use backend::bookstore::{AbstractBookstore, KyoboBookstore};

#[test]
fn test_observation_hash_of_empty_value_is_empty_string() {
    common::setup_env();
    assert_eq!(auth::observation_hash(""), "");
}

#[test]
fn test_kyobo_fallback_links_stop_at_max_results() {
    common::setup_env();
    let store = KyoboBookstore::new(false);
    let anchors: String = (0..store.max_results + 5).map(|n| format!(r#"<a href="/detail/{n}">책 {n}</a>"#)).collect();
    let soup = format!("<html><body>{anchors}</body></html>");

    let links = store.extract_search_links(&soup);

    let expected: Vec<String> = (0..store.max_results).map(|n| format!("/detail/{n}")).collect();
    assert_eq!(links, expected);
}
