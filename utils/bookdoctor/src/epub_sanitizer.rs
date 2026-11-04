//! `--sanitize`: EPUB을 허용 목록으로 재구성한다. 텍스트와 이미지를 보여 주는 데 필요 없는 요소(스크립트, 실행 파일,
//! 원격 로드, 외부 개체)를 지우고, 지운 멤버를 가리키는 manifest 항목도 함께 지운다. 인터랙티브 책은 깨지는 것을 감수한다.
//!
//! 순서: 검사 → 보안 지표(HIGH 이상)가 있는 파일만 대상 → `rebuild` → `verify`(본문 글자와 구조) → 재검사 → 적용.
//! 적용은 원본을 휴지통 디렉터리에 두고, 같은 inode에 덮어쓰고 mtime을 되돌린다(ES 문서 id가 inode라서 `mv` 교체를 쓰지 않는다).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Cursor, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::magic_sniffer::{Sniffed, sniff};
use crate::{Category, Severity};
use htmlkit::{decode_entities, text_of};
use regex::Regex;
use text_extractor::path::{posix_dirname, posix_join, posix_normpath, unquote};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::cli_parser::Options;
use crate::format_dispatcher::scan_file;

/// 멤버 하나와 전체를 읽는 상한. 압축 폭탄이 메모리를 채우지 못하게 한다.
const MAX_MEMBER: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;

pub(crate) const HTML_EXTS: &[&str] = &["xhtml", "html", "htm"];
const XML_EXTS: &[&str] = &["xml", "opf", "ncx", "smil"];
const PASSIVE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "avif", "ico", "jp2", "jxl", "ttf", "otf", "woff", "woff2", "eot", "ttc", "mp3", "m4a", "aac", "ogg", "oga", "opus", "wav", "mp4",
    "m4v", "ogv", "webm",
];

/// 시작 태그째로 지우고 끝 태그도 지우되 안쪽 내용은 남기는 요소.
const DROP_TAGS: &[&str] = &["base", "form", "input", "textarea", "select", "button"];
/// 내용까지 통째로 지우는 요소.
/// raw text처럼 파싱되는 요소(`noscript`, `xmp`, `noframes`, `plaintext`)도 같이 지운다. 브라우저와 스캐너가 내용을 다르게 끊을 수 있는 자리다.
const REMOVE_ELEMENTS: &[&str] = &["script", "object", "embed", "applet", "iframe", "frame", "frameset", "noembed", "foreignobject", "noscript", "xmp", "noframes", "plaintext"];
const URL_ATTRS: &[&str] = &["href", "src", "action", "formaction", "background", "poster", "data", "codebase", "manifest", "ping", "srcset"];
const SAFE_SCHEMES: &[&str] = &["http", "https", "mailto", "tel", "sms", "epub", "cid", "urn", "tag"];

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("잘못된 정규식 {p}: {e}"))
}

static CSS_IMPORT: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)@import\s+[^;{}]*;?"));
static CSS_URL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)(?:url|src)\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)"#));
static CSS_IMAGE_SET: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)(?:-webkit-)?image-set\((?:[^()"']|"[^"]*"|'[^']*'|\([^()]*\))*\)"#));
static CSS_STRING: LazyLock<Regex> = LazyLock::new(|| re(r#""([^"]*)"|'([^']*)'"#));
static CSS_EXPRESSION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)expression\s*\((?:[^()]|\([^()]*\))*\)"));
static CSS_HOOKS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:-moz-binding|behavior)\s*:[^;}]*"));
static SELF_CLOSING_RAW: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<(script|style)\b([^>]*?)/>"));
static XML_DECL: LazyLock<Regex> = LazyLock::new(|| re(r"^<\?xml\s[^<>?]*\?>"));
static OPF_ITEM: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)[ \t]*<(?:\w+:)?item\b[^>]*>(?:\s*</(?:\w+:)?item>)?[ \t]*\r?\n?"));
static ATTR_HREF: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\bhref\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
static ATTR_PROPERTIES: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)\s+properties\s*=\s*(?:"([^"]*)"|'([^']*)')"#));

/// 바꾼 내용을 종류별로 센다.
#[derive(Default, Debug)]
pub struct Changes(BTreeMap<&'static str, usize>);

impl Changes {
    fn add(&mut self, what: &'static str) {
        *self.0.entry(what).or_default() += 1;
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn summary(&self) -> String {
        self.0.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    }

    fn merge(&mut self, other: Changes) {
        for (k, n) in other.0 {
            *self.0.entry(k).or_default() += n;
        }
    }
}

// ---------------------------------------------------------------- URL 판정

/// 브라우저가 URL을 읽는 방식으로 정규화한다: 엔티티를 풀고, 공백·제어문자를 지우고, 소문자로 만들고, `\`를 `/`로 본다
/// (`/\evil`과 `\/evil`은 프로토콜 상대 주소 `//evil`이다).
fn normalize_url(v: &str) -> String {
    decode_entities(v).chars().filter(|c| !c.is_control() && !c.is_whitespace()).map(|c| if c == '\\' { '/' } else { c }).collect::<String>().to_lowercase()
}

fn scheme_of(n: &str) -> Option<&str> {
    let (s, _) = n.split_once(':')?;
    (!s.is_empty() && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))).then_some(s)
}

/// 안전한 `data:` 형식(이미지·글꼴)인가. SVG와 문서는 스크립트를 품을 수 있어 아니다.
fn safe_data_url(n: &str) -> bool {
    let mime = n.strip_prefix("data:").unwrap_or("").split([';', ',']).next().unwrap_or("");
    matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp") || mime.starts_with("font/") || mime.starts_with("application/font-") || mime.starts_with("application/x-font-")
}

/// URL을 남겨도 되는가. `resource`는 클릭 없이 자동으로 불러오는 자리다: 원격 주소도 남기지 않는다.
fn keep_url(value: &str, resource: bool) -> bool {
    if decode_entities(value).trim().starts_with("\\\\") {
        return false;
    }
    let n = normalize_url(value);
    if n.is_empty() {
        return true;
    }
    if n.starts_with("//") {
        return !resource;
    }
    match scheme_of(&n) {
        None => true,
        Some("http" | "https") => !resource,
        Some("data") => safe_data_url(&n),
        Some(s) if s.len() == 1 => true,
        Some(s) => SAFE_SCHEMES.contains(&s),
    }
}

fn is_resource_attr(tag: &str, attr: &str) -> bool {
    matches!(
        (tag, attr),
        ("img", "src" | "srcset")
            | ("link", "href")
            | ("video" | "audio" | "source" | "track" | "embed" | "input" | "frame" | "iframe", "src")
            | ("object", "data")
            | ("video", "poster")
            | ("image" | "use" | "feimage" | "script", "href")
            | (_, "background")
    )
}

