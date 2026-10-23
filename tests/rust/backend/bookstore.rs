//! 원본: tests/test_bookstore.py
//!
//! 서점 크롤러(`backend/bookstore.py`) 테스트. HTML 파싱, 검색 우선순위(ISBN > 제목+저자 > 제목),
//! 검색 결과 캐시, 웹툰 서점 JSON API 응답 처리를 확인한다.
//! 서점 HTTP는 `sysdeps::HttpSession` 대역으로 갈아 끼운다(docs/rust-test-seams.md 3.7. libs/bookstore의 `Fetcher` 대역과 같은 방식).
//!
//! 가정한 API (Python 이름을 그대로 쓴다. `_`로 시작하는 비공개 메서드도 같다):
//! - `backend::bookstore`
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
//!     - `[기본]` `tmp_dir() -> PathBuf`(`std::env::temp_dir()`, `tempfile.gettempdir` 대응 seam)
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

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use backend::bookstore::{
    _truncate_title, AbstractBookstore, AladinBookstore, BomtoonBookstore, JoaraBookstore, KakaoWebtoonBookstore, KyoboBookstore, LezhinBookstore, MAX_RESULTS, MunpiaBookstore, NaverSeriesBookstore,
    NaverShoppingBookstore, NaverWebtoonBookstore, RidibooksBookstore, SearchRow, SoupSource, ToptoonBookstore, Yes24Bookstore, abstract_build_search_url, abstract_extract_book_info,
    abstract_extract_search_links,
};
use serde_json::{Value, json};
use sysdeps::{FsOps, HttpError, HttpResponse, HttpSession};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// 공용 도구: 구상 서점 생성, 세션 대역, 결과 행
// ---------------------------------------------------------------------------

fn yes24(verbose: bool) -> Yes24Bookstore {
    common::setup_env();
    Yes24Bookstore::new(verbose)
}

fn aladin(verbose: bool) -> AladinBookstore {
    common::setup_env();
    AladinBookstore::new(verbose)
}

fn kyobo(verbose: bool) -> KyoboBookstore {
    common::setup_env();
    KyoboBookstore::new(verbose)
}

fn ridibooks(verbose: bool) -> RidibooksBookstore {
    common::setup_env();
    RidibooksBookstore::new(verbose)
}

fn naver_shopping(verbose: bool) -> NaverShoppingBookstore {
    common::setup_env();
    NaverShoppingBookstore::new(verbose)
}

fn naver_series(verbose: bool) -> NaverSeriesBookstore {
    common::setup_env();
    NaverSeriesBookstore::new(verbose)
}

fn munpia(verbose: bool) -> MunpiaBookstore {
    common::setup_env();
    MunpiaBookstore::new(verbose)
}

fn joara(verbose: bool) -> JoaraBookstore {
    common::setup_env();
    JoaraBookstore::new(verbose)
}

fn naver_webtoon(verbose: bool) -> NaverWebtoonBookstore {
    common::setup_env();
    NaverWebtoonBookstore::new(verbose)
}

fn kakao_webtoon(verbose: bool) -> KakaoWebtoonBookstore {
    common::setup_env();
    KakaoWebtoonBookstore::new(verbose)
}

fn bomtoon(verbose: bool) -> BomtoonBookstore {
    common::setup_env();
    BomtoonBookstore::new(verbose)
}

fn lezhin(verbose: bool) -> LezhinBookstore {
    common::setup_env();
    LezhinBookstore::new(verbose)
}

fn toptoon(verbose: bool) -> ToptoonBookstore {
    common::setup_env();
    ToptoonBookstore::new(verbose)
}

/// 검색 결과 한 줄. `(title, author, category, detail_url, search_url, isbn)`.
fn row(title: &str, author: &str, category: &str, detail_url: &str, search_url: &str, isbn: &str) -> SearchRow {
    (title.into(), author.into(), category.into(), detail_url.into(), search_url.into(), isbn.into())
}

/// `requests.Response` 대역. `Resp(status_code, text)`와 `json()` 반환값을 한 번에 만든다.
/// 본문은 `text`가 비어 있지 않으면 `text`, 아니면 `json`(Null이 아닐 때)의 직렬화다. 둘을 한 응답에 함께 담을 수는 없다.
fn http(status_code: u16, text: &str, json: Value) -> Result<HttpResponse, HttpError> {
    let body = if !text.is_empty() {
        text.to_owned()
    } else if json.is_null() {
        String::new()
    } else {
        json.to_string()
    };
    Ok(HttpResponse::from_text(status_code, &body))
}

#[derive(Clone, Debug)]
struct Call {
    url: String,
    params: BTreeMap<String, String>,
}

type Handler = Box<dyn Fn(&str, &[(String, String)]) -> Result<HttpResponse, HttpError> + Send + Sync>;

struct FakeSession {
    handler: Handler,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl HttpSession for FakeSession {
    fn get(&self, url: &str, params: &[(String, String)]) -> Result<HttpResponse, HttpError> {
        self.calls.lock().unwrap().push(Call { url: url.to_owned(), params: params.iter().cloned().collect() });
        (self.handler)(url, params)
    }
}

/// `session.get`을 `handler`로 바꾼 세션과, 받은 호출 기록을 돌려준다.
fn session(handler: impl Fn(&str, &[(String, String)]) -> Result<HttpResponse, HttpError> + Send + Sync + 'static) -> (Arc<dyn HttpSession>, Arc<Mutex<Vec<Call>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (Arc::new(FakeSession { handler: Box::new(handler), calls: Arc::clone(&calls) }), calls)
}

/// 어떤 URL이든 같은 응답을 주는 세션. `lambda *a, **k: Resp()` 대응.
fn constant(status_code: u16, text: &str, json: Value) -> Arc<dyn HttpSession> {
    let text = text.to_owned();
    session(move |_, _| http(status_code, &text, json.clone())).0
}

/// 요청하면 `RuntimeError("boom")`을 던지는 세션.
fn failing() -> Arc<dyn HttpSession> {
    session(|_, _| Err(HttpError::Other("boom".to_owned()))).0
}

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn urls(calls: &Arc<Mutex<Vec<Call>>>) -> Vec<String> {
    calls.lock().unwrap().iter().map(|c| c.url.clone()).collect()
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
                common::setup_env();
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

            fn tmp_dir(&self) -> PathBuf {
                self.tmp_dir.clone().unwrap_or_else(std::env::temp_dir)
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

/// `soup.find`가 특정 태그에서 예외를 던지게 바꾼 soup 대역. 나머지는 원본 `find`처럼 동작한다.
struct RaisingSoup {
    html: &'static str,
    raise_on: &'static str,
}

impl SoupSource for RaisingSoup {
    fn find(&self, name: &str) -> Result<Option<String>, String> {
        if name == self.raise_on {
            return Err("parse error".to_owned());
        }
        let inner = between(self.html, &format!("<{name}"), &format!("</{name}>"));
        Ok(if inner.is_empty() { None } else { Some(inner) })
    }

    fn find_all(&self, _name: &str) -> Result<Vec<String>, String> {
        Ok(vec![])
    }

    fn get_text(&self) -> Result<String, String> {
        Ok(String::new())
    }

    fn select(&self, _css: &str) -> Result<Vec<String>, String> {
        Ok(vec![])
    }
}

/// 모든 조회에서 `RuntimeError("boom")`을 던지는 soup 대역 (`BadSoup`, `BadText`).
struct BadSoup;

impl SoupSource for BadSoup {
    fn find(&self, _name: &str) -> Result<Option<String>, String> {
        Err("boom".to_owned())
    }

    fn find_all(&self, _name: &str) -> Result<Vec<String>, String> {
        Err("boom".to_owned())
    }

    fn get_text(&self) -> Result<String, String> {
        Err("boom".to_owned())
    }

    fn select(&self, _css: &str) -> Result<Vec<String>, String> {
        Err("boom".to_owned())
    }
}

// ---------------------------------------------------------------------------
// class TestBookstore(unittest.TestCase)
// ---------------------------------------------------------------------------

mod test_bookstore {
    use super::*;

    #[test]
    fn test_yes24_extract() {
        let store = yes24(false);
        let html = "<html><h2 class='gd_name'>Title</h2><span class='gd_auth'>Author</span></html>";
        let info = store.extract_book_info(html);
        assert_eq!(info["title"], "Title");
    }

    #[test]
    fn test_aladin_extract() {
        let store = aladin(false);
        let html = "<html><title>Title | Author | Aladin</title></html>";
        let info = store.extract_book_info(html);
        assert_eq!(info["title"], "Title");
    }
}

// ---- merged from test_bookstore_extra_branches.py ----

#[test]
fn test_yes24_fallback_links_and_no_category_isbn() {
    let store = yes24(false);
    let html = r#"<html><a class="gd_name" href="/product/goods/1">X</a></html>"#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://www.yes24.com/product/goods/1"]);
    assert_eq!(store._extract_yes24_category(html), "");
    assert_eq!(store._extract_yes24_isbn(html), "");
}

#[test]
fn test_aladin_meta_fallbacks_and_no_isbn() {
    let store = aladin(false);
    let html = r#"
    <html>
      <title>Title | Author | Aladin</title>
      <meta property="og:title" content="OT" />
      <meta property="og:author" content="OA" />
      <div id="Search3_Result"></div>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "OT");
    assert_eq!(info["author"], "OA");
    assert_eq!(store._extract_aladin_isbn("<html></html>"), "");
}

#[test]
fn test_ridibooks_search_failure_and_author_fallbacks() {
    let mut store = ridibooks(false);

    store = store.with_session(constant(500, "", Value::Null));
    assert!(store.search_by_keyword("k").is_empty());

    let html = r#"
    <html>
      <h1>HT</h1>
      <li>홍길동저자</li>
      <a href="/author/1">Auth</a>
      <meta name="author" content="MA" />
      <meta property="og:description" content="저자: MD, 기타" />
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "HT");
    assert_ne!(info["author"], "");
}

#[test]
fn test_naver_shopping_json_missing_and_bad() {
    let store = naver_shopping(false);
    assert!(store.extract_search_links("<html></html>").is_empty());

    assert!(store.extract_search_links("<html><script>{bad</script></html>").is_empty());
}

#[test]
fn test_naver_series_meta_description_fallback() {
    let store = naver_series(false);
    let html = r#"
    <html>
      <meta name="description" content="작가: 홍길동, 기타" />
      <div id="content"><ul class="end_info"><li class="info_lst"><ul><li><span><a>장르</a></span></li></ul></li></ul></div>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "홍길동");
}

