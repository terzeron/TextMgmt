//! Static PDF action / active-content scanner.
//!
//! This program NEVER renders the PDF, runs JavaScript, opens attachments, or
//! launches programs. PDF parsing itself is attack surface: scan untrusted
//! files in an unprivileged and preferably sandboxed environment.
//!
//! Exit: 0 = no indicators, 1 = indicator(s), 2 = incomplete / unreadable input
//! (and no indicators found).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

use corpus_common::{DEFAULT_WORKERS, limited, pool};
use lopdf::xref::XrefEntry;
use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};
use rayon::prelude::*;

const MAX_DEPTH: usize = 100;
/// epubscan의 --max-total-mb(기본 200)와 같은 이름·단위를 쓴다. 파일 하나의 최대 크기 MB.
pub const DEFAULT_MAX_TOTAL_MB: u64 = 256;
const PREVIEW_CHARS: usize = 100;
const JS_STREAM_LIMIT: usize = 64 * 1024;
const MAX_DECOMPRESSED_STREAM_BYTES: usize = 4 * 1024 * 1024;
/// 로딩 중 스트림 내용을 이만큼만 남긴다. lopdf는 모든 스트림 내용을 복사해서 들고 있어서, 이미지가 많은 PDF가 입력의 2배
/// 넘게 메모리를 쓴다(223MB 파일이 458MB). 한도를 256KB→64KB→4KB로 낮추면 454→302→240MB로 줄었다(입력은 226MB).
/// 64KB는 JavaScript 해제분 미리보기 한도와 같고, JS는 압축하면 보통 수 KB다. 4KB는 일반 JS까지 잘라서 쓸 수 없다.
/// 이보다 큰 JavaScript 스트림은 잘려서 끝까지 못 본 것이므로 `exceeds inspection limit`으로 알린다.
const STREAM_KEEP_BYTES: usize = 64 * 1024;
/// 숫자·이름·참조만 든 배열은 이 길이를 넘으면 로딩 중에 비운다. lopdf는 원소 하나를 `Object`(약 120B)로 만들어서
/// 파일에서 2바이트인 숫자가 50배로 부풀고, 숫자 배열이 많은 PDF는 입력의 25배까지 메모리를 쓴다(22MB가 549MB).
const ARRAY_KEEP_LEN: usize = 32;
/// Bound the amount of input PDF data held by concurrent scans. `lopdf` may
/// allocate more than the input size while parsing, so this is a conservative
/// concurrency guard rather than a hard process-memory limit.
const MAX_PARALLEL_INPUT_MB: u64 = 256;

/// File extensions that an embedded file or launch target should not carry.
const RISKY_EXT: &[&str] = &[
    "exe", "com", "scr", "pif", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "msi", "msp", "dll", "jar", "lnk", "reg", "cpl", "sh", "bash", "run", "bin", "appimage",
    "desktop", "py", "pl", "rb", "docm", "xlsm", "pptm", "swf",
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Low,
    Medium,
    High,
    /// 문서에 있을 이유가 없는 실행 수법(실행 파일·셸을 겨냥한 Launch, 첨부 자동 실행, 페이로드 복호화 후 eval).
    /// High는 "실행 가능한 구조가 있다"까지만 뜻한다.
    Critical,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Low => "LOW",
            Severity::Medium => "MEDIUM",
            Severity::High => "HIGH",
            Severity::Critical => "CRITICAL",
        }
    }
}

struct Finding {
    severity: Severity,
    kind: &'static str,
    object: ObjectId,
    path: String,
    detail: String,
}

struct Report {
    file: PathBuf,
    findings: Vec<Finding>,
    notes: Vec<String>,
    /// Set when the scan could not cover the whole file.
    incomplete: bool,
}

// ---------------------------------------------------------------- 로딩 중 줄이기

/// 스캐너가 들여다보지 않는 값인가. `walk`는 사전·스트림·배열만 따라가고, 참조는 평평한 객체 순회가 따로 처리한다.
fn is_plain(o: &Object) -> bool {
    matches!(o, Object::Null | Object::Boolean(_) | Object::Integer(_) | Object::Real(_) | Object::Name(_) | Object::Reference(_))
}

