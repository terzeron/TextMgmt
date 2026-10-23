//! 표지 전용 서점 검색. 이미지 bytes는 별도 요청에서만 내려받는다. Python `backend/cover_sources.py`에 대응한다.
//!
//! 네트워크·시계·파일시스템은 `sysdeps`(`HttpSession`, `Clock`, `FsOps`)로 받는다. 설계: docs/rust-test-seams.md 3.1, 3.5, 3.7, 3.8.

pub mod soup;
pub mod urlx;

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::{Map, Value, json};
use sysdeps::{Clock, FsOps, HttpError, HttpResponse, HttpSession, RealFs, SystemClock};

use self::soup::Soup;
use self::urlx::{Netloc, quote, urlencode, urljoin, urlsplit};

pub const STORE_NAMES: [&str; 8] = ["yes24", "aladin", "kyobo", "naver", "ridi", "munpia", "naverseries", "joara"];

/// 실제 서점 페이지/API에서 확인한 origin과 CDN만 허용한다.
fn store_hosts(store: &str) -> &'static [&'static str] {
    match store {
        "yes24" => &["yes24.com"],
        "aladin" => &["aladin.co.kr"],
        "kyobo" => &["kyobobook.co.kr"],
        "naver" => &["shopping.naver.com", "pstatic.net"],
        "ridi" => &["ridibooks.com", "ridicdn.net"],
        "munpia" => &["munpia.com"],
        "naverseries" => &["series.naver.com", "pstatic.net"],
        "joara" => &["joara.com"],
        _ => &[],
    }
}

/// 입력이나 서점 응답이 잘못됐을 때(Python `ValueError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueError(pub String);

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValueError {}

fn value_err<T>(message: &str) -> Result<T, ValueError> {
    Err(ValueError(message.to_string()))
}

/// 검색·내려받기 오류. `Http`는 `raise_for_status`의 `HTTPError`, `Cancelled`는 `FetchCancelled`에 해당한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    Value(String),
    Http {
        status: u16,
    },
    Cancelled,
    /// 연결 실패·시간 초과 같은 전송 오류.
    Transport(String),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::Value(message) => f.write_str(message),
            SearchError::Http { status } => write!(f, "서점 응답 오류: HTTP {status}"),
            SearchError::Cancelled => f.write_str("서점 요청이 취소되었습니다"),
            SearchError::Transport(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SearchError {}

impl From<ValueError> for SearchError {
    fn from(err: ValueError) -> Self {
        SearchError::Value(err.0)
    }
}

impl From<HttpError> for SearchError {
    fn from(err: HttpError) -> Self {
        SearchError::Transport(err.to_string())
    }
}

/// 취소 여부를 돌려주는 함수. 화면에서 더 이상 필요하지 않은 요청을 멈출 때 쓴다.
pub type CancelFn = Arc<dyn Fn() -> bool + Send + Sync>;

// ---------------------------------------------------------------------------
// 서점별 잠금
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum StoreLockError {
    UnknownStore(String),
    Cancelled,
    Timeout,
    Io(String),
}

impl fmt::Display for StoreLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreLockError::UnknownStore(_) => f.write_str("지원하지 않는 서점입니다"),
            StoreLockError::Cancelled => f.write_str("서점 요청이 취소되었습니다"),
            StoreLockError::Timeout => f.write_str("서점 요청이 밀려 있습니다. 다시 시도하세요"),
            StoreLockError::Io(message) => write!(f, "잠금 파일 오류: {message}"),
        }
    }
}

impl std::error::Error for StoreLockError {}

/// 잠금을 쥔 동안 살아 있다. 버리면 파일이 닫혀 flock이 풀린다.
#[derive(Debug)]
pub struct StoreLockGuard {
    _file: File,
}

pub fn store_lock(cache_dir: &Path, store: &str) -> Result<StoreLockGuard, StoreLockError> {
    store_lock_with(cache_dir, store, &RealFs, &SystemClock, &|| false)
}

