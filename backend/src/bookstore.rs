//! 온라인 서점 검색. 원본: `backend/bookstore.py`.
//!
//! 서점마다 검색 URL, 검색 결과 링크, 상세 페이지 정보 추출이 다르다. 공통 흐름(ISBN > 제목+저자 > 제목 우선순위,
//! 상세 페이지 병렬 수집, 임시 디렉터리 HTML 캐시)은 [`AbstractBookstore`]의 기본 메서드에 있다.
//!
//! - Yes24·알라딘·교보문고의 URL 조립과 HTML 파싱은 `libs/bookstore` crate(`bookstore_lib`)를 재사용한다.
//! - 리디북스·네이버쇼핑·네이버시리즈·문피아는 `scraper`로 새로 포팅했다.
//! - 조아라·웹툰 5종은 JSON API를 직접 부른다. HTML 훅은 쓰지 않는다.
//! - HTTP는 `sysdeps::HttpSession`, 파일 쓰기는 `sysdeps::FsOps`로 주입한다. 기본 HTTP는 `ureq`를 쓰며
//!   테스트는 `with_session`으로 대역을 주입한다.
//! - 조아라 API 키는 소스에 두지 않는다. 환경 변수 `TM_JOARA_API_KEY`에서 읽고, 없으면 빈 문자열이다.
//!   로그와 오류 문장에는 키 값을 남기지 않는다.
//!
//! 원본과 다른 점
//! - BeautifulSoup `html.parser` 대신 html5ever(`scraper`)로 파싱한다. 정상 HTML에서는 같다.
//!   `<li><li>` 처럼 html5ever가 형제로 펴는 깨진 중첩은 트리가 다르다(리디북스 헤더 저자는 비어 있지 않은 첫 `li`를 쓴다).
//! - 상세 페이지용 별도 세션과 쿠키 복사는 없다. 주입한 같은 세션을 스레드에서 함께 쓴다.
//! - 요청별 timeout(10초, 탑툰 목록 20초)은 `HttpSession::get`에 자리가 없어서 세션 구현이 정한다.
//! - 알라딘·교보문고의 검색 링크 상한은 `libs/bookstore`의 `MAX_RESULTS`(4)까지다. `max_results`가 더 커도 4개 넘게 뽑지 못한다.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use bookstore_lib::{BookInfo, Store};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::{Value, json};
use sysdeps::{FsOps, HttpError, HttpResponse, HttpSession, HttpTimeout, RealFs};

/// `(title, author, category, detail_url, search_url, isbn)`
pub type SearchRow = (String, String, String, String, String, String);

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

/// `soup.find("meta", attrs={key: value})`. 첫 번째 일치 태그다.
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

