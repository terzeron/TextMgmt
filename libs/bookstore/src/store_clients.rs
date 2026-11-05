//! 온라인 서점 검색.
//!
//! 서점마다 검색 URL, 검색 결과 링크, 상세 페이지 정보 추출이 다르다. 공통 흐름(ISBN > 제목+저자 > 제목 우선순위,
//! 상세 페이지 병렬 수집, 임시 디렉터리 HTML 캐시)은 [`Bookstore`]의 기본 메서드에 있다.
//!
//! - Yes24·알라딘·교보문고의 URL 조립과 HTML 파싱은 이 crate 최상위의 [`crate::Store`]를 재사용한다.
//! - 리디북스·네이버쇼핑·네이버시리즈·문피아는 `scraper`로 새로 포팅했다.
//! - 조아라·웹툰 5종은 JSON API를 직접 부른다. HTML 훅은 쓰지 않는다.
//! - HTTP는 `sysdeps::HttpSession`, 파일 쓰기는 `sysdeps::FsOps`로 주입한다. 기본 HTTP는 `ureq`를 쓰며
//!   테스트는 `with_session`으로 대역을 주입한다.
//! - 조아라 API 키는 소스에 두지 않는다. 환경 변수 `TM_JOARA_API_KEY`에서 읽고, 없으면 빈 문자열이다.
//!   로그와 오류 문장에는 키 값을 남기지 않는다.
//!
//! 구현 메모
//! - HTML은 html5ever(`scraper`)로 파싱한다.
//!   `<li><li>` 처럼 html5ever가 형제로 펴는 깨진 중첩은 형제로 읽는다(리디북스 헤더 저자는 비어 있지 않은 첫 `li`를 쓴다).
//! - 상세 페이지용 별도 세션과 쿠키 복사는 없다. 주입한 같은 세션을 스레드에서 함께 쓴다.
//! - 요청별 timeout(10초, 탑툰 목록 20초)은 `HttpSession::get`에 자리가 없어서 세션 구현이 정한다.
//! - 알라딘·교보문고의 검색 링크 상한은 crate 최상위의 [`crate::MAX_RESULTS`](4)까지다. `max_results`가 더 커도 4개 넘게 뽑지 못한다.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use sysdeps::has_content;

use crate::{BookInfo, Store};
use htmlkit::url_parser::{quote, quote_component, urljoin};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use sysdeps::{FsOps, HttpError, HttpResponse, HttpSession, HttpTimeout, RealFs};

/// 서점 검색 결과 한 줄.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchRow {
    pub title: String,
    pub author: String,
    pub category: String,
    /// 상세 페이지 URL
    pub detail_url: String,
    /// 이 결과를 찾은 검색 페이지 URL
    pub search_url: String,
    pub isbn: String,
}

/// 서점 하나가 돌려주는 검색 결과의 기본 최대 개수.
pub const MAX_RESULTS: usize = 4;

/// 조아라 API 키를 담은 환경 변수 이름. `cover_sources`도 같은 이름을 쓴다.
pub const JOARA_API_KEY_ENV: &str = "TM_JOARA_API_KEY";

/// 디스크 여유가 이 값(500MB)보다 적으면 HTML 캐시를 쓰지 않는다. tmpfs 고갈을 막는다.
const MIN_FREE_BYTES: u64 = 500 * 1024 * 1024;
const MAX_PARALLEL: usize = 8;

/// `mock.patch`로 서점 메서드를 갈아 끼우던 곳에 쓰는 대역. 문자열 하나를 받아 검색 결과를 돌려준다.
pub type SearchHook = Box<dyn Fn(&str) -> Vec<SearchRow> + Send + Sync>;

/// 환경 변수 조회 함수. `sysdeps::EnvFn`은 빌린 참조라서, 서점이 들고 있을 소유형이다.
pub type EnvSource = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

// ---------------------------------------------------------------------------
// 공용 도구
// ---------------------------------------------------------------------------

macro_rules! sel {
    ($css:expr) => {{
        static S: LazyLock<Selector> = LazyLock::new(|| Selector::parse($css).unwrap_or_else(|e| panic!("잘못된 선택자 {}: {e}", $css)));
        &*S
    }};
}