/// 배열이 어느 깊이에서도 평범한 값과 배열로만 이루어졌다면 노드 수를, 사전·스트림·문자열이 하나라도 있으면 None을 돌려준다.
/// 좌표 쌍 목록(`[[x y] [x y] …]`)처럼 중첩된 숫자 배열도 한 덩어리로 센다.
fn plain_nodes(items: &[Object]) -> Option<usize> {
    items.iter().try_fold(1usize, |n, o| match o {
        Object::Array(inner) => plain_nodes(inner).map(|c| n + c),
        o if is_plain(o) => Some(n + 1),
        _ => None,
    })
}

fn shrink_array(items: &mut Vec<Object>) {
    if plain_nodes(items).is_some_and(|n| n > ARRAY_KEEP_LEN) {
        *items = Vec::new();
        return;
    }
    items.iter_mut().for_each(shrink);
}

fn shrink_dict(d: &mut Dictionary) {
    d.iter_mut().for_each(|(_, v)| shrink(v));
}

/// 객체 하나를 스캔에 필요한 만큼만 남긴다. 제자리에서 줄인다.
fn shrink(obj: &mut Object) {
    match obj {
        Object::Array(items) => shrink_array(items),
        Object::Dictionary(d) => shrink_dict(d),
        Object::Stream(st) => {
            // 객체 스트림과 xref 스트림은 구조 해석에 쓰인다. 훅이 디코딩보다 먼저 돌아서, 자르면 안의 객체를 잃는다.
            let structural = st.dict.has_type(b"ObjStm") || st.dict.has_type(b"XRef");
            // 비우면 lopdf가 "길이 0 스트림"으로 보고 파일에서 다시 읽는다. 잘라서 비어 있지 않게 둔다.
            if !structural && st.content.len() > STREAM_KEEP_BYTES {
                st.content.truncate(STREAM_KEEP_BYTES);
                st.content.shrink_to_fit();
            }
            shrink_dict(&mut st.dict);
        }
        _ => {}
    }
}

/// lopdf 로딩 훅. 객체를 파싱하자마자 줄여서, 문서 전체가 아니라 객체 하나분만 일시적으로 쓴다.
/// 일반 경로는 제자리 수정한 객체를 그대로 쓰고, 객체 스트림 경로는 반환값을 쓰므로 복제본을 돌려준다(줄인 뒤라 작다).
fn shrink_on_load(id: ObjectId, obj: &mut Object) -> Option<(ObjectId, Object)> {
    shrink(obj);
    Some((id, obj.clone()))
}

// ---------------------------------------------------------------- helpers

/// Make attacker-controlled text safe to print: no control chars, bounded length.
fn sanitize(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for (n, c) in text.chars().enumerate() {
        if n >= PREVIEW_CHARS {
            out.push('…');
            break;
        }
        out.push(if c.is_control() { '.' } else { c });
    }
    out
}

fn name_of(obj: &Object) -> Option<String> {
    match obj {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

fn has_risky_ext(name: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(name).to_ascii_lowercase();
    // URL이면 호스트가 아니라 경로의 확장자를 본다. `https://example.com`의 `.com`은 DOS 실행 파일이 아니다.
    // `https://x.example/evil.exe`는 경로가 .exe라서 그대로 잡힌다.
    let target = if lower.starts_with("mailto:") {
        return false;
    } else if let Some((_, rest)) = lower.split_once("://") {
        let rest = rest.split(['?', '#']).next().unwrap_or("");
        rest.split_once('/').map_or("", |(_, path)| path)
    } else {
        lower.as_str()
    };
    // Windows ignores trailing dots/spaces, so strip them before the check.
    let trimmed = target.trim_end_matches(['.', ' ']);
    match trimmed.rsplit_once('.') {
        Some((_, ext)) => RISKY_EXT.contains(&ext),
        None => false,
    }
}

/// Text of a string / stream value, following one reference. Inert data only.
fn text_preview(doc: &Document, obj: &Object) -> String {
    match obj {
        Object::String(s, _) => sanitize(s),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::String(s, _)) => format!("{} {} R: {}", id.0, id.1, sanitize(s)),
            Ok(Object::Stream(st)) => {
                let data = st.decompressed_content_with_limit(JS_STREAM_LIMIT).unwrap_or_else(|_| st.content.clone());
                format!("{} {} R (stream): {}", id.0, id.1, sanitize(&data))
            }
            _ => format!("{} {} R", id.0, id.1),
        },
        Object::Stream(st) => sanitize(&st.content),
        _ => String::new(),
    }
}

