//! Static PDF action / active-content scanner.
//!
//! This program NEVER renders the PDF, runs JavaScript, opens attachments, or
//! launches programs. PDF parsing itself is attack surface: scan untrusted
//! files in an unprivileged and preferably sandboxed environment.
//!
//! Exit: 0 = no indicators, 1 = indicator(s), 2 = incomplete / unreadable input
//! (and no indicators found).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lopdf::{Dictionary, Document, Object, ObjectId};

const MAX_DEPTH: usize = 100;
const DEFAULT_MAX_BYTES: u64 = 256 * 1024 * 1024;
const PREVIEW_CHARS: usize = 100;
const JS_STREAM_LIMIT: usize = 64 * 1024;

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
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Severity::Low => "LOW",
            Severity::Medium => "MEDIUM",
            Severity::High => "HIGH",
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
    // Windows ignores trailing dots/spaces, so strip them before the check.
    let trimmed = lower.trim_end_matches(['.', ' ']);
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
                    self.add(Severity::High, "Launch action", id, path, target);
                }
                "JavaScript" => {
                    let js = d.get(b"JS").map(|v| text_preview(doc, v)).unwrap_or_default();
                    self.add(Severity::High, "JavaScript action", id, path, js);
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
                    self.add(Severity::High, "JavaScript code", id, &child, text_preview(doc, value));
                }
                "JavaScript" => {
                    self.add(Severity::Medium, "JavaScript name tree", id, &child, String::new());
                }
                "Win" | "Mac" | "Unix" => {
                    let cmd = value_dict_text(doc, value, &[&b"F"[..], &b"P"[..]]);
                    self.add(Severity::High, "platform launch parameters", id, &child, cmd);
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
    let mut sc = Scanner { doc, findings: Vec::new() };
    sc.walk(&Object::Dictionary(doc.trailer.clone()), (0, 0), "trailer", 0);
    for (id, obj) in &doc.objects {
        sc.walk(obj, *id, "", 0);
    }
    // Stable, most severe first.
    sc.findings.sort_by(|a, b| b.severity.cmp(&a.severity).then(a.object.cmp(&b.object)));
    report.findings = sc.findings;
}

fn scan_file(path: &Path, max_bytes: u64) -> Report {
    let mut report = Report { file: path.to_path_buf(), findings: Vec::new(), notes: Vec::new(), incomplete: false };
    let fail = |report: &mut Report, msg: String| {
        report.incomplete = true;
        report.notes.push(msg);
    };

    match fs::metadata(path) {
        Ok(m) if m.len() > max_bytes => {
            fail(&mut report, format!("skipped: {} bytes exceeds --max-bytes", m.len()));
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
    let parsed = std::panic::catch_unwind(|| Document::load_mem(&data));
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

fn print_text(r: &Report) {
    // The file name is attacker-influenced too.
    let name: String = r.file.to_string_lossy().chars().map(|c| if c.is_control() { '.' } else { c }).collect();
    let status = if !r.findings.is_empty() {
        "INDICATORS"
    } else if r.incomplete {
        "INCOMPLETE"
    } else {
        "clean"
    };
    println!("{name}: {status}");
    for n in &r.notes {
        println!("  note: {n}");
    }
    for f in &r.findings {
        let loc = if f.path.is_empty() { "-" } else { f.path.as_str() };
        print!("  [{}] {} (obj {} {}, {})", f.severity.label(), f.kind, f.object.0, f.object.1, loc);
        if !f.detail.is_empty() {
            print!(": {}", f.detail);
        }
        println!();
    }
}

fn print_json(reports: &[Report]) {
    let mut out = String::from("[");
    for (i, r) in reports.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"file\":{},\"incomplete\":{},\"notes\":[{}],\"findings\":[",
            json_str(&r.file.to_string_lossy()),
            r.incomplete,
            r.notes.iter().map(|n| json_str(n)).collect::<Vec<_>>().join(",")
        );
        for (j, f) in r.findings.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            let _ = write!(
                out,
                "{{\"severity\":{},\"kind\":{},\"object\":[{},{}],\"path\":{},\"detail\":{}}}",
                json_str(f.severity.label()),
                json_str(f.kind),
                f.object.0,
                f.object.1,
                json_str(&f.path),
                json_str(&f.detail)
            );
        }
        out.push_str("]}");
    }
    out.push(']');
    println!("{out}");
}

