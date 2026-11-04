//! 원본: tests/test_bookstore.py
//!
//! 서점 크롤러(`backend/bookstore.py`) 테스트. HTML 파싱, 검색 우선순위(ISBN > 제목+저자 > 제목),
//! 검색 결과 캐시, 웹툰 서점 JSON API 응답 처리를 확인한다.
//! 서점 HTTP는 `sysdeps::HttpSession` 대역으로 갈아 끼운다(docs/rust-test-seams.md 3.7. libs/bookstore의 `Fetcher` 대역과 같은 방식).
//!
//! 가정한 API (Python 이름을 그대로 쓴다. `_`로 시작하는 비공개 메서드도 같다):
//! - `bookstore::store_clients`
//!   - `type SearchRow = (String, String, String, String, String, String)` = `(title, author, category, detail_url, search_url, isbn)`
//!   - `const MAX_RESULTS: usize = 4` (`AbstractBookstore.MAX_RESULTS`)
//!   - `fn _truncate_title(title: &str) -> String` (Python의 정적/인스턴스 호출을 모두 이 함수로 옮긴다)
//!   - `sysdeps::{HttpSession, HttpResponse, HttpError}` (구 `bookstore::{Session, HttpResponse}`)
//!     - `trait HttpSession: Send + Sync { fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError>; }` (`requests.Session.get` 대응. 예외는 `Err`)
//!     - `HttpResponse::from_text(status: u16, text: &str) -> HttpResponse` (fake 생성용. `.text()`/`.json()`은 본문을 소비한다)
//!     - `HttpError::Other(String)` (문서에 변형 목록이 없어 `EsError::Other`와 같은 모양으로 가정한 것)
//!   - `trait BookstoreSearch { fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String); }`
//!   - `trait AbstractBookstore`: soup 인자는 BeautifulSoup 대신 HTML 문자열(`&str`)이다. 기본값이 있는 메서드는 `[기본]`으로 표시한다.
//!     - 필수: `base_url(&self) -> &str`, `verbose(&self) -> bool`, `session(&self) -> &dyn HttpSession`,
//!       `build_search_url(&self, keyword: &str) -> String`, `extract_search_links(&self, soup: &str) -> Vec<String>`,
//!       `extract_book_info(&self, soup: &str) -> Value` (`{title, author, category, isbn}`)
//!     - `[기본]` `supports_isbn_search() -> bool`(false), `author_first_search() -> bool`(false), `max_results() -> usize`(`MAX_RESULTS`)
//!     - `[기본]` `build_isbn_search_url(isbn)`(= `build_search_url(isbn)`), `search_by_keyword(keyword)`, `search_by_isbn(isbn)`
//!     - `[기본]` `search(isbn, title, author) -> (Vec<SearchRow>, String, String)` (결과, 실제 검색어, 방식 `isbn`/`title_author`/`title`/`unknown`)
//!     - `[기본]` `_fetch_search_results(url) -> Vec<SearchRow>`, `_load_html_from_tmp(url) -> Option<String>`, `_save_html_to_tmp(html, url)`
//!     - `[기본]` `tmp_dir() -> Option<PathBuf>`(`default_cache_dir()`, 사용자 전용 0700 디렉터리)
//!       (구 `new_detail_session()`은 없앴다. 상세 페이지 스레드도 `with_session`으로 주입한 같은 `Arc<dyn HttpSession>`을 쓴다)
//!   - 구상 클래스 `Yes24Bookstore`, `AladinBookstore`, `KyoboBookstore`, `RidibooksBookstore`, `NaverShoppingBookstore`,
//!     `NaverSeriesBookstore`, `MunpiaBookstore`, `JoaraBookstore`, `NaverWebtoonBookstore`, `KakaoWebtoonBookstore`,
//!     `BomtoonBookstore`, `LezhinBookstore`, `ToptoonBookstore`: `new(verbose: bool) -> Self`,
//!     `with_session(self, Arc<dyn HttpSession>) -> Self` (구 pub 필드 `session`), pub 필드 `max_results: usize`, `supports_isbn_search: bool`
//!   - `Yes24Bookstore::_extract_yes24_category(&self, soup: &str) -> String`, `_extract_yes24_isbn`,
//!     `AladinBookstore::_extract_aladin_isbn`, `_strip_trailing_number(&self, title: &str) -> String`,
//!     `RidibooksBookstore::_extract_ridi_isbn`
//!   - 대역 주입 seam(mock.patch 대응, 구상 클래스 공통): `with_fetch_search_results(self, ..) -> Self`, `with_search_by_keyword(self, ..) -> Self`
//!     (`Box<dyn Fn(&str) -> Vec<SearchRow> + Send + Sync>`. 구 `set_*`)
//!   - `trait SoupSource`: `find(&self, name: &str) -> Result<Option<String>, String>`, `find_all(&self, name: &str) -> Result<Vec<String>, String>`,
//!     `get_text(&self) -> Result<String, String>`, `select(&self, css: &str) -> Result<Vec<String>, String>` (예외를 던지는 soup 대역용)
//!     - `Yes24Bookstore::_extract_yes24_category_soup/_extract_yes24_isbn_soup/extract_book_info_soup(&self, &dyn SoupSource)`,
//!       `AladinBookstore::_extract_aladin_isbn_soup`, `RidibooksBookstore::_extract_ridi_isbn_soup`,
//!       `NaverShoppingBookstore::extract_book_info_soup`
//!   - `sysdeps::FsOps` (구 `StoreFs`. 이 파일은 `disk_free`와 `write`만 쓴다. 나머지 메서드는 trait 기본 구현), `Yes24Bookstore::with_fs(self, Arc<dyn FsOps>) -> Self`
//!     (`shutil.disk_usage`와 `open` 대응. `write`의 `data`는 UTF-8 HTML 바이트)
//!   - 추상 메서드 본문(Python은 `None`): `abstract_build_search_url(&dyn AbstractBookstore, &str) -> Option<String>`,
//!     `abstract_extract_search_links(&dyn AbstractBookstore, &str) -> Option<Vec<String>>`,
//!     `abstract_extract_book_info(&dyn AbstractBookstore, &str) -> Option<Value>`

