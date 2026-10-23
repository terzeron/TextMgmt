//! EPUB 정적 위험 검사. 스크립트·활성 콘텐츠·위험한 ZIP 구조를 찾는다.
//!
//! 렌더링하거나 스크립트를 실행하거나 첨부를 열지 않는다. 문자열과 구조만 본다.
//! "지표 없음"은 안전하다는 보증이 아니다. 한도(`Limits`)를 넘어 못 본 부분은 `notes`에 남기고
//! 상태를 `INCOMPLETE`로 표시한다.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::LazyLock;

use detect_mojibake::reader::{Zip, open_zip};
use detect_mojibake::util::{posix_dirname, posix_join, posix_normpath, unquote};
use regex::Regex;
use serde::Serialize;
use zip::CompressionMethod;

const MAX_TEXT: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    None,
    Findings,
    Incomplete,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub location: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecurityReport {
    pub file: String,
    pub status: Status,
    pub entries: usize,
    pub inspected_bytes: u64,
    pub findings: Vec<Finding>,
    /// 한도 초과나 읽기 실패로 검사하지 못한 부분.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// 멤버 하나에서 읽는 최대 압축 해제 바이트.
    pub max_member: u64,
    /// 파일 전체에서 읽는 최대 압축 해제 바이트.
    pub max_total: u64,
    pub max_files: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_member: 20 * 1024 * 1024, max_total: 200 * 1024 * 1024, max_files: 10_000 }
    }
}

// ---------------------------------------------------------------- 상수

const EXEC_EXTS: &[&str] = &[
    "exe", "dll", "scr", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "jse", "wsf", "wsh", "hta", "msi", "msp", "jar", "lnk", "reg", "cpl", "sh", "bash", "zsh", "py", "pl", "rb", "php", "apk",
    "app", "dmg", "so", "dylib", "bin", "run", "swf", "class",
];
const HTML_EXTS: &[&str] = &["xhtml", "html", "htm", "svg"];
const XML_EXTS: &[&str] = &["xml", "opf", "ncx", "xsl", "xslt", "smil", "dtd"];
const WIN_RESERVED: &[&str] =
    &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];
const DANGEROUS_SCHEMES: &[&str] = &[
    "file",
    "smb",
    "ms-msdt",
    "search-ms",
    "ms-officecmd",
    "ms-word",
    "ms-excel",
    "ms-powerpoint",
    "ms-appinstaller",
    "ms-cxh",
    "mhtml",
    "jar",
    "shell",
    "ssh",
    "ftp",
    "sftp",
    "telnet",
    "ldap",
    "vnc",
    "rdp",
];
const SAFE_SCHEMES: &[&str] = &["http", "https", "mailto", "tel", "sms", "epub", "cid", "urn", "tag", "data"];
const SCRIPT_MEDIA_TYPES: &[&str] = &[
    "application/javascript",
    "text/javascript",
    "application/x-javascript",
    "application/ecmascript",
    "text/ecmascript",
    "application/x-sh",
    "application/x-msdownload",
    "application/x-msdos-program",
    "application/x-executable",
    "application/java-archive",
    "application/x-bat",
];
const JS_SCRIPT_TYPES: &[&str] = &["", "text/javascript", "application/javascript", "module", "text/ecmascript", "application/ecmascript", "text/jscript", "jscript", "application/x-javascript"];
const URL_ATTRS: &[&str] = &["href", "src", "action", "formaction", "xlink:href", "background", "poster", "data", "codebase", "manifest", "ping"];

// ---------------------------------------------------------------- 정규식

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("잘못된 정규식 {p}: {e}"))
}

type Rule = (Severity, &'static str, &'static str, LazyLock<Regex>);

macro_rules! rule {
    ($sev:expr, $code:expr, $detail:expr, $pat:expr) => {
        ($sev, $code, $detail, LazyLock::new(|| re($pat)))
    };
}