#[test]
fn test_munpia_fallback_author() {
    let store = munpia(false);
    let html = r#"
    <html>
      <meta name="title" content="T" />
      <a href="/writer/1">Writer</a>
      <p class="meta-path">A > B</p>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "Writer");
}

#[test]
fn test_truncate_title_variants() {
    assert_eq!(_truncate_title("A - B"), "A");
    assert_eq!(_truncate_title("A－B"), "A");
    assert_eq!(_truncate_title("A - B - C"), "A");
    assert_eq!(_truncate_title("A：B"), "A");
}

#[test]
fn test_search_precedence_and_fallbacks() {
    let mut store = Hooked::new(Kind::Dummy);
    store.fetch = Some(Box::new(|url| {
        if url.contains("/isbn/") {
            return vec![row("T", "A", "C", url, url, "ISBN")];
        }
        if url.contains("author title") {
            return vec![row("T2", "A2", "C2", url, url, "")];
        }
        vec![]
    }));

    let (results, keyword, method) = store.search("123", "title", "author");
    assert_eq!(method, "isbn");
    assert_eq!(keyword, "123");
    assert!(!results.is_empty());

    let (results, keyword, method) = store.search("", "title", "author");
    assert_eq!(method, "title_author");
    assert_eq!(keyword, "author title");
    assert!(!results.is_empty());

    let (results, keyword, method) = store.search("", "only", "");
    assert_eq!(method, "title");
    assert_eq!(keyword, "only");
    assert!(results.is_empty());
}

#[test]
fn test_fetch_search_results_cached_and_trimmed() {
    let mut store = Cached::new(Kind::Dummy);
    store = store.with_session(constant(200, "<html></html>", Value::Null));

    let cached: BTreeMap<&str, &str> = BTreeMap::from([
        ("https://example.com/detail/1", "<html><h1>Title1</h1><span class='author'>Auth</span><p class='category'>A > B > C > D</p><p class='isbn'>123</p></html>"),
        ("https://example.com/detail/2", "   "),
    ]);
    store.load = Some(Box::new(move |url| cached.get(url).map(|s| s.to_string())));
    store.save = Some(Box::new(|_, _| {}));

    let results = store._fetch_search_results("https://example.com/search?q=x");
    assert_eq!(results, vec![row("Title1", "Auth", "A > B > C", "https://example.com/detail/1", "https://example.com/search?q=x", "123")]);
}

#[test]
fn test_fetch_search_results_request_error() {
    let mut store = Plain::new(Kind::Dummy);
    store = store.with_session(failing());
    assert!(store._fetch_search_results("https://example.com/search?q=x").is_empty());
}

#[test]
fn test_search_isbn_not_supported() {
    let mut store = Hooked::new(Kind::Dummy);
    store.supports_isbn_search = false;
    store.keyword = Some(Box::new(|_| vec![row("t", "a", "c", "u", "s", "i")]));
    let (results, keyword, method) = store.search("123", "title", "author");
    assert_eq!(method, "title_author");
    assert_eq!(keyword, "author title");
    assert!(!results.is_empty());
}

#[test]
fn test_save_and_load_html_cache() {
    let tmp = TempDir::new().unwrap();
    let mut store = Plain::new(Kind::Dummy);
    store.tmp_dir = Some(tmp.path().to_path_buf());
    let url = "https://example.com/detail/1";
    let html = "<html>ok</html>";
    store._save_html_to_tmp(html, url);
    let loaded = store._load_html_from_tmp(url);
    assert_eq!(loaded.as_deref(), Some(html));
}

#[test]
fn test_base_build_isbn_search_and_empty_search() {
    // 기본 `search_by_isbn`은 실제 요청을 보내므로 세션 대역이 실패를 돌려준다(Python은 네트워크 실패를 삼킨다).
    let store = Plain::new(Kind::BaseOnly).with_verbose(true);
    assert_eq!(store.build_isbn_search_url("123"), store.build_search_url("123"));
    let (results, keyword, method) = store.search("", "", "");
    assert!(results.is_empty());
    assert_eq!(keyword, "");
    assert_eq!(method, "unknown");
    assert!(store.search_by_isbn("123").is_empty());
}

#[test]
#[ignore = "추상 메서드의 기본 본문(None 반환)은 Rust trait에 대응이 없다. abstract_* 함수는 항상 None이라 검증할 동작이 없다"]
fn test_base_class_abstract_methods_are_callable() {
    let store = Plain::new(Kind::BaseOnly);
    assert_eq!(abstract_build_search_url(&store, "x"), None);
    assert_eq!(abstract_extract_search_links(&store, "<html></html>"), None);
    assert_eq!(abstract_extract_book_info(&store, "<html></html>"), None);
}

#[test]
fn test_fetch_search_results_threaded_cache_miss() {
    let mut store = Cached::new(Kind::Dummy).with_verbose(true);
    store.max_results = 2;
    store.links = Some(vec!["https://example.com/detail/1".to_owned(), "https://example.com/detail/2".to_owned()]);
    store.load = Some(Box::new(|_| None));

    let saved = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&saved);
    store.save = Some(Box::new(move |_html, _url| {
        counter.fetch_add(1, Ordering::SeqCst);
    }));

    // 검색 페이지는 빈 HTML, 상세 페이지는 /1 만 성공하고 그 밖에는 예외. 상세 스레드도 주입한 같은 세션을 쓴다.
    store = store.with_session(
        session(|url, _| {
            if url.ends_with("/1") {
                return http(200, "<html><h1>T</h1><span class='author'>A</span><p class='category'>A > B > C > D</p><p class='isbn'>I</p></html>", Value::Null);
            }
            if url.contains("/detail/") {
                return Err(HttpError::Other("boom".to_owned()));
            }
            http(200, "<html></html>", Value::Null)
        })
        .0,
    );

    let results = store._fetch_search_results("https://example.com/search?q=x");
    assert_eq!(results, vec![row("T", "A", "A > B > C", "https://example.com/detail/1", "https://example.com/search?q=x", "I")]);
    assert_eq!(saved.load(Ordering::SeqCst), 1);
}

#[test]
fn test_fetch_search_results_empty_links_and_blank_html() {
    let mut store = Cached::new(Kind::Dummy).with_verbose(true);
    store = store.with_session(constant(200, "<html></html>", Value::Null));

    store.links = Some(vec![]);
    assert!(store._fetch_search_results("https://example.com/search?q=x").is_empty());

    store.links = Some(vec!["https://example.com/detail/1".to_owned()]);
    store.load = Some(Box::new(|_| Some("   ".to_owned())));
    assert!(store._fetch_search_results("https://example.com/search?q=x").is_empty());
}

#[test]
fn test_save_and_load_html_cache_errors() {
    let tmp = TempDir::new().unwrap();
    // open이 OSError를 던지는 상황: 임시 디렉터리 자리에 일반 파일을 놓아 쓰기와 읽기를 막는다.
    let blocked = tmp.path().join("blocked");
    fs::write(&blocked, "file blocks directory").unwrap();
    let mut store = Cached::new(Kind::Dummy).with_verbose(true);
    store.tmp_dir = Some(blocked);

    store._save_html_to_tmp("<html></html>", "https://example.com/detail/1");

    assert_eq!(store._load_html_from_tmp("https://example.com/detail/1"), None);
}

#[test]
fn test_yes24_primary_links_and_info_extraction() {
    let store = yes24(true);
    let html = r#"
    <html>
      <ul id="yesSchList">
        <li><div class="itemUnit"><div class="item_info"><div class="info_row info_name">
          <a class="gd_name" href="/product/goods/999">Book</a>
        </div></div></div></li>
      </ul>
      <h2 class="gd_name">Yes Title</h2>
      <span class="gd_auth"><a>Yes Author</a></span>
      <div>관련분류 <a href="/product/category/display/1">A</a> <a href="/product/category/display/2">B</a></div>
      <div>ISBN13 1234567890123</div>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://www.yes24.com/product/goods/999"]);
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Yes Title");
    assert_eq!(info["author"], "Yes Author");
    assert_eq!(info["category"], "A > B");
    assert_eq!(info["isbn"], "1234567890123");
    assert!(store.build_search_url("hello").contains("query=hello"));
    assert!(store.build_isbn_search_url("999").contains("query=999"));
}

#[test]
fn test_yes24_author_text_and_isbn10() {
    let store = yes24(false);
    let html = r#"
    <html>
      <h2 class="gd_name">Yes Title</h2>
      <span class="gd_auth">Yes Author</span>
      <div>ISBN10 1234567890</div>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "Yes Author");
    assert_eq!(info["isbn"], "1234567890");
}

#[test]
fn test_yes24_category_and_isbn_exception_paths() {
    let store = yes24(false);

    assert_eq!(store._extract_yes24_category_soup(&BadSoup), "");
    assert_eq!(store._extract_yes24_isbn_soup(&BadSoup), "");
}

#[test]
fn test_aladin_extract_links_and_isbn_paths() {
    let store = aladin(true);
    let html = r#"
    <html>
      <div id="Search3_Result">
        <a href="/shop/wproduct.aspx?ItemId=1">A</a>
        <a href="/shop/wproduct.aspx?ItemId=1">A</a>
        <a href="/shop/wproduct.aspx?ItemId=2_CommentReview">R</a>
        <a href="/shop/wproduct.aspx?ItemId=3">B</a>
      </div>
      <div id="Ere_prod_allwrap">
        <div class="Ere_prod_mconts_R">
          <div class="conts_info_list1">
            <ul><li>ISBN 9781234567890</li></ul>
          </div>
        </div>
      </div>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=1", "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=3"]);
    assert_eq!(store._extract_aladin_isbn(html), "9781234567890");
}

#[test]
fn test_aladin_strip_trailing_number_and_search_order() {
    let mut store = aladin(false);
    assert_eq!(store._strip_trailing_number("마왕의 딸 3"), "마왕의 딸");

    store = store.with_fetch_search_results(Box::new(|url| vec![row("T", "A", "C", url, url, "")]));
    let (results, keyword, method) = store.search("", "마왕의 딸 3", "작가");
    assert_eq!(method, "title_author");
    assert_eq!(keyword, "작가 마왕의 딸");
    assert!(!results.is_empty());
}

#[test]
fn test_ridibooks_extract_fallbacks_meta_and_author_link() {
    let store = ridibooks(false);
    let html = r#"
    <html>
      <meta name="title" content="Meta Title" />
      <a href="/author/1">Link Author</a>
      <meta name="description" content="저자: Desc Author, 기타" />
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Meta Title");
    assert_eq!(info["author"], "Link Author");
}