macro_rules! rx {
    ($pattern:expr) => {{
        static R: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).unwrap_or_else(|e| panic!("잘못된 정규식 {}: {e}", $pattern)));
        &*R
    }};
}

/// bs4 `get_text(strip=True)`: 문자열마다 앞뒤 공백을 떼고 빈 것은 버린 뒤 이어 붙인다.
fn text_strip(el: ElementRef) -> String {
    el.text().map(str::trim).filter(|t| !t.is_empty()).collect()
}

/// bs4 `get_text(separator=" ", strip=True)`.
fn text_strip_spaced(el: ElementRef) -> String {
    el.text().map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
}

/// bs4 `get_text()`.
fn text_raw(el: ElementRef) -> String {
    el.text().collect()
}

fn attr<'a>(el: &ElementRef<'a>, name: &str) -> Option<&'a str> {
    el.value().attr(name)
}

/// `markup.find("meta", attrs={key: value})`. 첫 번째 일치 태그다.
fn first_meta<'a>(doc: &'a Html, key: &str, value: &str) -> Option<ElementRef<'a>> {
    doc.select(sel!("meta")).find(|m| m.value().attr(key) == Some(value))
}

/// 태그가 있고 `content`가 비어 있지 않으면 그 값.
fn meta_content<'a>(meta: Option<ElementRef<'a>>) -> Option<&'a str> {
    meta.and_then(|m| m.value().attr("content")).filter(|c| !c.is_empty())
}

/// `find_next_sibling("div")`. 뒤따르는 형제 요소 중 첫 `div`.
fn next_sibling_div<'a>(el: ElementRef<'a>) -> Option<ElementRef<'a>> {
    el.next_siblings().filter_map(ElementRef::wrap).find(|e| e.value().name() == "div")
}

/// `class_=re.compile(r"^prefix")`: 클래스 하나라도 접두사로 시작하면 일치한다.
fn has_class_prefix(el: &ElementRef, prefix: &str) -> bool {
    el.value().classes().any(|c| c.starts_with(prefix))
}

fn find_by_class_prefix<'a>(doc: &'a Html, tag: &'static Selector, prefix: &str) -> Option<ElementRef<'a>> {
    doc.select(tag).find(|e| has_class_prefix(e, prefix))
}

/// `<title>`의 `.string`: 자식이 문자열 하나일 때만 그 문자열이다.
fn title_string(doc: &Html) -> Option<String> {
    let title = doc.select(sel!("title")).next()?;
    let mut children = title.children();
    let only = children.next()?;
    only.value().as_text().map(|t| t.to_string()).filter(|t| !t.is_empty())
}

/// JSON 값의 글자 표기. 문자열은 그대로, 없음은 빈 문자열, 나머지는 JSON 표기다.
fn json_text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `obj.get(key) or ""`
fn str_field(obj: &Value, key: &str) -> String {
    json_text(obj.get(key))
}

/// `obj.get(key) or []`을 배열로. 배열이 아니면 빈 목록이다.
fn array_field<'a>(obj: &'a Value, key: &str) -> &'a [Value] {
    obj.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `dict.fromkeys(names)` 뒤 `", ".join`: 순서를 지키고 중복을 뺀다.
