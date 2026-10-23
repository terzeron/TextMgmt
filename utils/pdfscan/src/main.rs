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
            let child = format!("{path}/{key_name}");
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
            c if (c as u32) < 0x20 => {
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

fn usage() -> ExitCode {
    eprintln!("usage: pdfscan [--json] [--max-bytes N] <file-or-dir>...");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut json = false;
    let mut max_bytes = DEFAULT_MAX_BYTES;
    let mut inputs: Vec<PathBuf> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--json" => json = true,
            "--max-bytes" => match args.next().and_then(|v| v.parse().ok()) {
                Some(n) => max_bytes = n,
                None => return usage(),
            },
            "-h" | "--help" => return usage(),
            _ => inputs.push(PathBuf::from(a)),
        }
    }
    if inputs.is_empty() {
        return usage();
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