/// `flock`은 같은 호스트의 thread/process뿐 아니라 공유 볼륨 replica도 직렬화한다.
pub fn store_lock_with(cache_dir: &Path, store: &str, fs: &dyn FsOps, clock: &dyn Clock, cancelled: &dyn Fn() -> bool) -> Result<StoreLockGuard, StoreLockError> {
    if !STORE_NAMES.contains(&store) {
        return Err(StoreLockError::UnknownStore(store.to_string()));
    }
    std::fs::create_dir_all(cache_dir).map_err(|e| StoreLockError::Io(e.to_string()))?;
    let file = OpenOptions::new().read(true).append(true).create(true).open(cache_dir.join(format!("{store}.lock"))).map_err(|e| StoreLockError::Io(e.to_string()))?;
    let deadline = clock.monotonic() + 120.0;
    loop {
        if cancelled() {
            return Err(StoreLockError::Cancelled);
        }
        match fs.lock_exclusive(&file, true) {
            Ok(()) => break,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if clock.monotonic() > deadline {
                    return Err(StoreLockError::Timeout);
                }
                clock.sleep(0.05);
            }
            Err(err) => return Err(StoreLockError::Io(err.to_string())),
        }
    }
    if cancelled() {
        return Err(StoreLockError::Cancelled);
    }
    Ok(StoreLockGuard { _file: file })
}

// ---------------------------------------------------------------------------
// 서점별 요청 간격과 차단 backoff
// ---------------------------------------------------------------------------

// 서점이 짧은 시간의 연속 요청을 418/429로 막는 경우가 있다(네이버는 4~5건 뒤).
// 요청 간격을 두고, 막히면 일정 시간 요청을 멈춘 뒤 한 건만 시험한다.
fn store_min_interval(store: &str) -> f64 {
    if store == "naver" { 2.0 } else { 0.0 }
}

pub const BLOCK_STATUSES: [u16; 2] = [418, 429];
pub const BLOCK_BACKOFF_START: f64 = 60.0;
pub const BLOCK_BACKOFF_MAX: f64 = 300.0;

/// 서점이 요청을 제한 중이라 `retry_after`초 동안 요청하지 않는다.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreBlocked {
    pub retry_after: f64,
}

impl fmt::Display for StoreBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "store blocked for {}s", self.retry_after)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateError {
    Blocked(StoreBlocked),
    Cancelled,
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::Blocked(blocked) => blocked.fmt(f),
            GateError::Cancelled => f.write_str("서점 요청이 취소되었습니다"),
        }
    }
}

impl std::error::Error for GateError {}

pub fn blocked_status(err: &SearchError) -> bool {
    matches!(err, SearchError::Http { status } if BLOCK_STATUSES.contains(status))
}

/// 서점별 요청 간격과 차단 backoff. `store_lock` 안에서만 쓴다.
///
/// 상태를 cache_dir의 파일에 두어 공유 볼륨의 replica끼리 같은 차단 상태를 본다.
pub struct StoreGate {
    store: String,
    path: PathBuf,
    clock: Arc<dyn Clock>,
    fs: Arc<dyn FsOps>,
    cancel: CancelFn,
}

impl StoreGate {
    pub fn new(cache_dir: &Path, store: &str) -> StoreGate {
        Self::with_clock(cache_dir, store, Arc::new(SystemClock))
    }

    pub fn with_clock(cache_dir: &Path, store: &str, clock: Arc<dyn Clock>) -> StoreGate {
        StoreGate { store: store.to_string(), path: cache_dir.join(format!("{store}.gate.json")), clock, fs: Arc::new(RealFs), cancel: Arc::new(|| false) }
    }

    /// 상태 파일을 쓰는 `FsOps`를 바꾼다.
    pub fn with_fs(mut self, fs: Arc<dyn FsOps>) -> StoreGate {
        self.fs = fs;
        self
    }

    pub fn with_cancel(mut self, cancel: CancelFn) -> StoreGate {
        self.cancel = cancel;
        self
    }

    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    fn load(&self) -> Map<String, Value> {
        match std::fs::read_to_string(&self.path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok()) {
            Some(Value::Object(map)) => map,
            _ => Map::new(),
        }
    }

    fn save(&self, state: &Map<String, Value>) {
        // 상태를 못 남겨도 검색 자체는 계속한다.
        let _ = self.fs.write(&self.path, Value::Object(state.clone()).to_string().as_bytes());
    }

    fn number(state: &Map<String, Value>, key: &str) -> f64 {
        state.get(key).and_then(Value::as_f64).unwrap_or(0.0)
    }