static JS_RULES: [Rule; 9] = [
    rule!(Severity::High, "DYNAMIC_CODE_EXEC", "eval/Function/string timer", r#"\beval\s*\(|\bFunction\s*\(|\b(?:setTimeout|setInterval)\s*\(\s*["'`]"#),
    rule!(Severity::Medium, "DOCUMENT_WRITE", "document.write", r"\bdocument\s*\.\s*write(?:ln)?\s*\("),
    rule!(Severity::Medium, "NETWORK_API", "network API", r"\bXMLHttpRequest\b|\bfetch\s*\(|\bnew\s+WebSocket\b|\bsendBeacon\b|\bEventSource\b"),
    rule!(Severity::Medium, "NAVIGATION", "window.open or location change", r"\bwindow\s*\.\s*open\s*\(|\blocation\s*\.\s*(?:href|replace|assign)\b|\b(?:top|parent)\s*\.\s*location\b"),
    rule!(
        Severity::High,
        "HOST_API",
        "host/shell API",
        r#"\bActiveXObject\b|\bWScript\b|Shell\.Application|\brequire\s*\(\s*["'](?:child_process|fs|os|net|http)["']\s*\)|\bprocess\s*\.\s*binding\b"#
    ),
    rule!(Severity::Low, "STORAGE_API", "storage or cookie access", r"\blocalStorage\b|\bsessionStorage\b|\bdocument\s*\.\s*cookie\b|\bindexedDB\b"),
    rule!(Severity::Medium, "DYNAMIC_IMPORT", "dynamic import", r"\bimportScripts\s*\(|\bimport\s*\("),
    rule!(Severity::Medium, "OBFUSCATION", "decode/charcode call", r"\batob\s*\(|\bunescape\s*\(|String\s*\.\s*fromCharCode\s*\((?:[^)]*,){8,}"),
    rule!(Severity::Medium, "OBFUSCATION", "long escape run", r"(?:\\x[0-9a-fA-F]{2}){16,}|(?:\\u[0-9a-fA-F]{4}){12,}"),
];

static CSS_RULES: [Rule; 6] = [
    rule!(Severity::High, "CSS_EXPRESSION", "CSS expression()", r"(?i)expression\s*\("),
    rule!(Severity::High, "CSS_MOZ_BINDING", "-moz-binding", r"(?i)-moz-binding"),
    rule!(Severity::Medium, "CSS_BEHAVIOR", "behavior property", r"(?i)behavior\s*:"),
    rule!(Severity::Medium, "REMOTE_CSS_IMPORT", "remote @import", r#"(?i)@import\s+(?:url\(\s*)?["']?\s*(?:https?:)?//"#),
    rule!(Severity::High, "JS_URL", "javascript: in url()", r#"(?i)url\(\s*["']?\s*(?:javascript|vbscript)\s*:"#),
    rule!(Severity::Low, "REMOTE_RESOURCE", "remote url()", r#"(?i)url\(\s*["']?\s*(?:https?:)?//"#),
];

static CSS_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)/\*.*?\*/"));
static ENTITY_DECL: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<!ENTITY\s+[^>]*>"));
static EXTERNAL_ID: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(?:SYSTEM|PUBLIC)\b"));
static DOCTYPE_EXTERNAL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)<!DOCTYPE\s+[^\[>]*?\b(?:SYSTEM|PUBLIC)\b([^\[>]*)"#));
static XML_STYLESHEET: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<\?xml-stylesheet\b[^?]*\?>"));
static REFRESH_URL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)url\s*=\s*["']?\s*([^"'\s;]+)"#));
static SCHEME: LazyLock<Regex> = LazyLock::new(|| re(r"^([a-z][a-z0-9+.\-]*):"));
static KNOWN_PUBLIC_IDS: &[&str] = &["-//W3C//DTD", "-//NISO//DTD", "-//IDPF//DTD", "-//OEBPS//DTD", "-//W3C//ENTITIES"];

// ---------------------------------------------------------------- 출력 안전

/// 보고서에 들어가는 문자열: 제어문자(C0·C1)는 `.`로 바꾸고 길이를 자른다.
fn safe(s: &str) -> String {
    s.chars().take(MAX_TEXT).map(|c| if c.is_control() { '.' } else { c }).collect()
}

// ---------------------------------------------------------------- HTML 토큰화

#[derive(Debug)]
struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    closing: bool,
    /// `<script>`·`<style>` 안의 원시 텍스트.
    body: Option<String>,
}

impl Tag {
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn parse_tag(text: &str, start: usize) -> (Tag, usize) {
    let bytes = text.as_bytes();
    let closing = bytes[start + 1] == b'/';
    let mut i = start + 1 + usize::from(closing);
    let name_start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'/' | b'>') {
        i += 1;
    }
    let name = text[name_start..i].to_lowercase();
    let mut attrs = Vec::new();
    while i < bytes.len() && bytes[i] != b'>' {
        if bytes[i].is_ascii_whitespace() || bytes[i] == b'/' {
            i += 1;
            continue;
        }
        let ns = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let key = text[ns..i].to_lowercase();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'"' | b'\'') {
                let q = bytes[i];
                let vs = i + 1;
                i = vs;
                while i < bytes.len() && bytes[i] != q {
                    i += 1;
                }
                value = text[vs..i].to_owned();
                i = (i + 1).min(bytes.len());
            } else {
                let vs = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = text[vs..i].to_owned();
            }
        }
        attrs.push((key, value));
    }
    (Tag { name, attrs, closing, body: None }, (i + 1).min(bytes.len()))
}

/// 태그 목록. 주석·CDATA·선언은 건너뛰고, 닫히지 않은 주석은 나머지를 주석으로 본다(브라우저와 같다).
fn tokenize(text: &str) -> Vec<Tag> {
    let bytes = text.as_bytes();
    let lower = text.to_ascii_lowercase();
    let mut tags = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find('<') {
        i += off;
        let rest = &text[i..];
        if rest.starts_with("<!--") {
            match text[i + 4..].find("-->") {
                Some(e) => i += 4 + e + 3,
                None => break,
            }
        } else if rest.starts_with("<![CDATA[") {
            match text[i..].find("]]>") {
                Some(e) => i += e + 3,
                None => break,
            }
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            match text[i..].find('>') {
                Some(e) => i += e + 1,
                None => break,
            }
        } else if bytes.get(i + 1).is_some_and(u8::is_ascii_alphabetic) || (bytes.get(i + 1) == Some(&b'/') && bytes.get(i + 2).is_some_and(u8::is_ascii_alphabetic)) {
            let (mut tag, next) = parse_tag(text, i);
            i = next;
            let local = local_name(&tag.name).to_owned();
            if !tag.closing && (local == "script" || local == "style") {
                let close = format!("</{}", tag.name);
                let end = lower[i..].find(&close).map_or(text.len(), |e| i + e);
                tag.body = Some(text[i..end].to_owned());
                i = end;
            }
            tags.push(tag);
        } else {
            i += 1;
        }
    }
    tags
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let end = rest.find(|c: char| !c.is_ascii_alphanumeric() && c != '#' && c != '&').unwrap_or(rest.len());
        let body = &rest[1..end];
        let consumed = end + usize::from(rest[end..].starts_with(';'));
        let decoded = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = body.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match body {
                "tab" | "Tab" => Some('\t'),
                "newline" | "NewLine" => Some('\n'),
                "colon" => Some(':'),
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lpar" => Some('('),
                "rpar" => Some(')'),
                "sol" => Some('/'),
                "bsol" => Some('\\'),
                "nbsp" => Some('\u{a0}'),
                _ => None,
            }
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[consumed..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// 브라우저가 스킴을 알아보는 방식으로 정규화한다: 엔티티를 풀고 공백·제어문자를 지우고 소문자로 만든다.
fn normalize_url(v: &str) -> String {
    decode_entities(v).chars().filter(|c| !c.is_control() && *c != ' ').collect::<String>().to_lowercase()
}

#[derive(Debug, PartialEq)]
enum UrlKind {
    Script,
    Dangerous,
    Unc,
    Remote,
    UnknownScheme,
    Plain,
}

fn classify_url(value: &str) -> UrlKind {
    let decoded = decode_entities(value);
    let trimmed = decoded.trim();
    if trimmed.starts_with("\\\\") {
        return UrlKind::Unc;
    }
    let n = normalize_url(value);
    if n.starts_with("javascript:") || n.starts_with("vbscript:") || n.starts_with("data:text/html") || n.starts_with("data:application/xhtml") {
        return UrlKind::Script;
    }
    if n.starts_with("//") {
        return UrlKind::Remote;
    }
    match SCHEME.captures(&n) {
        None => UrlKind::Plain,
        Some(c) => {
            let s = &c[1];
            if DANGEROUS_SCHEMES.contains(&s) {
                UrlKind::Dangerous
            } else if s == "http" || s == "https" {
                UrlKind::Remote
            } else if SAFE_SCHEMES.contains(&s) || s.len() == 1 {
                UrlKind::Plain
            } else {
                UrlKind::UnknownScheme
            }
        }
    }
}

// ---------------------------------------------------------------- 검사기

#[derive(Default)]
struct ManifestEntry {
    scripted: bool,
    remote_resources: bool,
}

struct Scanner {
    findings: Vec<Finding>,
    notes: Vec<String>,
    seen: HashSet<(String, String)>,
    html_active: Vec<String>,
    html_remote: Vec<String>,
    manifest: HashMap<String, ManifestEntry>,
}

impl Scanner {
    fn add(&mut self, severity: Severity, code: &str, location: &str, detail: &str) {
        if self.seen.insert((code.to_owned(), location.to_owned())) {
            self.findings.push(Finding { severity, code: code.to_owned(), location: safe(location), detail: safe(detail) });
        }
    }

    fn partial(&mut self, location: &str, msg: &str) {
        self.notes.push(safe(&format!("{location}: {msg}")));
    }

    fn zip_name(&mut self, name: &str) {
        let decoded = unquote(name).replace('\\', "/");
        let drive = decoded.len() >= 2 && decoded.as_bytes()[0].is_ascii_alphabetic() && decoded.as_bytes()[1] == b':';
        if decoded.split('/').any(|c| c == "..") {
            self.add(Severity::High, "ZIP_PATH_TRAVERSAL", name, "Entry name climbs out of the archive");
        }
        if decoded.starts_with('/') || drive {
            self.add(Severity::High, "ZIP_ABSOLUTE_PATH", name, "Absolute path or drive letter");
        }
        if name.contains('\\') {
            self.add(Severity::Medium, "ZIP_BACKSLASH_PATH", name, "Backslash in entry name");
        }
        if name.chars().any(char::is_control) {
            self.add(Severity::Medium, "ZIP_CONTROL_CHAR_NAME", name, "Control character in entry name");
        }
        let comps: Vec<&str> = decoded.split('/').collect();
        for (i, c) in comps.iter().enumerate() {
            let stem = c.trim_end_matches(['.', ' ']).split('.').next().unwrap_or("").to_lowercase();
            if WIN_RESERVED.contains(&stem.as_str()) {
                self.add(Severity::Medium, "WINDOWS_RESERVED_NAME", name, "Windows device name");
            }
            if c.contains(':') && !(drive && i == 0) {
                self.add(Severity::Medium, "ZIP_ALT_DATA_STREAM", name, "Colon in name (NTFS stream)");
            }
            if c.chars().count() > 255 {
                self.add(Severity::Low, "ZIP_NAME_TOO_LONG", name, "Path component over 255 characters");
            }
        }
        if name.chars().count() > 1024 {
            self.add(Severity::Low, "ZIP_NAME_TOO_LONG", name, "Entry name over 1024 characters");
        }
    }

    fn local_header(&mut self, raw: &mut Option<File>, header_start: u64, central_name: &[u8], name: &str) {
        let Some(f) = raw.as_mut() else { return };
        let mut h = [0u8; 30];
        if f.seek(SeekFrom::Start(header_start)).is_err() || f.read_exact(&mut h).is_err() || &h[..4] != b"PK\x03\x04" {
            self.add(Severity::High, "BAD_LOCAL_HEADER", name, "ZIP local header absent or invalid");
            return;
        }
        let name_len = usize::from(u16::from_le_bytes([h[26], h[27]]));
        let mut local = vec![0u8; name_len];
        if f.read_exact(&mut local).is_err() {
            self.add(Severity::High, "TRUNCATED_LOCAL_HEADER", name, "Incomplete local filename");
            return;
        }
        if local != central_name {
            self.add(Severity::High, "ZIP_HEADER_NAME_MISMATCH", name, &format!("Local filename differs: {}", String::from_utf8_lossy(&local)));
        }
    }

    fn scan_js(&mut self, text: &str, name: &str) {
        for (sev, code, detail, rx) in JS_RULES.iter() {
            if rx.is_match(text) {
                self.add(*sev, code, name, detail);
            }
        }
    }

    fn scan_css(&mut self, text: &str, name: &str) {
        let stripped = CSS_COMMENT.replace_all(text, " ");
        for (sev, code, detail, rx) in CSS_RULES.iter() {
            if rx.is_match(&stripped) {
                self.add(*sev, code, name, detail);
            }
        }
    }

    fn url_attr(&mut self, tag: &str, attr: &str, value: &str, name: &str, resource: bool) {
        match classify_url(value) {
            UrlKind::Script => self.add(Severity::High, "JS_URL", name, &format!("{tag} {attr}: script URL")),
            UrlKind::Dangerous => self.add(Severity::High, "DANGEROUS_URI_SCHEME", name, &format!("{tag} {attr}: {}", decode_entities(value))),
            UrlKind::Unc => self.add(Severity::High, "UNC_PATH", name, &format!("{tag} {attr}: UNC path")),
            UrlKind::UnknownScheme => self.add(Severity::Low, "UNKNOWN_URI_SCHEME", name, &format!("{tag} {attr}: {}", decode_entities(value))),
            UrlKind::Remote if resource => {
                self.add(Severity::Low, "REMOTE_RESOURCE", name, &format!("{tag} {attr}: remote load"));
                self.html_remote.push(name.to_owned());
            }
            UrlKind::Remote | UrlKind::Plain => {}
        }
    }

    fn scan_doctype(&mut self, text: &str, name: &str) {
        for m in ENTITY_DECL.find_iter(text) {
            if EXTERNAL_ID.is_match(m.as_str()) {
                self.add(Severity::High, "XXE_ENTITY", name, "External entity declaration");
            } else {
                self.add(Severity::Medium, "ENTITY_DECLARATION", name, "Entity declaration (expansion risk)");
            }
        }
        if let Some(c) = DOCTYPE_EXTERNAL.captures(text) {
            // 표준 XHTML·SVG·NCX DTD는 EPUB2에 흔해서 Low로 낮춘다. 그 밖의 외부 DTD는 Medium이다.
            let known = KNOWN_PUBLIC_IDS.iter().any(|id| c[0].contains(id));
            self.add(if known { Severity::Low } else { Severity::Medium }, "EXTERNAL_DTD", name, "DOCTYPE references an external DTD");
        }
    }

    fn scan_html(&mut self, text: &str, name: &str) {
        self.scan_doctype(text, name);
        let mut form_seen = false;
        for tag in tokenize(text) {
            if tag.closing {
                continue;
            }
            let t = local_name(&tag.name).to_owned();
            self.html_tag(&tag, &t, name, &mut form_seen);
        }
    }

    fn html_tag(&mut self, tag: &Tag, t: &str, name: &str, form_seen: &mut bool) {
        let mut active = false;
        for (k, v) in &tag.attrs {
            if k.len() >= 4 && k.starts_with("on") && k[2..].bytes().all(|b| b.is_ascii_lowercase()) {
                self.add(Severity::High, "EVENT_HANDLER_ATTR", name, &format!("{t} {k}"));
                active = true;
            }
            if k == "style" {
                let css = decode_entities(v);
                self.scan_css(&css, name);
            }
        }
        let rel_resource = |attr: &str| {
            matches!(
                (t, attr),
                ("img", "src" | "srcset")
                    | ("link", "href")
                    | ("video" | "audio" | "source" | "track" | "embed" | "input" | "frame" | "iframe", "src")
                    | ("object", "data")
                    | ("video", "poster")
                    | ("image" | "use" | "feimage" | "script", "href" | "xlink:href")
                    | (_, "background")
            )
        };
        for (k, v) in &tag.attrs {
            if URL_ATTRS.contains(&k.as_str()) {
                let resource = rel_resource(k);
                if k == "src" && t == "script" {
                    continue;
                }
                self.url_attr(t, k, v, name, resource);
            }
        }
        match t {
            "script" => {
                let ty = tag.attr("type").map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase()).unwrap_or_default();
                let js = JS_SCRIPT_TYPES.contains(&ty.as_str());
                self.add(if js { Severity::High } else { Severity::Low }, "SCRIPT_TAG", name, &format!("script element type={ty:?}"));
                if let Some(src) = tag.attr("src") {
                    if matches!(classify_url(src), UrlKind::Remote) {
                        self.add(Severity::High, "REMOTE_SCRIPT", name, &format!("script src: {}", decode_entities(src)));
                    } else {
                        self.url_attr("script", "src", src, name, false);
                    }
                }
                if js {
                    active = true;
                    if let Some(body) = &tag.body {
                        self.scan_js(body, name);
                    }
                }
            }
            "style" => {
                if let Some(body) = &tag.body {
                    self.scan_css(body, name);
                }
            }
            "object" | "embed" | "applet" => self.add(Severity::High, "ACTIVE_ELEMENT", name, &format!("{t} element")),
            "iframe" | "frame" | "frameset" => {
                let remote = tag.attr("src").is_some_and(|s| classify_url(s) == UrlKind::Remote);
                let high = tag.attr("srcdoc").is_some() || remote;
                self.add(if high { Severity::High } else { Severity::Medium }, "ACTIVE_ELEMENT", name, &format!("{t} element"));
            }
            "form" | "input" | "textarea" | "select" | "button" => {
                if !*form_seen {
                    *form_seen = true;
                    self.add(Severity::Medium, "FORM_ELEMENT", name, "form controls");
                }
                active = true;
                if t == "form" && tag.attr("action").is_some_and(|a| classify_url(a) == UrlKind::Remote) {
                    self.add(Severity::High, "REMOTE_FORM_ACTION", name, "form posts to a remote URL");
                }
            }
            "meta" => {
                if tag.attr("http-equiv").is_some_and(|v| v.trim().eq_ignore_ascii_case("refresh")) {
                    let target = tag.attr("content").and_then(|c| REFRESH_URL.captures(c).map(|m| m[1].to_owned()));
                    let risky = target.as_deref().is_some_and(|u| matches!(classify_url(u), UrlKind::Remote | UrlKind::Script | UrlKind::Dangerous));
                    self.add(if risky { Severity::High } else { Severity::Medium }, "META_REFRESH", name, "meta refresh");
                }
            }
            "base" => {
                if tag.attr("href").is_some() {
                    self.add(Severity::Medium, "BASE_HREF", name, "base element rewrites relative URLs");
                }
            }
            "foreignobject" => self.add(Severity::Medium, "SVG_FOREIGN_OBJECT", name, "foreignObject embeds HTML in SVG"),
            "animate" | "set" | "animatetransform" | "animatemotion" => {
                let target = tag.attr("attributename").unwrap_or("").to_lowercase();
                if target == "href" || target == "xlink:href" {
                    self.add(Severity::Medium, "SVG_ANIMATE_HREF", name, "animation rewrites href");
                    for key in ["to", "values", "from"] {
                        if let Some(v) = tag.attr(key).filter(|v| classify_url(v) == UrlKind::Script) {
                            self.add(Severity::High, "JS_URL", name, &format!("{t} {key}: {v}"));
                        }
                    }
                }
            }
            _ => {}
        }
        if active {
            self.html_active.push(name.to_owned());
        }
    }

    fn scan_xml(&mut self, text: &str, name: &str) {
        self.scan_doctype(text, name);
        if let Some(m) = XML_STYLESHEET.find(text) {
            let remote = m.as_str().contains("http://") || m.as_str().contains("https://") || m.as_str().contains("href=\"//");
            self.add(if remote { Severity::Medium } else { Severity::Low }, "XML_STYLESHEET", name, "xml-stylesheet processing instruction");
        }
        if text.contains("http://www.w3.org/2001/XInclude") {
            self.add(Severity::Medium, "XINCLUDE", name, "XInclude can pull in other files");
        }
        if name.to_lowercase().ends_with(".opf") {
            self.scan_opf(text, name);
        }
    }

    fn scan_opf(&mut self, text: &str, name: &str) {
        let Ok(doc) = roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }) else {
            self.partial(name, "OPF is not well-formed XML");
            return;
        };
        let base = posix_dirname(name);
        for item in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
            let Some(href) = item.attribute("href") else { continue };
            let media = item.attribute("media-type").unwrap_or("").to_lowercase();
            let props: Vec<&str> = item.attribute("properties").unwrap_or("").split_whitespace().collect();
            match classify_url(href) {
                UrlKind::Remote => self.add(Severity::Medium, "REMOTE_MANIFEST_ITEM", name, &format!("manifest item loads {href}")),
                UrlKind::Dangerous => self.add(Severity::High, "DANGEROUS_URI_SCHEME", name, &format!("manifest item {href}")),
                UrlKind::Script => self.add(Severity::High, "JS_URL", name, "manifest item uses a script URL"),
                UrlKind::Unc => self.add(Severity::High, "UNC_PATH", name, "manifest item uses a UNC path"),
                UrlKind::UnknownScheme => self.add(Severity::Low, "UNKNOWN_URI_SCHEME", name, &format!("manifest item {href}")),
                UrlKind::Plain => {
                    let resolved = posix_normpath(&posix_join(&base, unquote(href).split('#').next().unwrap_or("")));
                    if resolved == ".." || resolved.starts_with("../") || resolved.starts_with('/') {
                        self.add(Severity::High, "MANIFEST_PATH_ESCAPE", name, &format!("manifest href leaves the archive: {href}"));
                    }
                    self.manifest.insert(resolved, ManifestEntry { scripted: props.contains(&"scripted"), remote_resources: props.contains(&"remote-resources") });
                }
            }
            if SCRIPT_MEDIA_TYPES.contains(&media.as_str()) {
                self.add(Severity::Medium, "SCRIPT_MEDIA_TYPE", name, &format!("manifest media-type {media}"));
            }
        }
    }
}

