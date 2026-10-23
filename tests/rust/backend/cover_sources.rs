//! 원본: tests/test_cover_sources.py
//!
//! 서점별 표지 검색·URL 검증·잠금·차단 backoff를 확인한다. 순수 파서와 실제 파일 잠금은 그대로 돌리고,
//! 네트워크·DNS·시계·클라이언트 factory를 monkeypatch로 바꾸던 테스트는 통일 seam(`HttpSession`, `FsOps`, `Clock`, `SearchHooks`)으로 주입한다.
//! 설계: docs/rust-test-seams.md 3.1, 3.5, 3.7, 3.8.
//!
//! 가정한 API (아직 구현 전이다. Python 이름 그대로이고 비공개 `_x`는 `pub fn x`로 본다):
//! - `backend::cover_sources::{extract_html_candidates(store: &str, url: &str, html: &str) -> Vec<Value>, parse_naver_candidates(html: &str) -> Vec<Value>}`
//! - `backend::cover_sources::{parse_joara_candidates(data: &Value) -> Result<Vec<Value>, ValueError>, parse_munpia_candidates(data: &Value) -> Result<Vec<Value>, ValueError>}`
//! - `backend::cover_sources::validate_url(store: &str, url: &str) -> Result<String, ValueError>`
//! - `backend::cover_sources::validate_url_with(store, url, resolve: &dyn Fn(&str, u16) -> std::io::Result<Vec<IpAddr>>) -> Result<String, ValueError>` (Python `socket.getaddrinfo` 대체)
//! - `backend::cover_sources::store_lock(dir: &Path, store: &str) -> Result<StoreLockGuard, StoreLockError>`
//!   - `enum StoreLockError { UnknownStore(String), Cancelled, Timeout }`
//!   - `store_lock_with(dir: &Path, store: &str, fs: &dyn FsOps, clock: &dyn Clock) -> Result<StoreLockGuard, StoreLockError>` (문서 3.5. `FsOps::lock_exclusive`가 `WouldBlock`이면 `BlockingIOError`, 시간은 `Clock::monotonic`)
//!   - 취소 입력은 문서에 없다. `cancelled: &dyn Fn() -> bool`을 5번째 인자로 더 받는다고 가정한다(`test_cancelled_and_timed_out_store_locks`만 쓴다).
//! - `backend::cover_sources::fetch_bytes_with(store, url, session: &dyn HttpSession, max_bytes: Option<usize>, validate: &dyn Fn(&str, &str) -> Result<String, ValueError>) -> Result<Vec<u8>, ValueError>`
//!   - `sysdeps::HttpSession { fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError>; }` (구 `Session`)
//!   - `sysdeps::HttpResponse::from_chunks(status: u16, headers: Vec<(String, String)>, chunks: Vec<Vec<u8>>, closed: Arc<AtomicBool>)`, `is_redirect()`, `location()`
//! - `backend::cover_sources::{SearchHooks, StoreClient, search_candidates_with, SearchError}`
//!   - `SearchHooks::default()`, `.with_session_factory(Arc<dyn Fn(&str) -> Arc<dyn HttpSession> + Send + Sync>)`(인자는 store. 구 `with_fetch`), `.with_client_factory(store: &str, Fn() -> Box<dyn StoreClient>)`,
//!     `.with_cancel(Arc<dyn Fn() -> bool + Send + Sync>)`(구 `set_request_cancelled` 가드)
//!   - `search_candidates_with(store: &str, metadata: &Value, hooks: &SearchHooks) -> Result<Value, SearchError>` (`result["status"]`, `["candidates"]`, `["search_url"]`)
//!   - `enum SearchError { Value(String), Http { status: u16 }, Cancelled }`
//!   - `trait StoreClient { fn supports_isbn_search(&self) -> bool; fn api_key(&self) -> String; fn api_url(&self) -> String; fn base_url(&self) -> String;
//!     fn build_search_url(&self, keyword: &str) -> String; fn extract_search_links(&self, html: &str) -> Vec<String>; fn extract_book_info(&self, html: &str) -> Value;
//!     fn set_session_header(&self, name: &str, value: &str); fn close(&self); }`
//! - `backend::cover_sources::{blocked_status(err: &SearchError) -> bool, StoreBlocked { pub retry_after: f64 }, StoreGate, GateError}`
//!   - `StoreGate::new(dir: &Path, store: &str)`, `.with_clock(dir, store, clock: Arc<dyn Clock>)`(구조체 생성자), `StoreGate::with_fs(self, fs: Arc<dyn FsOps>)`(구 `with_writer`. 상태 파일 쓰기는 `FsOps::write`),
//!     `StoreGate::with_cancel(self, Arc<dyn Fn() -> bool + Send + Sync>)`(`SearchHooks::with_cancel`과 같은 모양. 문서에 없는 가정)
//!   - `before_request(&self) -> Result<(), GateError>` (`GateError::Blocked(StoreBlocked)` | `GateError::Cancelled`), `record_blocked(&self) -> f64`, `record_success(&self)`, `path(&self) -> PathBuf`
//! - 시계는 `sysdeps::Clock`(`now`, `monotonic`, `sleep`)이며 `common::clock::FakeClock`이 구현한다.

