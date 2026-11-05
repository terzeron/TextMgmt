//!
//! 서점 크롤러 테스트. HTML 파싱, 검색 우선순위(ISBN > 제목+저자 > 제목),
//! 검색 결과 캐시, 웹툰 서점 JSON API 응답 처리를 확인한다.
//! 서점 HTTP는 `common::http_client::HttpSession` 대역으로 갈아 끼운다(docs/rust-test-seams.md 3.7. libs/bookstore의 `Fetcher` 대역과 같은 방식).
//!
//! 가정한 API:
//! - `bookstore::store_clients`
//!   - `struct SearchRow { title, author, category, detail_url, search_url, isbn }`
//!   - `const MAX_RESULTS: usize = 4` (`Bookstore.MAX_RESULTS`)
//!   - `fn truncate_title(title: &str) -> String`
//!   - `common::http_client::{HttpSession, HttpResponse, HttpError}` (구 `bookstore::{Session, HttpResponse}`)
//!     - `trait HttpSession: Send + Sync { fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError>; }` (`requests.Session.get` 대응. 예외는 `Err`)
//!     - `HttpResponse::from_text(status: u16, text: &str) -> HttpResponse` (fake 생성용. `.text()`/`.json()`은 본문을 소비한다)
//!     - `HttpError::Other(String)` (문서에 변형 목록이 없어 `EsError::Other`와 같은 모양으로 가정한 것)
//!   - `trait BookstoreSearch { fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String); }`
//!   - `trait Bookstore`: markup 인자는 HTML 문자열(`&str`)이다. 기본값이 있는 메서드는 `[기본]`으로 표시한다.
//!     - 필수: `base_url(&self) -> &str`, `verbose(&self) -> bool`, `session(&self) -> &dyn HttpSession`,
//!       `build_search_url(&self, keyword: &str) -> String`, `extract_search_links(&self, markup: &str) -> Vec<String>`,
//!       `extract_book_info(&self, markup: &str) -> Value` (`{title, author, category, isbn}`)
//!     - `[기본]` `supports_isbn_search() -> bool`(false), `author_first_search() -> bool`(false), `max_results() -> usize`(`MAX_RESULTS`)
//!     - `[기본]` `build_isbn_search_url(isbn)`(= `build_search_url(isbn)`), `search_by_keyword(keyword)`, `search_by_isbn(isbn)`
//!     - `[기본]` `search(isbn, title, author) -> (Vec<SearchRow>, String, String)` (결과, 실제 검색어, 방식 `isbn`/`title_author`/`title`/`unknown`)
//!     - `[기본]` `fetch_search_results(url) -> Vec<SearchRow>`, `load_html_from_tmp(url) -> Option<String>`, `save_html_to_tmp(html, url)`
//!     - `[기본]` `tmp_dir() -> Option<PathBuf>`(`default_cache_dir()`, 사용자 전용 0700 디렉터리)
//!       (구 `new_detail_session()`은 없앴다. 상세 페이지 스레드도 `with_session`으로 주입한 같은 `Arc<dyn HttpSession>`을 쓴다)
//!   - 구상 클래스 `Yes24Bookstore`, `AladinBookstore`, `KyoboBookstore`, `RidibooksBookstore`, `NaverShoppingBookstore`,
//!     `NaverSeriesBookstore`, `MunpiaBookstore`, `JoaraBookstore`, `NaverWebtoonBookstore`, `KakaoWebtoonBookstore`,
//!     `BomtoonBookstore`, `LezhinBookstore`, `ToptoonBookstore`: `new(verbose: bool) -> Self`,
//!     `with_session(self, Arc<dyn HttpSession>) -> Self` (구 pub 필드 `session`), pub 필드 `max_results: usize`, `supports_isbn_search: bool`
//!   - `Yes24Bookstore::extract_yes24_category(&self, markup: &str) -> String`, `extract_yes24_isbn`,
//!     `AladinBookstore::extract_aladin_isbn`, `strip_trailing_number(&self, title: &str) -> String`,
//!     `RidibooksBookstore::extract_ridi_isbn`
//!   - 대역 주입 seam(mock.patch 대응, 구상 클래스 공통): `with_fetch_search_results(self, ..) -> Self`, `with_search_by_keyword(self, ..) -> Self`
//!     (`Box<dyn Fn(&str) -> Vec<SearchRow> + Send + Sync>`. 구 `set_*`)
//!   - `trait MarkupSource`: `find(&self, name: &str) -> Result<Option<String>, String>`, `find_all(&self, name: &str) -> Result<Vec<String>, String>`,
//!     `get_text(&self) -> Result<String, String>`, `select(&self, css: &str) -> Result<Vec<String>, String>` (예외를 던지는 markup 대역용)
//!     - `Yes24Bookstore::extract_yes24_category_from_source/_extract_yes24_isbn_from_source/extract_book_info_from_source(&self, &dyn MarkupSource)`,
//!       `AladinBookstore::extract_aladin_isbn_from_source`, `RidibooksBookstore::extract_ridi_isbn_from_source`,
//!       `NaverShoppingBookstore::extract_book_info_from_source`
//!   - `sysdeps::FsOps` (구 `StoreFs`. 이 파일은 `disk_free`와 `write`만 쓴다. 나머지 메서드는 trait 기본 구현), `Yes24Bookstore::with_fs(self, Arc<dyn FsOps>) -> Self`
//!     (`shutil.disk_usage`와 `open` 대응. `write`의 `data`는 UTF-8 HTML 바이트)