#[test]
fn test_ridibooks_header_typo_and_og_author() {
    let store = ridibooks(false);
    let html = r#"
    <html>
      <div id="iSLANDS__Header"><ul><li><li>저자 작가</li></li></ul></div>
      <meta property="og:author" content="OG Author" />
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "작가");
}

#[test]
fn test_naver_shopping_extract_fallback_author_and_title() {
    let store = naver_shopping(false);
    let html = r#"
    <html>
      <div class="bookTitle_book_name__x">Title</div>
      <div class="bookTitle_info_content__y">Author</div>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Title");
    assert_eq!(info["author"], "Author");
}

#[test]
fn test_munpia_meta_description_author() {
    let store = munpia(false);
    let html = r#"
    <html>
      <meta property="og:title" content="T" />
      <meta property="og:description" content="홍길동 - 소개" />
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "홍길동");
}

#[test]
fn test_aladin_isbn_fallback_and_exception() {
    let store = aladin(false);
    let html = r#"
    <html>
      <div id="Ere_prod_allwrap">
        <div class="Ere_prod_middlewrap">
          <div class="Ere_prod_mconts_R">
            <ul><li>기타 ISBN 1234567890123</li></ul>
          </div>
        </div>
      </div>
    </html>
    "#;
    assert_eq!(store._extract_aladin_isbn(html), "1234567890123");

    assert_eq!(store._extract_aladin_isbn_soup(&BadSoup), "");
}

#[test]
fn test_ridibooks_search_empty_and_exception() {
    let mut store = ridibooks(true);

    store = store.with_session(constant(200, "", json!({"books": []})));
    assert!(store.search_by_keyword("k").is_empty());

    store = store.with_session(failing());
    assert!(store.search_by_keyword("k").is_empty());
}

#[test]
fn test_ridibooks_search_success_and_extract() {
    let mut store = ridibooks(true);

    store = store.with_session(constant(
        200,
        "",
        json!({
            "books": [
                {"b_id": "1", "title": "T1", "author": "A1", "parent_category_name": "P1", "category_name": "C1", "parent_category_name2": "P2", "category_name2": "C2"},
                {"b_id": "2", "title": "T2", "author": "A2", "parent_category_name": "", "category_name": "C3", "parent_category_name2": "", "category_name2": "C4"},
            ]
        }),
    ));
    let results = store.search_by_keyword("keyword");
    let search_url = store.build_search_url("keyword");
    assert_eq!(results, vec![row("T1", "A1", "P1 > C1 || P2 > C2", "https://ridibooks.com/books/1", &search_url, ""), row("T2", "A2", "C3 || C4", "https://ridibooks.com/books/2", &search_url, ""),]);

    let html = r#"
    <html>
      <meta property="og:title" content="R Title" />
      <div id="ISLANDS__Header"><ul><li><li>저자 홍길동</li></li></ul></div>
      <section id="books_contents"><section class="detail_body"><ul>
        <li><a href="/category/1">A</a><a href="/category/2">B</a></li>
      </ul></section></section>
      <div>ISBN</div><div>1234567890123</div>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "R Title");
    assert_eq!(info["author"], "홍길동");
    assert_eq!(info["category"], "A > B");
    assert_eq!(info["isbn"], "1234567890123");
}

#[test]
fn test_ridibooks_isbn_exception() {
    let store = ridibooks(false);

    assert_eq!(store._extract_ridi_isbn_soup(&BadSoup), "");
}

#[test]
fn test_naver_shopping_links_and_extract() {
    let store = naver_shopping(true);
    let data =
        json!({"props": {"pageProps": {"dehydratedState": {"queries": [{"queryKey": ["SearchAll"], "state": {"data": {"SearchAll": {"bookSasResult": {"itemList": [{"id": "1"}, {"id": "2"}]}}}}}]}}}});
    let html = format!("<html><script>{data}</script></html>");
    let links = store.extract_search_links(&html);
    assert_eq!(links, ["https://search.shopping.naver.com/book/catalog/1", "https://search.shopping.naver.com/book/catalog/2"]);

    let detail_html = r#"
    <html>
      <h2 class="bookTitle_book_name__abc">N Title</h2>
      <div class="bookTitle_info_title__x">저자</div>
      <div class="bookTitle_info_content__y">N Author</div>
      <div class="bookCatalogTop_breadcrumb__z">A > B</div>
    </html>
    "#;
    let info = store.extract_book_info(detail_html);
    assert_eq!(info["title"], "N Title");
    assert_eq!(info["author"], "N Author");
    assert_eq!(info["category"], "A > B");
}

#[test]
fn test_naver_shopping_items_not_list() {
    let store = naver_shopping(false);
    let data = json!({"props": {"pageProps": {"dehydratedState": {"queries": [{"queryKey": ["SearchAll"], "state": {"data": {"SearchAll": {"bookSasResult": {"itemList": {}}}}}}]}}}});
    let html = format!("<html><script>{data}</script></html>");
    assert!(store.extract_search_links(&html).is_empty());
}

#[test]
fn test_munpia_links_and_meta_desc() {
    let store = munpia(true);
    let html = r#"
    <html>
      <div id="SEARCH-BOX" class="section2">
        <div class="ebook_lists">
          <div class="article_wrap">
            <div class="article">
              <dl class="detail"><dt><a href="/view/1">A</a></dt></dl>
              <dl class="detail"><dt><a>Missing</a></dt></dl>
              <dl class="detail"><dt><a href="https://novel.munpia.com/view/2">B</a></dt></dl>
            </div>
          </div>
        </div>
      </div>
      <meta property="og:description" content="홍길동 - 설명" />
      <p class="meta-path">A > B</p>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://novel.munpia.com/view/1", "https://novel.munpia.com/view/2"]);
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "홍길동");
    assert_eq!(info["category"], "A > B");
    assert!(store.build_search_url("a b").ends_with("a%20b/order/search_result"));
}

#[test]
fn test_naver_series_links_and_authors() {
    let store = naver_series(true);
    let html = r#"
    <html>
      <title>Series Title</title>
      <ul class="lst_list">
        <li><a class="N=a:nov.title" href="/novel/1">A</a></li>
        <li><a class="N=a:com.title" href="/comic/2">B</a></li>
      </ul>
      <div id="_otherProductByPerson"><strong>작가</strong><strong>홍길동</strong></div>
      <div id="content"><ul class="end_info"><li class="info_lst"><ul><li><span><a>장르</a></span></li></ul></li></ul></div>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://series.naver.com/novel/1", "https://series.naver.com/comic/2"]);
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Series Title");
    assert_eq!(info["author"], "홍길동");
    assert_eq!(info["category"], "장르");
    assert!(store.build_search_url("hello").ends_with("q=hello"));
}

#[test]
fn test_naver_series_meta_og_fallback() {
    let store = naver_series(false);
    let html = r#"
    <html>
      <meta property="og:description" content="작가: 김작가」" />
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "김작가");
}

// ---- coverage: uncovered lines ----

/// Line 238: hierarchical selector가 실패했을 때 verbose 대체 경로 로그
#[test]
fn test_yes24_extract_search_links_fallback_verbose() {
    let store = yes24(true);
    let html = r#"<html><a class="gd_name" href="/product/goods/12345">Book</a></html>"#;
    let links = store.extract_search_links(html);
    assert_eq!(links.len(), 1);
    assert!(links[0].contains("/product/goods/12345"));
}

/// Lines 287-288: 책 정보를 뽑다가 예외가 나도 부분 결과를 돌려준다
#[test]
fn test_yes24_extract_book_info_exception() {
    let store = yes24(false);
    let soup = RaisingSoup { html: "<html><h2 class='gd_name'>Title</h2></html>", raise_on: "span" };

    let info = store.extract_book_info_soup(&soup);
    // Should not raise, returns partial info
    assert!(info.is_object());
}

/// Line 383: AladinBookstore.build_isbn_search_url
#[test]
fn test_aladin_build_isbn_search_url() {
    let store = aladin(false);
    let url = store.build_isbn_search_url("9781234567890");
    assert!(url.contains("9781234567890"));
    assert!(url.contains("SearchWord"));
}