fn join_unique(names: impl IntoIterator<Item = String>, sep: &str) -> String {
    let mut seen: Vec<String> = Vec::new();
    for name in names {
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen.join(sep)
}

fn params(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn book_info(title: &str, author: &str, category: &str, isbn: &str) -> BookInfo {
    BookInfo { title: title.to_string(), author: author.to_string(), category: category.to_string(), isbn: isbn.to_string() }
}

/// 제목에서 부제 구분자(` - `, `－`, `-`, `：`) 왼쪽 부분만 남긴다.
pub fn truncate_title(title: &str) -> String {
    crate::truncate_title(title)
}

// ---------------------------------------------------------------------------
// 캐시 파일 이름: URL 이름공간 UUID v5(`NAMESPACE_URL`, url)
// ---------------------------------------------------------------------------

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (slot, word) in w.iter_mut().zip(block.chunks_exact(4)) {
            *slot = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (chunk, word) in out.chunks_exact_mut(4).zip(h) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// 기본 HTML 캐시 디렉터리 `<temp>/tm-bookstore-<euid>`(0700). 공유 `/tmp`에 예측 가능한 이름으로 쓰면
/// 다른 사용자가 미리 심은 파일이나 symlink를 따라갈 수 있어 사용자 전용 디렉터리를 쓴다.
pub fn default_cache_dir() -> Option<PathBuf> {
    sysdeps::private_temp_dir("tm-bookstore")
}

/// 상세 페이지 HTML 캐시 파일 이름. `uuid5(NAMESPACE_URL, url)`에 `.html`을 붙인다.
pub fn cache_file_name(url: &str) -> String {
    const NAMESPACE_URL: [u8; 16] = [0x6b, 0xa7, 0xb8, 0x11, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8];
    let mut data = NAMESPACE_URL.to_vec();
    data.extend_from_slice(url.as_bytes());
    let digest = sha1(&data);
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}.html", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

/// 조아라가 요구하는 `deviceuid`. 값은 검증하지 않는다. 호출마다 새 UUID v4(하이픈 없는 16진)를 만든다.
fn random_hex32() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut out = String::with_capacity(32);
    for salt in 0..2u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u128(nanos);
        let _ = write!(out, "{:016x}", hasher.finish());
    }
    out
}

// ---------------------------------------------------------------------------
// 기본 세션, markup 대역 trait
// ---------------------------------------------------------------------------

/// HTTP를 사용할 수 없는 테스트 경로에서 쓴다.
pub struct UnavailableSession;

impl HttpSession for UnavailableSession {
    fn get(&self, _url: &str, _params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::Connection("기본 HTTP 세션이 아직 없다. with_session으로 세션을 주입해야 한다".to_owned()))
    }
}

const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/142.0.0.0 Safari/537.36";

/// yes24는 브라우저형 UA의 첫 요청에 쿠키를 심고 메인 페이지로 302한다. 세션이 쿠키를 저장하지 않아 검색 결과를 못 받으므로 `libs/bookstore`처럼 curl UA를 쓴다.
const CURL_USER_AGENT: &str = "curl/7.79.1";

fn user_agent_for(store: &str) -> &'static str {
    if store == "yes24" { CURL_USER_AGENT } else { BROWSER_USER_AGENT }
}

/// `ureq` 기반 동기 HTTP GET 세션. 응답 상태와 본문을 보존해 서점별 오류 규칙에 맡긴다.
pub struct UreqHttpSession {
    config: ureq::config::Config,
    agents: Mutex<Vec<ureq::Agent>>,
    headers: Vec<(String, String)>,
}

impl UreqHttpSession {
    pub fn new() -> Self {
        Self::with_redirect_limit(10, BROWSER_USER_AGENT)
    }

    /// 서점 구조체용 세션. `type_name`은 `Yes24Bookstore` 같은 구조체 이름이다.
    fn for_bookstore(type_name: &str) -> Self {
        Self::with_redirect_limit(10, user_agent_for(&type_name.trim_end_matches("Bookstore").to_ascii_lowercase()))
    }

    fn with_redirect_limit(max_redirects: u32, user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder().max_redirects(max_redirects).http_status_as_error(false).build();
        Self { agents: Mutex::new(vec![ureq::Agent::new_with_config(config.clone())]), config, headers: vec![("User-Agent".to_string(), user_agent.to_string())] }
    }

    pub fn for_store(store: &str) -> Self {
        // cover_sources는 각 redirect target을 검사하므로 ureq의 자동 follow를 막는다.
        let mut session = Self::with_redirect_limit(0, user_agent_for(store));
        if store == "ridi" {
            session.headers.extend([
                ("Referer".to_string(), "https://ridibooks.com/".to_string()),
                ("Accept".to_string(), "application/json,text/html".to_string()),
                ("Accept-Language".to_string(), "ko-KR,ko;q=0.9".to_string()),
            ]);
        }
        session
    }
}

impl Default for UreqHttpSession {
    fn default() -> Self {
        Self::new()
    }
}