mod common;

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::cover_sources::{
    GateError, SearchError, SearchHooks, StoreBlocked, StoreClient, StoreGate, StoreLockError, blocked_status, extract_html_candidates, fetch_bytes_with, parse_joara_candidates,
    parse_munpia_candidates, parse_naver_candidates, search_candidates_with, store_lock, store_lock_with, validate_url, validate_url_with,
};
use common::{
    clock::FakeClock,
    faulty_fs::{FaultyFs, other_err},
};
use serde_json::{Value, json};
use sysdeps::{HttpError, HttpResponse, HttpSession, RealFs, SystemClock};

#[test]
fn test_html_candidates_use_real_page_images() {
    for store in ["yes24", "aladin", "kyobo", "ridi", "munpia", "naverseries"] {
        let candidates = extract_html_candidates(
            store,
            "https://example.invalid/book",
            r#"<html><meta property="og:title" content="책"><meta property="og:image" content="https://image.yes24.com/real.jpg"></html>"#,
        );
        assert_eq!(candidates[0]["image_url"], "https://image.yes24.com/real.jpg", "store={store}");
        assert!(!candidates[0]["title"].as_str().unwrap_or("").is_empty(), "store={store}");
    }
}

#[test]
fn test_naver_and_joara_search_candidates() {
    let html = r#"<a class="bookListItem_info_top__abc" href="https://cr3.shopping.naver.com/book/adcr?nvMid=123"><img alt="데미안" src="https://shopping-phinf.pstatic.net/book.jpg"><div class="bookListItem_define_item__abc"><span>저자</span><span>헤세</span></div></a>"#;
    assert_eq!(parse_naver_candidates(html)[0]["author"], "헤세");
    let data = json!({"status": 1, "data": {"list": [{"book_code": "123", "subject": "책", "member_name": "저자", "cover": "https://cf-image.joara.com/book.png"}]}});
    assert_eq!(parse_joara_candidates(&data).unwrap()[0]["image_url"], "https://cf-image.joara.com/book.png");
}

/// 검색 요청 URL의 query 값. Python `parse_qs(urlparse(url).query)`.
fn query_values(url: &str) -> BTreeMap<String, Vec<String>> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.entry(decode(k)).or_default().push(decode(v));
    }
    out
}

