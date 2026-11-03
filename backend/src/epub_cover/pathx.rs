//! Python `posixpath`와 `urllib.parse`의 필요한 부분. EPUB 안의 상대 경로를 ZIP 멤버 이름으로 바꾸는 데 쓴다.

// normpath·join·dirname·unquote는 `text_extractor::path`의 것을 이 모듈 이름으로 다시 내보낸다.
pub use text_extractor::path::{posix_dirname as dirname, posix_join as join, posix_normpath as normpath, unquote};

/// `posixpath.splitext(path)[1]`. 파일 이름 앞쪽의 점은 확장자로 보지 않는다.
pub fn ext(path: &str) -> String {
    let base_start = path.rfind('/').map_or(0, |i| i + 1);
    let base = &path[base_start..];
    let leading = base.len() - base.trim_start_matches('.').len();
    match base[leading..].rfind('.') {
        Some(i) => base[leading + i..].to_string(),
        None => String::new(),
    }
}

/// `posixpath.relpath(path, start)`(현재 디렉터리를 쓰지 않고 둘 다 상대 경로라고 본다).
pub fn relpath(path: &str, start: &str) -> String {
    let normal = |p: &str| -> Vec<String> { normpath(p).split('/').filter(|c| !c.is_empty() && *c != ".").map(str::to_string).collect() };
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

/// Python `_member_path(base, href)`: 상대 `href`를 ZIP 멤버 이름으로 푼다. 바깥을 가리키거나 URL이면 빈 문자열이다.
pub fn member_path(base: &str, href: &str) -> String {
    let (scheme, netloc, path) = urlparse_path(href);
    if !scheme.is_empty() || !netloc.is_empty() || path.is_empty() {
        return String::new();
    }
    let member = normpath(&join(base, &unquote(&path)));
    if member.starts_with('/') || member.starts_with("../") { String::new() } else { member }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normpath_matches_posixpath() {
        assert_eq!(normpath("a/b/../c"), "a/c");
        assert_eq!(normpath("../a"), "../a");
        assert_eq!(normpath("/../a"), "/a");
        assert_eq!(normpath("//a"), "//a");
        assert_eq!(normpath(""), ".");
        assert_eq!(normpath("./"), ".");
    }

    #[test]
    fn member_path_resolves_like_python() {
        assert_eq!(member_path("OPS/Text", "../a%20b.jpg"), "OPS/a b.jpg");
        assert_eq!(member_path("OPS", "https://example.com/a.jpg"), "");
        assert_eq!(member_path("OPS", "../../x.jpg"), "");
        assert_eq!(member_path("OPS", "#top"), "");
        assert_eq!(member_path("OPS", ""), "");
        assert_eq!(member_path("OPS", "a.xhtml#frag"), "OPS/a.xhtml");
        assert_eq!(member_path("", "a.jpg?x=1"), "a.jpg");
    }

    #[test]
    fn relpath_matches_posixpath() {
        assert_eq!(relpath("OEBPS/tm-split-a.jpg", "OEBPS/Text"), "../tm-split-a.jpg");
        assert_eq!(relpath("a/b.jpg", ""), "a/b.jpg");
        assert_eq!(relpath("a/b.jpg", "a"), "b.jpg");
    }

    #[test]
    fn dirname_and_ext() {
        assert_eq!(dirname("OPS/book.opf"), "OPS");
        assert_eq!(dirname("book.opf"), "");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(ext("OPS/cover.JPG"), ".JPG");
        assert_eq!(ext("OPS/.hidden"), "");
        assert_eq!(ext("a.tar.gz"), ".gz");
    }

    #[test]
    fn unquote_handles_korean_and_bad_escapes() {
        assert_eq!(unquote("%ED%95%9C%EA%B8%80"), "한글");
        assert_eq!(unquote("100%"), "100%");
        assert_eq!(unquote("%zz"), "%zz");
        assert_eq!(unquote("%28a%29"), "(a)");
    }
}
