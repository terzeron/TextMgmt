//! 여러 테스트 파일이 함께 쓰는 준비물(helper·대역). 같은 본문의 사본을 한곳에 모았다.
#![allow(dead_code)]

use bookstore::store_clients::SearchRow;
use serde_json::{json, Value};
use sysdeps::{HttpError, HttpResponse};

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

pub fn empty_info() -> Value {
    json!({"title": "", "author": "", "category": "", "isbn": ""})
}

/// `<a href='...'>`의 href 값을 문서 순서대로 모은다.
pub fn single_quoted_hrefs(html: &str) -> Vec<String> {
    html.split("href='").skip(1).filter_map(|rest| rest.split('\'').next()).map(str::to_owned).collect()
}