/// `urllib.parse.quote(s, safe=...)`: 영문·숫자·`_.-~`와 `safe`에 든 글자만 그대로 둔다.
fn quote(value: &str, safe: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.as_bytes().contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// `urllib.parse.urljoin`의 흔한 경우: 절대 URL, `//host`, `/path`, 상대 경로.
fn urljoin(base: &str, href: &str) -> String {
    if href.contains("://") {
        return href.to_owned();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let origin_end = base[scheme_end..].find('/').map_or(base.len(), |i| scheme_end + i);
    if let Some(rest) = href.strip_prefix("//") {
        return format!("{}//{rest}", &base[..scheme_end.saturating_sub(2)]);
    }
    if href.starts_with('/') {
        return format!("{}{href}", &base[..origin_end]);
    }
    format!("{}/{href}", &base[..origin_end])
}

/// Python 진리값. `None`, `False`, `0`, `""`, `[]`, `{}`은 거짓이다.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// Python `str(value)` 대응. 문자열은 그대로, 없음은 빈 문자열, 나머지는 JSON 표기다.
fn py_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `obj.get(key) or ""`
fn str_field(obj: &Value, key: &str) -> String {
    py_str(obj.get(key))
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

fn info_value(title: &str, author: &str, category: &str, isbn: &str) -> Value {
    json!({"title": title, "author": author, "category": category, "isbn": isbn})
}

fn empty_info() -> Value {
    info_value("", "", "", "")
}

fn lib_info(info: BookInfo) -> Value {
    info_value(&info.title, &info.author, &info.category, &info.isbn)
}

/// 제목에서 부제 구분자(` - `, `－`, `-`, `：`) 왼쪽 부분만 남긴다.
pub fn _truncate_title(title: &str) -> String {
    bookstore_lib::truncate_title(title)
}

// ---------------------------------------------------------------------------
// 캐시 파일 이름: Python `uuid.uuid5(uuid.NAMESPACE_URL, url)`와 같은 이름을 만든다
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

/// 조아라가 요구하는 `deviceuid`. 값은 검증하지 않는다. 호출마다 새로 만든다(Python `uuid4().hex`).
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
// 기본 세션, soup 대역 trait
// ---------------------------------------------------------------------------

/// HTTP를 사용할 수 없는 테스트 경로에서 쓴다.
pub struct UnavailableSession;

impl HttpSession for UnavailableSession {
    fn get(&self, _url: &str, _params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::Connection("기본 HTTP 세션이 아직 없다. with_session으로 세션을 주입해야 한다".to_owned()))
    }
}

/// `ureq` 기반 동기 HTTP GET 세션. 응답 상태와 본문을 보존해 서점별 오류 규칙에 맡긴다.
pub struct UreqHttpSession {
    config: ureq::config::Config,
    agents: Mutex<Vec<ureq::Agent>>,
    headers: Vec<(String, String)>,
}

impl UreqHttpSession {
    pub fn new() -> Self {
        Self::with_redirect_limit(10)
    }

    fn with_redirect_limit(max_redirects: u32) -> Self {
        let config = ureq::Agent::config_builder().max_redirects(max_redirects).http_status_as_error(false).build();
        Self {
            agents: Mutex::new(vec![ureq::Agent::new_with_config(config.clone())]),
            config,
            headers: vec![("User-Agent".to_string(), "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/142.0.0.0 Safari/537.36".to_string())],
        }
    }

    pub fn for_store(store: &str) -> Self {
        // cover_sources는 각 redirect target을 검사하므로 ureq의 자동 follow를 막는다.
        let mut session = Self::with_redirect_limit(0);
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

/// BeautifulSoup 객체 대역. 조회가 예외를 던지는 soup(`Err`)을 만들 때 쓴다.
/// 반환 문자열은 태그 안쪽 텍스트다.
pub trait SoupSource {
    fn find(&self, name: &str) -> Result<Option<String>, String>;
    fn find_all(&self, name: &str) -> Result<Vec<String>, String>;
    fn get_text(&self) -> Result<String, String>;
    fn select(&self, css: &str) -> Result<Vec<String>, String>;
}

// ---------------------------------------------------------------------------
// AbstractBookstore
// ---------------------------------------------------------------------------

/// 서점 검색 공통 인터페이스. soup 인자는 BeautifulSoup 대신 HTML 문자열이다.
pub trait AbstractBookstore: Sync {
    fn base_url(&self) -> &str;
    fn verbose(&self) -> bool;
    fn session(&self) -> &dyn HttpSession;
    fn build_search_url(&self, keyword: &str) -> String;
    /// 검색 결과 페이지에서 상세 페이지 링크를 뽑는다.
    fn extract_search_links(&self, soup: &str) -> Vec<String>;
    /// 상세 페이지에서 `{title, author, category, isbn}`을 뽑는다. 못 뽑은 값은 빈 문자열이다.
    fn extract_book_info(&self, soup: &str) -> Value;

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

    /// 파일 쓰기·읽기 능력. 테스트는 디스크 여유와 쓰기 오류를 흉내 낸다.
    fn fs(&self) -> &dyn FsOps {
        &RealFs
    }

    /// 상세 페이지 HTML 캐시 디렉터리 (`tempfile.gettempdir`)
    fn tmp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }

    /// ISBN 검색 URL. 기본은 일반 검색과 같다.
    fn build_isbn_search_url(&self, isbn: &str) -> String {
        self.build_search_url(isbn)
    }

    fn search_by_keyword(&self, keyword: &str) -> Vec<SearchRow> {
        self._fetch_search_results(&self.build_search_url(keyword))
    }

    fn search_by_isbn(&self, isbn: &str) -> Vec<SearchRow> {
        if !self.supports_isbn_search() {
            if self.verbose() {
                log::info!("{}은 ISBN 검색을 지원하지 않습니다", self.base_url());
            }
            return Vec::new();
        }
        self._fetch_search_results(&self.build_isbn_search_url(isbn))
    }

    /// ISBN, 제목, 저자를 선택적으로 써서 검색한다. 우선순위는 ISBN > 제목+저자 > 제목이다.
    /// 반환값은 `(결과, 실제 검색어, 방식)`이고 방식은 `isbn`/`title_author`/`title`/`unknown`이다.
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        default_search(self, isbn, title, author)
    }

    /// 검색 URL에서 결과를 가져오는 공통 로직. 상세 페이지는 병렬로 받고 순서는 링크 순서다.
    fn _fetch_search_results(&self, url: &str) -> Vec<SearchRow> {
        default_fetch_search_results(self, url)
    }

    fn _save_html_to_tmp(&self, html: &str, url: &str) {
        let dir = self.tmp_dir();
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
    fn _load_html_from_tmp(&self, url: &str) -> Option<String> {
        let path = self.tmp_dir().join(cache_file_name(url));
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

/// `AbstractBookstore.search`의 본문. 서브클래스가 `super().search(...)`를 부르던 곳에서 쓴다.
pub fn default_search<S: AbstractBookstore + ?Sized>(store: &S, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
    let title = if title.is_empty() { String::new() } else { _truncate_title(title) };
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

/// `AbstractBookstore._fetch_search_results`의 본문.
pub fn default_fetch_search_results<S: AbstractBookstore + ?Sized>(store: &S, url: &str) -> Vec<SearchRow> {
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
        let info = store.extract_book_info(&html);
        let category = str_field(&info, "category");
        let category = if category.is_empty() { category } else { category.split('>').map(str::trim).take(3).collect::<Vec<_>>().join(" > ") };
        results.push((str_field(&info, "title"), str_field(&info, "author"), category, (*detail_url).clone(), url.to_owned(), str_field(&info, "isbn")));
    }
    results
}

fn fetch_detail<S: AbstractBookstore + ?Sized>(store: &S, detail_url: &str) -> Option<String> {
    if let Some(html) = store._load_html_from_tmp(detail_url) {
        return Some(html);
    }
    match store.session().get_with_timeout(detail_url, &[], HttpTimeout::from_secs(10, 10)) {
        Ok(resp) => {
            let html = resp.text();
            store._save_html_to_tmp(&html, detail_url);
            Some(html)
        }
        Err(e) => {
            log::error!("상세 페이지 요청 실패: {detail_url} - {e}");
            None
        }
    }
}

/// 추상 메서드의 본문. Python은 `pass`라서 `None`을 돌려준다. Rust trait의 필수 메서드는 본문이 없어서
/// 어떤 서점을 넘겨도 항상 `None`이다. 테스트가 이름을 가져오는 용도로만 남겼다.
pub fn abstract_build_search_url(_store: &dyn AbstractBookstore, _keyword: &str) -> Option<String> {
    None
}

/// [`abstract_build_search_url`]과 같다.
pub fn abstract_extract_search_links(_store: &dyn AbstractBookstore, _soup: &str) -> Option<Vec<String>> {
    None
}

/// [`abstract_build_search_url`]과 같다.
pub fn abstract_extract_book_info(_store: &dyn AbstractBookstore, _soup: &str) -> Option<Value> {
    None
}

/// 서점 조회 하나. 분류 서비스가 서점 세 곳을 이 모양으로만 안다. 시험에서는 대역으로 바꾼다.
pub trait BookstoreSearch: Send + Sync {
    /// `(결과, 실제 검색어, 방식)`. 결과는 `(제목, 저자, 카테고리, 상세 URL, 검색 URL, ISBN)`이다.
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String);

    fn api_search_url(&self, _keyword: &str) -> String {
        String::new()
    }
}

impl<T: AbstractBookstore + Send> BookstoreSearch for T {
    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        AbstractBookstore::search(self, isbn, title, author)
    }

    fn api_search_url(&self, keyword: &str) -> String {
        AbstractBookstore::build_search_url(self, keyword)
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
                    session: Arc::new(UreqHttpSession::new()),
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

            /// `_fetch_search_results`를 대체한다 (`mock.patch.object` 대응).
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

/// `impl AbstractBookstore` 안에서 서점 공통 접근자를 채운다.
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

        fn tmp_dir(&self) -> PathBuf {
            self.tmp_dir.clone().unwrap_or_else(std::env::temp_dir)
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

        fn _fetch_search_results(&self, url: &str) -> Vec<SearchRow> {
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
                None => self._fetch_search_results(&self.build_search_url(keyword)),
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
        fn extract_search_links(&self, _soup: &str) -> Vec<String> {
            Vec::new()
        }

        fn extract_book_info(&self, _soup: &str) -> Value {
            empty_info()
        }
    };
}

// ---------------------------------------------------------------------------
// Yes24
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// Yes24. URL 조립과 HTML 파싱은 `libs/bookstore`를 쓴다.
    Yes24Bookstore, supports_isbn = true
);

impl Yes24Bookstore {
    /// 상세 페이지 카테고리. `관련분류` 아래 카테고리 링크를 `a > b`로 잇는다.
    pub fn _extract_yes24_category(&self, soup: &str) -> String {
        Store::Yes24.extract_book_info(soup).category
    }

    /// 상세 페이지 ISBN. ISBN13, ISBN10, ISBN, 숫자 10~13자리 순으로 찾는다.
    pub fn _extract_yes24_isbn(&self, soup: &str) -> String {
        Store::Yes24.extract_book_info(soup).isbn
    }

    /// 조회가 실패할 수 있는 soup용. 실패하면 빈 문자열이다.
    pub fn _extract_yes24_category_soup(&self, soup: &dyn SoupSource) -> String {
        match soup.select(r#"a[href*="/product/category/display/"]"#) {
            Ok(labels) => join_unique(labels.into_iter().map(|l| l.trim().to_owned()).filter(|l| !l.is_empty()), " > "),
            Err(e) => {
                log::error!("카테고리 추출 중 오류: {e}");
                String::new()
            }
        }
    }

    /// 조회가 실패할 수 있는 soup용. 실패하면 빈 문자열이다.
    pub fn _extract_yes24_isbn_soup(&self, soup: &dyn SoupSource) -> String {
        match soup.get_text() {
            Ok(text) => isbn_from_text(&text),
            Err(e) => {
                log::error!("ISBN 추출 중 오류: {e}");
                String::new()
            }
        }
    }

    /// 조회가 실패할 수 있는 soup용. 도중에 실패하면 거기까지 뽑은 부분 결과를 돌려준다.
    pub fn extract_book_info_soup(&self, soup: &dyn SoupSource) -> Value {
        let mut title = String::new();
        let mut author = String::new();
        let partial = (|| -> Result<(String, String), String> {
            if let Some(t) = soup.find("h2")? {
                title = t.trim().to_owned();
            }
            if let Some(a) = soup.find("span")? {
                author = a.trim().to_owned();
            }
            Ok((self._extract_yes24_category_soup(soup), self._extract_yes24_isbn_soup(soup)))
        })();
        match partial {
            Ok((category, isbn)) => info_value(&title, &author, &category, &isbn),
            Err(e) => {
                log::error!("책 정보 추출 중 오류: {e}");
                info_value(&title, &author, "", "")
            }
        }
    }
}

/// 페이지 글자에서 ISBN을 찾는다. Yes24의 패턴 순서를 따른다.
fn isbn_from_text(text: &str) -> String {
    for pattern in [rx!(r"ISBN13\s*(\d{13})"), rx!(r"ISBN10\s*(\d{10})"), rx!(r"ISBN\s*(\d{10,13})"), rx!(r"(\d{10,13})")] {
        if let Some(c) = pattern.captures(text) {
            return c[1].to_owned();
        }
    }
    String::new()
}

impl AbstractBookstore for Yes24Bookstore {
    bookstore_common!(Store::Yes24.base_url(), author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        Store::Yes24.build_search_url(keyword)
    }

    fn build_isbn_search_url(&self, isbn: &str) -> String {
        Store::Yes24.build_isbn_search_url(isbn)
    }

    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let links = Store::Yes24.extract_search_links(soup);
        if self.verbose {
            log::info!("yes24에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        lib_info(Store::Yes24.extract_book_info(soup))
    }
}

// ---------------------------------------------------------------------------
// 알라딘
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 알라딘. URL 조립과 HTML 파싱은 `libs/bookstore`를 쓴다.
    AladinBookstore, supports_isbn = true
);

impl AladinBookstore {
    /// 제목 끝의 권수 번호를 뗀다. 예: `마왕의 딸 3` -> `마왕의 딸`.
    pub fn _strip_trailing_number(&self, title: &str) -> String {
        bookstore_lib::strip_trailing_number(title)
    }

    pub fn _extract_aladin_isbn(&self, soup: &str) -> String {
        Store::Aladin.extract_book_info(soup).isbn
    }

    /// 조회가 실패할 수 있는 soup용. 실패하면 빈 문자열이다.
    pub fn _extract_aladin_isbn_soup(&self, soup: &dyn SoupSource) -> String {
        let find = || -> Result<String, String> {
            for text in soup.select("#Ere_prod_allwrap div.Ere_prod_mconts_R > div.conts_info_list1 > ul li")? {
                let text = text.trim();
                if text.starts_with("ISBN")
                    && let Some(m) = rx!(r"(\d{13})").captures(text)
                {
                    return Ok(m[1].to_owned());
                }
            }
            for text in soup.select("#Ere_prod_allwrap > div.Ere_prod_middlewrap div.Ere_prod_mconts_R ul > li")? {
                let text = text.trim();
                if text.contains("ISBN")
                    && let Some(m) = rx!(r"(\d{10,13})").captures(text)
                {
                    return Ok(m[1].to_owned());
                }
            }
            Ok(String::new())
        };
        find().unwrap_or_else(|e| {
            log::error!("ISBN 알라딘 추출 중 오류: {e}");
            String::new()
        })
    }
}

impl AbstractBookstore for AladinBookstore {
    bookstore_common!(Store::Aladin.base_url(), author_first = true);
    default_keyword_search!();

    fn search(&self, isbn: &str, title: &str, author: &str) -> (Vec<SearchRow>, String, String) {
        let title = if title.is_empty() { String::new() } else { self._strip_trailing_number(title) };
        default_search(self, isbn, &title, author)
    }

    fn build_search_url(&self, keyword: &str) -> String {
        Store::Aladin.build_search_url(keyword)
    }

    fn build_isbn_search_url(&self, isbn: &str) -> String {
        Store::Aladin.build_isbn_search_url(isbn)
    }

    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let mut links = Store::Aladin.extract_search_links(soup);
        links.truncate(self.max_results);
        if self.verbose {
            log::info!("알라딘에서 {}개의 상세 페이지 링크를 찾았습니다.", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        lib_info(Store::Aladin.extract_book_info(soup))
    }
}

// ---------------------------------------------------------------------------
// 교보문고
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 교보문고. URL 조립과 HTML 파싱은 `libs/bookstore`를 쓴다.
    KyoboBookstore, supports_isbn = true
);

impl AbstractBookstore for KyoboBookstore {
    bookstore_common!(Store::Kyobo.base_url(), author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        Store::Kyobo.build_search_url(keyword)
    }

    fn build_isbn_search_url(&self, isbn: &str) -> String {
        Store::Kyobo.build_isbn_search_url(isbn)
    }

    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let mut links = Store::Kyobo.extract_search_links(soup);
        links.truncate(self.max_results);
        if self.verbose {
            log::info!("Kyobo에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        lib_info(Store::Kyobo.extract_book_info(soup))
    }
}

// ---------------------------------------------------------------------------
// 리디북스
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 리디북스. 검색은 `search-api.ridibooks.com` JSON을 직접 부른다. ISBN 검색은 지원하지 않는다.
    RidibooksBookstore, supports_isbn = false
);

const RIDI_BASE_URL: &str = "https://ridibooks.com";
const RIDI_API_URL: &str = "https://search-api.ridibooks.com/search";

impl RidibooksBookstore {
    /// 첫 `ISBN` 라벨 `div` 다음 형제 `div`에서 13자리 코드를 뽑는다.
    pub fn _extract_ridi_isbn(&self, soup: &str) -> String {
        let doc = Html::parse_document(soup);
        for label in doc.select(sel!("div")) {
            if text_strip(label) != "ISBN" {
                continue;
            }
            if let Some(sibling) = next_sibling_div(label) {
                let code = text_strip(sibling);
                if rx!(r"^\d{13}$").is_match(&code) {
                    return code;
                }
            }
        }
        String::new()
    }

    /// 조회가 실패할 수 있는 soup용. 실패하면 빈 문자열이다.
    /// 형제 관계를 알 수 없어서 `find_all("div")` 목록에서 `ISBN` 바로 다음 항목을 본다.
    pub fn _extract_ridi_isbn_soup(&self, soup: &dyn SoupSource) -> String {
        match soup.find_all("div") {
            Ok(divs) => {
                let texts: Vec<&str> = divs.iter().map(|d| d.trim()).collect();
                texts.windows(2).find(|w| w[0] == "ISBN" && rx!(r"^\d{13}$").is_match(w[1])).map_or_else(String::new, |w| w[1].to_owned())
            }
            Err(e) => {
                log::error!("ISBN 리디북스 추출 중 오류: {e}");
                String::new()
            }
        }
    }

    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let resp = match self.session.get_with_timeout(RIDI_API_URL, &params(&[("keyword", keyword)]), HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => {
                if self.verbose {
                    log::error!("RIDI 검색 실패: {e}");
                }
                return Vec::new();
            }
        };
        if resp.status_code != 200 {
            if self.verbose {
                log::warn!("RIDI API 응답 실패: {}", resp.status_code);
            }
            return Vec::new();
        }
        let data = resp.json();
        let books = array_field(&data, "books");
        if books.is_empty() {
            if self.verbose {
                log::info!("RIDI 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for book in books.iter().take(self.max_results) {
            let book_id = book.get("b_id");
            let (title, author) = (str_field(book, "title"), str_field(book, "author"));
            let pair = |parent: &str, child: &str| -> Option<String> {
                let (parent, child) = (str_field(book, parent), str_field(book, child));
                if !parent.is_empty() && !child.is_empty() {
                    Some(format!("{parent} > {child}"))
                } else if !child.is_empty() {
                    Some(child)
                } else if !parent.is_empty() {
                    Some(parent)
                } else {
                    None
                }
            };
            let paths: Vec<String> = [pair("parent_category_name", "category_name"), pair("parent_category_name2", "category_name2")].into_iter().flatten().collect();
            let detail_url = if truthy(book_id) { format!("{RIDI_BASE_URL}/books/{}", py_str(book_id)) } else { String::new() };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, paths.join(" || "), detail_url, search_url.clone(), String::new()));
            }
        }
        if self.verbose {
            log::info!("RIDI에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for RidibooksBookstore {
    bookstore_common!(RIDI_BASE_URL, author_first = true);
    api_keyword_search!(search_api);

    fn build_search_url(&self, keyword: &str) -> String {
        format!("{RIDI_BASE_URL}/search?q={}&adult_exclude=n", quote(keyword, "/"))
    }

    /// RIDI는 `search_by_keyword`를 대체하므로 쓰이지 않는다.
    fn extract_search_links(&self, _soup: &str) -> Vec<String> {
        Vec::new()
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        let doc = Html::parse_document(soup);
        let (mut title, mut author, mut category) = (String::new(), String::new(), String::new());

        // 제목: og:title 메타 우선, 실패하면 <h1>
        let meta = first_meta(&doc, "property", "og:title").or_else(|| first_meta(&doc, "name", "title"));
        if let Some(content) = meta_content(meta) {
            title = content.trim().to_owned();
        } else if let Some(h1) = doc.select(sel!("h1")).next().map(text_strip).filter(|t| !t.is_empty()) {
            title = h1;
        }

        // 헤더 영역의 저자 (id 대소문자 오타 포함). html5ever는 `<li><li>`를 형제로 펴므로 비어 있지 않은 첫 li를 쓴다.
        let mut header: Vec<ElementRef> = doc.select(sel!("div#ISLANDS__Header ul > li")).collect();
        if header.is_empty() {
            header = doc.select(sel!("div#iSLANDS__Header ul > li")).collect();
        }
        if let Some(raw) = header.into_iter().map(text_strip).find(|t| !t.is_empty()) {
            author = raw.replace("저자", "").trim().to_owned();
        }
        // '저자'로 끝나는 li 텍스트
        if author.is_empty() {
            for li in doc.select(sel!("li")) {
                let txt = text_strip_spaced(li);
                if txt.ends_with("저자") && txt.chars().count() > "저자".chars().count() {
                    let raw = txt.strip_suffix("저자").unwrap_or(&txt);
                    // '외13명' 형태에 공백을 넣는다
                    author = rx!(r"외(\d+명)").replace_all(raw, " 외 ${1}").trim().to_owned();
                    break;
                }
            }
        }
        // /author 링크
        if author.is_empty()
            && let Some(a) = doc.select(sel!(r#"a[href*="/author"]"#)).next()
        {
            author = text_strip(a);
        }
        // 메타 태그
        if author.is_empty()
            && let Some(content) = meta_content(first_meta(&doc, "name", "author").or_else(|| first_meta(&doc, "property", "og:author")))
        {
            author = content.trim().to_owned();
        }
        // meta description의 `저자: 이름, ...`
        if author.is_empty()
            && let Some(desc) = meta_content(first_meta(&doc, "property", "og:description").or_else(|| first_meta(&doc, "name", "description")))
            && let Some(m) = rx!(r"저자[:：]\s*([^,]+)").captures(desc)
        {
            author = m[1].trim().to_owned();
        }

        // 카테고리: 각 li가 하나의 경로, 다중 경로는 " || "로 잇는다
        let mut paths = Vec::new();
        for li in doc.select(sel!("#books_contents section.detail_body ul li")) {
            let parts: Vec<String> = li.select(sel!("a[href^='/category/']")).map(text_strip).filter(|t| !t.is_empty()).collect();
            if !parts.is_empty() {
                paths.push(parts.join(" > "));
            }
        }
        if !paths.is_empty() {
            category = paths.join(" || ");
        }

        info_value(&title, &author, &category, &self._extract_ridi_isbn(soup))
    }
}

// ---------------------------------------------------------------------------
// 네이버쇼핑
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 네이버쇼핑 도서.
    NaverShoppingBookstore, supports_isbn = false
);

const NAVER_SHOPPING_BASE_URL: &str = "https://search.shopping.naver.com";

impl NaverShoppingBookstore {
    /// 조회가 실패할 수 있는 soup용. 도중에 실패하면 거기까지 뽑은 부분 결과를 돌려준다.
    pub fn extract_book_info_soup(&self, soup: &dyn SoupSource) -> Value {
        let find_title = || -> Result<String, String> {
            let found = match soup.find("h2")? {
                Some(t) => Some(t),
                None => soup.find("div")?,
            };
            Ok(found.map(|t| t.trim().to_owned()).unwrap_or_default())
        };
        match find_title() {
            Ok(title) => info_value(&title, "", "", ""),
            Err(e) => {
                log::error!("네이버쇼핑 상세 정보 추출 중 오류: {e}");
                empty_info()
            }
        }
    }
}

impl AbstractBookstore for NaverShoppingBookstore {
    bookstore_common!(NAVER_SHOPPING_BASE_URL, author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        format!("{NAVER_SHOPPING_BASE_URL}/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&query={}&sort=REL", quote(keyword, "/"))
    }

    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let mut links = Vec::new();
        let doc = Html::parse_document(soup);
        // 모든 <script>에서 JSON 텍스트를 찾는다
        let raw = doc.select(sel!("script")).map(text_raw).find(|t| t.trim().starts_with('{') && t.contains("SearchAll"));
        let Some(raw) = raw else {
            if self.verbose {
                log::warn!("네이버쇼핑 JSON 스크립트를 찾을 수 없습니다");
            }
            return links;
        };
        let data: Value = match serde_json::from_str(&raw) {
            Ok(d) => d,
            Err(e) => {
                if self.verbose {
                    log::error!("네이버쇼핑 JSON 파싱 실패: {e}");
                }
                return links;
            }
        };
        // React Query state에서 SearchAll 아이템을 꺼낸다
        let queries = data.pointer("/props/pageProps/dehydratedState/queries").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let mut seen: Vec<&Value> = Vec::new();
        for q in queries {
            let is_search_all = q.get("queryKey").and_then(Value::as_array).and_then(|k| k.first()).is_some_and(|k| k == "SearchAll");
            if !is_search_all {
                continue;
            }
            match q.pointer("/state/data/SearchAll/bookSasResult/itemList") {
                None => {}
                Some(Value::Array(items)) => {
                    for item in items {
                        let id = item.get("id");
                        if let Some(id) = id.filter(|_| truthy(id))
                            && !seen.contains(&id)
                        {
                            seen.push(id);
                            links.push(format!("{NAVER_SHOPPING_BASE_URL}/book/catalog/{}", py_str(Some(id))));
                        }
                    }
                }
                Some(_) => {}
            }
            break;
        }
        if self.verbose {
            log::info!("네이버쇼핑에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    /// CSS 클래스에 해시 접미사가 붙어서 접두사로 찾는다.
    fn extract_book_info(&self, soup: &str) -> Value {
        let doc = Html::parse_document(soup);
        let title_prefix = "bookTitle_book_name__";
        let title = find_by_class_prefix(&doc, sel!("h2"), title_prefix).or_else(|| find_by_class_prefix(&doc, sel!("div"), title_prefix)).map(text_strip).unwrap_or_default();

        // 저자: '저자' 라벨이 있는 info_title 다음의 info_content
        let mut author = String::new();
        for label in doc.select(sel!("div")).filter(|e| has_class_prefix(e, "bookTitle_info_title__")) {
            if text_strip(label) == "저자" {
                let content = label.next_siblings().filter_map(ElementRef::wrap).find(|e| e.value().name() == "div" && has_class_prefix(e, "bookTitle_info_content__"));
                if let Some(content) = content {
                    author = text_strip(content);
                }
                break;
            }
        }
        if author.is_empty() {
            author = find_by_class_prefix(&doc, sel!("div"), "bookTitle_info_content__").map(text_strip).unwrap_or_default();
        }

        let category = find_by_class_prefix(&doc, sel!("div"), "bookCatalogTop_breadcrumb__").map(|e| text_raw(e).split('>').map(str::trim).collect::<Vec<_>>().join(" > ")).unwrap_or_default();
        info_value(&title, &author, &category, "")
    }
}

// ---------------------------------------------------------------------------
// 문피아
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 문피아.
    MunpiaBookstore, supports_isbn = false
);

const MUNPIA_BASE_URL: &str = "https://novel.munpia.com";

impl AbstractBookstore for MunpiaBookstore {
    bookstore_common!(MUNPIA_BASE_URL, author_first = true);
    default_keyword_search!();

    /// 공백은 `%20`으로 인코딩한다.
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{MUNPIA_BASE_URL}/page/hd.platinum/view/search/keyword/{}/order/search_result", quote(keyword, ""))
    }

    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let doc = Html::parse_document(soup);
        let mut links: Vec<String> = Vec::new();
        for a in doc.select(sel!("div#SEARCH-BOX.section2 > div.ebook_lists > div.article_wrap > div.article > dl.detail > dt > a")) {
            let Some(href) = attr(&a, "href").filter(|h| !h.is_empty()) else { continue };
            let full = if href.starts_with('/') { urljoin(MUNPIA_BASE_URL, href) } else { href.to_owned() };
            if !links.contains(&full) {
                links.push(full);
            }
        }
        if self.verbose {
            log::info!("문피아에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        let doc = Html::parse_document(soup);
        let mut title = String::new();
        let mut author = String::new();
        let mut category = String::new();
        if let Some(content) = meta_content(first_meta(&doc, "property", "og:title").or_else(|| first_meta(&doc, "name", "title"))) {
            title = content.trim().to_owned();
        }
        // 저자: og:description의 '저자명 - 설명'. 없으면 작가 링크
        if let Some(content) = meta_content(first_meta(&doc, "property", "og:description").or_else(|| first_meta(&doc, "name", "description"))) {
            author = content.trim().split(" - ").next().unwrap_or("").to_owned();
        } else if let Some(a) = doc.select(sel!(r#"a[href^="/writer/"]"#)).next() {
            author = text_strip(a);
        }
        if let Some(path) = doc.select(sel!("p.meta-path")).next() {
            category = text_strip(path).split('>').map(str::trim).collect::<Vec<_>>().join(" > ");
        }
        info_value(&title, &author, &category, "")
    }
}

// ---------------------------------------------------------------------------
// 네이버시리즈
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 네이버시리즈. 웹소설과 만화를 함께 검색한다.
    NaverSeriesBookstore, supports_isbn = false
);

const NAVER_SERIES_BASE_URL: &str = "https://series.naver.com";

impl AbstractBookstore for NaverSeriesBookstore {
    bookstore_common!(NAVER_SERIES_BASE_URL, author_first = false);
    default_keyword_search!();

    fn build_search_url(&self, keyword: &str) -> String {
        format!("{NAVER_SERIES_BASE_URL}/search/search.series?t=all&q={}", quote(keyword, "/"))
    }

    /// 웹소설은 `nov.title`, 만화는 `com.title` 링크다.
    fn extract_search_links(&self, soup: &str) -> Vec<String> {
        let doc = Html::parse_document(soup);
        let mut links: Vec<String> = Vec::new();
        for a in doc.select(sel!(r#"ul.lst_list li a[class^="N=a:"]"#)) {
            if let Some(href) = attr(&a, "href").filter(|h| !h.is_empty()) {
                let full = urljoin(NAVER_SERIES_BASE_URL, href);
                if !links.contains(&full) {
                    links.push(full);
                }
            }
        }
        if self.verbose {
            log::info!("NaverSeries에서 {}개의 상세 페이지 링크를 찾았습니다", links.len());
        }
        links
    }

    fn extract_book_info(&self, soup: &str) -> Value {
        let doc = Html::parse_document(soup);
        let title = title_string(&doc).map(|t| t.trim().to_owned()).unwrap_or_default();
        let mut author = String::new();
        // 작가 정보 컨테이너의 <strong>
        if let Some(container) = doc.select(sel!("div#_otherProductByPerson")).next() {
            let strongs: Vec<ElementRef> = container.select(sel!("strong")).collect();
            if strongs.len() >= 2 {
                author = text_strip(strongs[1]);
            }
        }
        // meta name='description', 그다음 og:description
        if author.is_empty()
            && let Some(desc) = meta_content(first_meta(&doc, "name", "description"))
            && let Some(m) = rx!(r"작가[:：]\s*([^,]+)").captures(desc)
        {
            author = m[1].trim().to_owned();
        }
        if author.is_empty()
            && let Some(desc) = meta_content(first_meta(&doc, "property", "og:description"))
            && let Some(m) = rx!(r"작가[:：]\s*([^,」]+)").captures(desc)
        {
            author = m[1].trim().to_owned();
        }
        let category = doc.select(sel!("div#content > ul.end_info > li.info_lst > ul > li span a")).next().map(text_strip).unwrap_or_default();
        info_value(&title, &author, &category, "")
    }
}

// ---------------------------------------------------------------------------
// 조아라
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 조아라 웹소설. 검색 결과를 서버에서 그리지 않아서 `api.joara.com` JSON을 직접 부른다.
    ///
    /// API 키는 환경 변수 `TM_JOARA_API_KEY`에서 읽는다. `with_env`로 조회 함수를 바꿀 수 있다.
    JoaraBookstore, supports_isbn = false, env: EnvSource = Arc::new(|name| std::env::var(name).ok())
);

const JOARA_BASE_URL: &str = "https://www.joara.com";
const JOARA_API_URL: &str = "https://api.joara.com/v2/search/query";

impl JoaraBookstore {
    /// 환경 변수 조회 함수를 바꾼다. 테스트가 프로세스 환경을 건드리지 않고 키를 넣는 자리다.
    pub fn with_env(mut self, env: EnvSource) -> Self {
        self.env = env;
        self
    }

    /// 없으면 빈 문자열이다.
    fn api_key(&self) -> String {
        (self.env)(JOARA_API_KEY_ENV).unwrap_or_default()
    }

    /// 오류 문장에서 키 값을 가린다. 세션 구현이 요청 주소를 오류에 넣을 수 있기 때문이다.
    fn redact(&self, message: &str, key: &str) -> String {
        if key.is_empty() { message.to_owned() } else { message.replace(key, "****") }
    }

    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let api_key = self.api_key();
        let max = self.max_results.to_string();
        let device_uid = random_hex32();
        let query = params(&[
            ("api_key", &api_key),
            ("device", "mw"),
            ("devicetoken", "mw"),
            // API가 필수로 요구하지만 값은 검증하지 않는다. 호출마다 새로 만든다.
            ("deviceuid", &device_uid),
            ("query", keyword),
            ("target", "subject"),
            ("page", "1"),
            ("offset", &max),
            ("store", "all"),
            ("orderby", "score"),
        ]);

        let resp = match self.session.get_with_timeout(JOARA_API_URL, &query, HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => {
                if self.verbose {
                    log::error!("조아라 검색 실패: {}", self.redact(&e.to_string(), &api_key));
                }
                return Vec::new();
            }
        };
        if resp.status_code != 200 {
            if self.verbose {
                log::warn!("조아라 API 응답 실패: {}", resp.status_code);
            }
            return Vec::new();
        }
        let data = resp.json();
        // 조아라는 파라미터가 빠져도 HTTP 200을 주고 status로만 실패를 알린다.
        if data.get("status").and_then(Value::as_i64) != Some(1) {
            if self.verbose {
                log::warn!("조아라 API 오류: {}", self.redact(&str_field(&data, "message"), &api_key));
            }
            return Vec::new();
        }
        let books = data.get("data").map_or(&[][..], |d| array_field(d, "list"));
        if books.is_empty() {
            if self.verbose {
                log::info!("조아라 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for book in books.iter().take(self.max_results) {
            let code = if truthy(book.get("book_code")) { py_str(book.get("book_code")) } else { String::new() };
            let (title, author, category) = (str_field(book, "subject"), str_field(book, "member_name"), str_field(book, "category_name"));
            let detail_url = if code.is_empty() { String::new() } else { format!("{JOARA_BASE_URL}/book/{code}") };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, category, detail_url, search_url.clone(), String::new()));
            }
        }
        if self.verbose {
            log::info!("조아라에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for JoaraBookstore {
    bookstore_common!(JOARA_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 조아라 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{JOARA_BASE_URL}/search?target=subject&word={}&search=", quote(keyword, "/"))
    }

    // 조아라 검색은 제목(target=subject)만 본다. 저자를 붙이면 늘 0건이라 제목만 넘긴다.
    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 네이버 웹툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 네이버 웹툰. `comic.naver.com/api/search/webtoon` JSON을 직접 부른다.
    NaverWebtoonBookstore, supports_isbn = false
);

const NAVER_WEBTOON_BASE_URL: &str = "https://comic.naver.com";
const NAVER_WEBTOON_API_URL: &str = "https://comic.naver.com/api/search/webtoon";

impl NaverWebtoonBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let resp = match self.session.get_with_timeout(NAVER_WEBTOON_API_URL, &params(&[("keyword", keyword), ("page", "1")]), HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => {
                if self.verbose {
                    log::error!("네이버 웹툰 검색 실패: {e}");
                }
                return Vec::new();
            }
        };
        if resp.status_code != 200 {
            if self.verbose {
                log::warn!("네이버 웹툰 API 응답 실패: {}", resp.status_code);
            }
            return Vec::new();
        }
        let data = resp.json();
        let webtoons = array_field(&data, "searchList");
        if webtoons.is_empty() {
            if self.verbose {
                log::info!("네이버 웹툰 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for webtoon in webtoons.iter().take(self.max_results) {
            let title_id = webtoon.get("titleId");
            let (title, author) = (str_field(webtoon, "titleName"), str_field(webtoon, "displayAuthor"));
            let category = array_field(webtoon, "genreList").iter().map(|g| str_field(g, "description")).filter(|d| !d.is_empty()).collect::<Vec<_>>().join("/");
            let detail_url = if truthy(title_id) { format!("{NAVER_WEBTOON_BASE_URL}/webtoon/list?titleId={}", py_str(title_id)) } else { String::new() };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, category, detail_url, search_url.clone(), String::new()));
            }
        }
        if self.verbose {
            log::info!("네이버 웹툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for NaverWebtoonBookstore {
    bookstore_common!(NAVER_WEBTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 네이버 웹툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{NAVER_WEBTOON_BASE_URL}/search?keyword={}", quote(keyword, "/"))
    }

    // 제목에 저자를 붙인 키워드는 늘 0건이다. 제목만 넘긴다.
    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 카카오웹툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 카카오웹툰. `gateway-kw.kakao.com` JSON을 직접 부른다.
    KakaoWebtoonBookstore, supports_isbn = false
);

const KAKAO_WEBTOON_BASE_URL: &str = "https://webtoon.kakao.com";
const KAKAO_WEBTOON_API_URL: &str = "https://gateway-kw.kakao.com/search/v2/content";
/// authors에는 출판사(PUBLISHER)도 섞여 있다. 사람만 저자로 본다.
const KAKAO_AUTHOR_TYPES: [&str; 3] = ["AUTHOR", "ILLUSTRATOR", "ORIGINAL_STORY"];

impl KakaoWebtoonBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let limit = self.max_results.to_string();
        let resp = match self.session.get_with_timeout(KAKAO_WEBTOON_API_URL, &params(&[("word", keyword), ("offset", "0"), ("limit", &limit)]), HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => {
                if self.verbose {
                    log::error!("카카오웹툰 검색 실패: {e}");
                }
                return Vec::new();
            }
        };
        if resp.status_code != 200 {
            if self.verbose {
                log::warn!("카카오웹툰 API 응답 실패: {}", resp.status_code);
            }
            return Vec::new();
        }
        let data = resp.json();
        let contents = data.get("data").map_or(&[][..], |d| array_field(d, "content"));
        if contents.is_empty() {
            if self.verbose {
                log::info!("카카오웹툰 검색 결과가 없습니다");
            }
            return Vec::new();
        }

        let mut results = Vec::new();
        for content in contents.iter().take(self.max_results) {
            let content_id = content.get("id");
            let title = str_field(content, "title");
            let names = array_field(content, "authors").iter().filter(|a| KAKAO_AUTHOR_TYPES.contains(&str_field(a, "type").as_str())).map(|a| str_field(a, "name")).filter(|n| !n.is_empty());
            // 글·그림이 같은 사람이면 한 번만 쓴다
            let author = join_unique(names, ", ");
            let seo_id = str_field(content, "seoId");
            let slug = if seo_id.is_empty() { &title } else { &seo_id };
            let detail_url = if truthy(content_id) { format!("{KAKAO_WEBTOON_BASE_URL}/content/{}/{}", quote(slug, "/"), py_str(content_id)) } else { String::new() };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, str_field(content, "genre"), detail_url, search_url.clone(), String::new()));
            }
        }
        if self.verbose {
            log::info!("카카오웹툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for KakaoWebtoonBookstore {
    bookstore_common!(KAKAO_WEBTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 카카오웹툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{KAKAO_WEBTOON_BASE_URL}/search?keyword={}", quote(keyword, "/"))
    }

    // 제목에 저자를 붙인 키워드는 0건이다. 제목만 넘긴다.
    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 봄툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 봄툰. `balcony-search-api` JSON을 직접 부른다. 웹소설(novel)이 섞여 오므로 뺀다.
    BomtoonBookstore, supports_isbn = false
);

const BOMTOON_BASE_URL: &str = "https://www.bomtoon.com";
const BOMTOON_API_URL: &str = "https://www.bomtoon.com/api/balcony-search-api/search";
const BOMTOON_NOVEL_TYPE: &str = "novel";
const BOMTOON_CATEGORY_TAG_COUNT: usize = 3;

impl BomtoonBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let query = params(&[
            ("searchText", keyword),
            ("contentsType", "ALL"),
            ("page", "0"),
            ("size", "50"),
            ("device", "WEB"),
            ("isExcludeAdult", "false"),
            ("onlyComplete", "false"),
            ("isSearchTool", "true"),
            ("sortType", "ACCURACY"),
        ]);
        let resp = match self.session.get_with_timeout(BOMTOON_API_URL, &query, HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => {
                if self.verbose {
                    log::error!("봄툰 검색 실패: {e}");
                }
                return Vec::new();
            }
        };
        if resp.status_code != 200 {
            if self.verbose {
                log::warn!("봄툰 API 응답 실패: {}", resp.status_code);
            }
            return Vec::new();
        }
        let data = resp.json();
        let contents = data.get("data").map_or(&[][..], |d| array_field(d, "results"));

        let mut results = Vec::new();
        for content in contents {
            if content.get("contentsType").and_then(Value::as_str) == Some(BOMTOON_NOVEL_TYPE) {
                continue;
            }
            let alias = str_field(content, "alias");
            let title = str_field(content, "title");
            let author = join_unique(array_field(content, "author").iter().map(|a| py_str(Some(a))), ", ");
            let category = array_field(content, "tags").iter().take(BOMTOON_CATEGORY_TAG_COUNT).map(|t| py_str(Some(t))).collect::<Vec<_>>().join("/");
            let detail_url = if alias.is_empty() { String::new() } else { format!("{BOMTOON_BASE_URL}/detail/{}", quote(&alias, "/")) };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, category, detail_url, search_url.clone(), String::new()));
            }
            if results.len() >= self.max_results {
                break;
            }
        }
        if self.verbose {
            log::info!("봄툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for BomtoonBookstore {
    bookstore_common!(BOMTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 봄툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{BOMTOON_BASE_URL}/search?q={}&ref=input", quote(keyword, "/"))
    }

    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 레진코믹스
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 레진코믹스. `lz-api` advanced-search JSON을 직접 부른다.
    LezhinBookstore, supports_isbn = false
);

const LEZHIN_BASE_URL: &str = "https://www.lezhin.com";
const LEZHIN_API_URL: &str = "https://www.lezhin.com/lz-api/v2/advanced-search";
/// artists에는 출판사(publisher)·레이블(label)도 섞여 있다. 사람만 저자로 본다.
const LEZHIN_AUTHOR_ROLES: [&str; 4] = ["writer", "painter", "scripter", "original"];

impl LezhinBookstore {
    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let limit = self.max_results.to_string();
        let query = params(&[("q", keyword), ("t", "all"), ("order", "popular"), ("offset", "0"), ("limit", &limit)]);
        let resp = match self.session.get_with_timeout(LEZHIN_API_URL, &query, HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => {
                if self.verbose {
                    log::error!("레진코믹스 검색 실패: {e}");
                }
                return Vec::new();
            }
        };
        if resp.status_code != 200 {
            if self.verbose {
                log::warn!("레진코믹스 API 응답 실패: {}", resp.status_code);
            }
            return Vec::new();
        }
        let data = resp.json();
        let contents = data.get("data").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);

        let mut results = Vec::new();
        for content in contents.iter().take(self.max_results) {
            let alias = str_field(content, "alias");
            let title = str_field(content, "title");
            let names = array_field(content, "artists").iter().filter(|a| LEZHIN_AUTHOR_ROLES.contains(&str_field(a, "role").as_str())).map(|a| str_field(a, "name")).filter(|n| !n.is_empty());
            let author = join_unique(names, ", ");
            let category = array_field(content, "genres").iter().map(|g| py_str(Some(g))).collect::<Vec<_>>().join("/");
            let detail_url = if alias.is_empty() { String::new() } else { format!("{LEZHIN_BASE_URL}/ko/comic/{}", quote(&alias, "/")) };
            if !title.is_empty() && !detail_url.is_empty() {
                results.push((title, author, category, detail_url, search_url.clone(), String::new()));
            }
        }
        if self.verbose {
            log::info!("레진코믹스에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for LezhinBookstore {
    bookstore_common!(LEZHIN_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 레진코믹스 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{LEZHIN_BASE_URL}/ko/search?t=all&q={}", quote(keyword, "/"))
    }

    title_only_search!();
    unused_html_hooks!();
}

// ---------------------------------------------------------------------------
// 탑툰
// ---------------------------------------------------------------------------

bookstore_struct!(
    /// 탑툰. 전체 작품 목록 JSON(comicTotal)을 받아 제목으로 직접 거른다.
    /// JSON 주소에는 내용 해시가 들어 있어 바뀌므로 매번 페이지 HTML에서 찾는다.
    ToptoonBookstore, supports_isbn = false
);

const TOPTOON_BASE_URL: &str = "https://toptoon.com";

impl ToptoonBookstore {
    /// 공백을 없애고 소문자로 바꿔 제목 포함 여부를 비교한다.
    fn normalize(text: &str) -> String {
        pyfmt::py_lowercase(&text.split_whitespace().collect::<String>())
    }

    fn search_api(&self, keyword: &str) -> Vec<SearchRow> {
        let search_url = self.build_search_url(keyword);
        let needle = Self::normalize(keyword);
        if needle.is_empty() {
            return Vec::new();
        }
        let fail = |e: HttpError| {
            if self.verbose {
                log::error!("탑툰 검색 실패: {e}");
            }
            Vec::new()
        };

        let page = match self.session.get_with_timeout(&search_url, &[], HttpTimeout::from_secs(10, 10)) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        if page.status_code != 200 {
            if self.verbose {
                log::warn!("탑툰 페이지 응답 실패: {}", page.status_code);
            }
            return Vec::new();
        }
        let page_text = page.text();
        let Some(found) = rx!(r"https://[a-z0-9.-]+\.cloudfront\.net/production/comicTotal/[0-9a-f]+\.json").find(&page_text) else {
            if self.verbose {
                log::warn!("탑툰 작품 목록 JSON 주소를 찾지 못했습니다");
            }
            return Vec::new();
        };

        let catalog_resp = match self.session.get_with_timeout(found.as_str(), &[], HttpTimeout::from_secs(20, 20)) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        if catalog_resp.status_code != 200 {
            if self.verbose {
                log::warn!("탑툰 작품 목록 응답 실패: {}", catalog_resp.status_code);
            }
            return Vec::new();
        }
        let catalog = catalog_resp.json();

        // 제목이 같은 작품, 제목이 키워드로 시작하는 작품, 나머지 순. 같은 순위는 조회수 순.
        let mut matches: Vec<(u8, f64, &Value)> = Vec::new();
        for comic in catalog.as_array().map_or(&[][..], Vec::as_slice) {
            let Some(meta) = comic.get("meta") else { continue };
            let normalized = Self::normalize(&str_field(meta, "title"));
            if !normalized.contains(&needle) || !truthy(meta.get("comicsListUrl")) {
                continue;
            }
            let rank = if normalized == needle {
                0
            } else if normalized.starts_with(&needle) {
                1
            } else {
                2
            };
            matches.push((rank, -meta.get("viewCount").and_then(Value::as_f64).unwrap_or(0.0), comic));
        }
        matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));

        let mut results = Vec::new();
        for (_, _, comic) in matches.into_iter().take(self.max_results) {
            let meta = &comic["meta"];
            let author = [meta.pointer("/authorList/authorStr"), meta.pointer("/author/authorString")].into_iter().find(|v| truthy(*v)).map(|v| py_str(v)).unwrap_or_default();
            let category = array_field(meta, "genre").iter().map(|g| str_field(g, "name")).filter(|n| !n.is_empty()).collect::<Vec<_>>().join("/");
            results.push((str_field(meta, "title"), author, category, format!("{TOPTOON_BASE_URL}{}", str_field(meta, "comicsListUrl")), search_url.clone(), String::new()));
        }
        if self.verbose {
            log::info!("탑툰에서 {}개의 검색 결과를 찾았습니다", results.len());
        }
        results
    }
}

impl AbstractBookstore for ToptoonBookstore {
    bookstore_common!(TOPTOON_BASE_URL, author_first = false);
    api_keyword_search!(search_api);

    /// 사용자가 눌러 볼 수 있는 탑툰 검색 결과 페이지 URL
    fn build_search_url(&self, keyword: &str) -> String {
        format!("{TOPTOON_BASE_URL}/hashtag?keyword={}", quote(keyword, "/"))
    }

    title_only_search!();
    unused_html_hooks!();
}

#[cfg(test)]
mod utility_tests {
    use super::*;

    #[test]
    fn truthy_matches_python_scalar_and_container_values() {
        assert!(!truthy(None));
        assert!(!truthy(Some(&Value::Null)));
        assert!(!truthy(Some(&json!(false))));
        assert!(truthy(Some(&json!(true))));
        assert!(!truthy(Some(&json!(0))));
        assert!(truthy(Some(&json!(-1))));
        assert!(!truthy(Some(&json!(""))));
        assert!(truthy(Some(&json!("x"))));
        assert!(!truthy(Some(&json!([]))));
        assert!(truthy(Some(&json!([1]))));
        assert!(!truthy(Some(&json!({}))));
        assert!(truthy(Some(&json!({"x": 1}))));
    }

    #[test]
    fn default_ureq_session_uses_standard_configuration() {
        let session = UreqHttpSession::default();

        assert_eq!(session.headers.len(), 1);
        assert_eq!(session.headers[0].0, "User-Agent");
    }

    #[test]
    fn urljoin_handles_absolute_protocol_relative_root_and_relative_paths() {
        let base = "https://example.com/books/list";
        assert_eq!(urljoin(base, "https://cdn.example.com/book"), "https://cdn.example.com/book");
        assert_eq!(urljoin(base, "//cdn.example.com/book"), "https://cdn.example.com/book");
        assert_eq!(urljoin(base, "/book"), "https://example.com/book");
        assert_eq!(urljoin(base, "book"), "https://example.com/book");
        assert_eq!(urljoin("example.com", "/book"), "example.com/book");
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