/// 서점 API를 불러 200 응답만 돌려준다. 실패하면 `verbose`일 때 `{label} 검색 실패`나 `{label} API 응답 실패`를 남긴다.
/// `secret`은 오류 문구에서 `****`로 가린다(빈 문자열이면 가리지 않는다).
fn fetch_ok(session: &dyn HttpSession, verbose: bool, label: &str, url: &str, query: &[(String, String)], timeout: HttpTimeout, secret: &str) -> Option<HttpResponse> {
    let resp = match session.get_with_timeout(url, query, timeout) {
        Ok(resp) => resp,
        Err(e) => {
            if verbose {
                let message = e.to_string();
                log::error!("{label} 검색 실패: {}", if secret.is_empty() { message } else { message.replace(secret, "****") });
            }
            return None;
        }
    };
    if resp.status_code != 200 {
        if verbose {
            log::warn!("{label} API 응답 실패: {}", resp.status_code);
        }
        return None;
    }
    Some(resp)
}

fn map_ureq_error(error: ureq::Error) -> HttpError {
    match &error {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => HttpError::Timeout,
        _ => HttpError::Connection(error.to_string()),
    }
}

fn read_body_chunks(mut body: ureq::BodyReader<'static>, sender: SyncSender<std::io::Result<Vec<u8>>>) {
    loop {
        let mut chunk = vec![0; 8192];
        match body.read(&mut chunk) {
            Ok(0) => return,
            Ok(size) => {
                chunk.truncate(size);
                if sender.send(Ok(chunk)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        }
    }
}

/// 외부 응답 본문의 상한. 다 읽은 뒤에 검사하면 메모리를 지키지 못하므로 읽는 도중에 끊는다.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

fn read_body_with_idle_timeout(body: ureq::BodyReader<'static>, agent: ureq::Agent, timeout: std::time::Duration) -> Result<(Vec<u8>, ureq::Agent), HttpError> {
    let (sender, receiver) = sync_channel(1);
    std::thread::spawn(move || read_body_chunks(body, sender));
    let mut output = Vec::new();
    loop {
        match receiver.recv_timeout(timeout) {
            Ok(Ok(chunk)) if output.len() + chunk.len() > MAX_RESPONSE_BYTES => {
                // 수신 스레드는 receiver가 사라지면 send 실패로 끝난다.
                agent.shutdown();
                return Err(HttpError::Connection(format!("응답 본문이 상한 {MAX_RESPONSE_BYTES}바이트를 넘습니다")));
            }
            Ok(Ok(chunk)) => output.extend(chunk),
            Ok(Err(error)) => {
                return if error.kind() == std::io::ErrorKind::TimedOut { Err(HttpError::Timeout) } else { Err(HttpError::Connection(error.to_string())) };
            }
            Err(RecvTimeoutError::Disconnected) => return Ok((output, agent)),
            Err(RecvTimeoutError::Timeout) => {
                agent.shutdown();
                return Err(HttpError::Timeout);
            }
        }
    }
}

impl HttpSession for UreqHttpSession {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        self.get_with_timeout(url, params, HttpTimeout::from_secs(10, 10))
    }

    fn get_with_timeout(&self, url: &str, params: &[(String, String)], timeout: HttpTimeout) -> Result<HttpResponse, HttpError> {
        // Lease a separate pool so an idle-body timeout can cancel its reader without closing concurrent requests.
        let agent = self.agents.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop().unwrap_or_else(|| ureq::Agent::new_with_config(self.config.clone()));
        let mut request = agent.get(url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        for (key, value) in params {
            request = request.query(key, value);
        }
        let request = request.config().timeout_connect(Some(timeout.connect)).timeout_recv_response(Some(timeout.read)).timeout_recv_body(None).build();
        let response = request.call().map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let headers = response.headers().iter().map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string())).collect();
        let (_, body) = response.into_parts();
        let (body, agent) = read_body_with_idle_timeout(body.into_reader(), agent, timeout.read)?;
        self.agents.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(agent);
        Ok(HttpResponse::from_chunks(status, headers, vec![body], Arc::new(AtomicBool::new(false))))
    }
}

/// 파싱한 HTML 대역. 조회가 실패하는 문서(`Err`)를 만들 때 쓴다.
/// 반환 문자열은 태그 안쪽 텍스트다.
pub trait MarkupSource {
    fn find(&self, name: &str) -> Result<Option<String>, String>;
    fn find_all(&self, name: &str) -> Result<Vec<String>, String>;
    fn get_text(&self) -> Result<String, String>;
    fn select(&self, css: &str) -> Result<Vec<String>, String>;
}

// ---------------------------------------------------------------------------
// Bookstore
// ---------------------------------------------------------------------------