fn decode(raw: &str) -> String {
    let bytes = raw.replace('+', " ").into_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 요청 URL마다 `body`를 돌려주는 가짜 `HttpSession`. 구 `fetch_bytes` monkeypatch(`with_fetch`)를 대신한다.
// TODO(common): `common`에 `HttpSession` 대역이 없다(bookstore.rs의 `FakeSession`은 따로 있다). 생기면 지운다.
type Responder = Arc<dyn Fn(&str, &str) -> Vec<u8> + Send + Sync>;

struct BodySession {
    store: String,
    respond: Responder,
}

impl HttpSession for BodySession {
    fn get(&self, url: &str, _params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        Ok(HttpResponse::from_chunks(200, Vec::new(), vec![(self.respond)(&self.store, url)], Arc::new(AtomicBool::new(false))))
    }
}

/// `respond(store, url)`이 본문을 정하는 세션 factory. 모든 store가 같은 함수를 쓴다.
fn session_factory(respond: impl Fn(&str, &str) -> Vec<u8> + Send + Sync + 'static) -> backend::cover_sources::SessionFactory {
    let respond: Responder = Arc::new(respond);
    Arc::new(move |store| Arc::new(BodySession { store: store.to_string(), respond: respond.clone() }) as Arc<dyn HttpSession>)
}

/// 요청 URL을 모으는 가짜 세션. 응답 본문은 `body`로 고정한다.
fn recording_fetch(body: &'static str) -> (SearchHooks, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    let hooks = SearchHooks::default().with_session_factory(session_factory(move |_store, url| {
        sink.lock().unwrap().push(url.to_string());
        body.as_bytes().to_vec()
    }));
    (hooks, seen)
}

#[test]
fn test_aladin_covers_use_clean_book_keyword_instead_of_all_products() {
    for (title, keyword) in [("불멸과 소멸 1 이상문", "불멸과 소멸 1 이상문"), ("우리말의 상상력 1 어휘의 기원", "우리말의 상상력 1 어휘의 기원")] {
        let (hooks, seen) = recording_fetch(r#"<div id="Search3_Result">검색 결과 없음</div>"#);
        let result = search_candidates_with("aladin", &json!({"title": title}), &hooks).unwrap();
        let query = query_values(&seen.lock().unwrap()[0]);
        assert_eq!(query["SearchTarget"], ["Book"], "title={title}");
        assert_eq!(query["SearchWord"], [keyword], "title={title}");
        assert_eq!(result["status"], "not_found", "title={title}");
    }
}

#[test]
fn test_aladin_cover_search_keeps_isbn() {
    let (hooks, seen) = recording_fetch("검색 결과 없음");
    let _ = search_candidates_with("aladin", &json!({"title": "제목 저자 1", "isbn": "9781234567890"}), &hooks);
    assert_eq!(query_values(&seen.lock().unwrap()[0])["SearchWord"], ["9781234567890"]);
}

#[test]
fn test_isbn_capable_stores_preserve_priority_and_filename_title_fallback() {
    for store in ["yes24", "aladin", "kyobo"] {
        for isbn in ["9781234567890", ""] {
            let (hooks, seen) = recording_fetch("검색 결과 없음");
            let result = search_candidates_with(store, &json!({"title": "랜들 개릿 다아시 경의 모험 1", "isbn": isbn}), &hooks).unwrap();
            let keyword = if isbn.is_empty() { "랜들 개릿 다아시 경의 모험 1" } else { isbn };
            let first = seen.lock().unwrap()[0].clone();
            assert!(query_values(&first).values().any(|v| v == &[keyword.to_string()]), "store={store}, isbn={isbn:?}");
            assert_eq!(result["search_url"], first.as_str(), "store={store}, isbn={isbn:?}");
        }
    }
}

#[test]
fn test_aladin_common_no_image_placeholder_is_not_a_cover() {
    let html = r#"<meta property="og:title" content="제목"><meta property="og:image" content="https://image.aladin.co.kr/img/shop/2018/img_no.jpg">"#;
    assert_eq!(extract_html_candidates("aladin", "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=1", html), Vec::<Value>::new());
}

#[test]
fn test_munpia_current_json_search() {
    let data = json!({"code": "M000_00000", "result": {"searchNovelTabDtos": [{"novelId": 104753, "title": "전지적 독자 시점", "author": "싱숑", "coverUrl": "https://cdn1.munpia.com/cover.jpg"}]}});
    let result = parse_munpia_candidates(&data).unwrap();
    assert_eq!(result[0]["image_url"], "https://cdn1.munpia.com/cover.jpg");
    assert_eq!(result[0]["detail_url"], "https://www.munpia.com/novel/detail/104753");
}

#[test]
fn test_store_locks_serialize_same_store_and_allow_different_stores() {
    #[derive(Default)]
    struct State {
        active: BTreeMap<String, usize>,
        maximum: BTreeMap<String, usize>,
        total: usize,
        max_total: usize,
    }

    let tmp = tempfile::tempdir().unwrap();
    let state = Mutex::new(State::default());

    let fetch = |store: &str| {
        let _guard = store_lock(tmp.path(), store).unwrap();
        {
            let mut s = state.lock().unwrap();
            let now = *s.active.entry(store.to_string()).and_modify(|n| *n += 1).or_insert(1);
            let max = s.maximum.entry(store.to_string()).or_insert(0);
            *max = (*max).max(now);
            s.total += 1;
            s.max_total = s.max_total.max(s.total);
        }
        std::thread::sleep(Duration::from_millis(30));
        {
            let mut s = state.lock().unwrap();
            *s.active.get_mut(store).unwrap() -= 1;
            s.total -= 1;
        }
    };

    let fetch = &fetch;
    std::thread::scope(|scope| {
        for store in ["yes24", "yes24", "aladin", "aladin", "kyobo", "kyobo"] {
            scope.spawn(move || fetch(store));
        }
    });
    let s = state.lock().unwrap();
    assert!(s.maximum.values().all(|count| *count == 1), "{:?}", s.maximum);
    assert!(s.max_total > 1, "max_total={}", s.max_total);
}

#[test]
fn test_download_rejects_unapproved_urls() {
    for url in ["http://image.yes24.com/a", "https://localhost/a", "https://image.yes24.com.evil.invalid/a", "https://image.yes24.com@127.0.0.1/a", "https://image.yes24.com:8443/a"] {
        assert!(validate_url("yes24", url).is_err(), "url={url}");
    }
}

fn dns(addresses: &'static [&'static str]) -> impl Fn(&str, u16) -> std::io::Result<Vec<IpAddr>> {
    move |_host, _port| Ok(addresses.iter().map(|a| a.parse().unwrap()).collect())
}