// ---------------------------------------------------------------- CSS

/// CSS 이스케이프(`\75`, `\75 `, `\u`)를 푼다. 줄바꿈 앞의 `\`는 이어 붙임이라 지운다.
fn css_unescape(css: &str) -> String {
    // CSS 규격의 입력 전처리: CRLF·CR·FF는 LF 하나로, NUL은 U+FFFD로 바꾼다. 이스케이프 뒤의 CRLF가 공백 하나로 먹히게 한다.
    let normalized = css.replace("\r\n", "\n").replace(['\r', '\x0c'], "\n").replace('\0', "\u{FFFD}");
    let chars: Vec<char> = normalized.chars().collect();
    let mut out = String::with_capacity(css.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if chars[j] == '\n' {
            i = j + 1;
            continue;
        }
        let start = j;
        while j < chars.len() && j - start < 6 && chars[j].is_ascii_hexdigit() {
            j += 1;
        }
        if j > start {
            let code = u32::from_str_radix(&chars[start..j].iter().collect::<String>(), 16).unwrap_or(0xFFFD);
            out.push(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{FFFD}'));
            if j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
        } else {
            out.push(chars[j]);
            j += 1;
        }
        i = j;
    }
    out
}

/// CSS에서 원격 로드, 스크립트 훅(`expression()`, `-moz-binding`, `behavior`)을 지운다.
/// 이스케이프로 감춘 위험(`\75rl(...)`)은 이스케이프를 푼 글로 찾는다. 푼 글에서 지울 것이 나오면 푼 글을 정리해 쓰고,
/// 아니면 원문을 그대로 둔다(한자 글꼴 이름의 `\5B8B` 같은 정상 이스케이프를 보존한다).
pub fn strip_css(css: &str) -> String {
    if css.contains('\\') {
        let unescaped = css_unescape(css);
        let cleaned = strip_css_plain(&unescaped);
        if cleaned != unescaped {
            return cleaned;
        }
    }
    strip_css_plain(css)
}

fn strip_css_plain(css: &str) -> String {
    let no_import = CSS_IMPORT.replace_all(css, |c: &regex::Captures| {
        let m = &c[0];
        let n = normalize_url(m);
        if n.contains("//") || n.contains("javascript:") || n.contains("vbscript:") || n.contains("data:") { String::new() } else { m.to_owned() }
    });
    let no_url = CSS_URL.replace_all(&no_import, |c: &regex::Captures| {
        let target = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map_or("", |m| m.as_str());
        if keep_url(target, true) { c[0].to_owned() } else { "url()".to_owned() }
    });
    // `image-set()`의 문자열 인자와 `url()`은 모두 URL이다. 하나라도 남기면 안 되는 주소면 함수째 `none`으로 바꾼다.
    let no_set = CSS_IMAGE_SET.replace_all(&no_url, |c: &regex::Captures| {
        let all_local = CSS_STRING.captures_iter(&c[0]).all(|s| keep_url(s.get(1).or_else(|| s.get(2)).map_or("", |m| m.as_str()), true)) && !c[0].to_ascii_lowercase().contains("//");
        if all_local { c[0].to_owned() } else { "none".to_owned() }
    });
    let no_expr = CSS_EXPRESSION.replace_all(&no_set, "");
    CSS_HOOKS.replace_all(&no_expr, "").into_owned()
}

/// 속성값을 큰따옴표 안에 쓸 수 있게 이스케이프한다. 작은따옴표와 `>`도 이스케이프해서 어떤 따옴표 규칙으로 읽어도 값 밖으로 나가지 못한다.
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ---------------------------------------------------------------- 마크업

struct Attr<'a> {
    name: &'a str,
    value: Option<&'a str>,
    start: usize,
    end: usize,
    vstart: usize,
    vend: usize,
}

struct StartTag<'a> {
    text: &'a str,
    name: &'a str,
    attrs: Vec<Attr<'a>>,
    self_closing: bool,
    terminated: bool,
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase()
}

/// 시작 태그의 길이와 `>`로 닫혔는지. 따옴표는 `=` 바로 뒤에서 열릴 때만 따옴표로 본다(`it's` 같은 글이 태그를 삼키지 않게).
fn tag_len(rest: &str) -> (usize, bool) {
    let b = rest.as_bytes();
    let (mut quote, mut prev) = (0u8, 0u8);
    for (i, &c) in b.iter().enumerate().skip(1) {
        if quote != 0 {
            if c == quote {
                quote = 0;
                prev = c;
            }
            continue;
        }
        if (c == b'"' || c == b'\'') && prev == b'=' {
            quote = c;
        } else if c == b'>' {
            return (i + 1, true);
        }
        if !c.is_ascii_whitespace() {
            prev = c;
        }
    }
    (b.len(), false)
}

fn parse_start_tag(rest: &str) -> Option<StartTag<'_>> {
    if !rest.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let (len, terminated) = tag_len(rest);
    let text = &rest[..len];
    let b = text.as_bytes();
    let mut i = 1;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'/' && b[i] != b'>' {
        i += 1;
    }
    let name = &text[1..i];
    let body_end = if terminated { b.len() - 1 } else { b.len() };
    let trimmed = text[..body_end].trim_end();
    let self_closing = terminated && trimmed.ends_with('/');
    let limit = if self_closing { trimmed.len() - 1 } else { body_end };
    let mut attrs = Vec::new();
    while i < limit {
        while i < limit && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        if i >= limit {
            break;
        }
        let start = i;
        while i < limit && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let attr_name = &text[start..i];
        let mut j = i;
        while j < limit && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < limit && b[j] == b'=' {
            j += 1;
            while j < limit && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let (vstart, vend, end) = if j < limit && (b[j] == b'"' || b[j] == b'\'') {
                let q = b[j];
                let close = text[j + 1..limit].find(q as char).map_or(limit, |k| j + 1 + k);
                (j + 1, close, (close + 1).min(limit))
            } else {
                let mut k = j;
                while k < limit && !b[k].is_ascii_whitespace() {
                    k += 1;
                }
                (j, k, k)
            };
            attrs.push(Attr { name: attr_name, value: Some(&text[vstart..vend]), start, end, vstart, vend });
            i = end;
        } else {
            attrs.push(Attr { name: attr_name, value: None, start, end: i, vstart: i, vend: i });
        }
    }
    Some(StartTag { text, name, attrs, self_closing, terminated })
}