/// 서점 검색 공통 인터페이스. markup 인자는 HTML 문자열이다.
pub trait Bookstore: Sync {
    fn base_url(&self) -> &str;
    fn verbose(&self) -> bool;
    fn session(&self) -> &dyn HttpSession;
    fn build_search_url(&self, keyword: &str) -> String;
    /// 검색 결과 페이지에서 상세 페이지 링크를 뽑는다.
    fn extract_search_links(&self, markup: &str) -> Vec<String>;
    /// 상세 페이지에서 `{title, author, category, isbn}`을 뽑는다. 못 뽑은 값은 빈 문자열이다.
    fn extract_book_info(&self, markup: &str) -> BookInfo;

    /// ISBN 검색 지원 여부
    fn supports_isbn_search(&self) -> bool {
        false
    }

    /// 제목+저자 검색에서 저자를 앞에 둘지 여부
    fn author_first_search(&self) -> bool {
        false
    }

    fn max_results(&self) -> usize {
        MAX_RESULTS
    }

    /// 마지막 검색 요청을 서점이 자동 요청 차단으로 거절했는가. 차단을 알리는 서점만 덮어쓴다.
    fn request_blocked(&self) -> bool {
        false
    }

    /// 파일 쓰기·읽기 능력. 테스트는 디스크 여유와 쓰기 오류를 흉내 낸다.
    fn fs(&self) -> &dyn FsOps {
        &RealFs
    }

    /// 상세 페이지 HTML 캐시 디렉터리. `None`이면 캐시를 쓰지도 읽지도 않는다.
    fn tmp_dir(&self) -> Option<PathBuf> {
        default_cache_dir()
    }

    /// ISBN 검색 URL. 기본은 일반 검색과 같다.
    fn build_isbn_search_url(&self, isbn: &str) -> String {
        self.build_search_url(isbn)
    }

    fn search_by_keyword(&self, keyword: &str) -> Vec<SearchRow> {
        self.fetch_search_results(&self.build_search_url(keyword))
    }

    fn search_by_isbn(&self, isbn: &str) -> Vec<SearchRow> {
        if !self.supports_isbn_search() {
            if self.verbose() {
                log::info!("{}은 ISBN 검색을 지원하지 않습니다", self.base_url());
            }
            return Vec::new();
        }
        self.fetch_search_results(&self.build_isbn_search_url(isbn))
    }

    /// ISBN, 제목, 저자를 선택적으로 써서 검색한다. 우선순위는 ISBN > 제목+저자 > 제목이다.
    /// 반환값은 `(결과, 실제 검색어, 방식)`이고 방식은 `isbn`/`title_author`/`title`/`unknown`이다.
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        default_search(self, isbn, title, author)
    }

    /// 검색 URL에서 결과를 가져오는 공통 로직. 상세 페이지는 병렬로 받고 순서는 링크 순서다.
    fn fetch_search_results(&self, url: &str) -> Vec<SearchRow> {
        default_fetch_search_results(self, url)
    }

    fn save_html_to_tmp(&self, html: &str, url: &str) {
        let Some(dir) = self.tmp_dir() else { return };
        match self.fs().disk_free(&dir) {
            Ok(free) if free < MIN_FREE_BYTES => return,
            // 여유 공간을 못 재도 쓰기는 시도한다
            Ok(_) | Err(_) => {}
        }
        let path = dir.join(cache_file_name(url));
        match self.fs().write(&path, html.as_bytes()) {
            Ok(()) => {
                if self.verbose() {
                    log::info!("Saved HTML to {}", path.display());
                }
            }
            // errno 28: No space left on device. 조용히 넘어간다
            Err(e) if e.raw_os_error() == Some(28) => {}
            Err(e) => log::debug!("Failed to save HTML to tmp: {e}"),
        }
    }

    /// URL에 해당하는 임시 저장 HTML. 없거나 읽지 못하면 `None`이다.
    fn load_html_from_tmp(&self, url: &str) -> Option<String> {
        let path = self.tmp_dir()?.join(cache_file_name(url));
        let result = (|| {
            if !self.fs().is_file(&path)? {
                return Ok(None);
            }
            if self.verbose() {
                log::info!("Loading cached HTML from {}", path.display());
            }
            let mut text = String::new();
            self.fs().open(&path)?.read_to_string(&mut text)?;
            Ok::<_, std::io::Error>(Some(text))
        })();
        result.unwrap_or_else(|e| {
            log::error!("Failed to load HTML from tmp: {e}");
            None
        })
    }
}