#[test]
fn test_download_rejects_private_dns() {
    assert!(validate_url_with("yes24", "https://image.yes24.com/a", &dns(&["127.0.0.1"])).is_err());
}

#[test]
fn test_public_dns_and_empty_dns() {
    for empty in [false, true] {
        let url = "https://image.yes24.com/a";
        if empty {
            assert!(validate_url_with("yes24", url, &dns(&[])).is_err(), "empty={empty}");
        } else {
            assert_eq!(validate_url_with("yes24", url, &dns(&["8.8.8.8"])).unwrap(), url, "empty={empty}");
        }
    }
}

#[test]
fn test_cancelled_and_timed_out_store_locks() {
    let tmp = tempfile::tempdir().unwrap();
    let unknown = store_lock(tmp.path(), "unknown");
    assert!(matches!(unknown, Err(StoreLockError::UnknownStore(_))));
    let cancelled = store_lock_with(tmp.path(), "yes24", &RealFs, &SystemClock, &|| true);
    assert!(matches!(cancelled, Err(StoreLockError::Cancelled)));
    // flock이 항상 BlockingIOError, monotonic이 0 다음 121을 돌려준다: 첫 시도가 실패하면 시계를 121초 보낸다.
    let clock = FakeClock::new(0.0);
    let fs = FaultyFs::new();
    let ticking = clock.clone();
    fs.set_lock_exclusive(move |_file, _nonblocking| {
        ticking.advance(121.0);
        Some(std::io::ErrorKind::WouldBlock.into())
    });
    let timed_out = store_lock_with(tmp.path(), "yes24", &*fs, &*clock, &|| false);
    assert!(matches!(timed_out, Err(StoreLockError::Timeout)));
}

/// Python의 `MagicMock` 응답을 차례로 돌려주고 요청 URL을 기록하는 `HttpSession`.
struct ScriptedSession {
    responses: Mutex<Vec<HttpResponse>>,
    requested: Mutex<Vec<String>>,
}

impl HttpSession for ScriptedSession {
    fn get(&self, url: &str, _params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        self.requested.lock().unwrap().push(url.to_string());
        Ok(self.responses.lock().unwrap().remove(0))
    }
}