/// `from`부터 같은 이름의 요소가 닫힐 때까지의 끝 위치. 같은 이름이 안에 중첩되면 깊이를 센다. 닫히지 않으면 문서 끝이다.
fn skip_element(src: &str, from: usize, full_name: &str) -> usize {
    let lower = src[from..].to_ascii_lowercase();
    let (open, close) = (format!("<{}", full_name.to_ascii_lowercase()), format!("</{}", full_name.to_ascii_lowercase()));
    let name_char = |b: Option<&u8>| b.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b':'));
    let (mut depth, mut pos) = (1usize, 0usize);
    loop {
        let (o, c) = (lower[pos..].find(&open), lower[pos..].find(&close));
        match (o, c) {
            (_, None) => return src.len(),
            (Some(o), Some(c)) if o < c => {
                let at = pos + o + open.len();
                if !name_char(lower.as_bytes().get(at)) {
                    let (len, _) = tag_len(&lower[pos + o..]);
                    if !lower[pos + o..pos + o + len].trim_end_matches('>').trim_end().ends_with('/') {
                        depth += 1;
                    }
                }
                pos = at;
            }
            (_, Some(c)) => {
                let at = pos + c + close.len();
                if name_char(lower.as_bytes().get(at)) {
                    pos = at;
                    continue;
                }
                depth -= 1;
                pos = at;
                if depth == 0 {
                    return from + lower[pos..].find('>').map_or(lower.len(), |g| pos + g + 1);
                }
            }
        }
    }
}

/// 선언(`<!DOCTYPE …>`)의 끝. 내부 부분집합(`[ … ]`) 안의 `>`는 끝이 아니다.
fn decl_end(rest: &str) -> usize {
    let (mut depth, mut quote) = (0i32, 0u8);
    for (i, &c) in rest.as_bytes().iter().enumerate() {
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
            continue;
        }
        match c {
            b'"' | b'\'' => quote = c,
            b'[' => depth += 1,
            b']' => depth -= 1,
            b'>' if depth <= 0 => return i + 1,
            _ => {}
        }
    }
    rest.len()
}

fn simplified_doctype(decl: &str) -> String {
    let name: String = decl["<!doctype".len()..].trim_start().chars().take_while(|c| !c.is_whitespace() && !matches!(c, '[' | '>' | '/')).collect();
    format!("<!DOCTYPE {}>", if name.is_empty() { "html" } else { &name })
}

fn rewrite_tag(tag: &StartTag<'_>, lname: &str, changes: &mut Changes) -> String {
    // (시작, 끝, 대체) 편집. 지우는 속성은 앞의 공백도 함께 지운다.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let text = tag.text;
    let drop_attr = |a: &Attr<'_>| {
        let mut s = a.start;
        while s > 0 && text.as_bytes()[s - 1].is_ascii_whitespace() {
            s -= 1;
        }
        (s, a.end, String::new())
    };
    let quoted = |a: &Attr<'_>| a.vstart > 0 && matches!(text.as_bytes()[a.vstart - 1], b'"' | b'\'');
    for a in &tag.attrs {
        let an = local(a.name);
        let value = a.value.unwrap_or("");
        let edits_before = edits.len();
        if a.name.contains(['<', '"', '\'']) {
            // 이름에 `<`나 따옴표가 든 속성은 파서마다 다르게 읽는다.
            edits.push(drop_attr(a));
            changes.add("비정상 속성 이름");
        } else if an.len() >= 4 && an.starts_with("on") && an[2..].bytes().all(|b| b.is_ascii_lowercase()) {
            edits.push(drop_attr(a));
            changes.add("이벤트 핸들러 속성");
        } else if matches!(an.as_str(), "srcdoc" | "formaction") {
            edits.push(drop_attr(a));
            changes.add("위험 속성");
        } else if an == "style" && a.value.is_some() {
            let decoded = decode_entities(value);
            let cleaned = strip_css(&decoded);
            if cleaned != decoded {
                // 원래 따옴표 종류(또는 따옴표 없음)와 무관하게 큰따옴표로 감싼 값으로 통째로 바꾼다.
                let (from, to) = if quoted(a) { (a.vstart - 1, a.end.max(a.vend)) } else { (a.vstart, a.vend) };
                edits.push((from, to, format!("\"{}\"", escape_attr(&cleaned))));
                changes.add("style 속성 정리");
            }
        } else if URL_ATTRS.contains(&an.as_str()) && a.value.is_some() {
            let resource = is_resource_attr(lname, &an);
            let candidates: Vec<&str> = if an == "srcset" { value.split(',').filter_map(|c| c.split_whitespace().next()).collect() } else { vec![value] };
            if candidates.iter().any(|c| !keep_url(c, resource)) {
                edits.push(drop_attr(a));
                changes.add(if resource { "원격·위험 자원 속성" } else { "위험 링크 속성" });
            }
        }
        // 남기는 속성값의 `<`·`>`는 항상 이스케이프한다. `</noscript>`처럼 raw text 문맥을 닫는 글이 값 안에 숨지 못하게 한다.
        if edits.len() == edits_before && a.value.is_some() && value.contains(['<', '>']) {
            let escaped = value.replace('<', "&lt;").replace('>', "&gt;");
            if quoted(a) {
                edits.push((a.vstart, a.vend, escaped));
            } else {
                edits.push((a.vstart, a.vend, format!("\"{escaped}\"")));
            }
            changes.add("속성값 꺾쇠 이스케이프");
        }
    }
    if edits.is_empty() {
        return text.to_owned();
    }
    edits.sort_by_key(|e| e.0);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e, rep) in edits {
        if s < at {
            continue;
        }
        out.push_str(&text[at..s]);
        out.push_str(&rep);
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

/// HTML·XHTML·SVG·XML 텍스트에서 비렌더링 요소를 지운다. 같은 입력이면 같은 결과다.
pub fn strip_markup(src: &str, svg: bool) -> (String, Changes) {
    // 한 번 정리한 결과를 다시 정리해도 같아질 때까지 반복한다(정리 결과가 새 위험을 만들지 못하게 한다).
    let (mut text, mut changes) = strip_pass(src, svg);
    for _ in 0..4 {
        let (next, more) = strip_pass(&text, svg);
        if next == text {
            break;
        }
        text = next;
        changes.merge(more);
    }
    (text, changes)
}

fn strip_pass(src: &str, _svg: bool) -> (String, Changes) {
    let mut out = String::with_capacity(src.len());
    let mut ch = Changes::default();
    let mut i = 0;
    while i < src.len() {
        if src.as_bytes()[i] != b'<' {
            let j = src[i..].find('<').map_or(src.len(), |k| i + k);
            out.push_str(&src[i..j]);
            i = j;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<!--") {
            // 주석은 통째로 지운다. `<!-->`와 `<!--->`도 완전한 주석이다(`-->`를 `<!--`의 대시부터 찾는다).
            // 브라우저와 끝나는 곳이 달라도 지우는 쪽으로만 어긋나므로 안에 숨긴 마크업이 남지 않는다.
            let end = rest.find("-->").map_or(rest.len(), |k| k + 3);
            ch.add("주석");
            i += end;
        } else if rest.starts_with("<![CDATA[") {
            // CDATA는 마크업이 아니라 이스케이프한 글자로 남긴다.
            let close = rest.find("]]>");
            out.push_str(&escape_text(&rest[9..close.unwrap_or(rest.len())]));
            ch.add("CDATA 섹션");
            i += close.map_or(rest.len(), |k| k + 3);
        } else if rest.starts_with("<?") {
            // XML 선언만 문서 맨 앞에서 남긴다. 그 밖의 처리 명령은 브라우저처럼 첫 `>`에서 끝난 것으로 보고 지운다.
            let at_start = out.is_empty() || out == "\u{feff}";
            if at_start && let Some(m) = XML_DECL.find(rest) {
                out.push_str(m.as_str());
                i += m.end();
            } else {
                let end = rest.find('>').map_or(rest.len(), |k| k + 1);
                ch.add(if rest[..end].to_ascii_lowercase().starts_with("<?xml-stylesheet") { "xml-stylesheet" } else { "처리 명령" });
                i += end;
            }
        } else if rest.starts_with("<!") {
            // 브라우저는 모든 선언을 첫 `>`에서 끝낸다. DOCTYPE의 내부 부분집합(`[ … ]`)만 XML 규칙으로 통째로 지운다.
            let first_gt = rest.find('>').map_or(rest.len(), |k| k + 1);
            if rest[..first_gt].to_ascii_lowercase().starts_with("<!doctype") {
                if rest[..first_gt].contains('[') {
                    let end = decl_end(rest);
                    out.push_str(&simplified_doctype(&rest[..end]));
                    ch.add("DOCTYPE 내부 부분집합");
                    i += end;
                } else {
                    out.push_str(&rest[..first_gt]);
                    i += first_gt;
                }
            } else {
                ch.add("알 수 없는 선언");
                i += first_gt;
            }
        } else if rest.starts_with("</") {
            let end = rest.find('>').map_or(rest.len(), |k| k + 1);
            let name = local(rest[2..end].trim_end_matches('>').trim());
            if !(DROP_TAGS.contains(&name.as_str()) || REMOVE_ELEMENTS.contains(&name.as_str())) {
                out.push_str(&rest[..end]);
            }
            i += end;
        } else if let Some(tag) = parse_start_tag(rest) {
            i += tag.text.len();
            let lname = local(tag.name);
            let attr = |n: &str| tag.attrs.iter().find(|a| local(a.name) == n).and_then(|a| a.value);
            if !tag.terminated {
                ch.add("닫히지 않은 태그");
            } else if REMOVE_ELEMENTS.contains(&lname.as_str()) {
                ch.add(match lname.as_str() {
                    "script" => "script 요소",
                    "foreignobject" => "foreignObject 요소",
                    _ => "object·embed·iframe류 요소",
                });
                if !(tag.self_closing || lname == "embed") {
                    i = skip_element(src, i, tag.name);
                }
            } else if matches!(lname.as_str(), "animate" | "set" | "animatetransform" | "animatemotion") && attr("attributename").is_some_and(|v| local(v) == "href") {
                ch.add("href 재작성 애니메이션");
                if !tag.self_closing {
                    i = skip_element(src, i, tag.name);
                }
            } else if lname == "meta" && attr("http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh")) {
                ch.add("meta refresh");
            } else if lname == "link" && attr("href").is_some_and(|h| !keep_url(h, true)) {
                ch.add("원격 link");
            } else if DROP_TAGS.contains(&lname.as_str()) {
                ch.add(if lname == "base" { "base 요소" } else { "폼 요소" });
            } else if lname == "style" && !tag.self_closing {
                let close = src[i..].to_ascii_lowercase().find("</style").map_or(src.len(), |k| i + k);
                let css = &src[i..close];
                // XHTML·SVG에서는 `<style>` 안의 마크업도 실제로 파싱된다. CDATA 표지를 빼고도 `<`가 남으면 요소째 지운다.
                if css.replace("<![CDATA[", "").replace("]]>", "").contains('<') {
                    ch.add("style 내 마크업");
                    i = skip_element(src, i, tag.name);
                } else {
                    out.push_str(&rewrite_tag(&tag, &lname, &mut ch));
                    let cleaned = strip_css(css);
                    if cleaned != css {
                        ch.add("style 요소 정리");
                    }
                    out.push_str(&cleaned);
                    i = close;
                }
            } else {
                out.push_str(&rewrite_tag(&tag, &lname, &mut ch));
            }
        } else {
            out.push('<');
            i += 1;
        }
    }
    (out, ch)
}