/// 문자열이거나 문자열·스트림을 가리키는 참조의 원본 바이트. 상한을 넘는 스트림은 앞부분만 본다.
fn raw_bytes(doc: &Document, obj: &Object) -> Vec<u8> {
    match obj {
        Object::String(s, _) => s.clone(),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::String(s, _)) => s.clone(),
            // 탐지용이라 미리보기(64KB)보다 넉넉히 푼다. 64KB를 넘는 JS를 압축 원본으로 되돌리면 패딩만으로 탐지를 피한다.
            Ok(Object::Stream(st)) => st.decompressed_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES).unwrap_or_else(|_| st.content.iter().take(JS_STREAM_LIMIT).copied().collect()),
            _ => Vec::new(),
        },
        Object::Stream(st) => st.content.iter().take(JS_STREAM_LIMIT).copied().collect(),
        _ => Vec::new(),
    }
}

/// 파일 지정(문자열, 또는 /F·/UF를 가진 사전)의 원본 바이트.
fn filespec_bytes(doc: &Document, obj: &Object) -> Vec<u8> {
    match obj {
        Object::Dictionary(d) => [&b"F"[..], &b"UF"[..]].iter().find_map(|k| d.get(k).ok()).map(|v| raw_bytes(doc, v)).unwrap_or_default(),
        other => raw_bytes(doc, other),
    }
}

/// 참조하는 스트림이 로딩 중에 잘렸는가(`STREAM_KEEP_BYTES` 이상). 잘린 JavaScript는 끝까지 검사하지 못한 것이다.
fn is_truncated_stream(doc: &Document, obj: &Object) -> bool {
    matches!(obj, Object::Reference(id) if matches!(doc.get_object(*id), Ok(Object::Stream(st)) if st.content.len() >= STREAM_KEEP_BYTES))
}

/// 문서에서 열 이유가 없는 대상인가: 실행 파일 확장자이거나 셸·스크립트 인터프리터를 부른다.
fn launch_target_is_dangerous(target: &[u8]) -> bool {
    const SHELLS: &[&str] = &["cmd.exe", "cmd/c", "powershell", "pwsh", "mshta", "wscript", "cscript", "rundll32", "regsvr32", "/bin/sh", "/bin/bash"];
    let flat: String = String::from_utf8_lossy(target).to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    has_risky_ext(target) || SHELLS.iter().any(|sh| flat.contains(sh))
}