#[test]
fn test_fetch_bytes_redirects_and_size_limit() {
    for mode in ["success", "too_large", "redirect_limit"] {
        let first = || HttpResponse::from_chunks(302, vec![("Location".to_string(), "/image.jpg".to_string())], Vec::new(), Arc::new(AtomicBool::new(false)));
        let final_closed = Arc::new(AtomicBool::new(false));
        let last = HttpResponse::from_chunks(200, Vec::new(), vec![b"im".to_vec(), b"age".to_vec()], final_closed.clone());
        let responses = if mode == "redirect_limit" { (0..5).map(|_| first()).collect() } else { vec![first(), last] };
        let session = ScriptedSession { responses: Mutex::new(responses), requested: Mutex::new(Vec::new()) };
        let identity = |_store: &str, url: &str| -> Result<String, backend::cover_sources::ValueError> { Ok(url.to_string()) };
        if mode == "success" {
            assert_eq!(fetch_bytes_with("yes24", "https://image.yes24.com/start", &session, None, &identity).unwrap(), b"image", "mode={mode}");
            assert_eq!(session.requested.lock().unwrap().last().unwrap(), "https://image.yes24.com/image.jpg", "mode={mode}");
            assert!(final_closed.load(Ordering::SeqCst), "mode={mode}");
        } else {
            assert!(fetch_bytes_with("yes24", "https://image.yes24.com/start", &session, Some(3), &identity).is_err(), "mode={mode}");
        }
    }
}