// ---------------------------------------------------------------- 재구성

#[derive(Debug)]
pub struct Plan {
    /// 재구성한 EPUB 바이트.
    pub bytes: Vec<u8>,
    /// 지운 멤버와 이유.
    pub removed: Vec<(String, &'static str)>,
    /// 내용을 고친 멤버와 고친 종류.
    pub edited: Vec<(String, String)>,
}

pub(crate) fn ext_of(name: &str) -> String {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) data: Vec<u8>,
    pub(crate) method: CompressionMethod,
    pub(crate) modified: Option<zip::DateTime>,
}

/// 멤버 텍스트의 인코딩. 정리한 글을 같은 인코딩으로 되써서 원본의 인코딩 선언과 어긋나지 않게 한다.
#[derive(Clone, Copy)]
enum Enc {
    Utf8,
    Utf16 { le: bool },
}

/// UTF-8(BOM 유무)과 BOM이 있는 UTF-16을 읽는다. 그 밖(UTF-32, 다른 코드페이지)은 안전하게 정리할 수 없어 오류다.
fn decode_enc(name: &str, data: &[u8]) -> Result<(String, Enc), String> {
    let utf16 = |bytes: &[u8], le: bool| -> Result<(String, Enc), String> {
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&units).map(|t| (t, Enc::Utf16 { le })).map_err(|_| format!("{name}: UTF-16이 올바르지 않아 안전하게 정리할 수 없다"))
    };
    if data.starts_with(b"\xFF\xFE\x00\x00") || data.starts_with(b"\x00\x00\xFE\xFF") {
        return Err(format!("{name}: UTF-32라 안전하게 정리할 수 없다"));
    }
    if let Some(rest) = data.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, true);
    }
    if let Some(rest) = data.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, false);
    }
    String::from_utf8(data.to_vec()).map(|t| (t, Enc::Utf8)).map_err(|_| format!("{name}: UTF-8이 아니라 안전하게 정리할 수 없다"))
}