    pub fn before_request(&self) -> Result<(), GateError> {
        let mut state = self.load();
        let remaining = Self::number(&state, "blocked_until") - self.clock.now();
        if remaining > 0.0 {
            return Err(GateError::Blocked(StoreBlocked { retry_after: remaining.ceil() }));
        }
        let interval = store_min_interval(&self.store);
        loop {
            let wait = Self::number(&state, "last_request") + interval - self.clock.now();
            if wait <= 0.0 {
                break;
            }
            if (self.cancel)() {
                return Err(GateError::Cancelled);
            }
            self.clock.sleep(wait.min(0.2));
        }
        state.insert("last_request".to_string(), json!(self.clock.now()));
        self.save(&state);
        Ok(())
    }

    pub fn record_success(&self) {
        let mut state = self.load();
        state.remove("blocked_until");
        state.remove("backoff");
        self.save(&state);
    }

    pub fn record_blocked(&self) -> f64 {
        let mut state = self.load();
        let previous = Self::number(&state, "backoff").trunc();
        let backoff = if previous > 0.0 { (previous * 2.0).min(BLOCK_BACKOFF_MAX) } else { BLOCK_BACKOFF_START };
        state.insert("backoff".to_string(), json!(backoff));
        state.insert("blocked_until".to_string(), json!(self.clock.now() + backoff));
        self.save(&state);
        backoff
    }
}

// ---------------------------------------------------------------------------
// URL 검증과 내려받기
// ---------------------------------------------------------------------------

/// `ipaddress.ip_address(...).is_global`. 사설·루프백·링크 로컬·문서용·멀티캐스트 등은 전역 주소가 아니다.
pub fn is_global(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => is_global_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_global_v4(&v4),
            None => is_global_v6(v6),
        },
    }
}

fn is_global_v4(ip: &Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..128).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_global_v6(ip: &Ipv6Addr) -> bool {
    let seg = ip.segments();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (seg[0] & 0xfe00) == 0xfc00 // 고유 로컬
        || (seg[0] & 0xffc0) == 0xfe80 // 링크 로컬
        || (seg[0] == 0x2001 && seg[1] == 0x0db8) // 문서용
        || (seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64
        || seg[0] == 0x0100 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0) // 버림 접두사
}

/// 시스템 DNS(`getaddrinfo`)로 호스트를 푼다.
pub fn system_resolve(host: &str, port: u16) -> io::Result<Vec<IpAddr>> {
    Ok((host, port).to_socket_addrs()?.map(|a| a.ip()).collect())
}

pub fn validate_url(store: &str, url: &str) -> Result<String, ValueError> {
    validate_url_with(store, url, &system_resolve)
}

/// DNS 조회(`resolve`)를 주입할 수 있는 `validate_url`.
pub fn validate_url_with(store: &str, url: &str, resolve: &dyn Fn(&str, u16) -> io::Result<Vec<IpAddr>>) -> Result<String, ValueError> {
    let parsed = urlsplit(url);
    let netloc = Netloc::parse(parsed.netloc.as_deref().unwrap_or(""));
    let host = netloc.hostname.clone().unwrap_or_default();
    let port_ok = matches!(netloc.port(), Ok(None) | Ok(Some(443)));
    let has_credentials = netloc.username.as_deref().is_some_and(|u| !u.is_empty()) || netloc.password.as_deref().is_some_and(|p| !p.is_empty());
    if parsed.scheme != "https" || has_credentials || !port_ok {
        return value_err("허용하지 않는 표지 URL입니다");
    }
    if !store_hosts(store).iter().any(|domain| host == *domain || host.ends_with(&format!(".{domain}"))) {
        return value_err("허용하지 않는 서점 host입니다");
    }
    match resolve(&host, 443) {
        Ok(addresses) if !addresses.is_empty() && addresses.iter().all(is_global) => Ok(url.to_string()),
        Ok(_) => value_err("서점 host의 주소를 허용하지 않습니다"),
        Err(_) => value_err("서점 host의 주소를 확인하지 못했습니다"),
    }
}

pub type ValidateFn = dyn Fn(&str, &str) -> Result<String, ValueError>;

const MAX_STORE_BYTES: usize = 4 * 1024 * 1024;

/// `session`으로 `url`을 받는다. redirect는 최대 5번 따라가며 매번 `validate`로 URL을 검사한다.
pub fn fetch_bytes_with(store: &str, url: &str, session: &dyn HttpSession, max_bytes: Option<usize>, validate: &ValidateFn) -> Result<Vec<u8>, SearchError> {
    fetch_bytes_cancellable(store, url, session, max_bytes, validate, &|| false)
}