// ---------------------------------------------------------------- CLI

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    // symlink_metadata: never follow links out of the scanned tree.
    let Ok(meta) = fs::symlink_metadata(path) else {
        out.push(path.to_path_buf()); // reported as unreadable later
        return;
    };
    if meta.is_dir() {
        let Ok(rd) = fs::read_dir(path) else {
            out.push(path.to_path_buf());
            return;
        };
        let mut children: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        children.sort();
        for c in children {
            let is_pdf = c.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
            let is_dir = fs::symlink_metadata(&c).is_ok_and(|m| m.is_dir());
            if is_dir || is_pdf {
                collect(&c, out);
            }
        }
    } else {
        out.push(path.to_path_buf());
    }
}

/// 인자 없이 실행하거나 `-h`/`--help`를 주면 프로그램의 용도와 인자·옵션을 보여 준다.
fn print_usage(out: &mut dyn std::io::Write) -> std::io::Result<()> {
    write!(
        out,
        "pdfscan - PDF 정적 위험 검사. 렌더링하거나 스크립트를 실행하지 않고 객체 구조만 본다.

PDF 뷰어·편집기에서 임의의 명령을 실행할 여지가 있는 요소(Launch, JavaScript, OpenAction, /AA,
첨부 파일, 외부 연결 등)를 찾는다. PDF 파싱 자체가 공격면이므로 신뢰할 수 없는 파일은
권한 없는 샌드박스에서 검사한다.

사용법:
  pdfscan [--json] [--max-bytes N] <file-or-dir>...

인자:
  <file-or-dir>   검사할 PDF 파일 또는 디렉토리 (여러 개 가능, 디렉토리는 하위까지 훑는다)

옵션:
  --json          결과를 JSON으로 출력한다.
  --max-bytes N   이보다 큰 파일은 검사하지 않고 INCOMPLETE로 표시한다 (기본 {DEFAULT_MAX_BYTES})
  -h, --help      이 도움말

종료 코드: 0 지표 없음, 1 지표 있음, 2 검사하지 못한 파일이 있고 지표는 없음 또는 잘못된 인자
"
    )
}

/// 인자가 없거나 도움말을 요청했는가.
fn usage_requested(args: &[String]) -> bool {
    args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help")
}

/// 잘못된 인자일 때 stderr에 쓰는 한 줄 사용법이다.
fn bad_usage() -> ExitCode {
    eprintln!("usage: pdfscan [--json] [--max-bytes N] <file-or-dir>...  (자세한 설명: pdfscan --help)");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut json = false;
    let mut max_bytes = DEFAULT_MAX_BYTES;
    let mut inputs: Vec<PathBuf> = Vec::new();

    let argv: Vec<String> = std::env::args().skip(1).collect();
    if usage_requested(&argv) {
        let _ = print_usage(&mut std::io::stdout());
        return ExitCode::SUCCESS;
    }
    let mut args = argv.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--json" => json = true,
            "--max-bytes" => match args.next().and_then(|v| v.parse().ok()) {
                Some(n) => max_bytes = n,
                None => return bad_usage(),
            },
            _ => inputs.push(PathBuf::from(a)),
        }
    }
    if inputs.is_empty() {
        return bad_usage();
    }

    let mut files = Vec::new();
    for i in &inputs {
        collect(i, &mut files);
    }

    // Silence the default panic message; catch_unwind reports it as a note.
    std::panic::set_hook(Box::new(|_| {}));

    let reports: Vec<Report> = files.iter().map(|f| scan_file(f, max_bytes)).collect();

    if json {
        print_json(&reports);
    } else {
        for r in &reports {
            print_text(r);
        }
        let by_sev: BTreeMap<&str, usize> = reports.iter().flat_map(|r| r.findings.iter()).fold(BTreeMap::new(), |mut m, f| {
            *m.entry(f.severity.label()).or_default() += 1;
            m
        });
        eprintln!("scanned {} file(s); findings by severity: {by_sev:?}", reports.len());
    }

    if reports.iter().any(|r| !r.findings.is_empty()) {
        ExitCode::from(1)
    } else if reports.iter().any(|r| r.incomplete) {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
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
        for needle in ["pdfscan", "PDF", "사용법:", "<file-or-dir>", "--json", "--max-bytes", "-h, --help", "종료 코드", "Launch", "JavaScript"] {
            assert!(text.contains(needle), "{needle} 없음: {text}");
        }
        assert!(text.contains(&DEFAULT_MAX_BYTES.to_string()));
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