/// Line 394: len(links) >= MAX_RESULTS 이면 멈춘다
#[test]
fn test_aladin_extract_search_links_break_at_max() {
    let store = aladin(false);
    // 결과가 많은 HTML을 만든다
    let items: String = (0..5).map(|i| format!(r#"<a href="/shop/wproduct.aspx?ItemId={i}">Book {i}</a>"#)).collect();
    let html = format!(r#"<html><div id="Search3_Result">{items}</div></html>"#);
    let links = store.extract_search_links(&html);
    assert!(links.len() <= store.max_results);
}

/// Line 442: Aladin ISBN 추출
#[test]
fn test_aladin_extract_isbn() {
    let store = aladin(false);
    let html = r#"<html>
    <title>Title | Author | Aladin</title>
    <div id="Ere_prod_allwrap">
      <div class="Ere_prod_mconts_R">
        <div class="conts_info_list1">
          <ul><li>ISBN13 : 9781234567890</li></ul>
        </div>
      </div>
    </div>
    </html>"#;
    let info = store.extract_book_info(html);
    assert_eq!(info["isbn"], "9781234567890");
}

/// Line 516: verbose 상태에서 RIDI API 응답 실패
#[test]
fn test_ridibooks_api_failure_verbose() {
    let mut store = ridibooks(true);

    store = store.with_session(constant(500, "error", Value::Null));
    let results = store.search_by_keyword("test");
    assert!(results.is_empty());
}

/// Line 564: RidibooksBookstore.extract_search_links는 빈 목록을 돌려준다
#[test]
fn test_ridibooks_extract_search_links() {
    let store = ridibooks(false);
    let html = "<html><body>nothing</body></html>";
    let links = store.extract_search_links(html);
    assert!(links.is_empty());
}

/// Lines 600-602: Ridibooks meta author 대체 경로
#[test]
fn test_ridibooks_author_meta_fallback() {
    let store = ridibooks(false);
    let html = r#"<html>
    <meta property="og:title" content="My Book"/>
    <meta name="author" content="Meta Author"/>
    </html>"#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "Meta Author");
}

/// Lines 605-610: Ridibooks meta description 에서 저자 추출
#[test]
fn test_ridibooks_author_description_fallback() {
    let store = ridibooks(false);
    let html = r#"<html>
    <meta property="og:title" content="My Book"/>
    <meta property="og:description" content="저자: 설명작가, 기타정보"/>
    </html>"#;
    let info = store.extract_book_info(html);
    assert_eq!(info["author"], "설명작가");
}

/// Lines 631-632: NaverShoppingBookstore.build_search_url
#[test]
fn test_naver_shopping_build_search_url() {
    let store = naver_shopping(false);
    let url = store.build_search_url("테스트 검색");
    assert!(url.contains("search.shopping.naver.com"));
    assert!(url.contains("query="));
}

/// Line 646: verbose 상태에서 NaverShopping JSON script 를 못 찾음
#[test]
fn test_naver_shopping_json_not_found_verbose() {
    let store = naver_shopping(true);
    let html = "<html><body>no scripts here</body></html>";
    let links = store.extract_search_links(html);
    assert!(links.is_empty());
}

/// Lines 651-654: verbose 상태에서 NaverShopping JSON 파싱 오류
#[test]
fn test_naver_shopping_json_parse_error_verbose() {
    let store = naver_shopping(true);
    let html = r#"<html><script>{invalid json "SearchAll"</script></html>"#;
    let links = store.extract_search_links(html);
    assert!(links.is_empty());
}

/// Lines 708-709: NaverShopping extract_book_info 예외
#[test]
fn test_naver_shopping_extract_book_info_exception() {
    let store = naver_shopping(false);
    let soup = RaisingSoup { html: "<html><body>normal</body></html>", raise_on: "h2" };

    let info = store.extract_book_info_soup(&soup);
    assert!(info.is_object());
}

// ---- merged from test_bookstore_unit.py ----

#[test]
fn test_truncate_title() {
    assert_eq!(_truncate_title("A - B"), "A");
    assert_eq!(_truncate_title("A－B"), "A");
    assert_eq!(_truncate_title("A: B"), "A: B");
    assert_eq!(_truncate_title("A：B"), "A");
}

#[test]
fn test_search_prefers_isbn_when_supported() {
    let mut bs = Hooked::new(Kind::Unit);
    bs.supports_isbn_search = true;
    let called: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&called);
    bs.fetch = Some(Box::new(move |url| {
        sink.lock().unwrap().push(url.to_owned());
        vec![row("t", "a", "c", "d", "s", "i")]
    }));

    let (results, keyword, method) = bs.search("123", "t", "a");
    assert!(!results.is_empty());
    assert_eq!(keyword, "123");
    assert_eq!(method, "isbn");
    let called = called.lock().unwrap();
    assert!(!called.is_empty() && called[0].contains("123"));
}

#[test]
fn test_search_title_author_order() {
    let mut bs = Hooked::new(Kind::Unit);
    bs.supports_isbn_search = false;
    let called: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&called);
    bs.fetch = Some(Box::new(move |url| {
        sink.lock().unwrap().push(url.to_owned());
        vec![row("t", "a", "c", "d", "s", "i")]
    }));

    let (results, keyword, method) = bs.search("", "T", "A");
    assert!(!results.is_empty());
    assert_eq!(keyword, "T A");
    assert_eq!(method, "title_author");

    bs.author_first_search = true;
    let (_results, keyword, method) = bs.search("", "T", "A");
    assert_eq!(keyword, "A T");
    assert_eq!(method, "title_author");
}

#[test]
fn test_search_fallbacks() {
    let mut bs = Hooked::new(Kind::Unit);
    bs.fetch = Some(Box::new(|_| vec![]));

    let (results, keyword, method) = bs.search("", "T", "A");
    assert!(results.is_empty());
    assert_eq!(keyword, "T A");
    assert_eq!(method, "title_author");

    let (results, keyword, method) = bs.search("", "T", "");
    assert!(results.is_empty());
    assert_eq!(keyword, "T");
    assert_eq!(method, "title");

    bs.supports_isbn_search = true;
    let (results, keyword, method) = bs.search("123", "", "");
    assert!(results.is_empty());
    assert_eq!(keyword, "123");
    assert_eq!(method, "isbn");
}

#[test]
fn test_fetch_search_results_uses_cached_html() {
    let mut bs = Cached::new(Kind::Unit);
    bs = bs.with_session(constant(200, "<html></html>", Value::Null));
    bs.load = Some(Box::new(|_| Some("<html></html>".to_owned())));

    let results = bs._fetch_search_results(&bs.build_search_url("k"));
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].2, "A > B > C");
}

#[test]
fn test_save_and_load_html_from_tmp() {
    let bs = Plain::new(Kind::Unit);
    let url = "http://example.com/detail/123";
    let html = "<html>cached</html>";
    bs._save_html_to_tmp(html, url);
    let loaded = bs._load_html_from_tmp(url);
    assert_eq!(loaded.as_deref(), Some(html));
}

// ---- merged from test_bookstore_unit.py (above) ----

// ---- merged from test_bookstore_parsing.py ----

#[test]
fn test_truncate_title_separators() {
    assert_eq!(_truncate_title("A - B"), "A");
    assert_eq!(_truncate_title("A－B"), "A");
    assert_eq!(_truncate_title("A：B"), "A");
    assert_eq!(_truncate_title("A-B"), "A");
}

#[test]
fn test_search_prefers_isbn_then_title_author_then_title() {
    let mut store = Hooked::new(Kind::Parsing);
    store.isbn = Some(Box::new(|isbn| vec![row("t", "a", "", "", "", isbn)]));
    let (results, keyword, method) = store.search("123", "Title", "Auth");
    assert!(!results.is_empty());
    assert_eq!(keyword, "123");
    assert_eq!(method, "isbn");

    let mut store2 = Hooked::new(Kind::Parsing);
    store2.supports_isbn_search = false;
    store2.keyword = Some(Box::new(|_| vec![row("t", "a", "", "", "", "")]));
    let (results, keyword, method) = store2.search("", "Title", "Auth");
    assert!(!results.is_empty());
    assert_eq!(method, "title_author");
    assert_eq!(keyword, "Title Auth");

    let mut store3 = Hooked::new(Kind::Parsing);
    store3.supports_isbn_search = false;
    store3.keyword = Some(Box::new(|_| vec![row("t", "a", "", "", "", "")]));
    let (results, keyword, method) = store3.search("", "Title", "");
    assert!(!results.is_empty());
    assert_eq!(method, "title");
    assert_eq!(keyword, "Title");

    let mut store4 = Hooked::new(Kind::Parsing);
    store4.supports_isbn_search = false;
    store4.keyword = Some(Box::new(|_| vec![]));
    let (results, keyword, method) = store4.search("", "Title", "Auth");
    assert!(results.is_empty());
    assert_eq!(method, "title_author");
    assert_eq!(keyword, "Title Auth");
}

#[test]
fn test_fetch_search_results_uses_cached_html_parsing() {
    let mut store = Cached::new(Kind::Parsing);

    let search_html = "<html><a href='https://example.com/detail/1'>x</a></html>";
    let detail_html = "<html><h1>Book</h1><span class='author'>Auth</span><div class='cat'>A > B > C > D</div><div class='isbn'>123</div></html>";

    store = store.with_session(constant(200, search_html, Value::Null));
    store.load = Some(Box::new(move |_| Some(detail_html.to_owned())));

    let results = store._fetch_search_results("https://example.com/search?q=x");
    assert_eq!(results, vec![row("Book", "Auth", "A > B > C", "https://example.com/detail/1", "https://example.com/search?q=x", "123")]);
}

#[test]
fn test_save_and_load_html_cache_parsing() {
    let tmp = TempDir::new().unwrap();
    let mut store = Plain::new(Kind::Parsing);
    store.tmp_dir = Some(tmp.path().to_path_buf());
    let url = "https://example.com/detail/1";
    let html = "<html>cached</html>";
    store._save_html_to_tmp(html, url);
    let loaded = store._load_html_from_tmp(url);
    assert_eq!(loaded.as_deref(), Some(html));
}

#[test]
fn test_yes24_extract_search_links_and_category_and_isbn() {
    let store = yes24(false);
    let html = r#"
    <html>
      <ul id="yesSchList">
        <li><div class="itemUnit"><div class="item_info"><div class="info_row info_name">
          <a class="gd_name" href="/product/goods/123">A</a>
        </div></div></div></li>
      </ul>
      <div>관련분류
        <a href="/product/category/display/1">국내도서</a>
        <a href="/product/category/display/2">소설</a>
      </div>
      ISBN13 9781234567890
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://www.yes24.com/product/goods/123"]);
    assert_eq!(store._extract_yes24_category(html), "국내도서 > 소설");
    assert_eq!(store._extract_yes24_isbn(html), "9781234567890");
}

#[test]
fn test_yes24_extract_book_info_author_fallback() {
    let store = yes24(false);
    let html = "<html><h2 class='gd_name'>Title</h2><span class='gd_auth'>Auth</span></html>";
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Title");
    assert_eq!(info["author"], "Auth");
}

#[test]
fn test_aladin_strip_trailing_number_and_search_author_first() {
    let mut store = aladin(false);
    let captured: Arc<Mutex<String>> = Arc::default();
    let sink = Arc::clone(&captured);

    store = store.with_search_by_keyword(Box::new(move |keyword| {
        *sink.lock().unwrap() = keyword.to_owned();
        vec![row("t", "a", "", "", "", "")]
    }));
    let (results, keyword, method) = store.search("", "마왕의 딸 3", "홍길동");
    assert!(!results.is_empty());
    assert_eq!(method, "title_author");
    assert_eq!(keyword, "홍길동 마왕의 딸");
    assert_eq!(*captured.lock().unwrap(), "홍길동 마왕의 딸");
}

#[test]
fn test_aladin_extract_search_links_and_info() {
    let store = aladin(false);
    let html = r#"
    <html>
      <div id="Search3_Result">
        <a href="/shop/wproduct.aspx?ItemId=111">A</a>
        <a href="/shop/wproduct.aspx?ItemId=111">A2</a>
        <a href="/shop/wproduct.aspx?ItemId=222_CommentReview">Review</a>
        <a href="/shop/wproduct.aspx?ItemId=222">B</a>
      </div>
      <title>Title | Author | Aladin</title>
      <meta name="author" content="MetaAuthor" />
      <meta property="og:title" content="MetaTitle" />
      <ul id="ulCategory"><a>국내도서</a><a>소설</a></ul>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=111", "https://www.aladin.co.kr/shop/wproduct.aspx?ItemId=222"]);
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "MetaTitle");
    assert_eq!(info["author"], "MetaAuthor");
    assert_eq!(info["category"], "국내도서 > 소설");
}