fn fetch_bytes_cancellable(store: &str, url: &str, session: &dyn HttpSession, max_bytes: Option<usize>, validate: &ValidateFn, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, SearchError> {
    let max_bytes = max_bytes.unwrap_or(MAX_STORE_BYTES);
    let mut url = url.to_string();
    for _ in 0..5 {
        if cancelled() {
            return Err(SearchError::Cancelled);
        }
        validate(store, &url)?;
        let mut response: HttpResponse = session.get(&url, &[])?;
        if response.is_redirect() {
            url = urljoin(&url, response.location().unwrap_or(""));
            continue;
        }
        if response.status_code >= 400 {
            return Err(SearchError::Http { status: response.status_code });
        }
        let mut output = Vec::new();
        while let Some(chunk) = response.next_chunk() {
            if cancelled() {
                return Err(SearchError::Cancelled);
            }
            output.extend_from_slice(&chunk?);
            if output.len() > max_bytes {
                return Err(SearchError::Value("서점 응답이 크기 제한을 초과했습니다".to_string()));
            }
        }
        return Ok(output);
    }
    Err(SearchError::Value("서점 redirect 횟수를 초과했습니다".to_string()))
}

// ---------------------------------------------------------------------------
// 후보 추출
// ---------------------------------------------------------------------------

/// 서점 클라이언트. 검색 URL 조립과 HTML 해석을 맡는다. 테스트는 대역으로 바꾼다.
pub trait StoreClient {
    fn supports_isbn_search(&self) -> bool;
    fn api_key(&self) -> String;
    fn api_url(&self) -> String;
    fn base_url(&self) -> String;
    fn build_search_url(&self, keyword: &str) -> String;
    fn extract_search_links(&self, html: &str) -> Vec<String>;
    fn extract_book_info(&self, html: &str) -> Value;
    fn set_session_header(&self, name: &str, value: &str);
    fn close(&self);
}

fn book_info(title: &str, author: &str) -> Value {
    json!({"title": title, "author": author, "category": "", "isbn": ""})
}

fn base_for(store: &str) -> &'static str {
    match store {
        "yes24" => "https://www.yes24.com",
        "aladin" => "https://www.aladin.co.kr",
        "kyobo" => "https://search.kyobobook.co.kr",
        "naver" => "https://search.shopping.naver.com",
        "ridi" => "https://ridibooks.com",
        "munpia" => "https://novel.munpia.com",
        "naverseries" => "https://series.naver.com",
        "joara" => "https://www.joara.com",
        _ => "",
    }
}

static RIDI_MORE_AUTHORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"외(\d+명)").expect("정규식"));
static SERIES_AUTHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"작가[:：]\s*([^,]+)").expect("정규식"));
static SERIES_OG_AUTHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"작가[:：]\s*([^,」]+)").expect("정규식"));

/// 실제 서점용 기본 클라이언트. yes24·알라딘·교보문고는 `bookstore` crate에 맡긴다.
struct DefaultClient {
    store: String,
}

impl DefaultClient {
    fn bookstore(&self) -> Option<bookstore_lib::Store> {
        bookstore_lib::Store::from_name(&self.store)
    }

    fn ridi_info(html: &str) -> Value {
        let soup = Soup::parse(html);
        let meta = soup.meta_content("property", "og:title").filter(|c| !c.is_empty()).or_else(|| soup.meta_content("name", "title").filter(|c| !c.is_empty()));
        let title = match meta {
            Some(content) => content.trim().to_string(),
            None => soup.find_all(soup.root(), "h1").first().map(|h1| soup.text_strip(*h1)).unwrap_or_default(),
        };
        let mut author = String::new();
        // 'li' 중 '저자'로 끝나는 요소의 앞쪽 글자를 저자로 본다.
        for li in soup.find_all(soup.root(), "li") {
            let text = soup.text_strip_sep(li);
            if let Some(raw) = text.strip_suffix("저자").filter(|r| !r.is_empty()) {
                let spaced = RIDI_MORE_AUTHORS.replace_all(raw, " 외 $1");
                author = spaced.trim().to_string();
                break;
            }
        }
        if author.is_empty() {
            let link = soup.find_all(soup.root(), "a").into_iter().find(|a| soup.attr(*a, "href").is_some_and(|h| h.contains("/author")));
            if let Some(link) = link {
                author = soup.text_strip(link);
            }
        }
        book_info(&title, &author)
    }