/// `Bookstore::search`의 기본 흐름. 구현이 `search`를 덮어쓴 뒤에도 기본 흐름을 부를 때 쓴다.
pub fn default_search<S: Bookstore + ?Sized>(store: &S, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
    let title = if title.is_empty() { String::new() } else { truncate_title(title) };
    let combined = || if store.author_first_search() { format!("{author} {title}") } else { format!("{title} {author}") };
    let found = |rows: Vec<SearchRow>, keyword: &str, method: &str| (rows, keyword.to_owned(), method.to_owned());

    if !isbn.is_empty() && store.supports_isbn_search() {
        let rows = store.search_by_isbn(isbn);
        if !rows.is_empty() {
            return found(rows, isbn, "isbn");
        }
    }
    if !title.is_empty() && !author.is_empty() {
        let keyword = combined();
        let rows = store.search_by_keyword(&keyword);
        if !rows.is_empty() {
            return found(rows, &keyword, "title_author");
        }
    }
    if !title.is_empty() {
        let rows = store.search_by_keyword(&title);
        if !rows.is_empty() {
            return found(rows, &title, "title");
        }
    }
    // 결과가 없으면 가장 구체적인 검색어를 돌려준다
    if !title.is_empty() && !author.is_empty() {
        return found(Vec::new(), &combined(), "title_author");
    }
    if !title.is_empty() {
        return found(Vec::new(), &title, "title");
    }
    if !isbn.is_empty() {
        return found(Vec::new(), isbn, "isbn");
    }
    found(Vec::new(), "", "unknown")
}

/// `Bookstore.fetch_search_results`의 본문.
pub fn default_fetch_search_results<S: Bookstore + ?Sized>(store: &S, url: &str) -> Vec<SearchRow> {
    let html = match store.session().get_with_timeout(url, &[], HttpTimeout::from_secs(10, 10)) {
        Ok(resp) => resp.text(),
        Err(e) => {
            log::error!("검색 페이지 요청 실패: {e}");
            return Vec::new();
        }
    };
    let links = store.extract_search_links(&html);
    let detail_urls: Vec<&String> = links.iter().take(store.max_results()).collect();

    let mut pages: Vec<Option<String>> = Vec::with_capacity(detail_urls.len());
    for chunk in detail_urls.chunks(MAX_PARALLEL) {
        thread::scope(|scope| {
            let handles: Vec<_> = chunk.iter().map(|detail_url| scope.spawn(move || fetch_detail(store, detail_url))).collect();
            pages.extend(handles.into_iter().map(|h| h.join().unwrap_or(None)));
        });
    }

    let mut results = Vec::new();
    for (detail_url, html) in detail_urls.iter().zip(pages) {
        let Some(html) = html.filter(|h| !h.trim().is_empty()) else {
            if store.verbose() {
                log::warn!("상세 페이지가 비어있습니다: {detail_url}");
            }
            continue;
        };
        let BookInfo { title, author, category, isbn } = store.extract_book_info(&html);
        // 서점 분류는 대분류부터 세 단계까지만 보인다.
        let category = if category.is_empty() { category } else { category.split('>').map(str::trim).take(3).collect::<Vec<_>>().join(" > ") };
        results.push(SearchRow { title, author, category, detail_url: (*detail_url).clone(), search_url: url.to_owned(), isbn });
    }
    results
}

fn fetch_detail<S: Bookstore + ?Sized>(store: &S, detail_url: &str) -> Option<String> {
    if let Some(html) = store.load_html_from_tmp(detail_url) {
        return Some(html);
    }
    match store.session().get_with_timeout(detail_url, &[], HttpTimeout::from_secs(10, 10)) {
        Ok(resp) => {
            let html = resp.text();
            store.save_html_to_tmp(&html, detail_url);
            Some(html)
        }
        Err(e) => {
            log::error!("상세 페이지 요청 실패: {detail_url} - {e}");
            None
        }
    }
}