#[test]
fn test_aladin_extract_isbn_fallback() {
    let store = aladin(false);
    let html = r#"
    <html>
      <div id="Ere_prod_allwrap">
        <div class="Ere_prod_middlewrap">
          <div class="Ere_prod_mconts_R">
            <ul><li>기타</li><li>ISBN 9781234567890</li></ul>
          </div>
        </div>
      </div>
    </html>
    "#;
    assert_eq!(store._extract_aladin_isbn(html), "9781234567890");
}

#[test]
fn test_ridibooks_search_by_keyword_and_extract_info() {
    let mut store = ridibooks(false);

    store = store.with_session(constant(
        200,
        "",
        json!({"books": [{"b_id": "1", "title": "T", "author": "A", "parent_category_name": "P", "category_name": "C", "parent_category_name2": "P2", "category_name2": "C2"}]}),
    ));
    let results = store.search_by_keyword("키워드");
    assert_eq!(results, vec![row("T", "A", "P > C || P2 > C2", "https://ridibooks.com/books/1", "https://ridibooks.com/search?q=%ED%82%A4%EC%9B%8C%EB%93%9C&adult_exclude=n", "")]);

    let html = r#"
    <html>
      <meta property="og:title" content="Title" />
      <div id="ISLANDS__Header"><ul><li><li>홍길동저자</li></li></ul></div>
      <div id="books_contents">
        <section class="detail_body"><ul><li><a href="/category/1">A</a><a href="/category/2">B</a></li></ul></section>
      </div>
      <div>ISBN</div><div>9781234567890</div>
    </html>
    "#;
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Title");
    assert_eq!(info["author"], "홍길동");
    assert_eq!(info["category"], "A > B");
    assert_eq!(info["isbn"], "9781234567890");
}

#[test]
fn test_naver_shopping_links_and_info() {
    let store = naver_shopping(false);
    let data =
        json!({"props": {"pageProps": {"dehydratedState": {"queries": [{"queryKey": ["SearchAll", "book"], "state": {"data": {"SearchAll": {"bookSasResult": {"itemList": [{"id": "123"}]}}}}}]}}}});
    let html = format!("<html><script>{data}</script></html>");
    let links = store.extract_search_links(&html);
    assert_eq!(links, ["https://search.shopping.naver.com/book/catalog/123"]);

    let detail_html = r#"
    <html>
      <h2 class="bookTitle_book_name__abc">Title</h2>
      <div class="bookTitle_info_title__1">저자</div>
      <div class="bookTitle_info_content__1">Author</div>
      <div class="bookCatalogTop_breadcrumb__1">국내도서 > 소설</div>
    </html>
    "#;
    let info = store.extract_book_info(detail_html);
    assert_eq!(info["title"], "Title");
    assert_eq!(info["author"], "Author");
    assert_eq!(info["category"], "국내도서 > 소설");
}

#[test]
fn test_munpia_extract_links_and_info() {
    let store = munpia(false);
    let html = r#"
    <html>
      <div id="SEARCH-BOX" class="section2">
        <div class="ebook_lists">
          <div class="article_wrap">
            <div class="article">
              <dl class="detail"><dt><a href="/book/1">A</a></dt></dl>
            </div>
          </div>
        </div>
      </div>
      <meta property="og:title" content="Title" />
      <meta property="og:description" content="Author - Desc" />
      <p class="meta-path">장르 > 판타지</p>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://novel.munpia.com/book/1"]);
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Title");
    assert_eq!(info["author"], "Author");
    assert_eq!(info["category"], "장르 > 판타지");
}

#[test]
fn test_naver_series_links_and_info() {
    let store = naver_series(false);
    let html = r#"
    <html>
      <ul class="lst_list"><li><a class="N=a:foo" href="/detail/1">A</a></li></ul>
      <div id="_otherProductByPerson"><strong>dummy</strong><strong>작가명</strong></div>
      <div id="content"><ul class="end_info"><li class="info_lst"><ul><li><span><a>장르</a></span></li></ul></li></ul></div>
      <title>Title</title>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links, ["https://series.naver.com/detail/1"]);
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "Title");
    assert_eq!(info["author"], "작가명");
    assert_eq!(info["category"], "장르");
}

#[test]
fn test_kyobo_bookstore_links_and_info() {
    let store = kyobo(false);
    let html = r#"
    <html>
      <ul class="prod_list">
        <li class="prod_item">
          <a class="prod_info" href="/detail/S000001234">상세링크</a>
          <span class="prod_name">교보 도서 제목</span>
          <span class="author rep">교보 작가</span>
        </li>
      </ul>
    </html>
    "#;
    let links = store.extract_search_links(html);
    assert_eq!(links.len(), 1);
    assert!(links[0].contains("detail/S000001234"));

    let detail_html = r#"
    <html>
      <h1 class="prod_title">교보 도서 제목</h1>
      <span class="author rep">교보 작가</span>
      <ol class="breadcrumb_list">
        <li><a>홈</a></li>
        <li><a>국내도서</a></li>
        <li><a>소설</a></li>
        <li><a>한국소설</a></li>
      </ol>
      <dl>
        <dt>ISBN</dt>
        <dd>9788934900011</dd>
      </dl>
    </html>
    "#;
    let info = store.extract_book_info(detail_html);
    assert_eq!(info["title"], "교보 도서 제목");
    assert_eq!(info["author"], "교보 작가");
    assert_eq!(info["category"], "소설 > 한국소설");
    assert_eq!(info["isbn"], "9788934900011");
}

/// `shutil.disk_usage`와 `open` 대역. 쓰기 시도 횟수를 센다. 쓰는 메서드만 구현한다(`FsOps`의 나머지는 기본 구현).
/// 실제 디스크에 쓰지 않으려고 `common::faulty_fs::FaultyFs` 대신 로컬로 둔다.
// TODO(common): 쓰기를 삼키고 횟수만 세는 `FsOps` 대역(`write` 호출 수)이 `common/faulty_fs.rs`에 없다. 생기면 `FaultyFs`로 옮긴다.
struct ProbeFs {
    free: Result<u64, String>,
    write_error: Option<fn() -> io::Error>,
    writes: Arc<AtomicUsize>,
}

impl FsOps for ProbeFs {
    fn disk_free(&self, _dir: &Path) -> io::Result<u64> {
        self.free.clone().map_err(io::Error::other)
    }

    fn write(&self, _path: &Path, _data: &[u8]) -> io::Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        match self.write_error {
            Some(make) => Err(make()),
            None => Ok(()),
        }
    }
}

fn probe(free: Result<u64, String>, write_error: Option<fn() -> io::Error>) -> (Arc<dyn FsOps>, Arc<AtomicUsize>) {
    let writes = Arc::new(AtomicUsize::new(0));
    (Arc::new(ProbeFs { free, write_error, writes: Arc::clone(&writes) }), writes)
}

#[test]
fn test_save_html_to_tmp_disk_and_exception_branches() {
    // 202: free < 500MB -> 저장 건너뜀
    let (fs_low, writes) = probe(Ok(100 * 1024 * 1024), None);
    let store = yes24(true).with_fs(fs_low);
    store._save_html_to_tmp("<html></html>", "https://example.com/1");
    assert_eq!(writes.load(Ordering::SeqCst), 0, "여유 공간이 500MB 미만이면 쓰지 않는다");

    // 203-204: disk_usage 예외 발생 시 pass
    let (fs_check_fails, writes) = probe(Err("Disk check failed".to_owned()), None);
    let store = yes24(true).with_fs(fs_check_fails);
    store._save_html_to_tmp("<html></html>", "https://example.com/2");
    assert_eq!(writes.load(Ordering::SeqCst), 1, "여유 공간을 못 재도 쓰기는 시도한다");

    // 214-215: OSError(errno=28, No space left on device)
    let (fs_full, _) = probe(Ok(u64::MAX), Some(|| io::Error::from_raw_os_error(28)));
    let store = yes24(true).with_fs(fs_full);
    store._save_html_to_tmp("<html></html>", "https://example.com/3");

    // 218-219: 기타 Exception
    let (fs_other, _) = probe(Ok(u64::MAX), Some(|| io::Error::other("Unexpected")));
    let store = yes24(true).with_fs(fs_other);
    store._save_html_to_tmp("<html></html>", "https://example.com/4");
}

#[test]
fn test_kyobo_urls_and_fallback_links_and_title() {
    let store = kyobo(true);

    // 867-868: build_search_url
    let url = store.build_search_url("삼국지");
    assert!(url.contains("keyword=%EC%82%BC%EA%B5%AD%EC%A7%80"));

    // 871: build_isbn_search_url
    let isbn_url = store.build_isbn_search_url("9788934900011");
    assert!(isbn_url.contains("keyword=9788934900011"));

    // 886-894: extract_search_links fallback to a[href*="/detail/"] with verbose=True
    let html_fallback = r#"
    <html>
      <div>
        <a href="https://product.kyobobook.co.kr/detail/S00001">상세1</a>
        <a href="https://ebook-product.kyobobook.co.kr/detail/E00002">상세2</a>
      </div>
    </html>
    "#;
    let links = store.extract_search_links(html_fallback);
    assert_eq!(links.len(), 2);

    // 903-904: extract_book_info title from soup.title.string
    let html_title_only = r#"
    <html>
      <head>
        <title>교보문고 상세 도서 | 교보문고</title>
      </head>
      <body>
        <span class="author rep">김작가</span>
      </body>
    </html>
    "#;
    let info = store.extract_book_info(html_title_only);
    assert_eq!(info["title"], "교보문고 상세 도서");
    assert_eq!(info["author"], "김작가");
}

/// 신형 종이책 상세(Next.js)에서 제목과 카테고리를 뽑는다.
///
/// 교보 종이책 상세가 Next.js 로 바뀌면서 prod_title/breadcrumb_list 같은 시맨틱
/// 클래스가 사라졌다. 옛 셀렉터만 두면 카테고리가 빈 문자열이 되고, 자동분류에서
/// 교보가 표를 못 던져 다수결이 사실상 2곳으로 줄어든다.
#[test]
fn test_kyobo_nextjs_detail_page_category_and_title() {
    let store = kyobo(false);
    let detail_html = r#"
    <html>
      <head><title>채식주의자 - 교보문고</title></head>
      <body>
        <h1 class="text-xl font-bold">채식주의자</h1>
        <div class="flex items-center gap-1">
          <a href="https://store.kyobobook.co.kr/category/domestic">국내도서</a>
          <a href="https://store.kyobobook.co.kr/category/domestic/01">소설</a>
          <a href="https://store.kyobobook.co.kr/category/domestic/0101">한국소설</a>
          <a href="https://store.kyobobook.co.kr/category/domestic/010101">한국소설일반</a>
        </div>
        <span>ISBN</span>
      </body>
    </html>
    "#;
    let info = store.extract_book_info(detail_html);
    assert_eq!(info["title"], "채식주의자");
    // "국내도서"는 모든 책에 붙어 변별력이 없어 걸러낸다.
    assert_eq!(info["category"], "소설 > 한국소설 > 한국소설일반");
}