#![allow(clippy::type_complexity)]

/// `env_helpers`가 부르는 자리. 서점 코드는 backend 환경 변수(`TM_BOOK_DIR`, MySQL, ES)를 읽지 않으므로 맞출 값이 없다.
mod common {
}

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bookstore::store_clients::{
    AbstractBookstore, AladinBookstore, MAX_RESULTS, SearchRow, UreqHttpSession, Yes24Bookstore,
};
use serde_json::{Value, json};
use sysdeps::{HttpError, HttpResponse, HttpSession, HttpTimeout};

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
        UreqHttpSession::for_store("yes24").get(url, &[]).unwrap();
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
        UreqHttpSession::for_store("ridi").get(url, &[]).unwrap();
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

    let response = UreqHttpSession::for_store("ridi").get(&format!("http://{address}/redirect"), &[]).unwrap();

    assert_eq!(response.status_code, 302);
    assert_eq!(response.text(), "moved");
    server.join().unwrap();
}

#[test]
fn ureq_session_maps_connection_refusal_to_connection_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);

    let error = UreqHttpSession::new().get(&format!("http://{address}/refused"), &[]).unwrap_err();

    assert!(matches!(error, HttpError::Connection(_)));
}

#[test]
fn ureq_session_applies_request_body_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"late");
    });

    let timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) };
    let result = UreqHttpSession::new().get_with_timeout(&format!("http://{address}/slow"), &[], timeout);

    assert!(matches!(result, Err(HttpError::Timeout)), "{result:?}");
    server.join().unwrap();
}

#[test]
fn ureq_session_recovers_after_an_idle_body_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut slow, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(slow.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        slow.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").unwrap();
        slow.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let _ = slow.write_all(b"late");

        let (mut fast, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(fast.try_clone().unwrap());
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("GET /fast HTTP/1.1"), "{line}");
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        fast.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
    });

    let session = UreqHttpSession::new();
    let slow_timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) };
    assert!(matches!(session.get_with_timeout(&format!("http://{address}/slow"), &[], slow_timeout), Err(HttpError::Timeout)));
    let response = session.get_with_timeout(&format!("http://{address}/fast"), &[], HttpTimeout::from_secs(1, 1)).unwrap();
    assert_eq!(response.text(), "ok");
    server.join().unwrap();
}

#[test]
fn ureq_session_resets_body_timeout_for_each_received_chunk() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
        for chunk in [b"a".as_slice(), b"b".as_slice(), b"c".as_slice()] {
            std::thread::sleep(Duration::from_millis(60));
            write!(stream, "1\r\n{}\r\n", chunk[0] as char).unwrap();
            stream.flush().unwrap();
        }
        stream.write_all(b"0\r\n\r\n").unwrap();
    });

    let timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(100) };
    let response = UreqHttpSession::new().get_with_timeout(&format!("http://{address}/slow-stream"), &[], timeout);

    assert_eq!(response.unwrap().text(), "abc");
    server.join().unwrap();
}

