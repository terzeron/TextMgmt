use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use bookstore::{Fetcher, HttpFetcher, Store, search};

struct Fake {
    pages: HashMap<String, Result<String, String>>,
    requested: Mutex<Vec<String>>,
}

impl Fake {
    fn new(pages: Vec<(&str, Result<&str, &str>)>) -> Self {
        Fake { pages: pages.into_iter().map(|(u, r)| (u.to_owned(), r.map(str::to_owned).map_err(str::to_owned))).collect(), requested: Mutex::new(Vec::new()) }
    }
}

impl Fetcher for Fake {
    fn get(&self, _store: Store, url: &str) -> Result<String, String> {
        self.requested.lock().unwrap().push(url.to_owned());
        self.pages.get(url).cloned().unwrap_or_else(|| Err("404".into()))
    }
}

fn detail(title: &str, cat: &str) -> String {
    format!(r#"<h2 class="gd_name">{title}</h2><span class="gd_auth"><a>저자</a></span><div>관련분류 <a href="/product/category/display/1">{cat}</a></div> ISBN13 9788900000001"#)
}

const SEARCH: &str = "https://www.yes24.com/Product/Search?domain=ALL&query=";

fn yes24_results(links: &[&str]) -> String {
    let items: String =
        links.iter().map(|l| format!(r#"<li><div class="itemUnit"><div class="item_info"><div class="info_row info_name"><a class="gd_name" href="{l}">x</a></div></div></div></li>"#)).collect();
    format!(r#"<ul id="yesSchList">{items}</ul>"#)
}

#[test]
fn search_fetches_details_in_order_and_skips_failures_and_empty_pages() {
    let search_url = format!("{SEARCH}%EC%A0%9C%EB%AA%A9");
    let page = yes24_results(&["/product/goods/1", "/product/goods/2", "/product/goods/3", "/product/goods/4", "/product/goods/5"]);
    let fake = Fake::new(vec![
        (&search_url, Ok(&page)),
        ("https://www.yes24.com/product/goods/1", Ok(&detail("첫째", "만화 > 일본 > 소년 > 더깊이"))),
        ("https://www.yes24.com/product/goods/2", Err("timeout")),
        ("https://www.yes24.com/product/goods/3", Ok("   ")),
        ("https://www.yes24.com/product/goods/4", Ok(&detail("넷째", "소설"))),
    ]);
    let out = search(Store::Yes24, &fake, "", "제목", "");
    assert_eq!(out.method, "title");
    assert_eq!(out.keyword, "제목");
    let titles: Vec<_> = out.rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(titles, ["첫째", "넷째"]);
    assert_eq!(out.rows[0].category, "만화 > 일본 > 소년", "카테고리 문자열은 > 기준 세 단계까지");
    assert_eq!(out.rows[0].detail_url, "https://www.yes24.com/product/goods/1");
    assert_eq!(out.rows[0].search_url, search_url);
    assert_eq!(out.rows[0].isbn, "9788900000001");
    // 상세 페이지는 앞의 4개만 요청한다.
    let requested = fake.requested.lock().unwrap();
    assert_eq!(requested.len(), 1 + 4);
    assert!(!requested.iter().any(|u| u.ends_with("/5")));
}

#[test]
fn a_failed_search_page_gives_no_rows() {
    let fake = Fake::new(vec![]);
    let out = search(Store::Yes24, &fake, "", "제목", "");
    assert!(out.rows.is_empty());
    assert_eq!((out.keyword.as_str(), out.method), ("제목", "title"));
}

#[test]
fn title_and_author_are_tried_before_title_only_and_aladin_puts_the_author_first() {
    let fake = Fake::new(vec![]);
    let out = search(Store::Yes24, &fake, "", "제목", "저자");
    assert_eq!((out.keyword.as_str(), out.method), ("제목 저자", "title_author"));
    assert_eq!(fake.requested.lock().unwrap().len(), 2, "조합 검색 뒤 제목만 검색");

    let fake = Fake::new(vec![]);
    let out = search(Store::Aladin, &fake, "", "마왕의 딸 3", "저자");
    assert_eq!((out.keyword.as_str(), out.method), ("저자 마왕의 딸", "title_author"), "알라딘은 권수를 떼고 저자를 앞에 둔다");
}

#[test]
fn the_combined_search_wins_when_it_has_rows() {
    let url = format!("{SEARCH}%EC%A0%9C%EB%AA%A9%20%EC%A0%80%EC%9E%90");
    let page = yes24_results(&["/product/goods/9"]);
    let fake = Fake::new(vec![(&url, Ok(&page)), ("https://www.yes24.com/product/goods/9", Ok(&detail("결과", "만화")))]);
    let out = search(Store::Yes24, &fake, "", "제목", "저자");
    assert_eq!((out.keyword.as_str(), out.method, out.rows.len()), ("제목 저자", "title_author", 1));
}

#[test]
fn isbn_search_comes_first_and_the_subtitle_is_cut() {
    let url = format!("{SEARCH}9788900000001");
    let page = yes24_results(&["/product/goods/7"]);
    let fake = Fake::new(vec![(&url, Ok(&page)), ("https://www.yes24.com/product/goods/7", Ok(&detail("isbn 결과", "만화")))]);
    let out = search(Store::Yes24, &fake, "9788900000001", "제목 - 부제", "저자");
    assert_eq!((out.keyword.as_str(), out.method), ("9788900000001", "isbn"));

    let fake = Fake::new(vec![]);
    let out = search(Store::Yes24, &fake, "", "제목 - 부제", "");
    assert_eq!(out.keyword, "제목", "부제 구분자 왼쪽만 쓴다");
}

#[test]
fn nothing_to_search_for_reports_unknown_or_the_isbn() {
    let fake = Fake::new(vec![]);
    let out = search(Store::Yes24, &fake, "", "", "");
    assert_eq!((out.keyword.as_str(), out.method, out.rows.len()), ("", "unknown", 0));
    assert!(fake.requested.lock().unwrap().is_empty());
    let out = search(Store::Yes24, &fake, "9788900000001", "", "");
    assert_eq!(out.method, "isbn");
}

fn mock_http(response: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).unwrap();
        stream.write_all(response).unwrap();
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    (url, handle)
}

#[test]
fn http_fetcher_sends_the_store_headers_and_decodes_utf8_leniently() {
    let (url, server) = mock_http(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\nConnection: close\r\n\r\nhi \xff!!");
    let html = HttpFetcher::new().get(Store::Kyobo, &url).unwrap();
    assert_eq!(html, "hi \u{fffd}!!");
    let request = server.join().unwrap().to_lowercase();
    assert!(request.contains("user-agent: mozilla/5.0"), "{request}");
    assert!(request.contains("referer: https://www.kyobobook.co.kr/"), "{request}");
    assert!(request.contains("accept-language: ko-kr"), "{request}");

    let (url, server) = mock_http(b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
    assert_eq!(HttpFetcher::new().get(Store::Yes24, &url).unwrap(), "nope", "상태 코드와 무관하게 본문을 쓴다(원본과 같다)");
    let request = server.join().unwrap().to_lowercase();
    assert!(request.contains("user-agent: curl/7.79.1") && request.contains("referer: https://www.yes24.com"), "{request}");
}

#[test]
fn http_fetcher_reports_connection_errors() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/x", listener.local_addr().unwrap());
    drop(listener);
    assert!(HttpFetcher::new().get(Store::Yes24, &url).is_err());
}