/// `<title>` fallback 은 서점 이름만 떼고 제목 속 하이픈은 남긴다.
#[test]
fn test_kyobo_title_fallback_strips_only_store_suffix() {
    let store = kyobo(false);
    let html = "<html><head><title>데미안 - 소년의 기록 - 교보문고</title></head><body></body></html>";
    let info = store.extract_book_info(html);
    assert_eq!(info["title"], "데미안 - 소년의 기록");
}

/// 조아라는 제목(target=subject)만 검색한다. 저자를 붙이면 늘 0건이다.
#[test]
fn test_joara_search_url_and_author_is_ignored() {
    let mut store = joara(false);
    let url = store.build_search_url("데미안");
    assert!(url.starts_with("https://www.joara.com/search?target=subject&word="));
    assert!(url.contains("%EB%8D%B0%EB%AF%B8%EC%95%88"));

    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let search_url = url.clone();
    store = store.with_search_by_keyword(Box::new(move |keyword| {
        sink.lock().unwrap().push(keyword.to_owned());
        vec![row("데미안", "세리엔II", "퓨전", "https://www.joara.com/book/45402", &search_url, "")]
    }));
    // ISBN 과 저자를 넘겨도 제목만으로 한 번 부른다.
    let (results, keyword, method) = store.search("9788934900011", "데미안", "헤르만 헤세");
    assert_eq!(*seen.lock().unwrap(), ["데미안"]);
    assert_eq!(keyword, "데미안");
    assert_eq!(method, "title");
    assert_eq!(results[0].0, "데미안");
}

#[test]
fn test_joara_search_by_keyword_success() {
    let mut store = joara(true);

    store = store.with_session(constant(
        200,
        "",
        json!({
            "status": 1,
            "total_cnt": 2,
            "data": {
                "list": [
                    {"book_code": 45402, "subject": "데미안", "member_name": "세리엔II", "category_name": "퓨전"},
                    {"book_code": 10546, "subject": "데미안", "member_name": "Demian", "category_name": "판타지"},
                ]
            },
        }),
    ));
    let search_url = store.build_search_url("데미안");
    assert_eq!(
        store.search_by_keyword("데미안"),
        vec![row("데미안", "세리엔II", "퓨전", "https://www.joara.com/book/45402", &search_url, ""), row("데미안", "Demian", "판타지", "https://www.joara.com/book/10546", &search_url, ""),]
    );
}

/// 조아라는 파라미터가 빠져도 HTTP 200 에 status=0 을 준다. 본문을 봐야 실패를 안다.
#[test]
fn test_joara_search_by_keyword_failure_paths() {
    let mut store = joara(true);

    store = store.with_session(constant(200, "", json!({"status": 0, "message": "필수 파라메타값이 정의되지 않았습니다.[\"deviceuid\"]"})));
    assert!(store.search_by_keyword("데미안").is_empty());

    store = store.with_session(constant(200, "", json!({"status": 1, "data": {"list": []}})));
    assert!(store.search_by_keyword("데미안").is_empty());

    store = store.with_session(constant(500, "", json!({})));
    assert!(store.search_by_keyword("데미안").is_empty());

    store = store.with_session(failing());
    assert!(store.search_by_keyword("데미안").is_empty());
}

/// 조아라는 API 를 직접 부르므로 HTML 훅은 호출되지 않는다.
#[test]
fn test_joara_unused_html_hooks_are_inert() {
    let store = joara(false);
    let soup = "<html><body><a href='/book/1'>x</a></body></html>";
    assert!(store.extract_search_links(soup).is_empty());
    assert_eq!(store.extract_book_info(soup), empty_info());
}

/// 네이버 웹툰은 '제목 저자' 키워드가 늘 0건이다. 제목만 한 번 부른다.
#[test]
fn test_naver_webtoon_search_url_and_author_is_ignored() {
    let mut store = naver_webtoon(false);
    let url = store.build_search_url("유미의 세포들");
    assert!(url.starts_with("https://comic.naver.com/search?keyword="));
    assert!(url.contains("%EC%9C%A0%EB%AF%B8"));

    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let search_url = url.clone();
    store = store.with_search_by_keyword(Box::new(move |keyword| {
        sink.lock().unwrap().push(keyword.to_owned());
        vec![row("유미의 세포들", "이동건", "로맨스", "https://comic.naver.com/webtoon/list?titleId=651673", &search_url, "")]
    }));
    let (results, keyword, method) = store.search("9788934900011", "유미의 세포들", "이동건");
    assert_eq!(*seen.lock().unwrap(), ["유미의 세포들"]);
    assert_eq!(keyword, "유미의 세포들");
    assert_eq!(method, "title");
    assert_eq!(results[0].0, "유미의 세포들");
}

#[test]
fn test_naver_webtoon_search_by_keyword_success() {
    let mut store = naver_webtoon(true);
    store.max_results = 2; // 잘림 동작만 본다. 기본값은 test_bookstore_default_max_results_is_four

    let (fake, calls) = session(|_, _| {
        http(
            200,
            "",
            json!({
                "searchList": [
                    {"titleId": 651673, "titleName": "유미의 세포들", "displayAuthor": "이동건", "genreList": [{"type": "PURE", "description": "로맨스"}]},
                    {"titleId": 778153, "titleName": "유미의 세포들 외전", "displayAuthor": "이동건", "genreList": [{"type": "PURE", "description": "로맨스"}, {"type": "X", "description": "드라마"}]},
                    {"titleId": 1, "titleName": "잘려야 하는 세 번째", "displayAuthor": "", "genreList": []},
                ],
                "pageInfo": {"totalRows": 3},
            }),
        )
    });
    store = store.with_session(fake);
    let search_url = store.build_search_url("유미의 세포들");
    assert_eq!(
        store.search_by_keyword("유미의 세포들"),
        vec![
            row("유미의 세포들", "이동건", "로맨스", "https://comic.naver.com/webtoon/list?titleId=651673", &search_url, ""),
            row("유미의 세포들 외전", "이동건", "로맨스/드라마", "https://comic.naver.com/webtoon/list?titleId=778153", &search_url, ""),
        ]
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].url, "https://comic.naver.com/api/search/webtoon");
    assert_eq!(calls[0].params, params(&[("keyword", "유미의 세포들"), ("page", "1")]));
}

/// 세 웹툰 서점이 공유하는 실패 경로: 빈 결과·식별자 없음·HTTP 오류·예외는 모두 빈 리스트.
macro_rules! failure_path_checks {
    ($store:expr, $empty_payload:expr, $no_id_payload:expr) => {{
        $store = $store.with_session(constant(200, "", $empty_payload));
        assert!($store.search_by_keyword("없는작품").is_empty());
        $store = $store.with_session(constant(200, "", $no_id_payload));
        assert!($store.search_by_keyword("x").is_empty());
        $store = $store.with_session(constant(500, "", json!({})));
        assert!($store.search_by_keyword("x").is_empty());
        $store = $store.with_session(failing());
        assert!($store.search_by_keyword("x").is_empty());
    }};
}

#[test]
fn test_naver_webtoon_search_by_keyword_failure_paths() {
    let mut store = naver_webtoon(true);

    failure_path_checks!(store, json!({"searchList": [], "pageInfo": {"totalRows": 0}}), json!({"searchList": [{"titleName": "id 없음"}]}));
}

/// 카카오웹툰도 '제목 저자' 키워드가 0건이다. 제목만 한 번 부른다.
#[test]
fn test_kakao_webtoon_search_url_and_author_is_ignored() {
    let mut store = kakao_webtoon(false);
    let url = store.build_search_url("이태원 클라쓰");
    assert!(url.starts_with("https://webtoon.kakao.com/search?keyword="));
    assert!(url.contains("%EC%9D%B4%ED%83%9C%EC%9B%90"));

    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let search_url = url.clone();
    store = store.with_search_by_keyword(Box::new(move |keyword| {
        sink.lock().unwrap().push(keyword.to_owned());
        vec![row("이태원 클라쓰", "광진", "드라마", "https://webtoon.kakao.com/content/x/1338", &search_url, "")]
    }));
    let (results, keyword, method) = store.search("9788934900011", "이태원 클라쓰", "광진");
    assert_eq!(*seen.lock().unwrap(), ["이태원 클라쓰"]);
    assert_eq!(keyword, "이태원 클라쓰");
    assert_eq!(method, "title");
    assert_eq!(results[0].0, "이태원 클라쓰");
}

/// 저자는 글·그림·원작만 모으고 중복·출판사를 뺀다. seoId 가 없으면 제목으로 URL 을 만든다.
#[test]
fn test_kakao_webtoon_search_by_keyword_success() {
    let mut store = kakao_webtoon(true);
    store.max_results = 2; // 잘림 동작만 본다. 기본값은 test_bookstore_default_max_results_is_four

    let (fake, calls) = session(|_, _| {
        http(
            200,
            "",
            json!({
                "data": {
                    "content": [
                        {
                            "id": 1338,
                            "title": "이태원 클라쓰",
                            "seoId": "이태원-클라쓰",
                            "genre": "드라마",
                            "authors": [
                                {"order": 1, "name": "광진", "type": "AUTHOR"},
                                {"order": 2, "name": "광진", "type": "ILLUSTRATOR"},
                                {"order": 3, "name": "카카오웹툰 스튜디오", "type": "PUBLISHER"},
                            ],
                        },
                        {
                            "id": 2320,
                            "title": "나 혼자만 레벨업",
                            "genre": "학원/판타지",
                            "authors": [
                                {"order": 1, "name": "현군", "type": "AUTHOR"},
                                {"order": 2, "name": "장성락", "type": "ILLUSTRATOR"},
                                {"order": 3, "name": "추공", "type": "ORIGINAL_STORY"},
                                {"order": 4, "name": "디앤씨웹툰", "type": "PUBLISHER"},
                            ],
                        },
                        {"id": 3, "title": "잘려야 하는 세 번째", "authors": []},
                    ]
                },
                "meta": {"pagination": {"totalCount": 3}},
            }),
        )
    });
    store = store.with_session(fake);
    let search_url = store.build_search_url("레벨업");
    assert_eq!(
        store.search_by_keyword("레벨업"),
        vec![
            row("이태원 클라쓰", "광진", "드라마", "https://webtoon.kakao.com/content/%EC%9D%B4%ED%83%9C%EC%9B%90-%ED%81%B4%EB%9D%BC%EC%93%B0/1338", &search_url, ""),
            row(
                "나 혼자만 레벨업",
                "현군, 장성락, 추공",
                "학원/판타지",
                "https://webtoon.kakao.com/content/%EB%82%98%20%ED%98%BC%EC%9E%90%EB%A7%8C%20%EB%A0%88%EB%B2%A8%EC%97%85/2320",
                &search_url,
                ""
            ),
        ]
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].url, "https://gateway-kw.kakao.com/search/v2/content");
    assert_eq!(calls[0].params, params(&[("word", "레벨업"), ("offset", "0"), ("limit", "2")]));
}