fn encode_enc(text: &str, enc: Enc) -> Vec<u8> {
    match enc {
        Enc::Utf8 => text.as_bytes().to_vec(),
        Enc::Utf16 { le } => {
            let mut out = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
            for unit in text.encode_utf16() {
                out.extend(if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
            }
            out
        }
    }
}

pub(crate) fn decode(name: &str, data: &[u8]) -> Result<String, String> {
    decode_enc(name, data).map(|(t, _)| t)
}

pub(crate) fn read_members(bytes: &[u8]) -> Result<Vec<Member>, String> {
    let (members, broken) = read_members_lenient(bytes)?;
    match broken.into_iter().next() {
        Some((_, message)) => Err(message),
        None => Ok(members),
    }
}

/// 읽을 수 없는 멤버: `(이름, 오류 메시지)`.
pub(crate) type BrokenMembers = Vec<(String, String)>;

/// 읽을 수 없는 멤버를 오류로 올리지 않고 `(이름, 오류 메시지)`로 모아 돌려준다. 이름을 알 수 없는 멤버나 크기 한도 초과는 여전히 오류다.
pub(crate) fn read_members_lenient(bytes: &[u8]) -> Result<(Vec<Member>, BrokenMembers), String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ZIP을 열 수 없다: {e}"))?;
    let mut members = Vec::new();
    let mut broken = Vec::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let known = zip.name_for_index(i).and_then(Result::ok).map(std::borrow::Cow::into_owned);
        let mut entry = match zip.by_index(i) {
            Ok(entry) => entry,
            Err(e) => match known {
                Some(name) => {
                    broken.push((name, format!("멤버 {i}를 읽을 수 없다: {e}")));
                    continue;
                }
                None => return Err(format!("멤버 {i}를 읽을 수 없다: {e}")),
            },
        };
        let raw = entry.name_raw().to_vec();
        let name = String::from_utf8_lossy(&raw).into_owned();
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAX_MEMBER {
            return Err(format!("{name}: 멤버가 너무 크다"));
        }
        let mut data = Vec::with_capacity(entry.size().min(MAX_MEMBER) as usize);
        if let Err(e) = (&mut entry).take(MAX_MEMBER + 1).read_to_end(&mut data) {
            broken.push((name.clone(), format!("{name}: 읽을 수 없다: {e}")));
            continue;
        }
        total += data.len() as u64;
        if data.len() as u64 > MAX_MEMBER || total > MAX_TOTAL {
            return Err(format!("{name}: 압축을 푼 크기가 한도를 넘는다"));
        }
        let kept_utf8 = std::str::from_utf8(&raw).is_ok();
        members.push(Member { name: if kept_utf8 { name } else { format!("\u{0}{name}") }, data, method: entry.compression(), modified: entry.last_modified() });
    }
    Ok((members, broken))
}

/// 이름이 UTF-8이 아닌 멤버는 이름을 그대로 다시 쓸 수 없다. `read_members`가 앞에 NUL을 붙여 표시한다.
fn undecodable(name: &str) -> bool {
    name.starts_with('\0')
}

const MARKUP_MEDIA: &[&str] =
    &["application/xhtml+xml", "text/html", "image/svg+xml", "application/xml", "text/xml", "application/x-dtbncx+xml", "application/oebps-package+xml", "application/smil+xml", "application/smil"];

/// `media-type` 값을 비교할 수 있게 정규화한다: `;` 뒤의 매개변수를 떼고 앞뒤 공백을 지우고 소문자로 만든다.
fn base_media(raw: &str) -> String {
    raw.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// 리더가 마크업으로 여는 형식: 알려진 형식, `+xml` 접미사, `text/plain`·`text/css`가 아닌 `text/*`.
fn is_markup_media(media: &str) -> bool {
    MARKUP_MEDIA.contains(&media) || media.ends_with("+xml") || (media.starts_with("text/") && !matches!(media, "text/plain" | "text/css") && !is_script_media(media))
}

fn is_script_media(media: &str) -> bool {
    media.contains("javascript")
        || media.contains("ecmascript")
        || matches!(
            media,
            "application/java-archive" | "application/x-sh" | "application/x-msdownload" | "application/x-msdos-program" | "application/x-executable" | "application/x-bat" | "application/x-dosexec"
        )
}

/// 내용이 마크업처럼 시작하는가(BOM·앞 공백을 건너뛴다). 이미지·글꼴·미디어는 `<`로 시작하지 않는다.
/// UTF-16·UTF-32(BOM이 있든 없든)로 인코딩한 마크업도 알아본다.
fn looks_like_markup(data: &[u8]) -> bool {
    let head = &data[..data.len().min(256)];
    let first_char = |units: Vec<u32>| units.into_iter().filter_map(char::from_u32).find(|c| !c.is_whitespace());
    let utf16 = |bytes: &[u8], le: bool| -> Vec<u32> { bytes.chunks_exact(2).map(|c| u32::from(if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })).collect() };
    let utf32 =
        |bytes: &[u8], le: bool| -> Vec<u32> { bytes.chunks_exact(4).map(|c| if le { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) }).collect() };
    let decoded = if let Some(rest) = head.strip_prefix(b"\xFF\xFE\x00\x00") {
        first_char(utf32(rest, true))
    } else if let Some(rest) = head.strip_prefix(b"\x00\x00\xFE\xFF") {
        first_char(utf32(rest, false))
    } else if let Some(rest) = head.strip_prefix(b"\xFF\xFE") {
        first_char(utf16(rest, true))
    } else if let Some(rest) = head.strip_prefix(b"\xFE\xFF") {
        first_char(utf16(rest, false))
    } else if head.starts_with(b"<\x00\x00\x00") {
        first_char(utf32(head, true))
    } else if head.starts_with(b"\x00\x00\x00<") {
        first_char(utf32(head, false))
    } else if head.starts_with(b"<\x00") {
        first_char(utf16(head, true))
    } else if head.starts_with(b"\x00<") {
        first_char(utf16(head, false))
    } else {
        let d = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
        return d.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<');
    };
    decoded == Some('<')
}

static PACKAGE_ROOT: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)\A\s*(?:<\?xml[^>]*\?>\s*)?(?:<!--.*?-->\s*)*(?:<!DOCTYPE[^>]*>\s*)?<(?:\w+:)?package\b"));