    fn naverseries_info(html: &str) -> Value {
        let soup = Soup::parse(html);
        let title = soup.find_all(soup.root(), "title").first().map(|t| soup.text(*t).trim().to_string()).unwrap_or_default();
        let mut author = String::new();
        let container = soup.find_all(soup.root(), "div").into_iter().find(|d| soup.attr(*d, "id") == Some("_otherProductByPerson"));
        if let Some(container) = container {
            let strongs = soup.find_all(container, "strong");
            if strongs.len() >= 2 {
                author = soup.text_strip(strongs[1]);
            }
        }
        if author.is_empty()
            && let Some(desc) = soup.meta_content("name", "description").filter(|d| !d.is_empty())
            && let Some(m) = SERIES_AUTHOR.captures(&desc)
        {
            author = m[1].trim().to_string();
        }
        if author.is_empty()
            && let Some(desc) = soup.meta_content("property", "og:description").filter(|d| !d.is_empty())
            && let Some(m) = SERIES_OG_AUTHOR.captures(&desc)
        {
            author = m[1].trim().to_string();
        }
        book_info(&title, &author)
    }

    fn naverseries_links(html: &str) -> Vec<String> {
        let soup = Soup::parse(html);
        let mut links: Vec<String> = Vec::new();
        for a in soup.find_all(soup.root(), "a") {
            let in_list = soup.attr(a, "class").is_some_and(|c| c.starts_with("N=a:"))
                && soup.has_ancestor(a, |s, p| s.tag(p) == "li" && s.has_ancestor(p, |s2, q| s2.tag(q) == "ul" && s2.has_class(q, "lst_list")));
            if !in_list {
                continue;
            }
            if let Some(href) = soup.attr(a, "href").filter(|h| !h.is_empty()) {
                let full = urljoin(base_for("naverseries"), href);
                if !links.contains(&full) {
                    links.push(full);
                }
            }
        }
        links
    }
}

impl StoreClient for DefaultClient {
    fn supports_isbn_search(&self) -> bool {
        self.bookstore().is_some_and(bookstore_lib::Store::supports_isbn_search)
    }

    /// 조아라 API 키는 소스에 두지 않고 환경 변수 `TM_JOARA_API_KEY`에서 읽는다.
    fn api_key(&self) -> String {
        std::env::var("TM_JOARA_API_KEY").unwrap_or_default()
    }

    fn api_url(&self) -> String {
        "https://api.joara.com/v2/search/query".to_string()
    }

    fn base_url(&self) -> String {
        base_for(&self.store).to_string()
    }

    fn build_search_url(&self, keyword: &str) -> String {
        if let Some(store) = self.bookstore() {
            return store.build_search_url(keyword);
        }
        let encoded = quote(keyword);
        let base = base_for(&self.store);
        match self.store.as_str() {
            "ridi" => format!("{base}/search?q={encoded}&adult_exclude=n"),
            "naver" => format!("{base}/book/search?bookTabType=ALL&pageIndex=1&pageSize=40&query={encoded}&sort=REL"),
            "munpia" => format!("{base}/page/hd.platinum/view/search/keyword/{encoded}/order/search_result"),
            "naverseries" => format!("{base}/search/search.series?t=all&q={encoded}"),
            "joara" => format!("{base}/search?target=subject&word={encoded}&search="),
            _ => String::new(),
        }
    }

    fn extract_search_links(&self, html: &str) -> Vec<String> {
        if let Some(store) = self.bookstore() {
            return store.extract_search_links(html);
        }
        if self.store == "naverseries" { Self::naverseries_links(html) } else { Vec::new() }
    }

    fn extract_book_info(&self, html: &str) -> Value {
        if let Some(store) = self.bookstore() {
            let info = store.extract_book_info(html);
            return book_info(&info.title, &info.author);
        }
        match self.store.as_str() {
            "ridi" => Self::ridi_info(html),
            "naverseries" => Self::naverseries_info(html),
            _ => book_info("", ""),
        }
    }

    fn set_session_header(&self, _name: &str, _value: &str) {
        // 기본 세션은 `SearchHooks`의 세션 factory가 만든다. 헤더는 그쪽 구현이 정한다.
    }

