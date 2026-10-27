//! 온라인 서점(Yes24·알라딘·교보문고) 검색. 원본: `backend/bookstore.py`의 `AbstractBookstore`와 세 구현.
//!
//! 검색 페이지에서 상세 페이지 링크를 뽑고(`extract_search_links`), 상세 페이지를 받아
//! 제목·저자·카테고리·ISBN을 뽑는다(`extract_book_info`). 네트워크는 [`Fetcher`] 뒤에 둬서 테스트에서 갈아 끼운다.
//!
//! 원본과 다른 점: BeautifulSoup `html.parser` 대신 html5ever(`scraper`)로 파싱한다. 정상 HTML에서는 같고
//! 심하게 깨진 HTML에서는 트리가 다를 수 있다. 상세 페이지 HTML을 임시 디렉토리에 캐시하지 않고, 쿠키를 넘기지 않는다.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};

/// 상세 페이지를 받아 볼 검색 결과 수 상한.
pub const MAX_RESULTS: usize = 4;
const MAX_PARALLEL: usize = 8;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CURL_UA: &str = "curl/7.79.1";
const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const ACCEPT: &str = "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";
const ACCEPT_LANGUAGE: &str = "ko-KR,ko;q=0.9,en-US;q=0.8,en;q=0.7";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Yes24,
    Aladin,
    Kyobo,
}

/// 상세 페이지에서 뽑은 책 정보. 못 뽑은 값은 빈 문자열이다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookInfo {
    pub title: String,
    pub author: String,
    pub category: String,
    pub isbn: String,
}

/// 검색 결과 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRow {
    pub title: String,
    pub author: String,
    pub category: String,
    pub detail_url: String,
    pub search_url: String,
    pub isbn: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOutcome {
    pub rows: Vec<SearchRow>,
    /// 실제로 쓴 검색어
    pub keyword: String,
    /// `isbn`, `title_author`, `title`, `unknown`
    pub method: &'static str,
}

/// 페이지를 받아 오는 쪽.
pub trait Fetcher: Sync {
    fn get(&self, store: Store, url: &str) -> Result<String, String>;
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).unwrap_or_else(|e| panic!("잘못된 선택자 {css}: {e}"))
}

