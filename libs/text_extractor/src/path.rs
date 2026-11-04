//! 경로·URL 유틸. Python `posixpath`와 `urllib.parse`의 동작에 맞춘다.
//! EPUB 구조 검사(`bookdoctor`), 문서 읽기(`doc_reader`), 분류기가 함께 쓴다.

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
/// `posixpath.splitext(path)[1]`. 파일 이름 앞쪽의 점은 확장자로 보지 않는다.
pub fn posix_ext(path: &str) -> String {
    let base_start = path.rfind('/').map_or(0, |i| i + 1);
    let base = &path[base_start..];
    let leading = base.len() - base.trim_start_matches('.').len();
    match base[leading..].rfind('.') {
        Some(i) => base[leading + i..].to_string(),
        None => String::new(),
    }
}
/// `posixpath.relpath(path, start)`(현재 디렉터리를 쓰지 않고 둘 다 상대 경로라고 본다).
pub fn posix_relpath(path: &str, start: &str) -> String {
    let normal = |p: &str| -> Vec<String> { posix_normpath(p).split('/').filter(|c| !c.is_empty() && *c != ".").map(str::to_string).collect() };
    let (target, base) = (normal(path), normal(start));
    let common = target.iter().zip(&base).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); base.len() - common];
    parts.extend(target[common..].iter().cloned());
    if parts.is_empty() { ".".to_string() } else { parts.join("/") }
}
/// `urlparse(href)`가 돌려주는 `(scheme, netloc, path)`. 질의·조각·`;params`는 path에서 뺀다.
pub fn urlparse_path(href: &str) -> (String, String, String) {
    let mut rest = href.trim_start_matches(|c: char| c.is_ascii_control() || c == ' ').to_string();
    let mut scheme = String::new();
    if let Some(colon) = rest.find(':') {
        let candidate = &rest[..colon];
        let mut chars = candidate.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if valid {
            scheme = candidate.to_ascii_lowercase();
            rest = rest[colon + 1..].to_string();
        }
    }
    let mut netloc = String::new();
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        netloc = after[..end].to_string();
        rest = after[end..].to_string();
    }
    if let Some(i) = rest.find('#') {
        rest.truncate(i);
    }
    if let Some(i) = rest.find('?') {
        rest.truncate(i);
    }
    // `urlparse`는 마지막 경로 조각의 `;params`를 path에서 뺀다.
    if rest.contains(';') {
        let start = rest.rfind('/').unwrap_or(0);
        if let Some(i) = rest[start..].find(';') {
            rest.truncate(start + i);
        }
    }
    (scheme, netloc, rest)
}