/// JavaScript 원문을 글로 바꾼다. PDF 문자열은 UTF-16(BOM)일 수 있어서, 그대로 찾으면 인코딩만 바꿔 탐지를 피할 수 있다.
fn js_text(bytes: &[u8]) -> String {
    let units = |b: &[u8], be: bool| -> String {
        let u16s: Vec<u16> = b.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16_lossy(&u16s)
    };
    match bytes {
        [0xFE, 0xFF, rest @ ..] => units(rest, true),
        [0xFF, 0xFE, rest @ ..] => units(rest, false),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// 알려진 공격 수법이 든 JavaScript인가: 첨부 자동 실행, 로컬·UNC 경로 실행, 복호화한 페이로드의 eval, 셸코드 NOP sled.
fn js_is_exploit(source: &[u8]) -> bool {
    let flat: String = js_text(source).to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    (flat.contains("exportdataobject") && flat.contains("nlaunch"))
        || (flat.contains("launchurl") && (flat.contains("file:") || flat.contains("\\\\")))
        || flat.contains("eval(unescape(")
        || flat.contains("eval(string.fromcharcode(")
        || flat.matches("%u9090").count() >= 4
}

/// Join the listed keys of an inline dictionary value, e.g. /Win << /F /P >>.
fn value_dict_text(doc: &Document, value: &Object, keys: &[&[u8]]) -> String {
    let Object::Dictionary(d) = value else {
        return String::new();
    };
    keys.iter().filter_map(|k| d.get(k).ok().map(|v| text_preview(doc, v))).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- scanning

struct Scanner<'a> {
    doc: &'a Document,
    findings: Vec<Finding>,
}

impl Scanner<'_> {
    fn add(&mut self, sev: Severity, kind: &'static str, id: ObjectId, path: &str, detail: String) {
        self.findings.push(Finding { severity: sev, kind, object: id, path: path.to_owned(), detail });
    }

    fn walk(&mut self, obj: &Object, id: ObjectId, path: &str, depth: usize) {
        if depth > MAX_DEPTH {
            self.add(Severity::Low, "nesting limit exceeded", id, path, "structure deeper than the scanner follows".into());
            return;
        }
        match obj {
            Object::Dictionary(d) => self.dict(d, id, path, depth),
            Object::Stream(s) => self.dict(&s.dict, id, path, depth),
            Object::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    self.walk(v, id, &format!("{path}[{i}]"), depth + 1);
                }
            }
            _ => {}
        }
    }

    fn dict(&mut self, d: &Dictionary, id: ObjectId, path: &str, depth: usize) {
        let doc = self.doc;
        let action = d.get(b"S").ok().and_then(name_of);

        if let Some(s) = action.as_deref() {
            match s {
                "Launch" => {
                    let target = d.get(b"F").map(|v| text_preview(doc, v)).unwrap_or_default();
                    let dangerous = d.get(b"F").is_ok_and(|v| launch_target_is_dangerous(&filespec_bytes(doc, v)));
                    self.add(if dangerous { Severity::Critical } else { Severity::High }, "Launch action", id, path, target);
                }
                "JavaScript" => {
                    if d.get(b"JS").is_ok_and(|v| is_truncated_stream(doc, v)) {
                        self.add(Severity::High, "JavaScript stream exceeds inspection limit", id, path, String::new());
                    }
                    let js = d.get(b"JS").map(|v| text_preview(doc, v)).unwrap_or_default();
                    if d.get(b"JS").is_ok_and(|v| js_is_exploit(&raw_bytes(doc, v))) {
                        self.add(Severity::Critical, "exploit JavaScript", id, path, js);
                    } else {
                        self.add(Severity::High, "JavaScript action", id, path, js);
                    }
                }
                "RichMediaExecute" => self.add(Severity::High, "RichMediaExecute action", id, path, String::new()),
                "ImportData" | "SubmitForm" | "GoToR" | "GoToE" | "Rendition" | "Movie" | "Sound" => {
                    let target = d.get(b"F").or_else(|_| d.get(b"URI")).map(|v| text_preview(doc, v)).unwrap_or_default();
                    self.add(Severity::Medium, "external/media action", id, path, format!("/S /{s} {target}"));
                }
                "URI" => {
                    let uri = d.get(b"URI").map(|v| text_preview(doc, v)).unwrap_or_default();
                    self.add(Severity::Low, "URI action", id, path, uri);
                }
                _ => {}
            }
        }

        // File specification: an embedded or referenced executable-looking name.
        for key in [&b"F"[..], &b"UF"[..]] {
            if let Ok(Object::String(name, _)) = d.get(key) {
                let is_filespec = d.has(b"EF") || matches!(d.get(b"Type").ok().and_then(name_of).as_deref(), Some("Filespec")) || action.as_deref() == Some("Launch");
                if is_filespec && has_risky_ext(name) {
                    self.add(Severity::High, "executable-looking file name", id, path, sanitize(name));
                }
            }
        }

        for (key, value) in d.iter() {
            let key_name = String::from_utf8_lossy(key).into_owned();
            // 경로는 출력에 그대로 나간다. PDF가 정한 키 이름의 제어문자는 걷어낸다. 판정은 원래 이름으로 한다.
            let child = format!("{path}/{}", sanitize(key));
            match key_name.as_str() {
                // The action dict case is already reported through /S above.
                "JS" if action.as_deref() != Some("JavaScript") => {
                    if is_truncated_stream(doc, value) {
                        self.add(Severity::High, "JavaScript stream exceeds inspection limit", id, &child, String::new());
                    }
                    if js_is_exploit(&raw_bytes(doc, value)) {
                        self.add(Severity::Critical, "exploit JavaScript", id, &child, text_preview(doc, value));
                    } else {
                        self.add(Severity::High, "JavaScript code", id, &child, text_preview(doc, value));
                    }
                }
                "JavaScript" => {
                    self.add(Severity::Medium, "JavaScript name tree", id, &child, String::new());
                }
                "Win" | "Mac" | "Unix" => {
                    let cmd = value_dict_text(doc, value, &[&b"F"[..], &b"P"[..]]);
                    let dangerous = match value {
                        Object::Dictionary(v) => [&b"F"[..], &b"P"[..]].iter().any(|k| v.get(k).is_ok_and(|x| launch_target_is_dangerous(&filespec_bytes(doc, x)))),
                        _ => false,
                    };
                    self.add(if dangerous { Severity::Critical } else { Severity::High }, "platform launch parameters", id, &child, cmd);
                }
                "OpenAction" => {
                    self.add(Severity::Medium, "OpenAction (runs on open)", id, &child, String::new());
                }
                "AA" => {
                    self.add(Severity::Medium, "additional-actions trigger", id, &child, String::new());
                }
                "EmbeddedFiles" | "EF" => {
                    self.add(Severity::Medium, "embedded file", id, &child, String::new());
                }
                "RichMediaContent" => {
                    self.add(Severity::Medium, "RichMedia content", id, &child, String::new());
                }
                "XFA" => {
                    self.add(Severity::Medium, "XFA form", id, &child, String::new());
                }
                "3DD" => {
                    self.add(Severity::Low, "3D content", id, &child, String::new());
                }
                "Subtype" => match name_of(value).as_deref() {
                    Some("FileAttachment") => self.add(Severity::Medium, "file attachment annotation", id, path, String::new()),
                    Some("Screen") => self.add(Severity::Low, "screen annotation", id, path, String::new()),
                    _ => {}
                },
                _ => {}
            }
            // References are resolved by the flat object loop, so only inline
            // children are followed here.
            self.walk(value, id, &child, depth + 1);
        }
    }
}