#[test]
fn ureq_session_rejects_a_body_over_the_size_limit() {
    // 다 읽은 뒤가 아니라 읽는 도중에 끊어야 메모리를 지킨다.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        let size = bookstore::store_clients::MAX_RESPONSE_BYTES + 1024 * 1024;
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n");
        let block = vec![b'x'; 64 * 1024];
        let mut sent = 0;
        while sent < size {
            if stream.write_all(&block).is_err() {
                return;
            }
            sent += block.len();
        }
    });

    let error = UreqHttpSession::new().get(&format!("http://{address}/huge"), &[]).unwrap_err();

    assert!(error.to_string().contains("상한"), "{error}");
    server.join().unwrap();
}

#[test]
fn ureq_session_reuses_a_connection_after_a_complete_response() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        for _ in 0..2 {
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(request.starts_with("GET / HTTP/1.1"), "{request}");
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").unwrap();
            stream.flush().unwrap();
        }
    });

    let session = UreqHttpSession::new();
    for _ in 0..2 {
        let response = session.get_with_timeout(&format!("http://{address}/"), &[], HttpTimeout::from_secs(1, 1)).unwrap();
        assert_eq!(response.text(), "ok");
    }
    server.join().unwrap();
}

#[test]
fn ureq_session_applies_response_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(250));
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });

    let timeout = HttpTimeout { connect: Duration::from_secs(1), read: Duration::from_millis(40) };
    let result = UreqHttpSession::new().get_with_timeout(&format!("http://{address}/slow"), &[], timeout);

    assert!(matches!(result, Err(HttpError::Timeout)), "{result:?}");
    server.join().unwrap();
}

#[test]
fn ureq_session_reports_a_body_that_ends_before_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc").unwrap();
        stream.flush().unwrap();
    });

    let result = UreqHttpSession::new().get_with_timeout(&format!("http://{address}/short"), &[], HttpTimeout::from_secs(1, 1));

    assert!(matches!(result, Err(HttpError::Connection(_))), "{result:?}");
    server.join().unwrap();
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Call {
    url: String,
    params: BTreeMap<String, String>,
    timeout: Option<HttpTimeout>,
}

type Handler = Box<dyn Fn(&str, &[(String, String)]) -> Result<HttpResponse, HttpError> + Send + Sync>;

struct FakeSession {
    handler: Handler,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl HttpSession for FakeSession {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        self.calls.lock().unwrap().push(Call { url: url.to_owned(), params: params.iter().cloned().collect(), timeout: None });
        (self.handler)(url, params)
    }

    fn get_with_timeout(&self, url: &str, params: &[(String, String)], timeout: HttpTimeout) -> Result<HttpResponse, HttpError> {
        self.calls.lock().unwrap().push(Call { url: url.to_owned(), params: params.iter().cloned().collect(), timeout: Some(timeout) });
        (self.handler)(url, params)
    }
}

/// `session.get`을 `handler`로 바꾼 세션과, 받은 호출 기록을 돌려준다.
fn session(handler: impl Fn(&str, &[(String, String)]) -> Result<HttpResponse, HttpError> + Send + Sync + 'static) -> (Arc<dyn HttpSession>, Arc<Mutex<Vec<Call>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (Arc::new(FakeSession { handler: Box::new(handler), calls: Arc::clone(&calls) }), calls)
}

/// 요청하면 `RuntimeError("boom")`을 던지는 세션.
fn failing() -> Arc<dyn HttpSession> {
    session(|_, _| Err(HttpError::Other("boom".to_owned()))).0
}

/// `open_tag`와 `close_tag` 사이 텍스트. bs4 `find(...).text` 대신 쓰는 단순 파서다. 없으면 빈 문자열.
fn between(html: &str, open_tag: &str, close_tag: &str) -> String {
    match html.find(open_tag) {
        Some(start) => {
            let rest = &html[start + open_tag.len()..];
            rest.find(close_tag).map_or(String::new(), |end| rest[..end].to_owned())
        }
        None => String::new(),
    }
}

/// `<a href='...'>`의 href 값을 문서 순서대로 모은다.
fn single_quoted_hrefs(html: &str) -> Vec<String> {
    html.split("href='").skip(1).filter_map(|rest| rest.split('\'').next()).map(str::to_owned).collect()
}

