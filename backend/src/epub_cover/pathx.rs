//! Python `posixpath`와 `urllib.parse`의 필요한 부분. EPUB 안의 상대 경로를 ZIP 멤버 이름으로 바꾸는 데 쓴다.

/// `posixpath.normpath`.
pub fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let mut initial_slashes = if path.starts_with('/') { 1 } else { 0 };
    if path.starts_with("//") && !path.starts_with("///") {
        initial_slashes = 2;
    }
    let mut parts: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial_slashes == 0 && parts.is_empty()) || parts.last() == Some(&"..") {
            parts.push(comp);
        } else if !parts.is_empty() {
            parts.pop();
        }
    }
    let mut out = parts.join("/");
    if initial_slashes > 0 {
        out = format!("{}{out}", "/".repeat(initial_slashes));
    }
    if out.is_empty() { ".".to_string() } else { out }
}

/// `posixpath.join(a, b)`.
pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_string()
    } else if a.is_empty() || a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// `posixpath.dirname`.
pub fn dirname(path: &str) -> String {
    let head = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    if !head.is_empty() && head.chars().any(|c| c != '/') { head.trim_end_matches('/').to_string() } else { head.to_string() }
}

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

/// `urllib.parse.unquote`. 잘못된 UTF-8은 U+FFFD로 바꾼다.
pub fn unquote(text: &str) -> String {
    if !text.contains('%') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