#[test]
fn test_parsers_handle_missing_fields_invalid_responses_and_limits() {
    assert_eq!(extract_html_candidates("yes24", "https://www.yes24.com/", "<html></html>"), Vec::<Value>::new());
    let portrait = extract_html_candidates(
        "naverseries",
        "https://series.naver.com/a",
        r#"<meta property="og:title" content="책"><meta property="og:image" content="wide.jpg"><a class="pic_area"><img src="portrait.jpg"></a>"#,
    );
    assert!(portrait[0]["image_url"].as_str().unwrap().ends_with("portrait.jpg"));
    let mut html = String::from(r#"<a class="bookListItem_info_top"><img></a><a class="bookListItem_info_top"></a>"#);
    html += &r#"<a class="bookListItem_info_top"><img alt="책" src="a.jpg"><div class="bookListItem_define_item"><span>출판사</span><span>출판</span></div><div class="bookListItem_define_item"><span>저자</span></div></a>"#.repeat(5);
    assert_eq!(parse_naver_candidates(&html).len(), 4);
    assert!(parse_joara_candidates(&json!({})).is_err());
    assert!(parse_munpia_candidates(&json!({})).is_err());
    assert_eq!(parse_joara_candidates(&json!({"status": 1})).unwrap(), Vec::<Value>::new());
    assert_eq!(parse_munpia_candidates(&json!({"code": "M000_00000"})).unwrap(), Vec::<Value>::new());
    assert_eq!(parse_joara_candidates(&json!({"status": 1, "data": {"list": [{"cover": "a"}, {"book_code": "1"}]}})).unwrap(), Vec::<Value>::new());
    assert_eq!(parse_munpia_candidates(&json!({"code": "M000_00000", "result": {"searchNovelTabDtos": [{"coverUrl": "a"}, {"novelId": 1}]}})).unwrap(), Vec::<Value>::new());
}

/// Python `Mock()` 클라이언트. 호출 인자와 닫힘 여부를 기록한다.
struct MockClient {
    supports_isbn: bool,
    no_links: bool,
    last_keyword: Mutex<Option<String>>,
    headers: Mutex<BTreeMap<String, String>>,
    closed: AtomicBool,
}

impl MockClient {
    fn new(supports_isbn: bool, no_links: bool) -> Arc<Self> {
        Arc::new(Self { supports_isbn, no_links, last_keyword: Mutex::new(None), headers: Mutex::new(BTreeMap::new()), closed: AtomicBool::new(false) })
    }
}

struct SharedClient(Arc<MockClient>);

impl StoreClient for SharedClient {
    fn supports_isbn_search(&self) -> bool {
        self.0.supports_isbn
    }
    fn api_key(&self) -> String {
        "test-key".to_string()
    }
    fn api_url(&self) -> String {
        "https://api.joara.com/search".to_string()
    }
    fn base_url(&self) -> String {
        "https://ridibooks.com".to_string()
    }
    fn build_search_url(&self, keyword: &str) -> String {
        *self.0.last_keyword.lock().unwrap() = Some(keyword.to_string());
        "https://example.org/search".to_string()
    }
    fn extract_search_links(&self, _html: &str) -> Vec<String> {
        if self.0.no_links { Vec::new() } else { vec!["https://example.org/detail".to_string()] }
    }
    fn extract_book_info(&self, _html: &str) -> Value {
        json!({"title": "책", "author": "저자"})
    }
    fn set_session_header(&self, name: &str, value: &str) {
        self.0.headers.lock().unwrap().insert(name.to_string(), value.to_string());
    }
    fn close(&self) {
        self.0.closed.store(true, Ordering::SeqCst);
    }
}

#[test]
fn test_all_store_search_flows_with_local_fixtures() {
    for store in ["yes24", "aladin", "kyobo", "naver", "ridi", "munpia", "naverseries", "joara"] {
        let client = MockClient::new(store == "yes24", store == "ridi");
        let shared = client.clone();
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = requests.clone();
        let hooks = SearchHooks::default().with_client_factory(store, move || Box::new(SharedClient(shared.clone()))).with_session_factory(session_factory(move |_store, url| {
            sink.lock().unwrap().push(url.to_string());
            match store {
                "joara" => json!({"status": 1, "data": {"list": [{"book_code": "1", "cover": "https://image.example.org/a", "subject": "책"}]}}).to_string().into_bytes(),
                "munpia" => json!({"code": "M000_00000", "result": {"searchNovelTabDtos": [{"novelId": 1, "coverUrl": "https://image.example.org/a", "title": "책"}]}}).to_string().into_bytes(),
                "naver" => r#"<a class="bookListItem_info_top"><img alt="책" src="https://image.example.org/a"></a>"#.as_bytes().to_vec(),
                _ => r#"<html><a href="/books/1">책</a><meta property="og:image" content="https://image.example.org/a">검색</html>"#.as_bytes().to_vec(),
            }
        }));
        let result = search_candidates_with(store, &json!({"title": "검색 제목", "isbn": "1234567890"}), &hooks).unwrap();
        assert_eq!(result["status"], "success", "store={store}");
        assert_eq!(result["candidates"][0]["title"], "책", "store={store}");
        assert!(client.closed.load(Ordering::SeqCst), "store={store}");
        if store != "aladin" {
            let expected = if store == "yes24" { "1234567890" } else { "검색 제목" };
            assert_eq!(client.last_keyword.lock().unwrap().as_deref(), Some(expected), "store={store}");
        }
        if store == "joara" || store == "munpia" {
            assert_eq!(query_values(&requests.lock().unwrap()[0])["query"], ["검색 제목"], "store={store}");
        }
        if store == "ridi" {
            assert_eq!(client.headers.lock().unwrap()["Referer"], "https://ridibooks.com/", "store={store}");
        }
    }
}

#[test]
fn test_empty_and_blocked_search_closes_session() {
    for (html, status) in [("검색 결과 없음", "not_found"), ("Just a moment", "error"), ("blocked", "error")] {
        let client = MockClient::new(false, true);
        let shared = client.clone();
        let hooks =
            SearchHooks::default().with_client_factory("yes24", move || Box::new(SharedClient(shared.clone()))).with_session_factory(session_factory(move |_store, _url| html.as_bytes().to_vec()));
        if status == "error" {
            assert!(matches!(search_candidates_with("yes24", &json!({"title": "책"}), &hooks), Err(SearchError::Value(_))), "html={html}");
        } else {
            assert_eq!(search_candidates_with("yes24", &json!({"title": "책"}), &hooks).unwrap()["status"], "not_found", "html={html}");
        }
        assert!(client.closed.load(Ordering::SeqCst), "html={html}");
        assert!(matches!(search_candidates_with("unknown", &json!({"title": "책"}), &hooks), Err(SearchError::Value(_))), "html={html}");
    }
}

#[test]
fn test_only_rate_limit_statuses_count_as_store_block() {
    for (status, blocked) in [(418, true), (429, true), (500, false), (404, false)] {
        assert_eq!(blocked_status(&SearchError::Http { status }), blocked, "status={status}");
        assert!(!blocked_status(&SearchError::Value("418".to_string())), "status={status}");
    }
}

fn blocked(result: Result<(), GateError>) -> StoreBlocked {
    match result {
        Err(GateError::Blocked(b)) => b,
        other => panic!("StoreBlocked를 기대했다: {other:?}"),
    }
}

#[test]
fn test_store_gate_backoff_doubles_up_to_cap_and_resets_after_success() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(1000.0);
    let gate = StoreGate::with_clock(tmp.path(), "naver", clock.clone());
    gate.before_request().unwrap();
    assert_eq!(gate.record_blocked(), 60.0);
    assert_eq!(blocked(gate.before_request()).retry_after, 60.0);
    clock.advance(30.0);
    assert_eq!(blocked(gate.before_request()).retry_after, 30.0);
    let expected = [120.0, 240.0, 300.0, 300.0];
    for wait in expected {
        clock.advance(301.0);
        gate.before_request().unwrap(); // 시험 요청
        assert_eq!(gate.record_blocked(), wait);
    }
    clock.advance(301.0);
    gate.before_request().unwrap();
    gate.record_success();
    gate.before_request().unwrap();
    assert_eq!(gate.record_blocked(), 60.0);
}

