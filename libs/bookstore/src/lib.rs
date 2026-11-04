//! 온라인 서점(Yes24·알라딘·교보문고) 검색. 원본: `backend/bookstore.py`의 `AbstractBookstore`와 세 구현.
//!
//! [`stores`]는 backend가 쓰는 전체 서점(리디북스·네이버·문피아·조아라·웹툰 포함) 구현이다. 세 서점의 URL 조립과 파싱은 이 최상위 모듈을 재사용한다.
//!
//! 이 모듈은 검색 URL을 만들고, 검색 페이지에서 상세 페이지 링크를 뽑고(`extract_search_links`),
//! 상세 페이지에서 제목·저자·카테고리·ISBN을 뽑는다(`extract_book_info`). 네트워크 요청과 검색 순서는 [`stores`]가 맡는다.
//!
//! 원본과 다른 점: BeautifulSoup `html.parser` 대신 html5ever(`scraper`)로 파싱한다. 정상 HTML에서는 같고
//! 심하게 깨진 HTML에서는 트리가 다를 수 있다.

pub mod store_clients;

use std::sync::LazyLock;

use htmlkit::url_parser::urljoin;
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};

/// 상세 페이지를 받아 볼 검색 결과 수 상한.
pub const MAX_RESULTS: usize = 4;

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