macro_rules! sel {
    ($css:expr) => {{
        static S: LazyLock<Selector> = LazyLock::new(|| selector($css));
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

fn attr<'a>(el: &ElementRef<'a>, name: &str) -> Option<&'a str> {
    el.value().attr(name)
}

/// `urllib.parse.urljoin`의 흔한 경우: 절대 URL, `//host`, `/path`, 상대 경로.
fn urljoin(base: &str, href: &str) -> String {
    if href.contains("://") {
        return href.to_owned();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let origin_end = base[scheme_end..].find('/').map_or(base.len(), |i| scheme_end + i);
    if let Some(rest) = href.strip_prefix("//") {
        return format!("{}//{rest}", &base[..scheme_end.saturating_sub(2).max(0)]);
    }
    if href.starts_with('/') {
        return format!("{}{href}", &base[..origin_end]);
    }
    let dir_end = base[origin_end..].rfind('/').map_or(origin_end, |i| origin_end + i + 1);
    let dir = if dir_end == origin_end { format!("{}/", &base[..origin_end]) } else { base[..dir_end].to_owned() };
    format!("{dir}{href}")
}

/// `urllib.parse.quote(s)`: 영문·숫자·`_.-~/`만 그대로 둔다.
fn quote(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 제목에서 부제 구분자(` - `, `－`, `-`, `：`) 왼쪽만 남긴다.
pub fn truncate_title(title: &str) -> String {
    let mut title = title.to_owned();
    for sep in [" - ", "－", "-", "："] {
        if let Some((left, _)) = title.clone().split_once(sep) {
            title = left.trim().to_owned();
        }
    }
    title
}

/// 제목 끝의 권수 번호를 뗀다. 예: `마왕의 딸 3` -> `마왕의 딸`.
pub fn strip_trailing_number(title: &str) -> String {
    rx!(r"\s+\d+\s*$").replace(title, "").into_owned()
}

fn push_unique(links: &mut Vec<String>, link: String) {
    if !links.contains(&link) {
        links.push(link);
    }
}

impl Store {
    pub const ALL: [Store; 3] = [Store::Yes24, Store::Aladin, Store::Kyobo];

    pub fn name(self) -> &'static str {
        match self {
            Store::Yes24 => "yes24",
            Store::Aladin => "aladin",
            Store::Kyobo => "kyobo",
        }
    }

    pub fn from_name(name: &str) -> Option<Store> {
        Store::ALL.into_iter().find(|s| s.name() == name)
    }

    pub fn base_url(self) -> &'static str {
        match self {
            Store::Yes24 => "https://www.yes24.com",
            Store::Aladin => "https://www.aladin.co.kr",
            Store::Kyobo => "https://search.kyobobook.co.kr",
        }
    }

    pub fn supports_isbn_search(self) -> bool {
        true
    }

    /// 저자와 제목을 함께 검색할 때 저자를 앞에 둔다.
    pub fn author_first(self) -> bool {
        self == Store::Aladin
    }

    pub fn user_agent(self) -> &'static str {
        if self == Store::Kyobo { CHROME_UA } else { CURL_UA }
    }

    fn referer(self) -> &'static str {
        if self == Store::Kyobo { "https://www.kyobobook.co.kr/" } else { self.base_url() }
    }

    pub fn build_search_url(self, keyword: &str) -> String {
        let q = quote(keyword);
        match self {
            Store::Yes24 => format!("{}/Product/Search?domain=ALL&query={q}", self.base_url()),
            Store::Aladin => format!("{}/search/wsearchresult.aspx?SearchTarget=All&SearchWord={q}", self.base_url()),
            Store::Kyobo => format!("{}/search?keyword={q}", self.base_url()),
        }
    }

    /// ISBN은 숫자뿐이라 인코딩하지 않는다.
    pub fn build_isbn_search_url(self, isbn: &str) -> String {
        match self {
            Store::Yes24 => format!("{}/Product/Search?domain=ALL&query={isbn}", self.base_url()),
            Store::Aladin => format!("{}/search/wsearchresult.aspx?SearchTarget=All&SearchWord={isbn}", self.base_url()),
            Store::Kyobo => format!("{}/search?keyword={isbn}", self.base_url()),
        }
    }

    /// 검색 결과 페이지에서 상세 페이지 링크를 뽑는다. 순서를 지키고 중복은 뺀다.
    pub fn extract_search_links(self, html: &str) -> Vec<String> {
        let doc = Html::parse_document(html);
        match self {
            Store::Yes24 => yes24_links(&doc),
            Store::Aladin => aladin_links(&doc),
            Store::Kyobo => kyobo_links(&doc),
        }
    }

    /// 상세 페이지에서 책 정보를 뽑는다.
    pub fn extract_book_info(self, html: &str) -> BookInfo {
        let doc = Html::parse_document(html);
        match self {
            Store::Yes24 => yes24_info(&doc, html),
            Store::Aladin => aladin_info(&doc),
            Store::Kyobo => kyobo_info(&doc),
        }
    }
}

