//! EPUB 정적 위험 검사. 스크립트·활성 콘텐츠·위험한 ZIP 구조를 찾는다.
//!
//! 렌더링하거나 스크립트를 실행하거나 첨부를 열지 않는다. 문자열과 구조만 본다.
//! "지표 없음"은 안전하다는 보증이 아니다. 한도(`Limits`)를 넘어 못 본 부분은 `notes`에 남기고
//! 상태를 `INCOMPLETE`로 표시한다.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::LazyLock;

use corpus_common::reader::{Zip, open_zip};
use corpus_common::util::{posix_dirname, posix_join, posix_normpath, unquote};
use htmlkit::{Tag, decode_entities, local_name, tokenize};
use regex::Regex;
use serde::Serialize;
use zip::CompressionMethod;

const MAX_TEXT: usize = 160;

pub use scan_common::Severity;

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

/// 공용 실행기가 요약(지표 수, 검사하지 못한 파일)을 만들 때 보는 창구.
impl scan_common::Report for SecurityReport {
    fn severities(&self) -> impl Iterator<Item = Severity> + '_ {
        self.findings.iter().map(|f| f.severity)
    }

    fn is_incomplete(&self) -> bool {
        self.status == Status::Incomplete
    }
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

// ---------------------------------------------------------------- 큰 파일

/// 중앙 디렉터리가 이보다 크면 열지 않는다. ZIP 라이브러리는 항목마다 메타데이터를 메모리에 올린다.
const MAX_DIRECTORY_ENTRIES: u64 = 1_000_000;
const MAX_DIRECTORY_BYTES: u64 = 256 * 1024 * 1024;

/// 파일 끝의 EOCD(zip64 포함)에서 항목 수와 중앙 디렉터리 크기를 읽는다. 열기 전에 거대한 디렉터리를 거르려는 용도다.
/// 알아보지 못하면 None이고, 그때는 평소처럼 연다.
fn zip_directory_bounds(path: &Path) -> Option<(u64, u64)> {
    const TAIL: u64 = 22 + 65_535;
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    f.take(TAIL).read_to_end(&mut tail).ok()?;
    let eocd = tail.windows(4).rposition(|w| w == b"PK\x05\x06")?;
    let rec = tail.get(eocd..eocd + 22)?;
    let le16 = |b: &[u8], o: usize| u64::from(u16::from_le_bytes([b[o], b[o + 1]]));
    let le32 = |b: &[u8], o: usize| u64::from(u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]));
    let (mut entries, mut size) = (le16(rec, 10), le32(rec, 12));
    if entries == 0xFFFF || size == 0xFFFF_FFFF {
        // zip64: EOCD 바로 앞의 로케이터가 zip64 EOCD 레코드의 위치를 알려 준다.
        let loc = tail.get(eocd.checked_sub(20)?..eocd)?;
        if &loc[..4] != b"PK\x06\x07" {
            return None; // 표식 값(0xFFFF)을 실제 값으로 읽으면 안 된다.
        }
        let off = u64::from_le_bytes(loc[8..16].try_into().ok()?);
        let mut f = File::open(path).ok()?;
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut z = [0u8; 56];
        f.read_exact(&mut z).ok()?;
        if &z[..4] != b"PK\x06\x06" {
            return None;
        }
        entries = u64::from_le_bytes(z[32..40].try_into().ok()?);
        size = u64::from_le_bytes(z[40..48].try_into().ok()?);
    }
    Some((entries, size))
}