#![allow(clippy::type_complexity)]

#[path = "../common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use crate::common::shared_fixtures::{Kind, failing};
use crate::common::shared_fixtures::{LoadHook, SaveHook, SearchHook, between, single_quoted_hrefs};
use ::common::http_client::HttpSession;
use bookstore::BookInfo;
use bookstore::store_clients::{AladinBookstore, Bookstore, MAX_RESULTS, SearchRow, Yes24Bookstore, store_session};

#[test]
fn ureq_session_preserves_http_status_headers_body_and_query() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        assert!(request.contains("/search?q=rust HTTP/1.1"), "{request}");
        let mut has_user_agent = false;
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header == "\r\n" || header.is_empty() {
                break;
            }
            has_user_agent |= header.to_ascii_lowercase().starts_with("user-agent: mozilla/");
        }
        assert!(has_user_agent);
        stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy").unwrap();
    });
    let response = AladinBookstore::new(false).session().get(&format!("http://{address}/search"), &[("q".to_string(), "rust".to_string())]).unwrap();
    assert_eq!(response.status_code, 503);
    assert_eq!(response.header("content-type"), Some("text/plain"));
    assert_eq!(response.text(), "busy");
    server.join().unwrap();
}

/// `get`이 보낸 요청의 User-Agent 값을 돌려준다.
fn user_agent_sent_by(get: impl FnOnce(&str)) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut user_agent = String::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            if header == "\r\n" || header.is_empty() {
                break;
            }
            if header.to_ascii_lowercase().starts_with("user-agent:") {
                user_agent = header["user-agent:".len()..].trim().to_string();
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
        user_agent
    });
    get(&format!("http://{address}/ua"));
    server.join().unwrap()
}

// yes24는 브라우저형 UA의 첫 요청을 쿠키를 심고 메인 페이지로 302한다. 세션은 쿠키를 저장하지 않으므로 검색 결과를 못 받는다.
#[test]
fn yes24_sessions_send_a_non_browser_user_agent() {
    let search = user_agent_sent_by(|url| {
        Yes24Bookstore::new(false).session().get(url, &[]).unwrap();
    });
    let cover = user_agent_sent_by(|url| {
        store_session("yes24").get(url, &[]).unwrap();
    });

    assert_eq!(search, "curl/7.79.1");
    assert_eq!(cover, "curl/7.79.1");
}

#[test]
fn other_store_sessions_keep_the_browser_user_agent() {
    let aladin = user_agent_sent_by(|url| {
        AladinBookstore::new(false).session().get(url, &[]).unwrap();
    });
    let ridi = user_agent_sent_by(|url| {
        store_session("ridi").get(url, &[]).unwrap();
    });

    assert!(aladin.starts_with("Mozilla/5.0"), "{aladin}");
    assert!(ridi.starts_with("Mozilla/5.0"), "{ridi}");
}

#[test]
fn ridi_session_sends_store_headers_and_preserves_redirect_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        reader.read_line(&mut request).unwrap();
        assert!(request.starts_with("GET /redirect HTTP/1.1"), "{request}");
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            headers.push_str(&line.to_ascii_lowercase());
        }
        assert!(headers.contains("referer: https://ridibooks.com/"), "{headers}");
        assert!(headers.contains("accept: application/json,text/html"), "{headers}");
        assert!(headers.contains("accept-language: ko-kr,ko;q=0.9"), "{headers}");
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /target\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmoved").unwrap();
    });

    let response = store_session("ridi").get(&format!("http://{address}/redirect"), &[]).unwrap();

    assert_eq!(response.status_code, 302);
    assert_eq!(response.text(), "moved");
    server.join().unwrap();
}