fn yes24_links(doc: &Html) -> Vec<String> {
    let base = Store::Yes24.base_url();
    let mut links = Vec::new();
    for a in doc.select(sel!(r#"ul#yesSchList > li > div.itemUnit > div.item_info > div.info_row.info_name a.gd_name[href^="/product/goods/"]"#)) {
        if let Some(href) = attr(&a, "href") {
            push_unique(&mut links, urljoin(base, href));
        }
    }
    if links.is_empty() {
        for a in doc.select(sel!("a.gd_name[href]")) {
            if let Some(href) = attr(&a, "href").filter(|h| h.starts_with("/product/goods/")) {
                push_unique(&mut links, urljoin(base, href));
            }
        }
    }
    links
}

fn aladin_links(doc: &Html) -> Vec<String> {
    let base = Store::Aladin.base_url();
    let mut links = Vec::new();
    let Some(result) = doc.select(sel!("div#Search3_Result")).next() else { return links };
    for a in result.select(sel!("a[href]")) {
        if links.len() >= MAX_RESULTS {
            break;
        }
        let Some(href) = attr(&a, "href") else { continue };
        if rx!(r"/shop/wproduct\.aspx\?ItemId=\d+").is_match(href) && !href.contains("_CommentReview") {
            push_unique(&mut links, urljoin(base, href));
        }
    }
    links
}

fn kyobo_links(doc: &Html) -> Vec<String> {
    let mut links = Vec::new();
    for item in doc.select(sel!("ul.prod_list > li.prod_item")).take(MAX_RESULTS) {
        if let Some(href) = item.select(sel!("a.prod_info, a.prod_link")).next().and_then(|a| attr(&a, "href")).filter(|h| !h.is_empty()) {
            push_unique(&mut links, urljoin("https://product.kyobobook.co.kr", href));
        }
    }
    if links.is_empty() {
        for a in doc.select(sel!(r#"a[href*="/detail/"], a[href*="ebook-product.kyobobook.co.kr"]"#)) {
            let href = attr(&a, "href").unwrap_or("");
            if !href.is_empty() && !links.iter().any(|l| l == href) {
                links.push(href.to_owned());
                if links.len() >= MAX_RESULTS {
                    break;
                }
            }
        }
    }
    links
}

fn yes24_info(doc: &Html, html: &str) -> BookInfo {
    let mut info = BookInfo::default();
    if let Some(title) = doc.select(sel!("h2.gd_name")).next() {
        info.title = text_strip(title);
    }
    if let Some(author) = doc.select(sel!("span.gd_auth")).next() {
        info.author = match author.select(sel!("a")).next() {
            Some(a) => text_strip(a),
            None => text_strip(author),
        };
    }
    info.category = yes24_category(doc);
    info.isbn = yes24_isbn(html);
    info
}

/// "관련분류" 글자가 든 문자열(주석 포함)의 부모 안에서 카테고리 링크를 모아 `a > b` 꼴로 만든다.
fn yes24_category(doc: &Html) -> String {
    for node in doc.tree.nodes() {
        // BeautifulSoup의 문자열 검색은 주석과 스크립트 안 글자도 포함한다. 실제 페이지는 주석 안 "관련분류 시작" 뒤에 링크가 있다.
        let text: &str = match node.value() {
            Node::Text(text) => text,
            Node::Comment(comment) => comment,
            _ => continue,
        };
        if !text.contains("관련분류") {
            continue;
        }
        let Some(parent) = node.parent().and_then(ElementRef::wrap) else { continue };
        let mut path: Vec<String> = Vec::new();
        for link in parent.select(sel!(r#"a[href*="/product/category/display/"]"#)) {
            let label = text_strip(link);
            if !label.is_empty() && !path.contains(&label) {
                path.push(label);
            }
        }
        if !path.is_empty() {
            return path.join(" > ");
        }
    }
    String::new()
}

fn yes24_isbn(html: &str) -> String {
    let page_text = htmlkit::text_of(html);
    for pattern in [rx!(r"ISBN13\s*(\d{13})"), rx!(r"ISBN10\s*(\d{10})"), rx!(r"ISBN\s*(\d{10,13})"), rx!(r"(\d{10,13})")] {
        if let Some(c) = pattern.captures(&page_text) {
            return c[1].to_owned();
        }
    }
    String::new()
}

fn first_meta<'a>(doc: &'a Html, key: &str, value: &str) -> Option<ElementRef<'a>> {
    doc.select(sel!("meta")).find(|m| m.value().attr(key) == Some(value))
}

fn aladin_info(doc: &Html) -> BookInfo {
    let mut info = BookInfo::default();
    // 1. <title>: `제목 | 부제 | 저자 | 알라딘` 꼴
    if let Some(title_tag) = doc.select(sel!("title")).next() {
        let mut children = title_tag.children();
        if let (Some(only), None) = (children.next(), children.next())
            && let Node::Text(text) = only.value()
        {
            let parts: Vec<&str> = text.split('|').map(str::trim).collect();
            if parts.len() >= 3 {
                info.author = parts[parts.len() - 2].to_owned();
                info.title = parts[..parts.len() - 2].join(" | ");
            }
        }
    }
    // 1.5 메타 저자. 앞의 태그가 있으면 내용이 비어도 뒤의 태그는 보지 않는다.
    let meta_author = first_meta(doc, "name", "author").or_else(|| first_meta(doc, "property", "og:author"));
    if let Some(content) = meta_author.as_ref().and_then(|m| attr(m, "content")).filter(|c| !c.is_empty()) {
        info.author = content.to_owned();
    }
    let meta_title = first_meta(doc, "property", "og:title").or_else(|| first_meta(doc, "name", "title"));
    if let Some(content) = meta_title.as_ref().and_then(|m| attr(m, "content")).filter(|c| !c.is_empty()) {
        info.title = content.to_owned();
    }
    if let Some(ul) = doc.select(sel!("ul#ulCategory")).next() {
        let parts: Vec<String> = ul.select(sel!("a")).map(text_strip).collect();
        if !parts.is_empty() {
            info.category = parts.join(" > ");
        }
    }
    info.isbn = aladin_isbn(doc);
    info
}

fn aladin_isbn(doc: &Html) -> String {
    for li in doc.select(sel!("#Ere_prod_allwrap div.Ere_prod_mconts_R > div.conts_info_list1 > ul li")) {
        let text = text_strip(li);
        if text.starts_with("ISBN")
            && let Some(m) = rx!(r"(\d{13})").captures(&text)
        {
            return m[1].to_owned();
        }
    }
    for li in doc.select(sel!("#Ere_prod_allwrap > div.Ere_prod_middlewrap div.Ere_prod_mconts_R ul > li")) {
        let text = text_strip(li);
        if text.contains("ISBN")
            && let Some(m) = rx!(r"(\d{10,13})").captures(&text)
        {
            return m[1].to_owned();
        }
    }
    String::new()
}

fn kyobo_info(doc: &Html) -> BookInfo {
    let mut info = BookInfo::default();
    if let Some(title) = doc.select(sel!("h1.prod_title, .prod_info_title, h2.gd_name, h1")).next() {
        info.title = text_strip(title);
    } else if let Some(page_title) = doc.select(sel!("title")).next() {
        let mut children = page_title.children();
        if let (Some(only), None) = (children.next(), children.next())
            && let Node::Text(text) = only.value()
        {
            info.title = rx!(r"\s*[-|]\s*교보문고\s*$").replace(text, "").trim().to_owned();
        }
    }
    if let Some(author) = doc.select(sel!(".author.rep, .author a, span.gd_auth a")).next() {
        info.author = text_strip(author);
    }
    let labels: Vec<String> = doc
        .select(sel!(r#"a[href*="store.kyobobook.co.kr/category/"], ol.breadcrumb_list > li > a, .breadcrumb a, .btn_sub_depth, .box_detail_category a"#))
        .map(text_strip)
        .filter(|t| !["홈", "국내도서", "eBook", "외국도서", "sam"].contains(&t.as_str()))
        .collect();
    if !labels.is_empty() {
        info.category = labels.join(" > ");
    }
    for label in doc.select(sel!("dt, th, span")) {
        if !text_strip(label).contains("ISBN") {
            continue;
        }
        let Some(parent) = label.ancestors().filter_map(ElementRef::wrap).find(|a| matches!(a.value().name(), "dl" | "tr" | "div")) else { continue };
        let Some(value) = parent.select(sel!("dd, td, span")).next() else { continue };
        let text: String = value.text().collect();
        if let Some(m) = rx!(r"(\d{10,13})").captures(&text) {
            info.isbn = m[1].to_owned();
            break;
        }
    }
    info
}

/// 한 서점에서 검색한다. 우선순위는 ISBN, 제목+저자, 제목이다. 부제 구분자 왼쪽만 제목으로 쓴다.
pub fn search(store: Store, fetcher: &dyn Fetcher, isbn: &str, title: &str, author: &str) -> SearchOutcome {
    let mut title = if title.is_empty() { String::new() } else { truncate_title(title) };
    if store == Store::Aladin && !title.is_empty() {
        title = strip_trailing_number(&title);
    }
    let outcome = |rows, keyword: &str, method| SearchOutcome { rows, keyword: keyword.to_owned(), method };
    let combined = || if store.author_first() { format!("{author} {title}") } else { format!("{title} {author}") };

    if !isbn.is_empty() && store.supports_isbn_search() {
        let rows = fetch_results(store, fetcher, &store.build_isbn_search_url(isbn));
        if !rows.is_empty() {
            return outcome(rows, isbn, "isbn");
        }
    }
    if !title.is_empty() && !author.is_empty() {
        let keyword = combined();
        let rows = fetch_results(store, fetcher, &store.build_search_url(&keyword));
        if !rows.is_empty() {
            return outcome(rows, &keyword, "title_author");
        }
    }
    if !title.is_empty() {
        let rows = fetch_results(store, fetcher, &store.build_search_url(&title));
        if !rows.is_empty() {
            return outcome(rows, &title, "title");
        }
    }
    if !title.is_empty() && !author.is_empty() {
        return outcome(Vec::new(), &combined(), "title_author");
    }
    if !title.is_empty() {
        return outcome(Vec::new(), &title, "title");
    }
    if !isbn.is_empty() {
        return outcome(Vec::new(), isbn, "isbn");
    }
    outcome(Vec::new(), "", "unknown")
}

/// 검색 URL의 결과 링크 앞쪽 `MAX_RESULTS`개로 상세 페이지를 병렬로 받아 행을 만든다. 순서는 링크 순서다.
fn fetch_results(store: Store, fetcher: &dyn Fetcher, search_url: &str) -> Vec<SearchRow> {
    let html = match fetcher.get(store, search_url) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("검색 페이지 요청 실패: {e}");
            return Vec::new();
        }
    };
    let links = store.extract_search_links(&html);
    let detail_urls: Vec<&String> = links.iter().take(MAX_RESULTS).collect();
    let pages: Vec<Option<String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = detail_urls
            .chunks(MAX_PARALLEL)
            .flat_map(|chunk| chunk.iter().map(|url| scope.spawn(move || fetcher.get(store, url).map_err(|e| eprintln!("상세 페이지 요청 실패: {url} - {e}")).ok())).collect::<Vec<_>>())
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or(None)).collect()
    });
    let mut rows = Vec::new();
    for (url, page) in detail_urls.iter().zip(pages) {
        let Some(page) = page.filter(|p| !p.trim().is_empty()) else { continue };
        let info = store.extract_book_info(&page);
        let category = if info.category.is_empty() { String::new() } else { info.category.split('>').map(str::trim).take(3).collect::<Vec<_>>().join(" > ") };
        rows.push(SearchRow { title: info.title, author: info.author, category, detail_url: (*url).clone(), search_url: search_url.to_owned(), isbn: info.isbn });
    }
    rows
}

/// ureq로 페이지를 받는다. 서점별 헤더를 보내고, 상태 코드와 무관하게 본문을 UTF-8로 읽는다(원본과 같다).
pub struct HttpFetcher {
    agent: ureq::Agent,
}

impl HttpFetcher {
    pub fn new() -> Self {
        HttpFetcher { agent: ureq::Agent::config_builder().timeout_global(Some(REQUEST_TIMEOUT)).http_status_as_error(false).build().into() }
    }
}

impl Default for HttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher for HttpFetcher {
    fn get(&self, store: Store, url: &str) -> Result<String, String> {
        let mut response = self
            .agent
            .get(url)
            .header("User-Agent", store.user_agent())
            .header("Accept", ACCEPT)
            .header("Accept-Language", ACCEPT_LANGUAGE)
            .header("Referer", store.referer())
            .call()
            .map_err(|e| e.to_string())?;
        let bytes = response.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
