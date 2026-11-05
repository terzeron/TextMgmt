//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]
use ::common::http_client::{HttpError, HttpResponse, HttpSession, HttpTimeout};
use bookstore::store_clients::SearchRow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub type Handler = Box<dyn Fn(&str, &[(String, String)]) -> Result<HttpResponse, HttpError> + Send + Sync>;

pub type LoadHook = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub type SaveHook = Box<dyn Fn(&str, &str) + Send + Sync>;

pub type SearchHook = Box<dyn Fn(&str) -> Vec<SearchRow> + Send + Sync>;

/// `open_tag`와 `close_tag` 사이 텍스트. bs4 `find(...).text` 대신 쓰는 단순 파서다. 없으면 빈 문자열.
pub fn between(html: &str, open_tag: &str, close_tag: &str) -> String {
    match html.find(open_tag) {
        Some(start) => {
            let rest = &html[start + open_tag.len()..];
            rest.find(close_tag).map_or(String::new(), |end| rest[..end].to_owned())
        }
        None => String::new(),
    }
}

/// `<a href='...'>`의 href 값을 문서 순서대로 모은다.
pub fn single_quoted_hrefs(html: &str) -> Vec<String> {
    html.split("href='").skip(1).filter_map(|rest| rest.split('\'').next()).map(str::to_owned).collect()
}

#[derive(Clone, Debug)]
pub struct Call {
    pub url: String,
    pub params: BTreeMap<String, String>,
    pub timeout: Option<HttpTimeout>,
}

pub struct FakeSession {
    pub handler: Handler,
    pub calls: Arc<Mutex<Vec<Call>>>,
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

// ---------------------------------------------------------------------------
// 테스트용 서점 클래스
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `DummyBookstore`/`DummyNoIsbnBookstore`: 링크 둘 고정, h1/span.author/p.category/p.isbn 파싱
    Dummy,
    /// `DummyBookstoreUnit`: 링크 둘 고정, 정보도 고정
    Unit,
    /// `DummyBookstoreParsing`: a[href] 링크, h1/span.author/div.cat/div.isbn 파싱
    Parsing,
    /// `BaseOnlyBookstore`: 링크 없음, 정보 비어 있음
    BaseOnly,
}

/// 요청하면 `RuntimeError("boom")`을 던지는 세션.
pub fn failing() -> Arc<dyn HttpSession> {
    session(|_, _| Err(HttpError::Other("boom".to_owned()))).0
}

/// `session.get`을 `handler`로 바꾼 세션과, 받은 호출 기록을 돌려준다.
pub fn session(handler: impl Fn(&str, &[(String, String)]) -> Result<HttpResponse, HttpError> + Send + Sync + 'static) -> (Arc<dyn HttpSession>, Arc<Mutex<Vec<Call>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (Arc::new(FakeSession { handler: Box::new(handler), calls: Arc::clone(&calls) }), calls)
}