    fn close(&self) {}
}

fn default_client(store: &str) -> Option<Box<dyn StoreClient>> {
    STORE_NAMES.contains(&store).then(|| Box::new(DefaultClient { store: store.to_string() }) as Box<dyn StoreClient>)
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

pub fn extract_html_candidates(store: &str, detail_url: &str, html: &str) -> Vec<Value> {
    match default_client(store) {
        Some(client) => extract_html_candidates_with(client.as_ref(), store, detail_url, html),
        None => Vec::new(),
    }
}

/// `client`로 상세 페이지를 해석해 후보 한 건을 만든다. 제목이나 이미지가 없으면 후보가 없다.
pub fn extract_html_candidates_with(client: &dyn StoreClient, store: &str, detail_url: &str, html: &str) -> Vec<Value> {
    let soup = Soup::parse(html);
    let info = client.extract_book_info(html);
    let info_title = info.get("title").map(value_text).unwrap_or_default();
    let title = if info_title.is_empty() { soup.meta_content("property", "og:title").unwrap_or_default() } else { info_title };
    let mut image_url = soup.meta_content("property", "og:image").unwrap_or_default();
    if store == "aladin" && urlsplit(&image_url).path.ends_with("/img_no.jpg") {
        return Vec::new();
    }
    if store == "naverseries" {
        // og:image는 600x314 공유용 crop이다. 실제 책의 173x260 이미지를 사용한다.
        let portrait = soup.find_all(soup.root(), "img").into_iter().find(|img| soup.has_ancestor(*img, |s, p| s.tag(p) == "a" && s.has_class(p, "pic_area")));
        if let Some(img) = portrait {
            image_url = soup.attr(img, "src").unwrap_or("").to_string();
        }
    }
    if title.is_empty() || image_url.is_empty() {
        return Vec::new();
    }
    let author = info.get("author").map(value_text).unwrap_or_default();
    vec![json!({"title": title, "author": author, "detail_url": detail_url, "image_url": urljoin(detail_url, &image_url)})]
}

pub fn parse_naver_candidates(html: &str) -> Vec<Value> {
    // 2026-10 페이지는 Next flight와 bookListItem DOM을 사용한다.
    let soup = Soup::parse(html);
    let mut candidates = Vec::new();
    for link in soup.find_all(soup.root(), "a") {
        if !soup.attr(link, "class").is_some_and(|c| c.contains("bookListItem_info_top")) {
            continue;
        }
        let Some(image) = soup.find_all(link, "img").first().copied() else { continue };
        let Some(src) = soup.attr(image, "src").filter(|s| !s.is_empty()) else { continue };
        let mut author = String::new();
        for field in soup.descendants(link) {
            if !soup.attr(field, "class").is_some_and(|c| c.contains("bookListItem_define_item")) {
                continue;
            }
            let spans = soup.find_all(field, "span");
            if spans.len() >= 2 && soup.text_strip(spans[0]) == "저자" {
                author = soup.text_strip(spans[1]);
            }
        }
        candidates.push(json!({"title": soup.attr(image, "alt").unwrap_or(""), "author": author, "detail_url": soup.attr(link, "href").unwrap_or(""), "image_url": src}));
        if candidates.len() == 4 {
            break;
        }
    }
    candidates
}

fn list_of<'a>(data: &'a Value, outer: &str, inner: &str) -> &'a [Value] {
    data.get(outer).and_then(|o| o.get(inner)).and_then(Value::as_array).map_or(&[], |list| &list[..list.len().min(4)])
}

fn status_is_one(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Number(n)) => n.as_f64() == Some(1.0),
        Some(Value::Bool(b)) => *b,
        _ => false,
    }
}

pub fn parse_joara_candidates(data: &Value) -> Result<Vec<Value>, ValueError> {
    if !status_is_one(data.get("status")) {
        return value_err("조아라 검색 요청을 처리하지 못했습니다");
    }
    let mut results = Vec::new();
    for book in list_of(data, "data", "list") {
        if truthy(book.get("cover")) && truthy(book.get("book_code")) {
            results.push(json!({
                "title": book.get("subject").cloned().unwrap_or(json!("")),
                "author": book.get("member_name").cloned().unwrap_or(json!("")),
                "detail_url": format!("https://www.joara.com/book/{}", value_text(&book["book_code"])),
                "image_url": book["cover"].clone(),
            }));
        }
    }
    Ok(results)
}

