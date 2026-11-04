//! URL 나누기·잇기·인코딩(`urlsplit`/`urljoin`/`urlencode`/`quote`/`unquote`)의 최소 구현. 표지 URL 검증·다운로드와 서점 검색 결과 링크 해석이 함께 쓴다.

/// `urlsplit` 결과. `netloc`은 `//`가 있을 때만 `Some`이다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    pub scheme: String,
    pub netloc: Option<String>,
    pub path: String,
    pub query: String,
    pub fragment: String,
}

fn split_scheme(url: &str) -> (String, &str) {
    if let Some(colon) = url.find(':') {
        let head = &url[..colon];
        let mut chars = head.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        if valid {
            return (head.to_ascii_lowercase(), &url[colon + 1..]);
        }
    }
    (String::new(), url)
}

pub fn urlsplit(url: &str) -> Split {
    let url = url.trim();
    let (scheme, rest) = split_scheme(url);
    let (rest, fragment) = rest.split_once('#').map_or((rest, ""), |(a, b)| (a, b));
    let (rest, query) = rest.split_once('?').map_or((rest, ""), |(a, b)| (a, b));
    let (netloc, path) = match rest.strip_prefix("//") {
        Some(after) => {
            let end = after.find('/').unwrap_or(after.len());
            (Some(after[..end].to_string()), &after[end..])
        }
        None => (None, rest),
    };
    Split { scheme, netloc, path: path.to_string(), query: query.to_string(), fragment: fragment.to_string() }
}

/// `urlsplit(...).hostname` 등 netloc 해석 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Netloc {
    pub username: Option<String>,
    pub password: Option<String>,
    /// 소문자. 대괄호는 벗긴다.
    pub hostname: Option<String>,
    /// 포트 문자열. 비어 있으면 `None`.
    port_text: Option<String>,
}

/// 포트가 숫자가 아니거나 범위를 벗어났다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPort;

impl Netloc {
    pub fn parse(netloc: &str) -> Netloc {
        let (userinfo, hostinfo) = match netloc.rfind('@') {
            Some(at) => (Some(&netloc[..at]), &netloc[at + 1..]),
            None => (None, netloc),
        };
        let (username, password) = match userinfo {
            Some(info) => match info.split_once(':') {
                Some((user, pass)) => (Some(user.to_string()), Some(pass.to_string())),
                None => (Some(info.to_string()), None),
            },
            None => (None, None),
        };
        let (host, port_text) = if let Some(rest) = hostinfo.strip_prefix('[') {
            match rest.split_once(']') {
                Some((host, after)) => (host.to_string(), after.strip_prefix(':').map(str::to_string)),
                None => (rest.to_string(), None),
            }
        } else {
            match hostinfo.split_once(':') {
                Some((host, port)) => (host.to_string(), Some(port.to_string())),
                None => (hostinfo.to_string(), None),
            }
        };
        let hostname = if host.is_empty() { None } else { Some(host.to_lowercase()) };
        Netloc { username, password, hostname, port_text: port_text.filter(|p| !p.is_empty()) }
    }

    /// `parsed.port`: 숫자가 아니거나 0~65535를 벗어나면 `Err`.
    pub fn port(&self) -> Result<Option<u16>, InvalidPort> {
        match &self.port_text {
            None => Ok(None),
            Some(text) if text.chars().all(|c| c.is_ascii_digit()) => text.parse::<u16>().map(Some).map_err(|_| InvalidPort),
            Some(_) => Err(InvalidPort),
        }
    }
}

fn remove_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "." => {
                if i == last {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if i == last {
                    out.push("");
                }
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

fn unsplit(scheme: &str, netloc: Option<&str>, path: &str, query: &str, fragment: &str) -> String {
    let mut out = String::new();
    if !scheme.is_empty() {
        out.push_str(scheme);
        out.push(':');
    }
    if let Some(netloc) = netloc {
        out.push_str("//");
        out.push_str(netloc);
    }
    out.push_str(path);
    if !query.is_empty() {
        out.push('?');
        out.push_str(query);
    }
    if !fragment.is_empty() {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// 기준 URL에 상대 URL을 잇는다. 흔한 경우를 RFC 3986 규칙으로 처리한다.
pub fn urljoin(base: &str, url: &str) -> String {
    if base.is_empty() {
        return url.to_string();
    }
    if url.is_empty() {
        return base.to_string();
    }
    let b = urlsplit(base);
    let u = urlsplit(url);
    if !u.scheme.is_empty() && u.scheme != b.scheme {
        return url.to_string();
    }
    let scheme = if u.scheme.is_empty() { b.scheme.as_str() } else { u.scheme.as_str() };
    if u.netloc.is_some() {
        return unsplit(scheme, u.netloc.as_deref(), &remove_dot_segments(&u.path), &u.query, &u.fragment);
    }
    if u.path.is_empty() {
        let query = if u.query.is_empty() { b.query.as_str() } else { u.query.as_str() };
        return unsplit(scheme, b.netloc.as_deref(), &b.path, query, &u.fragment);
    }
    let merged = if u.path.starts_with('/') {
        u.path.clone()
    } else {
        let dir = b.path.rfind('/').map_or("", |i| &b.path[..=i]);
        let dir = if b.netloc.is_some() && dir.is_empty() { "/" } else { dir };
        format!("{dir}{}", u.path)
    };
    unsplit(scheme, b.netloc.as_deref(), &remove_dot_segments(&merged), &u.query, &u.fragment)
}

fn percent_encode(text: &str, keep: impl Fn(u8) -> bool, space_plus: bool) -> String {
    let mut out = String::new();
    for &byte in text.as_bytes() {
        if keep(byte) {
            out.push(byte as char);
        } else if space_plus && byte == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-' | b'~')
}

/// 퍼센트 인코딩. 영문·숫자·`_.-~`와 `/`는 그대로 둔다.
pub fn quote(text: &str) -> String {
    percent_encode(text, |b| unreserved(b) || b == b'/', false)
}

/// 경로 조각 하나의 퍼센트 인코딩. `quote`와 같고 `/`도 인코딩한다.
pub fn quote_component(text: &str) -> String {
    percent_encode(text, unreserved, false)
}

/// 질의 문자열. 키와 값을 퍼센트 인코딩하고 공백은 `+`로 쓴다.
pub fn urlencode(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", percent_encode(k, unreserved, true), percent_encode(v, unreserved, true))).collect::<Vec<_>>().join("&")
}

/// 퍼센트 인코딩을 푼다. 잘못된 `%` 조각은 그대로 두고, 풀린 바이트가 UTF-8이 아니면 U+FFFD로 바꾼다.
pub fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2]))
        {
            out.push(high * 16 + low);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}