/// 테스트 서점 구조체를 만든다. `$over`에 넘긴 메서드만 기본 구현 대신 덮어쓴다.
/// 인스턴스 메서드를 바꾸던 곳은 이 덮어쓰기와 훅 필드로 옮긴다.
macro_rules! dummy_store {
    ($name:ident { $($over:item)* }) => {
        #[allow(dead_code)]
        struct $name {
            kind: Kind,
            base_url: String,
            verbose: bool,
            supports_isbn_search: bool,
            author_first_search: bool,
            max_results: usize,
            session: Arc<dyn HttpSession>,
            /// `extract_search_links` 대체값
            links: Option<Vec<String>>,
            tmp_dir: Option<PathBuf>,
            fetch: Option<SearchHook>,
            keyword: Option<SearchHook>,
            isbn: Option<SearchHook>,
            load: Option<LoadHook>,
            save: Option<SaveHook>,
        }

        #[allow(dead_code)]
        impl $name {
            fn new(kind: Kind) -> Self {
                let (base_url, supports_isbn_search, author_first_search) = match kind {
                    Kind::Dummy => ("https://example.com", true, true),
                    Kind::Unit => ("http://example.com", false, false),
                    Kind::Parsing => ("https://example.com", true, false),
                    Kind::BaseOnly => ("https://base.example.com", false, false),
                };
                Self {
                    kind,
                    base_url: base_url.to_owned(),
                    verbose: false,
                    supports_isbn_search,
                    author_first_search,
                    max_results: MAX_RESULTS,
                    session: failing(),
                    links: None,
                    tmp_dir: None,
                    fetch: None,
                    keyword: None,
                    isbn: None,
                    load: None,
                    save: None,
                }
            }

            fn with_verbose(mut self, verbose: bool) -> Self {
                self.verbose = verbose;
                self
            }

            fn with_session(mut self, session: Arc<dyn HttpSession>) -> Self {
                self.session = session;
                self
            }
        }

        impl Bookstore for $name {
            fn base_url(&self) -> &str {
                &self.base_url
            }

            fn verbose(&self) -> bool {
                self.verbose
            }

            fn supports_isbn_search(&self) -> bool {
                self.supports_isbn_search
            }

            fn author_first_search(&self) -> bool {
                self.author_first_search
            }

            fn max_results(&self) -> usize {
                self.max_results
            }

            fn session(&self) -> &dyn HttpSession {
                self.session.as_ref()
            }

            fn tmp_dir(&self) -> Option<PathBuf> {
                self.tmp_dir.clone().or_else(bookstore::store_clients::default_cache_dir)
            }

            fn build_search_url(&self, keyword: &str) -> String {
                format!("{}/search?q={}", self.base_url, keyword)
            }

            fn extract_search_links(&self, markup: &str) -> Vec<String> {
                if let Some(links) = &self.links {
                    return links.clone();
                }
                match self.kind {
                    Kind::Dummy | Kind::Unit => vec![format!("{}/detail/1", self.base_url), format!("{}/detail/2", self.base_url)],
                    Kind::Parsing => single_quoted_hrefs(markup),
                    Kind::BaseOnly => vec![],
                }
            }

            fn extract_book_info(&self, markup: &str) -> BookInfo {
                match self.kind {
                    Kind::Dummy => BookInfo { title: between(markup, "<h1>", "</h1>").into(), author: between(markup, "<span class='author'>", "</span>").into(), category: between(markup, "<p class='category'>", "</p>").into(), isbn: between(markup, "<p class='isbn'>", "</p>").into() },
                    Kind::Unit => BookInfo { title: "T".into(), author: "A".into(), category: "A > B > C > D".into(), isbn: "I".into() },
                    Kind::Parsing => BookInfo { title: between(markup, "<h1>", "</h1>").into(), author: between(markup, "<span class='author'>", "</span>").into(), category: between(markup, "<div class='cat'>", "</div>").into(), isbn: between(markup, "<div class='isbn'>", "</div>").into() },
                    Kind::BaseOnly => BookInfo::default(),
                }
            }

            $($over)*
        }
    };
}

// 기본 구현을 그대로 쓰는 서점
dummy_store!(Plain {});

// `load_html_from_tmp`/`save_html_to_tmp`만 바꾼 서점
dummy_store!(Cached {
    fn load_html_from_tmp(&self, url: &str) -> Option<String> {
        self.load.as_ref().and_then(|f| f(url))
    }

    fn save_html_to_tmp(&self, html: &str, url: &str) {
        if let Some(f) = &self.save {
            f(html, url);
        }
    }
});

// `fetch_search_results`/`search_by_keyword`/`search_by_isbn`을 바꾼 서점. 훅이 없으면 기본 동작을 따른다.
dummy_store!(Hooked {
    fn build_isbn_search_url(&self, isbn: &str) -> String {
        if self.kind == Kind::Dummy { format!("{}/isbn/{}", self.base_url, isbn) } else { self.build_search_url(isbn) }
    }

    fn fetch_search_results(&self, url: &str) -> Vec<SearchRow> {
        (self.fetch.as_ref().expect("fetch_search_results 훅이 필요하다"))(url)
    }

    fn search_by_keyword(&self, keyword: &str) -> Vec<SearchRow> {
        match &self.keyword {
            Some(f) => f(keyword),
            None => self.fetch_search_results(&self.build_search_url(keyword)),
        }
    }

    fn search_by_isbn(&self, isbn: &str) -> Vec<SearchRow> {
        match &self.isbn {
            Some(f) => f(isbn),
            None => self.fetch_search_results(&self.build_isbn_search_url(isbn)),
        }
    }
});