fn empty_info() -> Value {
    json!({"title": "", "author": "", "category": "", "isbn": ""})
}

// ---------------------------------------------------------------------------
// 테스트용 서점 클래스 (Python의 DummyBookstore, DummyBookstoreUnit, DummyBookstoreParsing, BaseOnlyBookstore)
// ---------------------------------------------------------------------------

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `DummyBookstore`/`DummyNoIsbnBookstore`: 링크 둘 고정, h1/span.author/p.category/p.isbn 파싱
    Dummy,
    /// `DummyBookstoreUnit`: 링크 둘 고정, 정보도 고정
    Unit,
    /// `DummyBookstoreParsing`: a[href] 링크, h1/span.author/div.cat/div.isbn 파싱
    Parsing,
    /// `BaseOnlyBookstore`: 링크 없음, 정보 비어 있음
    BaseOnly,
}

type SearchHook = Box<dyn Fn(&str) -> Vec<SearchRow> + Send + Sync>;
type LoadHook = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;
type SaveHook = Box<dyn Fn(&str, &str) + Send + Sync>;

/// 서브클래스를 흉내 내는 테스트 서점 구조체를 만든다. `$over`에 넘긴 메서드만 기본 구현 대신 덮어쓴다.
/// monkeypatch로 인스턴스 메서드를 바꾸던 곳은 이 덮어쓰기와 훅 필드로 옮긴다.
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

        impl AbstractBookstore for $name {
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

            fn extract_search_links(&self, soup: &str) -> Vec<String> {
                if let Some(links) = &self.links {
                    return links.clone();
                }
                match self.kind {
                    Kind::Dummy | Kind::Unit => vec![format!("{}/detail/1", self.base_url), format!("{}/detail/2", self.base_url)],
                    Kind::Parsing => single_quoted_hrefs(soup),
                    Kind::BaseOnly => vec![],
                }
            }

            fn extract_book_info(&self, soup: &str) -> Value {
                match self.kind {
                    Kind::Dummy => json!({
                        "title": between(soup, "<h1>", "</h1>"),
                        "author": between(soup, "<span class='author'>", "</span>"),
                        "category": between(soup, "<p class='category'>", "</p>"),
                        "isbn": between(soup, "<p class='isbn'>", "</p>"),
                    }),
                    Kind::Unit => json!({"title": "T", "author": "A", "category": "A > B > C > D", "isbn": "I"}),
                    Kind::Parsing => json!({
                        "title": between(soup, "<h1>", "</h1>"),
                        "author": between(soup, "<span class='author'>", "</span>"),
                        "category": between(soup, "<div class='cat'>", "</div>"),
                        "isbn": between(soup, "<div class='isbn'>", "</div>"),
                    }),
                    Kind::BaseOnly => empty_info(),
                }
            }

            $($over)*
        }
    };
}

// 기본 구현을 그대로 쓰는 서점
dummy_store!(Plain {});

// `_load_html_from_tmp`/`_save_html_to_tmp`만 바꾼 서점
dummy_store!(Cached {
    fn _load_html_from_tmp(&self, url: &str) -> Option<String> {
        self.load.as_ref().and_then(|f| f(url))
    }

    fn _save_html_to_tmp(&self, html: &str, url: &str) {
        if let Some(f) = &self.save {
            f(html, url);
        }
    }
});

// `_fetch_search_results`/`search_by_keyword`/`search_by_isbn`을 바꾼 서점. 훅이 없으면 기본 동작을 따른다.
dummy_store!(Hooked {
    fn build_isbn_search_url(&self, isbn: &str) -> String {
        if self.kind == Kind::Dummy { format!("{}/isbn/{}", self.base_url, isbn) } else { self.build_search_url(isbn) }
    }

    fn _fetch_search_results(&self, url: &str) -> Vec<SearchRow> {
        (self.fetch.as_ref().expect("_fetch_search_results 훅이 필요하다"))(url)
    }

    fn search_by_keyword(&self, keyword: &str) -> Vec<SearchRow> {
        match &self.keyword {
            Some(f) => f(keyword),
            None => self._fetch_search_results(&self.build_search_url(keyword)),
        }
    }

    fn search_by_isbn(&self, isbn: &str) -> Vec<SearchRow> {
        match &self.isbn {
            Some(f) => f(isbn),
            None => self._fetch_search_results(&self.build_isbn_search_url(isbn)),
        }
    }
});