#[test]
fn test_store_gate_state_is_shared_through_cache_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(1000.0);
    StoreGate::with_clock(tmp.path(), "naver", clock.clone()).record_blocked();
    assert!(matches!(StoreGate::with_clock(tmp.path(), "naver", clock.clone()).before_request(), Err(GateError::Blocked(_))));
    StoreGate::with_clock(tmp.path(), "yes24", clock.clone()).before_request().unwrap();
}

#[test]
fn test_store_gate_spaces_naver_requests_but_not_other_stores() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(1000.0);
    let naver = StoreGate::with_clock(tmp.path(), "naver", clock.clone());
    naver.before_request().unwrap();
    assert_eq!(clock.slept(), Vec::<f64>::new());
    naver.before_request().unwrap();
    assert!((clock.slept().iter().sum::<f64>() - 2.0).abs() < 1e-6, "{:?}", clock.slept());
    clock.clear_slept();
    let yes24 = StoreGate::with_clock(tmp.path(), "yes24", clock.clone());
    yes24.before_request().unwrap();
    yes24.before_request().unwrap();
    assert_eq!(clock.slept(), Vec::<f64>::new());
}

#[test]
fn test_store_gate_wait_can_be_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(1000.0);
    let gate = StoreGate::with_clock(tmp.path(), "naver", clock).with_cancel(Arc::new(|| true));
    gate.before_request().unwrap();
    assert!(matches!(gate.before_request(), Err(GateError::Cancelled)));
}

#[test]
fn test_store_gate_ignores_corrupt_state_file() {
    let tmp = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(1000.0);
    std::fs::write(tmp.path().join("naver.gate.json"), "{not json").unwrap();
    let gate = StoreGate::with_clock(tmp.path(), "naver", clock);
    gate.before_request().unwrap();
    assert_eq!(gate.record_blocked(), 60.0);
}

#[test]
fn test_store_gate_keeps_working_when_state_cannot_be_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = FaultyFs::new();
    fs.set_write(|_path: &Path| Some(other_err("read-only")));
    let gate = StoreGate::new(tmp.path(), "naver").with_fs(fs);

    assert_eq!(gate.record_blocked(), 60.0);
    assert!(!gate.path().exists());
    gate.before_request().unwrap(); // 상태가 없으니 막히지 않는다.
}