/// 서점 조회 하나. 분류 서비스가 서점 세 곳을 이 모양으로만 안다. 시험에서는 대역으로 바꾼다.
pub trait BookstoreSearch: Send + Sync {
    /// `(결과, 실제 검색어, 방식)`. 결과는 `(제목, 저자, 카테고리, 상세 URL, 검색 URL, ISBN)`이다.
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String);

    fn api_search_url(&self, _keyword: &str) -> String {
        String::new()
    }

    /// 마지막 검색이 서점의 자동 요청 차단으로 실패했는가. 차단이면 빈 결과가 "결과 없음"이 아니다.
    fn blocked(&self) -> bool {
        false
    }
}

impl<T: Bookstore + Send> BookstoreSearch for T {
    fn blocked(&self) -> bool {
        self.request_blocked()
    }

    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        Bookstore::search(self, isbn, title, author)
    }

    fn api_search_url(&self, keyword: &str) -> String {
        Bookstore::build_search_url(self, keyword)
    }
}

/// API가 요청한 이름에 맞는 서점 구현을 만든다.
pub fn create_bookstore_search(name: &str) -> Option<Box<dyn BookstoreSearch>> {
    let store: Box<dyn BookstoreSearch> = match name.to_ascii_lowercase().as_str() {
        "yes24" => Box::new(Yes24Bookstore::new(false)),
        "aladin" => Box::new(AladinBookstore::new(false)),
        "kyobo" => Box::new(KyoboBookstore::new(false)),
        "ridi" => Box::new(RidibooksBookstore::new(false)),
        "naver" => Box::new(NaverShoppingBookstore::new(false)),
        "naverseries" => Box::new(NaverSeriesBookstore::new(false)),
        "munpia" => Box::new(MunpiaBookstore::new(false)),
        "joara" => Box::new(JoaraBookstore::new(false)),
        "naverwebtoon" => Box::new(NaverWebtoonBookstore::new(false)),
        "kakaowebtoon" => Box::new(KakaoWebtoonBookstore::new(false)),
        "bomtoon" => Box::new(BomtoonBookstore::new(false)),
        "lezhin" => Box::new(LezhinBookstore::new(false)),
        "toptoon" => Box::new(ToptoonBookstore::new(false)),
        _ => return None,
    };
    Some(store)
}

// ---------------------------------------------------------------------------
// 구상 서점 공통 구조
// ---------------------------------------------------------------------------