fn scan_document(doc: &Document, report: &mut Report) {
    if doc.is_encrypted() {
        report.incomplete = true;
        report.notes.push("encrypted: string values may be unreadable".into());
    }
    let incomplete_object_streams: BTreeSet<u32> = doc
        .reference_table
        .entries
        .iter()
        .filter_map(|(object, entry)| match entry {
            XrefEntry::Compressed { container, .. } if !doc.objects.contains_key(&(*object, 0)) => Some(*container),
            _ => None,
        })
        .collect();
    for container in incomplete_object_streams {
        report.incomplete = true;
        report.notes.push(format!("objects in object stream {container} could not be decoded within the {} byte limit", MAX_DECOMPRESSED_STREAM_BYTES));
    }
    let mut sc = Scanner { doc, findings: Vec::new() };
    sc.walk(&Object::Dictionary(doc.trailer.clone()), (0, 0), "trailer", 0);
    for (id, obj) in &doc.objects {
        sc.walk(obj, *id, "", 0);
    }
    // Stable, most severe first.
    sc.findings.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.object.cmp(&b.object)));
    report.findings = sc.findings;
}

fn scan_file(path: &Path, max_total_mb: u64) -> Report {
    let mut report = Report { file: path.to_path_buf(), findings: Vec::new(), notes: Vec::new(), incomplete: false };
    let fail = |report: &mut Report, msg: String| {
        report.incomplete = true;
        report.notes.push(msg);
    };

    match fs::metadata(path) {
        Ok(m) if m.len() > max_total_mb * 1024 * 1024 => {
            fail(&mut report, format!("skipped: {} bytes exceeds --max-total-mb", m.len()));
            return report;
        }
        Err(e) => {
            fail(&mut report, format!("unreadable: {e}"));
            return report;
        }
        _ => {}
    }
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            fail(&mut report, format!("unreadable: {e}"));
            return report;
        }
    };
    if !data.windows(5).take(1024).any(|w| w == b"%PDF-") {
        fail(&mut report, "no %PDF- header in the first 1 KiB".into());
        return report;
    }

    // A malformed file must not take the whole batch down.
    let parsed = std::panic::catch_unwind(|| {
        Document::load_mem_with_options(&data, LoadOptions { filter: Some(shrink_on_load), max_decompressed_size: Some(MAX_DECOMPRESSED_STREAM_BYTES), ..LoadOptions::default() })
    });
    match parsed {
        Ok(Ok(doc)) => scan_document(&doc, &mut report),
        Ok(Err(e)) => fail(&mut report, format!("parse error: {e}")),
        Err(_) => fail(&mut report, "parser panicked on malformed input".into()),
    }
    report
}

