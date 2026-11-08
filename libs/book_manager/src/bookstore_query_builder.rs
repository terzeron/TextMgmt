//! 서점 검색어를 만든다. ISBN이 있으면 ISBN-13으로 맞추고, 파일 이름에서 저자명과 작품명을 뽑는다.
//!
//! 작품명은 프런트엔드 `Bookstore.jsx`의 `stripComicDirectoryMeta(stripComicDirectoryAuthor(..))`와 같은 규칙이다.
//! 저자명은 `stripComicDirectoryAuthor`가 떼어 내는 부분이다. 단, 끝 대괄호가 완결·권·화 같은 표기이면 저자로 보지 않는다.

use std::sync::LazyLock;

use isbn_extractor::{validate_isbn10, validate_isbn13};
use regex::Regex;

/// 서점 검색에 쓰는 값.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BookQuery {
    pub isbn13: Option<String>,
    pub title: String,
    pub author: String,
}

impl BookQuery {
    /// inode 캐시가 그때 쓴 저자명+작품명과 지금 것을 비교할 때 쓰는 키.
    pub fn title_key(&self) -> String {
        format!("{}\u{1f}{}", self.author, self.title)
    }
}

/// 하이픈·공백을 떼고 ISBN-13으로 맞춘다. 유효하지 않으면 `None`.
pub fn normalize_isbn13(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| *c != '-' && !c.is_whitespace()).map(|c| c.to_ascii_uppercase()).collect();
    match cleaned.len() {
        13 if validate_isbn13(&cleaned) => Some(cleaned),
        10 if validate_isbn10(&cleaned) => {
            let body = format!("978{}", &cleaned[..9]);
            let sum: u32 = body.chars().enumerate().map(|(i, c)| c.to_digit(10).unwrap_or(0) * if i % 2 == 0 { 1 } else { 3 }).sum();
            Some(format!("{body}{}", (10 - sum % 10) % 10))
        }
        _ => None,
    }
}

static LEADING_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([^\]]*)\]\s*").unwrap());
static LEADING_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\(([^)]*)\)\s*").unwrap());
static TRAILING_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\[([^\]]*)\]$").unwrap());
static TRAILING_AT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*@(.*)$").unwrap());
/// `COMIC_META_START`의 앞보기 부분. JS 정규식의 `\d`는 ASCII 숫자다.
static META_AHEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[0-9]{1,4}\s*(?:[-~]\s*[0-9]{1,4})?\s*(?:권|화|편|장)|완결|미완|연재|누락|잘림|미수록)").unwrap());
/// 끝 대괄호 안이 이 모양이면 저자가 아니라 권·화·완결 표기다.
static META_TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:[0-9]|완결|미완|연재|누락|잘림|미수록)").unwrap());

/// `stripComicDirectoryAuthor`와 같은 결과에 뽑은 저자명을 함께 돌려준다.
fn strip_author(name: &str) -> (String, String) {
    let trimmed = name.trim();
    let mut author = String::new();
    fn take(found: &str, author: &mut String) {
        if author.is_empty() {
            *author = found.trim().to_string();
        }
    }
    let mut s = trimmed.to_string();
    if let Some(c) = LEADING_BRACKET.captures(&s) {
        take(&c[1], &mut author);
        s = s[c.get(0).unwrap().end()..].to_string();
    }
    if let Some(c) = LEADING_PAREN.captures(&s) {
        take(&c[1], &mut author);
        s = s[c.get(0).unwrap().end()..].to_string();
    }
    if let Some(c) = TRAILING_BRACKET.captures(&s) {
        if !META_TOKEN.is_match(c[1].trim()) {
            take(&c[1], &mut author);
        }
        s = s[..c.get(0).unwrap().start()].to_string();
    }
    if let Some(c) = TRAILING_AT.captures(&s) {
        take(&c[1], &mut author);
        s = s[..c.get(0).unwrap().start()].to_string();
    }
    let stripped = s.trim();
    (if stripped.is_empty() { trimmed.to_string() } else { stripped.to_string() }, author)
}

/// `COMIC_META_START`(`(?:\s+|\s*[([])(?=앞보기)`)의 가장 왼쪽 일치 위치.
fn meta_start(s: &str) -> Option<usize> {
    let starts: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    for &i in &starts {
        let rest = &s[i..];
        let ws: Vec<usize> = rest.char_indices().take_while(|(_, c)| c.is_whitespace()).map(|(j, c)| j + c.len_utf8()).collect();
        // `\s+`: 길게 먼저, 짧게 되돌아가며 앞보기를 만족하는 길이를 찾는다.
        if ws.iter().rev().any(|&k| META_AHEAD.is_match(&rest[k..])) {
            return Some(i);
        }
        // `\s*[([]`
        for k in std::iter::once(0).chain(ws.iter().copied()).collect::<Vec<_>>().into_iter().rev() {
            let after = &rest[k..];
            if let Some(c) = after.chars().next()
                && (c == '(' || c == '[')
                && META_AHEAD.is_match(&after[c.len_utf8()..])
            {
                return Some(i);
            }
        }
    }
    None
}

/// `stripComicDirectoryMeta`: 괄호·권수·화수·완결 꼬리를 뗀다.
fn strip_meta(name: &str) -> String {
    let trimmed = name.trim();
    let before_paren = match trimmed.find(['(', '（']) {
        Some(i) if i > 0 => &trimmed[..i],
        _ => trimmed,
    };
    let stripped = match meta_start(before_paren) {
        Some(i) => before_paren[..i].trim(),
        None => before_paren.trim(),
    };
    if stripped.is_empty() { trimmed.to_string() } else { stripped.to_string() }
}

/// 파일 이름(확장자 없음)과 ES의 ISBN으로 검색어를 만든다.
pub fn query_from_file(file_stem: &str, raw_isbn: Option<&str>) -> BookQuery {
    let (without_author, author) = strip_author(file_stem);
    BookQuery { isbn13: raw_isbn.and_then(normalize_isbn13), title: strip_meta(&without_author), author }
}