pub fn parse_munpia_candidates(data: &Value) -> Result<Vec<Value>, ValueError> {
    if data.get("code").and_then(Value::as_str) != Some("M000_00000") {
        return value_err("문피아 검색 요청을 처리하지 못했습니다");
    }
    let mut results = Vec::new();
    for book in list_of(data, "result", "searchNovelTabDtos") {
        if truthy(book.get("coverUrl")) && truthy(book.get("novelId")) {
            results.push(json!({
                "title": book.get("title").cloned().unwrap_or(json!("")),
                "author": book.get("author").cloned().unwrap_or(json!("")),
                "detail_url": format!("https://www.munpia.com/novel/detail/{}", value_text(&book["novelId"])),
                "image_url": book["coverUrl"].clone(),
            }));
        }
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// 검색
// ---------------------------------------------------------------------------

/// 서점 이름으로 HTTP 세션을 만드는 함수.
pub type SessionFactory = Arc<dyn Fn(&str) -> Arc<dyn HttpSession> + Send + Sync>;
type ClientFactory = Arc<dyn Fn() -> Box<dyn StoreClient> + Send + Sync>;
type ValidateHook = Arc<dyn Fn(&str, &str) -> Result<String, ValueError> + Send + Sync>;

/// 실제 HTTP 구현이 연결되기 전의 기본 세션. 모든 요청이 연결 실패다.
struct UnavailableSession;

impl HttpSession for UnavailableSession {
    fn get(&self, _url: &str, _params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        Err(HttpError::Connection("실제 HttpSession 구현이 연결되지 않았습니다".to_string()))
    }
}

/// 검색이 쓰는 바깥 의존성(세션, 클라이언트, 취소, URL 검증). 기본값은 실제 구현이다.
#[derive(Clone)]
pub struct SearchHooks {
    session_factory: SessionFactory,
    client_factories: BTreeMap<String, ClientFactory>,
    cancel: CancelFn,
    validate: ValidateHook,
}

impl Default for SearchHooks {
    fn default() -> Self {
        SearchHooks {
            session_factory: Arc::new(|_store| Arc::new(UnavailableSession) as Arc<dyn HttpSession>),
            client_factories: BTreeMap::new(),
            cancel: Arc::new(|| false),
            validate: Arc::new(validate_url),
        }
    }
}

impl SearchHooks {
    /// 서점별 세션을 만드는 함수를 바꾼다. URL 검증(SSRF 방어)은 그대로 둔다.
    /// 검증을 끄거나 바꾸려면 `with_validate`로 명시해야 한다.
    pub fn with_session_factory(mut self, factory: SessionFactory) -> Self {
        self.session_factory = factory;
        self
    }

    /// 이 설정으로 URL을 검증한다. 허용하면 정규화한 URL을, 막으면 `ValueError`를 돌려준다.
    pub fn validate(&self, store: &str, url: &str) -> Result<String, ValueError> {
        (self.validate)(store, url)
    }

    pub fn with_client_factory(mut self, store: &str, factory: impl Fn() -> Box<dyn StoreClient> + Send + Sync + 'static) -> Self {
        self.client_factories.insert(store.to_string(), Arc::new(factory));
        self
    }

    pub fn with_cancel(mut self, cancel: CancelFn) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_validate(mut self, validate: impl Fn(&str, &str) -> Result<String, ValueError> + Send + Sync + 'static) -> Self {
        self.validate = Arc::new(validate);
        self
    }

    fn client(&self, store: &str) -> Option<Box<dyn StoreClient>> {
        match self.client_factories.get(store) {
            Some(factory) => Some(factory()),
            None => default_client(store),
        }
    }
}

/// `uuid.uuid4().hex`처럼 보이는 32자리 16진수. 암호학적 난수는 아니다(조아라 요청의 장치 식별 자리용).
fn random_hex() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = String::new();
    for _ in 0..2 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        out.push_str(&format!("{:016x}", hasher.finish()));
    }
    out
}

struct ClientGuard(Box<dyn StoreClient>);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub fn search_candidates(store: &str, metadata: &Value) -> Result<Value, SearchError> {
    search_candidates_with(store, metadata, &SearchHooks::default())
}