fn extension(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

fn read_limited(zip: &mut Zip, index: usize, limit: u64) -> Result<Vec<u8>, String> {
    let entry = zip.by_index(index).map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    entry.take(limit + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    Ok(data)
}

fn check_magic(s: &mut Scanner, data: &[u8], name: &str, ext: &str, textual: bool) {
    if !textual && data.starts_with(b"MZ") {
        s.add(Severity::High, "PE_EXECUTABLE_CONTENT", name, "Windows PE/DOS MZ header");
    }
    if data.starts_with(b"\x7fELF") {
        s.add(Severity::Medium, "ELF_BINARY_CONTENT", name, "ELF executable");
    }
    if [&b"\xfe\xed\xfa\xce"[..], b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xcf\xfa\xed\xfe"].iter().any(|m| data.starts_with(m)) {
        s.add(Severity::Medium, "MACHO_BINARY_CONTENT", name, "Mach-O executable");
    }
    if data.starts_with(b"\xca\xfe\xba\xbe") {
        // 큰 값(≥45)이면 Java 클래스 버전, 작은 값이면 Mach-O 유니버설 바이너리의 아키텍처 수다.
        let n = data.get(4..8).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        if n >= 45 {
            s.add(Severity::Medium, "JAVA_CLASS_CONTENT", name, "Java class file");
        } else {
            s.add(Severity::Medium, "MACHO_BINARY_CONTENT", name, "Mach-O universal binary");
        }
    }
    if data.starts_with(b"#!") {
        s.add(Severity::Medium, "SCRIPT_SHEBANG", name, "Shebang line");
    }
    if data.starts_with(b"PK\x03\x04") && ext != "epub" {
        s.add(Severity::Low, "NESTED_ARCHIVE", name, "ZIP archive inside the EPUB");
    }
}

fn check_member(s: &mut Scanner, zip: &mut Zip, index: usize, name: &str, limits: &Limits, budget: u64) -> u64 {
    let (size, csize, encrypted, method) = match zip.by_index_raw(index) {
        Ok(f) => (f.size(), f.compressed_size(), f.encrypted(), f.compression()),
        Err(_) => return 0,
    };
    if encrypted {
        s.partial(name, "ZIP-encrypted entry cannot be reliably inspected");
        return 0;
    }
    if !matches!(method, CompressionMethod::Stored | CompressionMethod::Deflated) {
        s.partial(name, "Unusual or unsupported ZIP compression method");
        return 0;
    }
    if size > limits.max_member {
        s.partial(name, &format!("Member larger than max inspection size: {size}"));
        return 0;
    }
    if size > budget {
        s.partial(name, "Total decompressed-byte scan limit reached");
        return 0;
    }
    if csize > 0 && size / csize > 200 && size > 1024 * 1024 {
        s.add(Severity::Medium, "HIGH_COMPRESSION_RATIO", name, "Potential decompression bomb");
    }
    let data = match read_limited(zip, index, limits.max_member) {
        Ok(d) if d.len() as u64 <= limits.max_member => d,
        Ok(_) => {
            s.partial(name, "Member exceeded read limit");
            return 0;
        }
        Err(e) => {
            s.partial(name, &format!("Could not decompress/check CRC: {e}"));
            return 0;
        }
    };

    let ext = extension(name);
    if EXEC_EXTS.contains(&ext.as_str()) {
        s.add(Severity::High, "EMBEDDED_EXECUTABLE", name, &format!("Executable/script file type: .{ext}"));
    }
    let is_html = HTML_EXTS.contains(&ext.as_str());
    let textual = is_html || XML_EXTS.contains(&ext.as_str()) || matches!(ext.as_str(), "js" | "mjs" | "css" | "txt");
    check_magic(s, &data, name, &ext, textual);

    let text = String::from_utf8_lossy(&data);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match ext.as_str() {
        _ if is_html => s.scan_html(text, name),
        "js" | "mjs" => {
            s.add(Severity::Medium, "SCRIPT_FILE", name, "JavaScript member present");
            s.scan_js(text, name);
        }
        "css" => s.scan_css(text, name),
        _ if XML_EXTS.contains(&ext.as_str()) => s.scan_xml(text, name),
        _ => {}
    }
    data.len() as u64
}

/// EPUB 하나를 검사한다. 읽을 수 없는 파일은 `INCOMPLETE`로 돌려준다.
pub fn scan_epub(path: &Path, limits: &Limits) -> SecurityReport {
    let file = safe(&path.to_string_lossy());
    let mut s = Scanner { findings: Vec::new(), notes: Vec::new(), seen: HashSet::new(), html_active: Vec::new(), html_remote: Vec::new(), manifest: HashMap::new() };
    let mut entries = 0;
    let mut inspected = 0u64;

    match open_zip(path) {
        None => s.partial(&file, "Cannot open as ZIP EPUB"),
        Some(mut zip) => {
            entries = zip.len();
            let names: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
            if names.len() > limits.max_files {
                s.partial("ZIP", &format!("Too many entries: {} (max {})", names.len(), limits.max_files));
            }
            let mut raw = File::open(path).ok();
            let mut folded: HashMap<String, String> = HashMap::new();
            for (i, name) in names.iter().enumerate() {
                s.zip_name(name);
                match zip.by_index_raw(i) {
                    Err(_) => s.add(Severity::High, "BAD_LOCAL_HEADER", name, "ZIP local header absent or invalid"),
                    Ok(f) => {
                        let (start, raw_name, symlink) = (f.header_start(), f.name_raw().to_vec(), f.is_symlink());
                        drop(f);
                        s.local_header(&mut raw, start, &raw_name, name);
                        if symlink {
                            s.add(Severity::Medium, "ZIP_SYMLINK", name, "ZIP member represented as symlink");
                        }
                    }
                }
                let key = unquote(name).trim_end_matches(['.', ' ']).to_lowercase();
                match folded.get(&key) {
                    Some(first) => s.add(Severity::Medium, "WINDOWS_FILENAME_COLLISION", name, &format!("Collides with: {first}")),
                    None => {
                        folded.insert(key, name.clone());
                    }
                }
            }

            let set: HashSet<&str> = names.iter().map(String::as_str).collect();
            if !set.contains("mimetype") {
                s.add(Severity::Medium, "MISSING_MIMETYPE", "ZIP", "EPUB mimetype entry missing");
            } else if names.first().map(String::as_str) != Some("mimetype") || zip.by_index_raw(0).map(|f| f.compression()).ok() != Some(CompressionMethod::Stored) {
                s.add(Severity::Low, "NONSTANDARD_MIMETYPE", "ZIP", "mimetype must be first and uncompressed");
            }
            if !set.contains("META-INF/container.xml") {
                s.add(Severity::Medium, "MISSING_CONTAINER_XML", "ZIP", "EPUB META-INF/container.xml missing");
            }

            for (i, name) in names.iter().enumerate().take(limits.max_files) {
                if name.ends_with('/') {
                    continue;
                }
                inspected += check_member(&mut s, &mut zip, i, name, limits, limits.max_total.saturating_sub(inspected));
            }
            if let Some(i) = names.iter().position(|n| n == "mimetype")
                && let Ok(data) = read_limited(&mut zip, i, 64)
                && data.trim_ascii() != b"application/epub+zip"
            {
                s.add(Severity::Medium, "BAD_MIMETYPE", "mimetype", "Expected application/epub+zip");
            }
        }
    }

    let active = std::mem::take(&mut s.html_active);
    for m in active {
        if s.manifest.get(&m).is_some_and(|e| !e.scripted) {
            s.add(Severity::Medium, "UNDECLARED_SCRIPTED_CONTENT", &m, "Scripted/form content missing OPF scripted property");
        }
    }
    let remote = std::mem::take(&mut s.html_remote);
    for m in remote {
        if s.manifest.get(&m).is_some_and(|e| !e.remote_resources) {
            s.add(Severity::Low, "UNDECLARED_REMOTE_RESOURCES", &m, "Remote resources missing OPF remote-resources property");
        }
    }

    s.findings.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.code.cmp(&b.code)).then_with(|| a.location.cmp(&b.location)));
    let status = if !s.notes.is_empty() {
        Status::Incomplete
    } else if s.findings.is_empty() {
        Status::None
    } else {
        Status::Findings
    };
    SecurityReport { file, status, entries, inspected_bytes: inspected, findings: s.findings, notes: s.notes }
}