// ---------------------------------------------------------------- output

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || (0x7f..=0x9f).contains(&(c as u32)) => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print_text(r: &Report, out: &mut dyn IoWrite) -> std::io::Result<()> {
    // The file name is attacker-influenced too.
    let name: String = r.file.to_string_lossy().chars().map(|c| if c.is_control() { '.' } else { c }).collect();
    let status = if !r.findings.is_empty() {
        "INDICATORS"
    } else if r.incomplete {
        "INCOMPLETE"
    } else {
        "clean"
    };
    writeln!(out, "{name}: {status}")?;
    for n in &r.notes {
        writeln!(out, "  note: {n}")?;
    }
    for f in &r.findings {
        let loc = if f.path.is_empty() { "-" } else { f.path.as_str() };
        write!(out, "  [{}] {} (obj {} {}, {})", f.severity.label(), f.kind, f.object.0, f.object.1, loc)?;
        if !f.detail.is_empty() {
            write!(out, ": {}", f.detail)?;
        }
        writeln!(out)?;
    }
    Ok(())
}

fn print_json_report(r: &Report, out: &mut dyn IoWrite) -> std::io::Result<()> {
    write!(out, "{{\"file\":{},\"incomplete\":{},\"notes\":[", json_str(&r.file.to_string_lossy()), r.incomplete)?;
    for (i, note) in r.notes.iter().enumerate() {
        if i > 0 {
            write!(out, ",")?;
        }
        write!(out, "{}", json_str(note))?;
    }
    write!(out, "],\"findings\":[")?;
    for (i, f) in r.findings.iter().enumerate() {
        if i > 0 {
            write!(out, ",")?;
        }
        write!(
            out,
            "{{\"severity\":{},\"kind\":{},\"object\":[{},{}],\"path\":{},\"detail\":{}}}",
            json_str(f.severity.label()),
            json_str(f.kind),
            f.object.0,
            f.object.1,
            json_str(&f.path),
            json_str(&f.detail)
        )?;
    }
    writeln!(out, "]}}")
}

// ---------------------------------------------------------------- CLI

/// epubscan과 같은 공통 옵션 세트. `-r`이 없으면 디렉토리는 1단계만 훑는다.
struct Args {
    paths: Vec<PathBuf>,
    recursive: bool,
    json: bool,
    workers: usize,
    limit: usize,
    max_total_mb: u64,
}

fn number<T: std::str::FromStr>(name: &str, value: &str) -> Result<T, String> {
    value.parse().map_err(|_| format!("argument --{name}: invalid int value: '{value}'"))
}