/// 패키지 문서(OPF) 이름들: `.opf` 확장자, `META-INF/container.xml`의 rootfile `full-path`(그대로와 퍼센트 디코딩한 두 형태 모두),
/// 루트 요소가 `<package>`인 XML. 확장자만 믿으면 다른 이름의 패키지 문서가 manifest 정리와 검증에서 빠진다.
/// `container.xml`이 있는데 해석할 수 없거나 가리키는 멤버가 없으면 오류다: 무엇이 패키지인지 모르면 고치지 않는다.
pub(crate) fn package_names(members: &[Member]) -> Result<BTreeSet<String>, String> {
    let existing: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let mut set = BTreeSet::new();
    for m in members {
        let ext = ext_of(&m.name);
        if ext == "opf" {
            set.insert(m.name.clone());
        }
        if m.name == "META-INF/container.xml" {
            let text = decode(&m.name, &m.data).map_err(|e| format!("container.xml의 인코딩을 읽을 수 없다: {e}"))?;
            let doc = roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
                .map_err(|e| format!("META-INF/container.xml을 해석할 수 없다: {e}"))?;
            for root in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "rootfile") {
                let raw = root.attribute("full-path").unwrap_or("");
                let found: Vec<String> = [posix_normpath(raw), posix_normpath(&unquote(raw))].into_iter().filter(|p| existing.contains(p.as_str())).collect();
                if found.is_empty() {
                    return Err(format!("container.xml이 가리키는 패키지 문서가 없다: {raw}"));
                }
                set.extend(found);
            }
        } else if (XML_EXTS.contains(&ext.as_str()) || ext.is_empty() || ext == "txt") && PACKAGE_ROOT.is_match(&String::from_utf8_lossy(&m.data[..m.data.len().min(4096)])) {
            set.insert(m.name.clone());
        }
    }
    Ok(set)
}

/// 패키지 문서로 쓸 수 있는지 확인하고 해석에 쓴 글을 돌려준다. 실행 파일 확장자가 아니고, 읽을 수 있는 인코딩이고,
/// 내용이 정말 `<package>` 루트의 XML이어야 한다. 하나라도 어긋나면 오류다: 패키지 문서를 읽지 못하고 넘어가면 선언 형식을 놓친다.
pub(crate) fn package_text(m: &Member) -> Result<String, String> {
    let name = m.name.trim_start_matches('\0');
    if crate::epub::security_checker::EXEC_EXTS.contains(&ext_of(name).as_str()) {
        return Err(format!("{name}: 실행 파일 확장자인 패키지 문서"));
    }
    let (text, _) = decode_enc(name, &m.data)?;
    let text = text.trim_start_matches('\u{feff}').to_owned();
    if !PACKAGE_ROOT.is_match(&text) {
        return Err(format!("{name}: 컨테이너가 가리키는 패키지 문서가 `<package>` 문서가 아니다"));
    }
    let doc = roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).map_err(|e| format!("{name}: 패키지 문서를 해석할 수 없다: {e}"))?;
    if doc.root_element().tag_name().name() != "package" {
        return Err(format!("{name}: 루트 요소가 `<package>`가 아니다"));
    }
    drop(doc);
    Ok(text)
}

/// manifest가 선언한 형식. 멤버 경로(OPF 기준으로 푼 것) → 소문자 media-type. 패키지 문서를 읽지 못하면 오류다.
fn declared_media(members: &[Member], packages: &BTreeSet<String>) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    for m in members.iter().filter(|m| packages.contains(&m.name)) {
        let text = package_text(m)?;
        let doc = roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).map_err(|e| format!("{}: {e}", m.name))?;
        let base = posix_dirname(&m.name);
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
            let (Some(href), Some(media)) = (item.attribute("href"), item.attribute("media-type")) else { continue };
            map.insert(posix_normpath(&posix_join(&base, unquote(href).split('#').next().unwrap_or(""))), base_media(media));
        }
    }
    Ok(map)
}

fn classify(name: &str, data: &[u8], media: Option<&str>, is_package: bool) -> Option<&'static str> {
    if crate::zip_name_rules::check(name.trim_start_matches('\0')).iter().any(|f| f.severity >= Severity::High) {
        return Some("위험한 멤버 이름");
    }
    if media.is_some_and(is_script_media) {
        return Some("스크립트 형식으로 선언됨");
    }
    // 컨테이너가 가리키는 패키지 문서는 확장자와 무관하게 남긴다(마크업으로 정리한다).
    if is_package {
        return None;
    }
    let ext = ext_of(name);
    let passive = PASSIVE_EXTS.contains(&ext.as_str());
    // 리더는 확장자가 아니라 선언된 형식으로 연다. 이미지·글꼴 확장자가 마크업·CSS라고 선언했거나 내용이 마크업이면 지운다.
    if passive && (media.is_some_and(|m| is_markup_media(m) || m == "text/css") || looks_like_markup(data)) {
        return Some("확장자와 선언 형식 불일치(마크업 위장)");
    }
    let textual = HTML_EXTS.contains(&ext.as_str()) || XML_EXTS.contains(&ext.as_str()) || matches!(ext.as_str(), "css" | "svg" | "txt");
    // 스캐너(`check_magic`)와 같은 기준이다: `MZ`로 시작하면 헤더가 온전하지 않아도 실행 파일 후보로 본다.
    if !textual && (data.starts_with(b"MZ") || matches!(sniff(data), Sniffed::Pe | Sniffed::Elf | Sniffed::MachO)) {
        return Some("실행 코드 내용");
    }
    if name == "mimetype" || name.starts_with("META-INF/") || textual || PASSIVE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    Some(if crate::epub::security_checker::EXEC_EXTS.contains(&ext.as_str()) { "실행 파일 형식" } else { "허용 목록 밖 형식" })
}

/// OPF에서 지운 멤버의 manifest 항목, 원격·위험 항목, 낡은 `scripted`·`remote-resources` 선언을 지운다.
fn rewrite_opf(text: &str, opf_name: &str, removed: &BTreeSet<String>, changes: &mut Changes) -> String {
    let base = posix_dirname(opf_name);
    let pruned = OPF_ITEM.replace_all(text, |c: &regex::Captures| {
        let item = &c[0];
        let Some(h) = ATTR_HREF.captures(item) else { return item.to_owned() };
        let href = h.get(1).or_else(|| h.get(2)).map_or("", |m| m.as_str());
        let resolved = posix_normpath(&posix_join(&base, unquote(href).split('#').next().unwrap_or("")));
        if removed.contains(&resolved) || !keep_url(href, true) {
            changes.add("manifest 항목");
            String::new()
        } else {
            item.to_owned()
        }
    });
    ATTR_PROPERTIES
        .replace_all(&pruned, |c: &regex::Captures| {
            let value = c.get(1).or_else(|| c.get(2)).map_or("", |m| m.as_str());
            let kept: Vec<&str> = value.split_whitespace().filter(|p| !matches!(*p, "scripted" | "remote-resources")).collect();
            if kept.len() == value.split_whitespace().count() {
                c[0].to_owned()
            } else if kept.is_empty() {
                String::new()
            } else {
                format!(" properties=\"{}\"", kept.join(" "))
            }
        })
        .into_owned()
}