/// 서점 구조체와 생성·주입 메서드를 만든다. `$extra`는 서점별 추가 필드다.
macro_rules! bookstore_struct {
    ($(#[$meta:meta])* $name:ident, supports_isbn = $isbn:expr $(, $field:ident : $ty:ty = $init:expr)*) => {
        $(#[$meta])*
        pub struct $name {
            verbose: bool,
            pub max_results: usize,
            pub supports_isbn_search: bool,
            session: Arc<dyn HttpSession>,
            fs: Arc<dyn FsOps>,
            tmp_dir: Option<PathBuf>,
            fetch_hook: Option<SearchHook>,
            keyword_hook: Option<SearchHook>,
            $($field: $ty,)*
        }

        impl $name {
            pub fn new(verbose: bool) -> Self {
                Self {
                    verbose,
                    max_results: MAX_RESULTS,
                    supports_isbn_search: $isbn,
                    session: Arc::new(UreqHttpSession::for_bookstore(stringify!($name))),
                    fs: Arc::new(RealFs),
                    tmp_dir: None,
                    fetch_hook: None,
                    keyword_hook: None,
                    $($field: $init,)*
                }
            }

            /// 요청에 쓸 세션을 바꾼다 (구 pub 필드 `session`).
            pub fn with_session(mut self, session: Arc<dyn HttpSession>) -> Self {
                self.session = session;
                self
            }

            /// 캐시 파일 쓰기·읽기 능력을 바꾼다.
            pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> Self {
                self.fs = fs;
                self
            }

            /// HTML 캐시 디렉터리를 바꾼다 (`tempfile.gettempdir` 대응).
            pub fn with_tmp_dir(mut self, dir: PathBuf) -> Self {
                self.tmp_dir = Some(dir);
                self
            }

            /// `fetch_search_results`를 대체한다 (`mock.patch.object` 대응).
            pub fn with_fetch_search_results(mut self, hook: SearchHook) -> Self {
                self.fetch_hook = Some(hook);
                self
            }

            /// `search_by_keyword`를 대체한다 (`mock.patch.object` 대응).
            pub fn with_search_by_keyword(mut self, hook: SearchHook) -> Self {
                self.keyword_hook = Some(hook);
                self
            }
        }
    };
}

/// `impl Bookstore` 안에서 서점 공통 접근자를 채운다.
macro_rules! bookstore_common {
    ($base:expr, author_first = $author_first:expr) => {
        fn base_url(&self) -> &str {
            $base
        }

        fn verbose(&self) -> bool {
            self.verbose
        }

        fn session(&self) -> &dyn HttpSession {
            self.session.as_ref()
        }

        fn fs(&self) -> &dyn FsOps {
            self.fs.as_ref()
        }

        fn tmp_dir(&self) -> Option<PathBuf> {
            self.tmp_dir.clone().or_else(default_cache_dir)
        }

        fn supports_isbn_search(&self) -> bool {
            self.supports_isbn_search
        }

        fn author_first_search(&self) -> bool {
            $author_first
        }

        fn max_results(&self) -> usize {
            self.max_results
        }

        fn fetch_search_results(&self, url: &str) -> Vec<SearchRow> {
            match &self.fetch_hook {
                Some(hook) => hook(url),
                None => default_fetch_search_results(self, url),
            }
        }
    };
}

/// 기본 `search_by_keyword`(검색 URL을 받아 파싱)를 쓰는 서점용. 대체 훅이 있으면 훅이 먼저다.
macro_rules! default_keyword_search {
    () => {
        fn search_by_keyword(&self, keyword: &str) -> Vec<SearchRow> {
            match &self.keyword_hook {
                Some(hook) => hook(keyword),
                None => self.fetch_search_results(&self.build_search_url(keyword)),
            }
        }
    };
}

/// JSON API를 직접 부르는 서점용. `$api`는 `&self, &str -> Vec<SearchRow>` 메서드 이름이다.
macro_rules! api_keyword_search {
    ($api:ident) => {
        fn search_by_keyword(&self, keyword: &str) -> Vec<SearchRow> {
            match &self.keyword_hook {
                Some(hook) => hook(keyword),
                None => self.$api(keyword),
            }
        }
    };
}

/// 웹툰 서점은 ISBN·저자를 쓰지 않고 제목만 한 번 부른다.
macro_rules! title_only_search {
    () => {
        fn search(&self, _isbn: &str, title: &str, _author: &str) -> (Vec<SearchRow>, String, String) {
            default_search(self, "", title, "")
        }
    };
}

/// API를 직접 부르는 서점의 HTML 훅. 호출되지 않는다.
macro_rules! unused_html_hooks {
    () => {
        fn extract_search_links(&self, _markup: &str) -> Vec<String> {
            Vec::new()
        }

        fn extract_book_info(&self, _markup: &str) -> BookInfo {
            BookInfo::default()
        }
    };
}

mod bookshop_clients;
mod web_novel_clients;
mod webtoon_clients;

pub use bookshop_clients::{AladinBookstore, KyoboBookstore, NaverShoppingBookstore, RidibooksBookstore, Yes24Bookstore};
pub use web_novel_clients::{JoaraBookstore, MunpiaBookstore, NaverSeriesBookstore};
pub use webtoon_clients::{BomtoonBookstore, KakaoWebtoonBookstore, LezhinBookstore, NaverWebtoonBookstore, ToptoonBookstore};

#[cfg(test)]
mod utility_tests {
    use super::*;

    #[test]
    fn default_ureq_session_uses_standard_configuration() {
        let session = UreqHttpSession::default();

        assert_eq!(session.headers.len(), 1);
        assert_eq!(session.headers[0].0, "User-Agent");
    }

    #[test]
    fn title_string_returns_raw_text_and_rejects_absent_or_empty_titles() {
        assert_eq!(title_string(&Html::parse_document("<html></html>")), None);
        assert_eq!(title_string(&Html::parse_document("<title></title>")), None);
        assert_eq!(title_string(&Html::parse_document("<title> Book </title>")), Some(" Book ".into()));
        assert_eq!(title_string(&Html::parse_document("<title><b>Book</b></title>")), Some("<b>Book</b>".into()));
        assert_eq!(title_string(&Html::parse_document("<title>one<!-- split -->two</title>")), Some("one<!-- split -->two".into()));
    }
}