fn parse(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { paths: Vec::new(), recursive: false, json: false, workers: DEFAULT_WORKERS, limit: 0, max_total_mb: DEFAULT_MAX_TOTAL_MB };
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-r" | "--recursive" => a.recursive = true,
            "--json" => a.json = true,
            _ if arg.starts_with("--") => {
                let (name, inline) = match arg[2..].split_once('=') {
                    Some((n, v)) => (n, Some(v.to_owned())),
                    None => (&arg[2..], None),
                };
                if !["workers", "limit", "max-total-mb"].contains(&name) {
                    return Err(format!("unrecognized arguments: {arg}"));
                }
                let value = inline.or_else(|| it.next().cloned()).ok_or_else(|| format!("argument --{name}: expected one argument"))?;
                match name {
                    "workers" => a.workers = number(name, &value)?,
                    "limit" => a.limit = number(name, &value)?,
                    _ => a.max_total_mb = number(name, &value)?,
                }
            }
            _ => a.paths.push(arg.into()),
        }
    }
    if a.paths.is_empty() {
        return Err("the following arguments are required: PATH...".into());
    }
    if a.max_total_mb == 0 {
        return Err("Limits must be positive".into());
    }
    Ok(a)
}

fn is_pdf(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

/// epubscan과 같은 훑기 규칙: 정렬된 항목, `-r`이 있을 때만 하위로 내려가고, 심볼릭 링크 디렉토리는 따라가지 않는다.
fn walk(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        out.push(dir.to_path_buf()); // reported as unreadable later
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() && !fs::symlink_metadata(&p).is_ok_and(|m| m.is_symlink()) {
            if recursive {
                walk(&p, recursive, out);
            }
        } else if is_pdf(&p) {
            out.push(p);
        }
    }
}

fn collect(args: &Args, out: &mut Vec<PathBuf>) {
    for p in &args.paths {
        // symlink_metadata: never follow links out of the scanned tree.
        match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => walk(p, args.recursive, out),
            _ => out.push(p.clone()), // missing or unreadable: reported as incomplete later
        }
    }
    // 중복은 처음 것만 남긴다. epubscan과 같다.
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
}

const USAGE: &str = "usage: pdfscan [-r] [--json] [--workers N] [--limit N] [--max-total-mb N] PATH...";

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
pub fn print_usage(out: &mut dyn IoWrite) -> std::io::Result<()> {
    write!(
        out,
        "pdfscan - PDF 정적 위험 검사. 렌더링하거나 스크립트를 실행하지 않고 객체 구조만 본다.

PDF 뷰어·편집기에서 임의의 명령을 실행할 여지가 있는 요소(Launch, JavaScript, OpenAction, /AA,
첨부 파일, 외부 연결 등)를 찾는다. PDF 파싱 자체가 공격면이므로 신뢰할 수 없는 파일은
권한 없는 샌드박스에서 검사한다.

사용법:
  pdfscan [-r] [--json] [--workers N] [--limit N] [--max-total-mb N] PATH...

인자:
  PATH   검사할 PDF 파일 또는 디렉토리 (여러 개 가능)

옵션:
  -r, --recursive     디렉토리를 하위까지 훑는다. 심볼릭 링크 디렉토리는 따라가지 않는다.
  --json              결과를 JSON으로 출력한다.
  --workers N         병렬 작업 상한 (기본 {DEFAULT_WORKERS}; 입력 크기에 따라 낮아질 수 있음)
  --limit N           앞에서 N건만 처리한다 (0이면 전체).
  --max-total-mb N    이보다 큰 파일은 검사하지 않고 INCOMPLETE로 표시한다 (기본 {DEFAULT_MAX_TOTAL_MB})
  -h, --help          이 도움말

종료 코드: 0 지표 없음, 1 지표 있음, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
"
    )
}

/// 인자가 없거나 도움말을 요청했는가.
pub fn usage_requested(args: &[String]) -> bool {
    args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help")
}