fn incomplete_report(file: String, note: &str) -> SecurityReport {
    SecurityReport { file, status: Status::Incomplete, entries: 0, inspected_bytes: 0, findings: Vec::new(), notes: vec![safe(note)] }
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
/// 클릭이나 렌더링만으로 원격 코드 실행·자격 증명 유출에 쓰인 적이 있는 스킴(예: Follina, CVE-2022-30190).
const EXPLOIT_SCHEMES: &[&str] = &["ms-msdt", "search-ms", "ms-appinstaller", "ms-officecmd"];
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
/// `<script type>`이 실행되지 않는 데이터 블록인지. 데이터 블록이 아니면 전부 실행 가능으로 본다.
/// 브라우저가 아는 JavaScript MIME 형식은 길고 구버전 변형(jscript.encode 등)도 있어서, 알려진 형식을
/// 나열하는 것보다 데이터 블록 쪽을 좁게 정의하는 편이 우회에 강하다.
fn is_executable_script_type(ty: &str) -> bool {
    let data = matches!(ty, "application/json" | "text/plain" | "importmap" | "speculationrules") || ty.ends_with("json") || ty.contains("template");
    !data
}

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

static JS_RULES: [Rule; 11] = [
    rule!(Severity::High, "DYNAMIC_CODE_EXEC", "eval/Function/string timer", r#"\beval\s*\(|\bFunction\s*\(|\b(?:setTimeout|setInterval)\s*\(\s*["'`]"#),
    rule!(Severity::Medium, "DOCUMENT_WRITE", "document.write", r"\bdocument\s*\.\s*write(?:ln)?\s*\("),
    rule!(Severity::Medium, "NETWORK_API", "network API", r"\bXMLHttpRequest\b|\bfetch\s*\(|\bnew\s+WebSocket\b|\bsendBeacon\b|\bEventSource\b"),
    rule!(Severity::Medium, "NAVIGATION", "window.open or location change", r"\bwindow\s*\.\s*open\s*\(|\blocation\s*\.\s*(?:href|replace|assign)\b|\b(?:top|parent)\s*\.\s*location\b"),
    rule!(Severity::High, "HOST_API", "host/shell API", r#"\bWScript\b|Shell\.Application|\brequire\s*\(\s*["'](?:child_process|fs|os|net|http)["']\s*\)|\bprocess\s*\.\s*binding\b"#),
    // 명령 실행·파일 쓰기·서버측 다운로드에 쓰는 ProgID 문자열을 생성자와 무관하게 본다(`var A = ActiveXObject; new A(..)` 같은 별칭을 막는다). jQuery 같은 라이브러리의 IE용 폴백(Microsoft.XMLDOM, XMLHTTP)은 걸리지 않는다.
    rule!(
        Severity::Critical,
        "HOST_COMMAND_EXEC",
        "shell/file/server-side download via ActiveX or child_process",
        r#"["'](?:WScript\.Shell|Shell\.Application|Scripting\.FileSystemObject|ADODB\.Stream|MSXML2\.ServerXMLHTTP)["']|\brequire\s*\(\s*["']child_process["']\s*\)"#
    ),
    rule!(Severity::Critical, "OBFUSCATED_LOADER", "eval of decoded payload", r"\b(?:eval|Function)\s*\(\s*(?:atob|unescape|decodeURIComponent|String\s*\.\s*fromCharCode)\s*\("),
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

/// 외부 엔티티가 로컬 파일이나 위험 프로토콜을 가리키면 파일 읽기 시도다. 일반 http DTD와 구분한다.
static XXE_LOCAL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)["'](?:file|php|expect|jar|gopher):|/etc/(?:passwd|shadow|hosts)|[a-z]:[\\/]windows"#));
static ACTIVEX: LazyLock<Regex> = LazyLock::new(|| re(r"\bActiveXObject\b"));
/// IE용 XML 파싱·AJAX 폴백에 쓰는 ProgID. 구형 JS 라이브러리(jQuery 등)에 흔해서 호스트 API로 보지 않는다.
static BENIGN_ACTIVEX_CALL: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)^\(\s*["'](?:microsoft|msxml2)\.(?:xmldom|xmlhttp|domdocument)(?:\.\d+(?:\.\d+)?)?["']\s*\)"#));
static BLOCK_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)/\*.*?\*/"));
/// 앞이 공백이거나 줄 처음인 `//`만 줄 주석으로 본다. `http://`의 `//`는 건드리지 않는다.
static LINE_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?m)(?:^|\s)//[^\n]*"));
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
    scan_common::text::sanitize(s, MAX_TEXT, None)
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

fn is_exploit_scheme(value: &str) -> bool {
    SCHEME.captures(&normalize_url(value)).is_some_and(|c| EXPLOIT_SCHEMES.contains(&&c[1]))
}

/// `ActiveXObject` 사용 중 정상이 아닌 것이 있는지. 위험한 모양을 나열하지 않고, 알려진 정상 모양만 허용한다.
/// 정상은 두 가지다: IE용 XML·AJAX 폴백 호출(`new ActiveXObject("Microsoft.XMLHTTP")`)과 호출이 아닌 기능 감지
/// (`typeof ActiveXObject`, `"ActiveXObject" in window`, `window.ActiveXObject ? …`, `if (window.ActiveXObject)`).
/// 그 밖의 모든 사용(별칭 대입, 인자 전달, 대괄호 접근, 모르는 ProgID, 동적 인자)은 위험으로 본다.
/// 주석을 지운 글과 원문을 모두 본다. 문자열 안의 `//`가 뒤따르는 호출을 가리는 우회를 원문의 호출 검사가 막는다.
fn has_risky_activex(text: &str) -> bool {
    let no_block = BLOCK_COMMENT.replace_all(text, " ");
    let stripped = LINE_COMMENT.replace_all(&no_block, " ");
    activex_misuse(&stripped, false) || activex_misuse(text, true)
}

/// `calls_only`이면 호출 모양만 검사한다(원문에서 주석 속 언급을 오탐하지 않으려고).
fn activex_misuse(text: &str, calls_only: bool) -> bool {
    ACTIVEX.find_iter(text).any(|m| {
        let after = text[m.end()..].trim_start();
        if after.starts_with('(') {
            return !BENIGN_ACTIVEX_CALL.is_match(after);
        }
        !calls_only && !is_feature_detection(&text[..m.start()], after)
    })
}

/// 호출이 아닌 `ActiveXObject` 언급이 기능 감지인지.
fn is_feature_detection(before: &str, after: &str) -> bool {
    let before = before.trim_end();
    let before = ["window", "self", "this", "globalThis"].iter().find_map(|o| before.strip_suffix('.').and_then(|b| b.trim_end().strip_suffix(o))).unwrap_or(before).trim_end();
    if before.ends_with("typeof") {
        return true;
    }
    if before.ends_with('!') {
        return true;
    }
    // `"ActiveXObject" in window`
    if let (Some(_), Some(rest)) = (before.strip_suffix(['"', '\'']), after.strip_prefix(['"', '\''])) {
        let rest = rest.trim_start();
        return rest.starts_with("in ") || rest.starts_with("in(");
    }
    // `window.ActiveXObject ? a : b`, `x && window.ActiveXObject`, `!window.ActiveXObject`, `=== undefined`
    if after.starts_with(['?', '&', '|', '!', '<', '>']) || after.starts_with("==") {
        return true;
    }
    // `if (window.ActiveXObject) {…}`는 기능 감지다. `new (window.ActiveXObject)("…")`처럼 괄호 뒤에 호출이 이어지면 아니다.
    if let Some(rest) = after.strip_prefix(')') {
        let rest = rest.trim_start();
        return condition_context(before) && (!rest.starts_with('(') || BENIGN_ACTIVEX_CALL.is_match(rest));
    }
    false
}

/// 직전 글이 조건식 안인지(`if (`, `while (`, `&&`, `||`, `!`). `f(ActiveXObject)`처럼 인자로 넘기는 경우는 아니다.
fn condition_context(before: &str) -> bool {
    let b = before.trim_end();
    if ["&&", "||", "!", "?"].iter().any(|op| b.ends_with(op)) {
        return true;
    }
    b.strip_suffix('(').is_some_and(|head| {
        let h = head.trim_end();
        ["if", "while", "&&", "||", "!"].iter().any(|k| h.ends_with(k))
    })
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
    media: String,
    scripted: bool,
    remote_resources: bool,
}

struct Scanner {
    findings: Vec<Finding>,
    notes: Vec<String>,
    /// (code, location)별로 findings의 위치를 기억한다. 같은 키가 다시 나오면 더 높은 심각도로 올린다.
    seen: HashMap<(String, String), usize>,
    html_active: Vec<String>,
    html_remote: Vec<String>,
    manifest: HashMap<String, ManifestEntry>,
    /// 전체 압축 해제 한도 때문에 검사하지 못한 멤버 수. 멤버마다 노트를 달면 노트가 항목 수만큼 쌓인다.
    budget_skipped: usize,
}

impl Scanner {
    fn add(&mut self, severity: Severity, code: &str, location: &str, detail: &str) {
        let key = (code.to_owned(), location.to_owned());
        if let Some(&i) = self.seen.get(&key) {
            // 먼저 나온 High가 뒤에 나온 Critical을 가리지 않게 한다.
            if severity > self.findings[i].severity {
                self.findings[i].severity = severity;
                self.findings[i].detail = safe(detail);
            }
        } else {
            self.seen.insert(key, self.findings.len());
            self.findings.push(Finding { severity, code: code.to_owned(), location: safe(location), detail: safe(detail) });
        }
    }

    fn partial(&mut self, location: &str, msg: &str) {
        let note = safe(&format!("{location}: {msg}"));
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }

    fn zip_name(&mut self, name: &str) {
        let decoded = unquote(name).replace('\\', "/");
        let drive = decoded.len() >= 2 && decoded.as_bytes()[0].is_ascii_alphabetic() && decoded.as_bytes()[1] == b':';
        if decoded.split('/').any(|c| c == "..") {
            self.add(Severity::Critical, "ZIP_PATH_TRAVERSAL", name, "Entry name climbs out of the archive");
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
        if has_risky_activex(text) {
            self.add(Severity::High, "HOST_API", name, "host/shell API");
        }
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
            UrlKind::Dangerous => {
                let sev = if is_exploit_scheme(value) { Severity::Critical } else { Severity::High };
                self.add(sev, "DANGEROUS_URI_SCHEME", name, &format!("{tag} {attr}: {}", decode_entities(value)));
            }
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
                let sev = if XXE_LOCAL.is_match(m.as_str()) { Severity::Critical } else { Severity::High };
                self.add(sev, "XXE_ENTITY", name, "External entity declaration");
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
        let (tags, malformed) = tokenize(text);
        if malformed {
            self.add(Severity::Low, "MALFORMED_MARKUP", name, "Unclosed comment");
        }
        for tag in tags {
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
                let js = is_executable_script_type(&ty);
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

    fn scan_xml(&mut self, text: &str, name: &str, is_opf: bool) {
        self.scan_doctype(text, name);
        if let Some(m) = XML_STYLESHEET.find(text) {
            let remote = m.as_str().contains("http://") || m.as_str().contains("https://") || m.as_str().contains("href=\"//");
            self.add(if remote { Severity::Medium } else { Severity::Low }, "XML_STYLESHEET", name, "xml-stylesheet processing instruction");
        }
        if text.contains("http://www.w3.org/2001/XInclude") {
            self.add(Severity::Medium, "XINCLUDE", name, "XInclude can pull in other files");
        }
        if is_opf {
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
                UrlKind::Dangerous => self.add(if is_exploit_scheme(href) { Severity::Critical } else { Severity::High }, "DANGEROUS_URI_SCHEME", name, &format!("manifest item {href}")),
                UrlKind::Script => self.add(Severity::High, "JS_URL", name, "manifest item uses a script URL"),
                UrlKind::Unc => self.add(Severity::High, "UNC_PATH", name, "manifest item uses a UNC path"),
                UrlKind::UnknownScheme => self.add(Severity::Low, "UNKNOWN_URI_SCHEME", name, &format!("manifest item {href}")),
                UrlKind::Plain => {
                    let resolved = posix_normpath(&posix_join(&base, unquote(href).split('#').next().unwrap_or("")));
                    if resolved == ".." || resolved.starts_with("../") || resolved.starts_with('/') {
                        self.add(Severity::High, "MANIFEST_PATH_ESCAPE", name, &format!("manifest href leaves the archive: {href}"));
                    }
                    self.manifest.insert(resolved, ManifestEntry { media: media.clone(), scripted: props.contains(&"scripted"), remote_resources: props.contains(&"remote-resources") });
                }
            }
            if SCRIPT_MEDIA_TYPES.contains(&media.as_str()) {
                self.add(Severity::Medium, "SCRIPT_MEDIA_TYPE", name, &format!("manifest media-type {media}"));
            }
        }
    }
}

/// 내용을 어떻게 읽을지. 선언된 media-type이 있으면 그것을, 없으면 확장자를 따른다(뷰어가 그렇게 한다).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Html,
    Xml,
    Css,
    Js,
    Other,
}

impl Kind {
    /// 마크업(HTML·XML)은 서로 호환된다. 그 밖의 종류가 다르면 선언과 확장자가 어긋난 것이다.
    fn class(self) -> u8 {
        match self {
            Kind::Html | Kind::Xml => 0,
            Kind::Css => 1,
            Kind::Js => 2,
            Kind::Other => 3,
        }
    }
}

fn kind_of_ext(ext: &str) -> Kind {
    match ext {
        e if HTML_EXTS.contains(&e) => Kind::Html,
        e if XML_EXTS.contains(&e) => Kind::Xml,
        "css" => Kind::Css,
        "js" | "mjs" => Kind::Js,
        _ => Kind::Other,
    }
}

fn kind_of_media(media: &str) -> Kind {
    match media {
        "application/xhtml+xml" | "text/html" | "image/svg+xml" => Kind::Html,
        "text/css" => Kind::Css,
        m if SCRIPT_MEDIA_TYPES.contains(&m) && m.contains("script") => Kind::Js,
        m if m.ends_with("+xml") || m.ends_with("/xml") => Kind::Xml,
        _ => Kind::Other,
    }
}

fn extension(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

fn read_limited(zip: &mut Zip, index: usize, limit: u64) -> Result<Vec<u8>, String> {
    let entry = zip.by_index(index).map_err(|e| e.to_string())?;
    // 선언 크기만큼 미리 잡는다. 비워 두고 읽으면 read_to_end가 용량을 두 배씩 늘려 최대 2배를 쓴다. 선언이 거짓이어도 상한까지만 잡는다.
    let mut data = Vec::with_capacity(entry.size().min(limit).saturating_add(1) as usize);
    entry.take(limit + 1).read_to_end(&mut data).map_err(|e| e.to_string())?;
    Ok(data)
}

/// 패키지 문서 이름들: `.opf` 확장자이거나 container.xml의 rootfile이 가리키는 파일.
fn package_documents(zip: &mut Zip, names: &[String], limits: &Limits) -> HashSet<String> {
    let mut found: HashSet<String> = names.iter().filter(|n| n.to_lowercase().ends_with(".opf")).cloned().collect();
    if let Some(i) = names.iter().position(|n| n == "META-INF/container.xml")
        && let Ok(data) = read_limited(zip, i, limits.max_member.min(5 * 1024 * 1024))
        && let Ok(doc) = roxmltree::Document::parse_with_options(&String::from_utf8_lossy(&data), roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
    {
        for root in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "rootfile") {
            let path = posix_normpath(&unquote(root.attribute("full-path").unwrap_or("")));
            if names.contains(&path) {
                found.insert(path);
            }
        }
    }
    found
}

fn check_magic(s: &mut Scanner, data: &[u8], name: &str, ext: &str, textual: bool) {
    // 실행 파일 확장자(.exe 등)면 번들된 설치 파일일 수 있다. 그 밖의 확장자(.jpg 등)로 위장한 실행 코드는 정상 문서에 있을 이유가 없다.
    let disguised = !EXEC_EXTS.contains(&ext);
    if !textual && data.starts_with(b"MZ") {
        s.add(
            if disguised { Severity::Critical } else { Severity::High },
            "PE_EXECUTABLE_CONTENT",
            name,
            if disguised { "Windows PE/DOS MZ header disguised by extension" } else { "Windows PE/DOS MZ header" },
        );
    }
    if data.starts_with(b"\x7fELF") {
        s.add(if disguised { Severity::Critical } else { Severity::Medium }, "ELF_BINARY_CONTENT", name, "ELF executable");
    }
    if [&b"\xfe\xed\xfa\xce"[..], b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xcf\xfa\xed\xfe"].iter().any(|m| data.starts_with(m)) {
        s.add(if disguised { Severity::Critical } else { Severity::Medium }, "MACHO_BINARY_CONTENT", name, "Mach-O executable");
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

fn check_member(s: &mut Scanner, zip: &mut Zip, index: usize, name: &str, limits: &Limits, budget: u64, opf_names: &HashSet<String>) -> u64 {
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
        s.budget_skipped += 1;
        return 0;
    }
    if csize > 0 && size / csize > 200 && size > 1024 * 1024 {
        s.add(Severity::Medium, "HIGH_COMPRESSION_RATIO", name, "Potential decompression bomb");
    }
    let data = match read_limited(zip, index, limits.max_member) {
        Ok(d) => d,
        Err(e) => {
            s.partial(name, &format!("Could not decompress/check CRC: {e}"));
            return 0;
        }
    };

    // 헤더가 거짓으로 작은 크기를 적어도 읽기는 상한에서 멈춘다. 상한을 넘겼다면 뒤쪽을 못 본 것이다.
    let mut data = data;
    if data.len() as u64 > limits.max_member {
        data.truncate(limits.max_member as usize);
        s.partial(name, "Member is larger than its declared size; only the first part was inspected");
    }

    let ext = extension(name);
    if EXEC_EXTS.contains(&ext.as_str()) {
        s.add(Severity::High, "EMBEDDED_EXECUTABLE", name, &format!("Executable/script file type: .{ext}"));
    }
    let ext_kind = kind_of_ext(&ext);
    let declared = s.manifest.get(&posix_normpath(name)).map(|e| (e.media.clone(), kind_of_media(&e.media))).filter(|(_, k)| *k != Kind::Other);
    if let Some((media, k)) = &declared
        && k.class() != ext_kind.class()
    {
        s.add(Severity::Medium, "MEDIA_TYPE_MISMATCH", name, &format!("Declared {media} but the extension is .{ext}"));
    }
    let kind = declared.map_or(ext_kind, |(_, k)| k);
    check_magic(s, &data, name, &ext, kind != Kind::Other || ext == "txt");

    let text = String::from_utf8_lossy(&data);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    match kind {
        Kind::Html => s.scan_html(text, name),
        Kind::Js => {
            s.add(Severity::Medium, "SCRIPT_FILE", name, "JavaScript member present");
            s.scan_js(text, name);
        }
        Kind::Css => s.scan_css(text, name),
        Kind::Xml => s.scan_xml(text, name, opf_names.contains(name)),
        Kind::Other => {}
    }
    data.len() as u64
}

/// EPUB 하나를 검사한다. 읽을 수 없는 파일은 `INCOMPLETE`로 돌려준다.
/// 파일 하나가 검사기를 panic시켜도 배치 전체가 죽지 않게 한다. 그 파일은 `INCOMPLETE`로 남는다.
pub fn scan_epub(path: &Path, limits: &Limits) -> SecurityReport {
    guarded(path, || scan_epub_unguarded(path, limits))
}

/// `scan`이 panic하면 그 파일을 `INCOMPLETE` 보고서로 바꾼다.
pub fn guarded(path: &Path, scan: impl FnOnce() -> SecurityReport) -> SecurityReport {
    catch_unwind(AssertUnwindSafe(scan)).unwrap_or_else(|_| incomplete_report(safe(&path.to_string_lossy()), "Scanner panicked on malformed input"))
}

fn scan_epub_unguarded(path: &Path, limits: &Limits) -> SecurityReport {
    let file = safe(&path.to_string_lossy());
    match fs::metadata(path) {
        Err(e) => return incomplete_report(file, &format!("Cannot open as ZIP EPUB: {e}")),
        Ok(m) if m.is_dir() => return incomplete_report(file, "Cannot open as ZIP EPUB: unreadable directory"),
        Ok(_) => {}
    }
    if let Some((entries, dir_bytes)) = zip_directory_bounds(path) {
        if entries > MAX_DIRECTORY_ENTRIES {
            return incomplete_report(file, &format!("Too many entries: {entries} (max {MAX_DIRECTORY_ENTRIES}); not opened"));
        }
        if dir_bytes > MAX_DIRECTORY_BYTES {
            return incomplete_report(file, &format!("Central directory too large: {dir_bytes} bytes (max {MAX_DIRECTORY_BYTES}); not opened"));
        }
    }
    let mut s = Scanner { findings: Vec::new(), notes: Vec::new(), seen: HashMap::new(), html_active: Vec::new(), html_remote: Vec::new(), manifest: HashMap::new(), budget_skipped: 0 };
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

            // 매니페스트를 먼저 읽는다. 선언된 media-type으로 각 멤버를 어떻게 읽을지 정해야 하기 때문이다.
            let opf_names = package_documents(&mut zip, &names, limits);
            for (i, name) in names.iter().enumerate() {
                if opf_names.contains(name)
                    && let Ok(data) = read_limited(&mut zip, i, limits.max_member)
                {
                    s.scan_opf(&String::from_utf8_lossy(&data), name);
                }
            }
            for (i, name) in names.iter().enumerate().take(limits.max_files) {
                if name.ends_with('/') {
                    continue;
                }
                inspected += check_member(&mut s, &mut zip, i, name, limits, limits.max_total.saturating_sub(inspected), &opf_names);
            }
            if s.budget_skipped > 0 {
                let skipped = s.budget_skipped;
                s.partial("ZIP", &format!("Total decompressed-byte scan limit reached: {skipped} entries not inspected"));
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