#[test]
fn test_kakao_webtoon_search_by_keyword_failure_paths() {
    let mut store = kakao_webtoon(true);

    failure_path_checks!(store, json!({"data": {"content": []}, "meta": {"pagination": {"totalCount": 0}}}), json!({"data": {"content": [{"title": "id 없음"}]}}));
}

/// 웹툰 서점은 API 를 직접 부르므로 HTML 훅은 호출되지 않는다.
#[test]
fn test_webtoon_unused_html_hooks_are_inert() {
    let soup = "<html><body><a href='/x/1'>x</a></body></html>";
    let stores: Vec<(&str, Box<dyn AbstractBookstore>)> = vec![
        ("NaverWebtoonBookstore", Box::new(naver_webtoon(false))),
        ("KakaoWebtoonBookstore", Box::new(kakao_webtoon(false))),
        ("BomtoonBookstore", Box::new(bomtoon(false))),
        ("LezhinBookstore", Box::new(lezhin(false))),
        ("ToptoonBookstore", Box::new(toptoon(false))),
    ];
    for (name, store) in &stores {
        assert!(store.extract_search_links(soup).is_empty(), "{name}");
        assert_eq!(store.extract_book_info(soup), empty_info(), "{name}");
    }
}

/// 봄툰·레진·탑툰은 웹툰이라 ISBN·저자를 쓰지 않고 제목만 한 번 부른다.
#[test]
fn test_new_webtoon_stores_ignore_author_and_isbn() {
    let mut bomtoon_store = bomtoon(false);
    let mut lezhin_store = lezhin(false);
    let mut toptoon_store = toptoon(false);
    assert!(!bomtoon_store.supports_isbn_search, "BomtoonBookstore");
    assert!(!lezhin_store.supports_isbn_search, "LezhinBookstore");
    assert!(!toptoon_store.supports_isbn_search, "ToptoonBookstore");

    macro_rules! check {
        ($name:expr, $store:expr) => {{
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let sink = Arc::clone(&seen);
            let base = $store.build_search_url("작품");
            $store = $store.with_search_by_keyword(Box::new(move |keyword| {
                sink.lock().unwrap().push(keyword.to_owned());
                vec![row("작품", "작가", "장르", "https://example.com/x", &base, "")]
            }));
            let (results, keyword, method) = $store.search("9788934900011", "작품", "작가");
            assert_eq!(*seen.lock().unwrap(), ["작품"], "{}", $name);
            assert_eq!((keyword.as_str(), method.as_str()), ("작품", "title"), "{}", $name);
            assert_eq!(results[0].0, "작품", "{}", $name);
        }};
    }
    check!("BomtoonBookstore", bomtoon_store);
    check!("LezhinBookstore", lezhin_store);
    check!("ToptoonBookstore", toptoon_store);
}

#[test]
fn test_bomtoon_search_url() {
    let url = bomtoon(false).build_search_url("용사");
    assert_eq!(url, "https://www.bomtoon.com/search?q=%EC%9A%A9%EC%82%AC&ref=input");
}

/// 웹소설(novel)은 건너뛰고 만화만 MAX_RESULTS 건 모은다. 저자 중복은 한 번만 쓴다.
#[test]
fn test_bomtoon_search_by_keyword_success() {
    let mut store = bomtoon(true);
    store.max_results = 2; // 잘림 동작만 본다. 기본값은 test_bookstore_default_max_results_is_four

    let (fake, calls) = session(|_, _| {
        http(
            200,
            "",
            json!({
                "data": {
                    "results": [
                        {"contentsId": 1, "title": "용사 소설", "alias": "novel1", "author": ["소설가"], "tags": ["판타지"], "contentsType": "novel"},
                        {"contentsId": 37760, "title": "이 용사, 전 마왕에 대하여", "alias": "dldydtk", "author": ["사와다 후로페", "사와다 후로페", "아마나 코우타"], "tags": ["판타지", "이계/이세계", "마왕", "용사"], "contentsType": "cartoon"},
                        {"contentsId": 27933, "title": "저, 용사가 아니니까요.", "alias": "NotWorrior", "author": ["이사키 우타"], "tags": ["판타지"], "contentsType": "comic"},
                        {"contentsId": 3, "title": "잘려야 하는 세 번째", "alias": "third", "author": [], "tags": [], "contentsType": "cartoon"},
                    ]
                }
            }),
        )
    });
    store = store.with_session(fake);
    let search_url = store.build_search_url("용사");
    assert_eq!(
        store.search_by_keyword("용사"),
        vec![
            row("이 용사, 전 마왕에 대하여", "사와다 후로페, 아마나 코우타", "판타지/이계/이세계/마왕", "https://www.bomtoon.com/detail/dldydtk", &search_url, ""),
            row("저, 용사가 아니니까요.", "이사키 우타", "판타지", "https://www.bomtoon.com/detail/NotWorrior", &search_url, ""),
        ]
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].url, "https://www.bomtoon.com/api/balcony-search-api/search");
    assert_eq!(calls[0].params["searchText"], "용사");
    assert_eq!(calls[0].params["contentsType"], "ALL");
}

#[test]
fn test_bomtoon_search_by_keyword_failure_paths() {
    let mut store = bomtoon(true);

    failure_path_checks!(store, json!({"data": {"results": []}}), json!({"data": {"results": [{"title": "alias 없음", "contentsType": "cartoon"}]}}));
}

#[test]
fn test_lezhin_search_url() {
    let url = lezhin(false).build_search_url("연애");
    assert_eq!(url, "https://www.lezhin.com/ko/search?t=all&q=%EC%97%B0%EC%95%A0");
}

/// 저자는 글·그림·각본·원작만 모으고 출판사·레이블은 뺀다.
#[test]
fn test_lezhin_search_by_keyword_success() {
    let mut store = lezhin(true);
    store.max_results = 2; // 잘림 동작만 본다. 기본값은 test_bookstore_default_max_results_is_four

    let (fake, calls) = session(|_, _| {
        http(
            200,
            "",
            json!({
                "data": [
                    {
                        "title": "체험! XX의 현장",
                        "alias": "experience",
                        "genres": ["BL"],
                        "artists": [
                            {"name": "강언니", "role": "writer"},
                            {"name": "미스피엠", "role": "painter"},
                            {"name": "강언니", "role": "scripter"},
                            {"name": "레진엔터", "role": "publisher"},
                            {"name": "레진BL", "role": "label"},
                        ],
                    },
                    {"title": "언슬립", "alias": "unsleep", "genres": ["BL", "드라마"], "artists": [{"name": "현외", "role": "original"}]},
                    {"title": "잘려야 하는 세 번째", "alias": "third", "genres": [], "artists": []},
                ],
                "hasNext": true,
            }),
        )
    });
    store = store.with_session(fake);
    let search_url = store.build_search_url("연애");
    assert_eq!(
        store.search_by_keyword("연애"),
        vec![
            row("체험! XX의 현장", "강언니, 미스피엠", "BL", "https://www.lezhin.com/ko/comic/experience", &search_url, ""),
            row("언슬립", "현외", "BL/드라마", "https://www.lezhin.com/ko/comic/unsleep", &search_url, ""),
        ]
    );
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].url, "https://www.lezhin.com/lz-api/v2/advanced-search");
    assert_eq!(calls[0].params, params(&[("q", "연애"), ("t", "all"), ("order", "popular"), ("offset", "0"), ("limit", "2")]));
}

#[test]
fn test_lezhin_search_by_keyword_failure_paths() {
    let mut store = lezhin(true);

    failure_path_checks!(store, json!({"data": []}), json!({"data": [{"title": "alias 없음"}]}));
}

#[test]
fn test_toptoon_search_url() {
    let url = toptoon(false).build_search_url("네크로");
    assert_eq!(url, "https://toptoon.com/hashtag?keyword=%EB%84%A4%ED%81%AC%EB%A1%9C");
}

const CATALOG_URL: &str = "https://d41fbnkczk68k.cloudfront.net/production/comicTotal/4e88df07070cad71b94e7fd9cf7e820bcab109c3f138947a87baba14912de8f0.json";

fn toptoon_comic(title: &str, comic_id: &str, views: u64, genres: &[&str], author_str: &str) -> Value {
    let genre: Vec<Value> = genres.iter().enumerate().map(|(i, g)| json!({"idx": i, "name": g})).collect();
    json!({
        "id": comic_id,
        "meta": {
            "title": title,
            "viewCount": views,
            "genre": genre,
            "authorList": {"authorStr": author_str},
            "author": {"authorString": "무시되는 값"},
            "comicsListUrl": format!("/comic/ep_list/{comic_id}"),
        },
    })
}

