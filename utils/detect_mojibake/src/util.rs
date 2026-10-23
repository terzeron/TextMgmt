//! 경로·URL 유틸. Python `posixpath`와 `urllib.parse`의 동작에 맞춘다.
//! EPUB 구조 검사(`epubscan`)와 분류 도구가 함께 쓴다.

use std::path::Path;

pub fn posix_normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let initial = if path.starts_with("//") && !path.starts_with("///") { 2 } else { usize::from(path.starts_with('/')) };
    let mut comps: Vec<&str> = Vec::new();
    for c in path.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(c);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let joined = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if joined.is_empty() { ".".into() } else { joined }
}
pub fn posix_join(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        b.to_owned()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}
/// `posixpath.dirname`: 마지막 '/'까지에서 끝의 '/'를 걷어낸다. 루트 슬래시만 남으면 유지한다.
pub fn posix_dirname(p: &str) -> String {
    let Some(i) = p.rfind('/') else { return String::new() };
    let head = &p[..=i];
    let trimmed = head.trim_end_matches('/');
    if trimmed.is_empty() { head.to_owned() } else { trimmed.to_owned() }
}
/// `urllib.parse.unquote`: 퍼센트 인코딩을 풀고 잘못된 UTF-8은 U+FFFD로 바꾼다.
pub fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}
/// 분류 경로(`DEFAULT_LIBRARY_ROOT` 아래 첫 구간). 밖이면 빈 문자열.
pub fn category_of(path: &Path) -> String {
    let root = Path::new(crate::DEFAULT_LIBRARY_ROOT);
    if !path.to_string_lossy().starts_with(crate::DEFAULT_LIBRARY_ROOT) {
        return String::new();
    }
    path.strip_prefix(root).ok().and_then(|p| p.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default()
}