/// 진입점. 종료 코드를 돌려준다: 지표 있음 1, 검사 못 한 파일만 있음 2, 깨끗함 0, 잘못된 인자 2.
pub fn run(argv: &[String], out: &mut dyn IoWrite) -> i32 {
    if usage_requested(argv) {
        let _ = print_usage(out);
        return 0;
    }
    let args = match parse(argv) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{USAGE}\npdfscan: error: {msg}");
            return 2;
        }
    };

    let mut files = Vec::new();
    collect(&args, &mut files);
    if files.is_empty() {
        eprintln!("No PDF files found");
        return 2;
    }
    let files = limited(files, args.limit);
    let workers_by_input_size = (MAX_PARALLEL_INPUT_MB / args.max_total_mb).max(1) as usize;
    let workers = args.workers.max(1).min(workers_by_input_size);
    let pool = pool(workers);
    let mut has_findings = false;
    let mut has_incomplete = false;
    let mut by_sev = BTreeMap::<&str, usize>::new();
    let mut first_json_report = true;
    if args.json && write!(out, "[").is_err() {
        return 1;
    }
    for batch in files.chunks(workers) {
        let reports: Vec<Report> = pool.install(|| batch.par_iter().map(|f| scan_file(f, args.max_total_mb)).collect());
        for report in &reports {
            has_findings |= !report.findings.is_empty();
            has_incomplete |= report.incomplete;
            for finding in &report.findings {
                *by_sev.entry(finding.severity.label()).or_default() += 1;
            }
            let result = if args.json {
                let result = if first_json_report { Ok(()) } else { write!(out, ",") };
                first_json_report = false;
                result.and_then(|()| print_json_report(report, out))
            } else {
                print_text(report, out)
            };
            if result.is_err() {
                return 1;
            }
            if out.flush().is_err() {
                return 1;
            }
        }
    }
    if args.json && writeln!(out, "]").is_err() {
        return 1;
    }
    if !args.json {
        eprintln!("scanned {} file(s); workers: {workers}; findings by severity: {by_sev:?}", files.len());
    }

    if has_findings {
        1
    } else if has_incomplete {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn usage_is_requested_for_no_arguments_and_help_flags() {
        assert!(usage_requested(&args(&[])));
        assert!(usage_requested(&args(&["-h"])));
        assert!(usage_requested(&args(&["--help"])));
        assert!(usage_requested(&args(&["a.pdf", "--help"])));
        assert!(!usage_requested(&args(&["a.pdf"])));
        assert!(!usage_requested(&args(&["--json", "a.pdf"])));
    }

    #[test]
    fn print_usage_explains_purpose_arguments_options_and_exit_codes() {
        let mut out = Vec::new();
        print_usage(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        for needle in ["pdfscan", "PDF", "사용법:", "PATH", "-r, --recursive", "--json", "--workers", "--limit", "--max-total-mb", "-h, --help", "종료 코드", "Launch", "JavaScript"] {
            assert!(text.contains(needle), "{needle} 없음: {text}");
        }
        assert!(text.contains(&DEFAULT_MAX_TOTAL_MB.to_string()));
    }

    #[test]
    fn json_str_escapes_c0_and_c1_controls() {
        assert_eq!(json_str("a\u{1b}b\u{85}c\u{7f}"), "\"a\\u001bb\\u0085c\\u007f\"");
    }

    #[test]
    fn finding_paths_never_carry_control_characters_from_key_names() {
        let doc = Document::new();
        let mut action = Dictionary::new();
        action.set("S", Object::Name(b"Launch".to_vec()));
        let mut outer = Dictionary::new();
        outer.set(b"\x1b[2J\x07evil".to_vec(), Object::Dictionary(action));
        let mut sc = Scanner { doc: &doc, findings: Vec::new() };
        sc.walk(&Object::Dictionary(outer), (1, 0), "", 0);
        assert!(!sc.findings.is_empty());
        for f in &sc.findings {
            assert!(!f.path.chars().any(char::is_control), "{:?}", f.path);
        }
    }
}