/// EPUB 바이트를 허용 목록으로 다시 만든다.
pub fn rebuild(src: &[u8]) -> Result<Plan, String> {
    let members = read_members(src)?;
    let packages = package_names(&members)?;
    let media = declared_media(&members, &packages)?;
    let mut removed: Vec<(String, &'static str)> = Vec::new();
    let mut kept: Vec<Member> = Vec::new();
    for m in members {
        match classify(&m.name, &m.data, media.get(m.name.trim_start_matches('\0')).map(String::as_str), packages.contains(&m.name)) {
            Some(reason) => removed.push((m.name.trim_start_matches('\0').to_owned(), reason)),
            None if undecodable(&m.name) => return Err(format!("{}: 이름이 UTF-8이 아니라 다시 쓸 수 없다", m.name.trim_start_matches('\0'))),
            None => kept.push(m),
        }
    }
    let removed_names: BTreeSet<String> = removed.iter().map(|(n, _)| n.clone()).collect();
    let mut edited: Vec<(String, String)> = Vec::new();
    for m in &mut kept {
        let ext = ext_of(&m.name);
        let is_opf = packages.contains(&m.name);
        let declared = media.get(m.name.as_str()).map(String::as_str);
        // 확장자와 선언 형식 중 하나라도 마크업이면 마크업으로, CSS면 CSS로 정리한다(둘 다일 수도 있다).
        let markup = is_opf || HTML_EXTS.contains(&ext.as_str()) || ext == "svg" || XML_EXTS.contains(&ext.as_str()) || declared.is_some_and(is_markup_media);
        let css = ext == "css" || declared == Some("text/css");
        if !markup && !css {
            continue;
        }
        let (original, enc) = decode_enc(&m.name, &m.data)?;
        let (mut text, mut changes) = if markup { strip_markup(&original, ext == "svg") } else { (original.clone(), Changes::default()) };
        if css {
            let cleaned = strip_css(&text);
            if cleaned != text {
                changes.add("CSS 정리");
                text = cleaned;
            }
        }
        let text = if is_opf {
            let mut more = Changes::default();
            let t = rewrite_opf(&text, &m.name, &removed_names, &mut more);
            changes.merge(more);
            t
        } else {
            text
        };
        let encoded = encode_enc(&text, enc);
        if encoded != m.data {
            edited.push((m.name.clone(), changes.summary()));
            m.data = encoded;
        }
    }
    let bytes = write_members(&mut kept)?;
    Ok(Plan { bytes, removed, edited })
}

/// 멤버를 EPUB ZIP으로 쓴다. `mimetype`은 항상 첫 멤버이고 압축하지 않는다.
pub(crate) fn write_members(kept: &mut [Member]) -> Result<Vec<u8>, String> {
    kept.sort_by_key(|m| m.name != "mimetype");
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for m in kept.iter() {
        let method = if m.name == "mimetype" || m.method == CompressionMethod::Stored { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        let mut opts = SimpleFileOptions::default().compression_method(method);
        if let Some(t) = m.modified {
            opts = opts.last_modified_time(t);
        }
        zip.start_file(m.name.as_str(), opts).map_err(|e| format!("{}: 쓸 수 없다: {e}", m.name))?;
        zip.write_all(&m.data).map_err(|e| e.to_string())?;
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    Ok(bytes)
}

// ---------------------------------------------------------------- 검증

/// 눈에 보이는 글자만 공백 없이 모은다. `text_of`는 HTML 규칙으로 읽어서 XHTML의 자기 닫힘 `<script src=".."/>`를 문서 끝까지 이어지는
/// 스크립트로 보고 본문을 버리므로, 자기 닫힘 `script`·`style`을 먼저 펼친다. 스크립트나 주석을 지우면 이어 붙는 텍스트 노드의
/// 경계 공백도 달라지므로 공백은 글자로 세지 않는다.
pub(crate) fn visible_text(markup: &str) -> String {
    let expanded = SELF_CLOSING_RAW.replace_all(markup, "<$1$2></$1>");
    text_of(&expanded).chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

/// OPF manifest 항목 중 대상이 없는 수와 spine에서 manifest에 없는 idref 수.
pub(crate) fn dangling(members: &[Member]) -> Result<(usize, usize), String> {
    let names: BTreeSet<&str> = members.iter().map(|m| m.name.as_str()).collect();
    let (mut missing, mut bad_spine) = (0, 0);
    let packages = package_names(members)?;
    for m in members.iter().filter(|m| packages.contains(&m.name)) {
        let text = package_text(m)?;
        let doc = roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).map_err(|e| format!("{}: OPF를 해석할 수 없다: {e}", m.name))?;
        let base = posix_dirname(&m.name);
        let mut ids = BTreeSet::new();
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
            ids.insert(item.attribute("id").unwrap_or("").to_owned());
            let href = item.attribute("href").unwrap_or("");
            if keep_url(href, true) && scheme_of(&normalize_url(href)).is_none() {
                let resolved = posix_normpath(&posix_join(&base, unquote(href).split('#').next().unwrap_or("")));
                if !names.contains(resolved.as_str()) {
                    missing += 1;
                }
            }
        }
        bad_spine += doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "itemref").filter(|n| !ids.contains(n.attribute("idref").unwrap_or(""))).count();
    }
    Ok((missing, bad_spine))
}

/// 재구성본이 원본의 읽을 수 있는 내용을 지키는지 본다: 열리는지, 본문 글자가 줄기만 했는지, manifest·spine이 더 깨지지 않았는지.
pub fn verify(orig: &[u8], new: &[u8]) -> Result<(), String> {
    let (old_members, new_members) = (read_members(orig)?, read_members(new)?);
    if let Some(first) = ZipArchive::new(Cursor::new(new)).map_err(|e| e.to_string())?.by_index(0).ok().map(|f| f.name().map(|n| n.into_owned()).unwrap_or_default())
        && old_members.iter().any(|m| m.name == "mimetype")
        && first != "mimetype"
    {
        return Err("mimetype이 첫 멤버가 아니다".into());
    }
    let old_by_name: BTreeMap<&str, &Member> = old_members.iter().map(|m| (m.name.as_str(), m)).collect();
    for m in new_members.iter().filter(|m| HTML_EXTS.contains(&ext_of(&m.name).as_str())) {
        let Some(old) = old_by_name.get(m.name.as_str()) else { return Err(format!("{}: 원본에 없는 본문 멤버", m.name)) };
        let (o, n) = (visible_text(&decode(&old.name, &old.data)?), visible_text(&decode(&m.name, &m.data)?));
        let lost = o.chars().count().saturating_sub(n.chars().count());
        if !is_subsequence(&n, &o) || lost > (o.chars().count() / 200).max(100) {
            return Err(format!("{}: 본문 텍스트가 달라졌다(잃은 글자 {lost})", m.name));
        }
    }
    let (old_dangling, new_dangling) = (dangling(&old_members)?, dangling(&new_members)?);
    if new_dangling.0 > old_dangling.0 || new_dangling.1 > old_dangling.1 {
        return Err(format!("manifest·spine 정합성이 나빠졌다: {old_dangling:?} → {new_dangling:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------- 실행

fn unique_trash_path(dir: &Path, file_name: &str) -> PathBuf {
    let first = dir.join(file_name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = file_name.rsplit_once('.').map_or((file_name, String::new()), |(s, e)| (s, format!(".{e}")));
    (1..).map(|n| dir.join(format!("{stem}.{n}{ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

/// 원본을 휴지통에 두고, 같은 inode에 새 내용을 덮어쓰고, mtime을 되돌린다.
pub(crate) fn apply(path: &Path, bytes: &[u8], trash_dir: &Path) -> io::Result<PathBuf> {
    let meta = fs::metadata(path)?;
    let mtime = meta.modified()?;
    fs::create_dir_all(trash_dir)?;
    let trash = unique_trash_path(trash_dir, &path.file_name().map_or_else(|| "book.epub".into(), |n| n.to_string_lossy().into_owned()));
    fs::copy(path, &trash)?;
    if fs::metadata(&trash)?.len() != meta.len() {
        return Err(io::Error::other("휴지통 사본의 크기가 원본과 다르다"));
    }
    fs::OpenOptions::new().write(true).open(&trash)?.set_modified(mtime)?;
    let mut f = fs::OpenOptions::new().write(true).open(path)?;
    f.write_all(bytes)?;
    f.set_len(bytes.len() as u64)?;
    f.sync_all()?;
    f.set_modified(mtime)?;
    if fs::metadata(path)?.ino() != meta.ino() {
        return Err(io::Error::other("inode가 바뀌었다"));
    }
    Ok(trash)
}

/// 터미널로 나가는 문자열의 제어문자(ESC 등)와 숨은 서식 문자를 지운다. 멤버 이름과 오류 메시지는 파일이 정한 값이다.
pub(crate) fn show(text: &str) -> String {
    crate::output_formatter::sanitize_display(text, usize::MAX, None)
}

pub(crate) fn has_serious_security(report: &crate::FileReport) -> bool {
    report.findings.iter().any(|f| f.category == Category::Security && f.severity >= Severity::High)
}

/// 임시 파일을 `O_EXCL`(`create_new`)과 모드 0600으로 만든다. 이름이 겹치거나 링크가 이미 있으면 다른 이름으로 다시 시도한다.
/// 파일을 새로 만든 경우에만 쓰므로 미리 심어 둔 심볼릭 링크를 따라가지 않는다.
pub(crate) fn create_private_temp(bytes: &[u8]) -> io::Result<PathBuf> {
    create_private_temp_ext(bytes, "epub")
}

/// `create_private_temp`와 같고 확장자를 고른다. 검사는 확장자로 형식을 가르므로 PDF는 `pdf`로 만든다.
pub(crate) fn create_private_temp_ext(bytes: &[u8], ext: &str) -> io::Result<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir();
    for _ in 0..100 {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
        let path = dir.join(format!("bookdoctor-sanitize-{}-{nanos:x}-{}.{ext}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(mut f) => {
                f.write_all(bytes)?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("임시 파일 이름을 정하지 못했다"))
}

/// 재구성본을 임시 파일에 써서 다시 검사한다. 보안 지표(HIGH 이상)가 하나라도 남으면 오류다: 허용 목록으로 고치지 못한 문제가 있는 파일은 적용하지 않는다.
fn rescan(bytes: &[u8], opts: &Options) -> Result<(), String> {
    let tmp = create_private_temp(bytes).map_err(|e| e.to_string())?;
    let report = scan_file(&tmp, opts);
    let _ = fs::remove_file(&tmp);
    let left: Vec<String> = report.findings.iter().filter(|f| f.category == Category::Security && f.severity >= Severity::High).map(|f| f.code.to_string()).collect();
    if left.is_empty() { Ok(()) } else { Err(format!("재검사에서 보안 지표가 남았다: {}", left.join(", "))) }
}

/// `--sanitize` 진입점. 종료 코드를 돌려준다(건너뛴 파일이 있어도 0, 쓰기 실패가 있으면 1).
pub fn run(opts: &Options, out: &mut dyn Write) -> io::Result<i32> {
    let paths = crate::cli_parser::collect(opts, |p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub") || e.eq_ignore_ascii_case("pdf")));
    let scan_opts = opts.clone();
    let (mut done, mut untouched, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize);
    for path in &paths {
        let shown = show(&path.to_string_lossy());
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            match crate::pdf::pdf_sanitizer::process(path, &shown, opts, &scan_opts, out)? {
                crate::pdf::pdf_sanitizer::Outcome::Done => done += 1,
                crate::pdf::pdf_sanitizer::Outcome::Untouched => untouched += 1,
                crate::pdf::pdf_sanitizer::Outcome::Skipped => skipped += 1,
                crate::pdf::pdf_sanitizer::Outcome::Failed => failed += 1,
            }
            continue;
        }
        let report = scan_file(path, &scan_opts);
        if !has_serious_security(&report) {
            untouched += 1;
            continue;
        }
        let planned = fs::read(path).map_err(|e| e.to_string()).and_then(|src| rebuild(&src).map(|plan| (src, plan)));
        let (src, plan) = match planned {
            Ok(v) => v,
            Err(e) => {
                writeln!(out, "SKIP {shown}: {}", show(&e))?;
                skipped += 1;
                continue;
            }
        };
        if plan.removed.is_empty() && plan.edited.is_empty() {
            writeln!(out, "UNCHANGED {shown}: 지울 요소가 없다(남은 지표는 허용 목록으로 해결되지 않는다)")?;
            untouched += 1;
            continue;
        }
        if let Err(e) = verify(&src, &plan.bytes).and_then(|()| rescan(&plan.bytes, &scan_opts)) {
            writeln!(out, "SKIP {shown}: 검증 실패: {}", show(&e))?;
            skipped += 1;
            continue;
        }
        writeln!(out, "SANITIZED {shown}: 멤버 {}개 제거, {}개 수정", plan.removed.len(), plan.edited.len())?;
        for (name, why) in &plan.removed {
            writeln!(out, "  - 제거 {} ({why})", show(name))?;
        }
        for (name, what) in &plan.edited {
            writeln!(out, "  - 수정 {} ({what})", show(name))?;
        }
        {
            let trash = opts.trash_dir();
            match apply(path, &plan.bytes, &trash) {
                Ok(saved) => writeln!(out, "  - 원본 보관 {}", show(&saved.to_string_lossy()))?,
                Err(e) => {
                    writeln!(out, "FAIL {shown}: {}", show(&e.to_string()))?;
                    failed += 1;
                    continue;
                }
            }
        }
        done += 1;
    }
    writeln!(out, "검사 {}건, 재구성 {done}건, 대상 아님·변경 없음 {untouched}건, 건너뜀 {skipped}건, 실패 {failed}건", paths.len())?;
    Ok(i32::from(failed > 0))
}