/// 페이지 HTML 에서 목록 JSON 주소를 찾아 받고, 제목 일치 > 키워드로 시작 > 포함 순, 같으면 조회수 순으로 고른다.
#[test]
fn test_toptoon_search_by_keyword_ranks_and_formats() {
    let mut store = toptoon(true);
    store.max_results = 2; // 잘림 동작만 본다. 기본값은 test_bookstore_default_max_results_is_four
    let catalog = Arc::new(Mutex::new(json!([
        toptoon_comic("회귀했더니 최강 네크로맨서", "slip", 900, &["판타지"], "작가"),
        toptoon_comic("네크로 시티", "city", 10, &["드라마", "판타지"], "작가"),
        toptoon_comic("네 크로", "exact", 1, &["판타지"], "작가"),
        toptoon_comic("무관한 작품", "other", 99999, &["판타지"], "작가"),
        {"id": "nolink", "meta": {"title": "네크로 링크없음"}},
    ])));

    let served = Arc::clone(&catalog);
    let (fake, calls) = session(move |url, _| {
        if url.starts_with("https://toptoon.com/hashtag") { http(200, &format!("<script>fileUrl: '{CATALOG_URL}',</script>"), Value::Null) } else { http(200, "", served.lock().unwrap().clone()) }
    });
    store = store.with_session(fake);
    let search_url = store.build_search_url("네크로");
    // 공백은 무시하고 비교하므로 "네 크로" 는 키워드와 같은 제목(1순위)이다.
    // 그다음 키워드로 시작하는 "네크로 시티", 나머지는 MAX_RESULTS(2) 에서 잘린다.
    assert_eq!(
        store.search_by_keyword("네크로"),
        vec![
            row("네 크로", "작가", "판타지", "https://toptoon.com/comic/ep_list/exact", &search_url, ""),
            row("네크로 시티", "작가", "드라마/판타지", "https://toptoon.com/comic/ep_list/city", &search_url, ""),
        ]
    );
    assert_eq!(urls(&calls), [search_url.clone(), CATALOG_URL.to_owned()]);

    // 같은 순위끼리는 조회수가 많은 쪽이 먼저다.
    *catalog.lock().unwrap() = json!([toptoon_comic("네크로 하나", "low", 1, &["판타지"], "작가"), toptoon_comic("네크로 둘", "high", 500, &["판타지"], "작가")]);
    let titles: Vec<String> = store.search_by_keyword("네크로").into_iter().map(|r| r.0).collect();
    assert_eq!(titles, ["네크로 둘", "네크로 하나"]);
}

#[test]
fn test_toptoon_search_by_keyword_falls_back_to_author_string() {
    let mut store = toptoon(false);
    let mut comic = toptoon_comic("크로스오버", "crossover", 0, &["판타지"], "작가");
    comic["meta"]["authorList"] = json!({});
    comic["meta"]["author"] = json!({"authorString": "MONNMONN"});

    store = store.with_session(session(move |url, _| if url.contains("hashtag") { http(200, CATALOG_URL, Value::Null) } else { http(200, "", json!([comic.clone()])) }).0);
    assert_eq!(store.search_by_keyword("크로스오버")[0].1, "MONNMONN");
}

#[test]
fn test_toptoon_search_by_keyword_failure_paths() {
    let mut store = toptoon(true);

    // 공백뿐인 키워드는 요청 없이 끝난다.
    store = store.with_session(session(|_, _| panic!("요청하면 안 된다")).0);
    assert!(store.search_by_keyword("   ").is_empty());

    // 페이지 HTTP 오류
    store = store.with_session(constant(500, "", json!([])));
    assert!(store.search_by_keyword("x").is_empty());

    // 페이지에 목록 JSON 주소가 없음
    store = store.with_session(constant(200, "<html></html>", json!([])));
    assert!(store.search_by_keyword("x").is_empty());

    // 목록 JSON HTTP 오류
    store = store.with_session(session(|url, _| if url.contains("hashtag") { http(200, CATALOG_URL, json!([])) } else { http(404, "", json!([])) }).0);
    assert!(store.search_by_keyword("x").is_empty());

    // 일치하는 제목이 없음
    store = store
        .with_session(session(|url, _| if url.contains("hashtag") { http(200, CATALOG_URL, json!([])) } else { http(200, "", json!([toptoon_comic("다른 작품", "a", 0, &["판타지"], "작가")])) }).0);
    assert!(store.search_by_keyword("없는작품").is_empty());

    store = store.with_session(failing());
    assert!(store.search_by_keyword("x").is_empty());
}

/// 서점 하나가 돌려주는 결과는 기본 최대 4개다.
#[test]
fn test_bookstore_default_max_results_is_four() {
    assert_eq!(MAX_RESULTS, 4);

    let mut store = naver_webtoon(false);

    let list: Vec<Value> = (1..6).map(|i| json!({"titleId": i, "titleName": format!("작품 {i}"), "displayAuthor": "", "genreList": []})).collect();
    store = store.with_session(constant(200, "", json!({"searchList": list})));
    let titles: Vec<String> = store.search_by_keyword("작품").into_iter().map(|r| r.0).collect();
    assert_eq!(titles, ["작품 1", "작품 2", "작품 3", "작품 4"]);
}

// ---------------------------------------------------------------------------
// 새 테스트 (Python 원본에 없음): 조아라 API 키 환경 변수, 캐시 파일 이름
// 키는 소스에 두지 않고 `TM_JOARA_API_KEY`에서 읽는다. 아래 값은 모두 더미다.
// ---------------------------------------------------------------------------

mod added_joara_key_and_cache_name {
    use super::*;
    use common::env::{EnvGuard, empty_env, map_env};
    use common::log_capture::LogCapture;
    use std::sync::Arc;

    const DUMMY_KEY: &str = "dummy-joara-key-0000";

    fn ok_payload() -> Value {
        json!({"status": 1, "data": {"list": [{"book_code": 1, "subject": "데미안", "member_name": "A", "category_name": "C"}]}})
    }

    #[test]
    fn test_joara_api_key_is_sent_from_injected_env() {
        let (fake, calls) = session(|_, _| http(200, "", ok_payload()));
        let store = joara(false).with_env(Arc::new(map_env(&[("TM_JOARA_API_KEY", DUMMY_KEY)]))).with_session(fake);
        assert_eq!(store.search_by_keyword("데미안").len(), 1);

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].url, "https://api.joara.com/v2/search/query");
        assert_eq!(calls[0].params["api_key"], DUMMY_KEY);
        assert_eq!(calls[0].params["query"], "데미안");
        assert_eq!(calls[0].params["offset"], "4");
        let uid = &calls[0].params["deviceuid"];
        assert!(uid.len() == 32 && uid.bytes().all(|b| b.is_ascii_hexdigit()), "deviceuid는 32자리 16진수");
    }

    #[test]
    fn test_joara_device_uid_changes_per_call() {
        let (fake, calls) = session(|_, _| http(200, "", ok_payload()));
        let store = joara(false).with_env(Arc::new(empty_env())).with_session(fake);
        store.search_by_keyword("a");
        store.search_by_keyword("a");
        let calls = calls.lock().unwrap();
        assert_ne!(calls[0].params["deviceuid"], calls[1].params["deviceuid"]);
    }

    #[test]
    fn test_joara_api_key_is_empty_when_env_missing() {
        let (fake, calls) = session(|_, _| http(200, "", ok_payload()));
        let store = joara(false).with_env(Arc::new(empty_env())).with_session(fake);
        store.search_by_keyword("데미안");
        assert_eq!(calls.lock().unwrap()[0].params["api_key"], "");
    }

    #[test]
    fn test_joara_default_env_reads_process_environment() {
        let mut env = EnvGuard::new();
        env.set("TM_JOARA_API_KEY", DUMMY_KEY);
        let (fake, calls) = session(|_, _| http(200, "", ok_payload()));
        let store = joara(false).with_session(fake);
        store.search_by_keyword("a");
        assert_eq!(calls.lock().unwrap()[0].params["api_key"], DUMMY_KEY);

        env.remove("TM_JOARA_API_KEY");
        store.search_by_keyword("a");
        assert_eq!(calls.lock().unwrap()[1].params["api_key"], "");
    }

    /// 세션 오류 문장이나 API 메시지에 키가 들어 와도 로그에는 가려서 남긴다.
    #[test]
    fn test_joara_logs_and_errors_do_not_contain_key() {
        let logs = LogCapture::start(log::Level::Trace);
        let env = || Arc::new(map_env(&[("TM_JOARA_API_KEY", DUMMY_KEY)]));

        // 요청 실패: 오류 문장에 요청 주소(키 포함)가 들어 있는 경우
        let leaky = session(|_, params| {
            let key = params.iter().find(|(k, _)| k == "api_key").map(|(_, v)| v.clone()).unwrap_or_default();
            Err(HttpError::Connection(format!("GET https://api.joara.com/v2/search/query?api_key={key} 실패")))
        })
        .0;
        let mut store = joara(true).with_env(env()).with_session(leaky);
        assert!(store.search_by_keyword("데미안").is_empty());

        // status != 1: 메시지에 키가 들어 있는 경우
        let message = format!("잘못된 키 {DUMMY_KEY}");
        store = store.with_session(constant(200, "", json!({"status": 0, "message": message})));
        assert!(store.search_by_keyword("데미안").is_empty());

        // 성공과 HTTP 오류 경로
        store = store.with_session(constant(200, "", ok_payload()));
        assert_eq!(store.search_by_keyword("데미안").len(), 1);
        store = store.with_session(constant(500, "", json!({})));
        assert!(store.search_by_keyword("데미안").is_empty());

        let text = logs.text();
        assert!(!text.is_empty(), "verbose 로그가 남아야 한다");
        assert!(!text.contains(DUMMY_KEY), "로그에 키가 나오면 안 된다: {text}");
        assert!(text.contains("****"), "가린 자리가 보여야 한다: {text}");
    }

    #[test]
    fn test_joara_log_without_key_keeps_message() {
        let logs = LogCapture::start(log::Level::Trace);
        let store = joara(true).with_env(Arc::new(empty_env())).with_session(failing());
        assert!(store.search_by_keyword("데미안").is_empty());
        assert!(logs.text().contains("boom"));
    }

    /// Python `uuid.uuid5(uuid.NAMESPACE_URL, url)`와 같은 이름이어야 기존 캐시 파일을 이어 쓴다.
    #[test]
    fn test_cache_file_name_matches_python_uuid5() {
        assert_eq!(backend::bookstore::cache_file_name("https://example.com/detail/1"), "2c75ebbd-c7d8-5945-b588-d1286cf77aee.html");
        assert_eq!(backend::bookstore::cache_file_name("http://example.com/detail/123"), "0a5f1021-24ca-5315-aa3e-96a26387cd63.html");
        assert_eq!(backend::bookstore::cache_file_name(""), "1b4db7eb-4057-5ddf-91e0-36dec72071f5.html");
    }

    #[test]
    fn test_save_html_writes_uuid5_named_file() {
        let tmp = TempDir::new().unwrap();
        let store = yes24(false).with_tmp_dir(tmp.path().to_path_buf());
        store._save_html_to_tmp("<html>x</html>", "https://example.com/detail/1");
        let written = fs::read_to_string(tmp.path().join("2c75ebbd-c7d8-5945-b588-d1286cf77aee.html")).unwrap();
        assert_eq!(written, "<html>x</html>");
    }
}