pub fn search_candidates_with(store: &str, metadata: &Value, hooks: &SearchHooks) -> Result<Value, SearchError> {
    let Some(client) = hooks.client(store) else {
        return Err(SearchError::Value("지원하지 않는 서점입니다".to_string()));
    };
    let client = ClientGuard(client);
    let client = client.0.as_ref();
    if store == "ridi" {
        // curl UA의 HTML challenge 대신 공개 웹 client와 호환되는 요청 header를 사용한다.
        for (name, value) in [
            ("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Safari/537.36"),
            ("Referer", "https://ridibooks.com/"),
            ("Accept", "application/json,text/html"),
            ("Accept-Language", "ko-KR,ko;q=0.9"),
        ] {
            client.set_session_header(name, value);
        }
    }
    let title = metadata.get("title").map(value_text).ok_or_else(|| SearchError::Value("title이 없습니다".to_string()))?;
    let isbn = if client.supports_isbn_search() { metadata.get("isbn").map(value_text).unwrap_or_default() } else { String::new() };
    let keyword = if isbn.is_empty() { title.clone() } else { isbn };
    let mut search_url = if store == "aladin" {
        format!("https://www.aladin.co.kr/search/wsearchresult.aspx?{}", urlencode(&[("SearchTarget", "Book"), ("SearchWord", &keyword)]))
    } else {
        client.build_search_url(&keyword)
    };
    let session = (hooks.session_factory)(store);
    let cancelled = || (hooks.cancel)();
    let fetch = |url: &str| fetch_bytes_cancellable(store, url, session.as_ref(), None, &*hooks.validate, &cancelled);
    let text = |raw: Vec<u8>| String::from_utf8_lossy(&raw).into_owned();
    let results: Vec<Value> = match store {
        "joara" => {
            let device_uid = random_hex();
            let api_key = client.api_key();
            let params = urlencode(&[
                ("api_key", &api_key),
                ("device", "mw"),
                ("devicetoken", "mw"),
                ("deviceuid", &device_uid),
                ("query", &title),
                ("target", "subject"),
                ("page", "1"),
                ("offset", "4"),
                ("store", "all"),
                ("orderby", "score"),
            ]);
            let raw = fetch(&format!("{}?{params}", client.api_url()))?;
            let data: Value = serde_json::from_slice(&raw).map_err(|e| SearchError::Value(e.to_string()))?;
            parse_joara_candidates(&data)?
        }
        "munpia" => {
            // 공식 PC client(2026-09)의 MAIN_SEARCH API와 coverUrl 필드를 사용한다.
            search_url = format!("https://www.munpia.com/search?{}", urlencode(&[("query", &title)]));
            let raw = fetch(&format!("https://www.munpia.com/api/v1/main/search?{}", urlencode(&[("query", &title), ("page", "0"), ("size", "4")])))?;
            let data: Value = serde_json::from_slice(&raw).map_err(|e| SearchError::Value(e.to_string()))?;
            parse_munpia_candidates(&data)?
        }
        _ => {
            let html = text(fetch(&search_url)?);
            let soup = Soup::parse(&html);
            let results = if store == "naver" {
                parse_naver_candidates(&html)
            } else {
                let mut links = client.extract_search_links(&html);
                if store == "ridi" && links.is_empty() {
                    for a in soup.find_all(soup.root(), "a") {
                        if let Some(href) = soup.attr(a, "href").filter(|h| h.starts_with("/books/")) {
                            let full = urljoin(&client.base_url(), href);
                            if !links.contains(&full) {
                                links.push(full);
                            }
                        }
                    }
                }
                let mut results = Vec::new();
                // 기존 AbstractBookstore의 병렬 실행기를 쓰지 않고 차례로 받는다.
                for detail_url in links.iter().take(4) {
                    let detail = text(fetch(detail_url)?);
                    results.extend(extract_html_candidates_with(client, store, detail_url, &detail));
                }
                results
            };
            if results.is_empty() && (html.contains("Just a moment") || !soup.text(soup.root()).contains("검색")) {
                return Err(SearchError::Value("서점 검색 페이지에 접근하지 못했습니다".to_string()));
            }
            results
        }
    };
    let status = if results.is_empty() { "not_found" } else { "success" };
    Ok(json!({"status": status, "search_url": search_url, "candidates": results.into_iter().take(4).collect::<Vec<_>>()}))
}
