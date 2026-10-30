//! OOXML(DOCX 계열) 검사. 구조(필수 부분, XML 파싱, 선언된 부분의 존재)를 본다.
//! 압축 해제는 `epub::security::read_limited`로 상한에서 멈춘다.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use corpus_common::reader::open_zip;
use regex::Regex;
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};

use crate::epub::security::{Limits, read_limited};

/// 이게 없으면 Word 문서가 아니다.
const REQUIRED_PARTS: [&str; 3] = ["[Content_Types].xml", "_rels/.rels", "word/document.xml"];

/// 부분 하나를 상한 안에서 읽는다. 상한을 넘으면 `None`이고 미완료로 남긴다. 읽을 수 없으면 `None`이다(해제 실패는 빠른 점검이 이미 알렸다).
fn read_part(zip: &mut corpus_common::reader::Zip, index: usize, name: &str, limits: &Limits, report: &mut FileReport) -> Option<Vec<u8>> {
    let data = read_limited(zip, index, limits.max_member).ok()?;
    if data.len() as u64 > limits.max_member {
        report.mark_incomplete(&format!("{name}: part larger than max inspection size"));
        return None;
    }
    Some(data)
}

/// DOCX의 구조를 검사해 `report`에 더한다. ZIP으로 열리지 않으면 아무것도 하지 않는다(빠른 점검이 `NOT_ZIP`을 낸다).
pub fn inspect_structure(path: &Path, limits: &Limits, report: &mut FileReport) {
    let Some(mut zip) = open_zip(path) else { return };
    let names: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
    let set: HashSet<&str> = names.iter().map(String::as_str).collect();
    for part in REQUIRED_PARTS {
        if !set.contains(part) {
            report.push(Category::Render, Severity::High, "OOXML_NOT_DOCX", part, "필수 부분이 없다");
        }
    }
    for part in REQUIRED_PARTS {
        let Some(index) = names.iter().position(|n| n == part) else { continue };
        let Some(data) = read_part(&mut zip, index, part, limits, report) else { continue };
        let text = String::from_utf8_lossy(&data);
        let parsed = roxmltree::Document::parse_with_options(text.strip_prefix('\u{feff}').unwrap_or(&text), roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() });
        match parsed {
            Err(e) => report.push(Category::Render, Severity::High, "MALFORMED_MARKUP", part, &format!("XML이 올바르지 않다: {e}")),
            Ok(doc) if part == "[Content_Types].xml" => {
                for node in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Override") {
                    let Some(declared) = node.attribute("PartName") else { continue };
                    let declared = declared.trim_start_matches('/');
                    if !set.contains(declared) {
                        report.push(Category::Render, Severity::Medium, "OOXML_PART_MISSING", declared, "[Content_Types].xml이 선언한 부분이 ZIP에 없다");
                    }
                }
            }
            Ok(_) => {}
        };
    }
}

// ---------------------------------------------------------------- 보안

/// 검사하는 부분의 상한. 부분이 수만 개인 패키지에서 시간을 쓰지 않으려는 것이다.
const MAX_PARTS_SCANNED: usize = 256;
/// 지시문·대상 문자열을 지표 상세에 담는 최대 글자 수.
const DETAIL_CHARS: usize = 160;

static ENTITY_EXTERNAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<!ENTITY\s[^>]*?\b(?:SYSTEM|PUBLIC)\b[^>]*>").unwrap());
static ENTITY_ANY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<!ENTITY\b").unwrap());
static DOCTYPE_EXTERNAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<!DOCTYPE\s+[^>\[]*?\b(?:SYSTEM|PUBLIC)\b").unwrap());
/// 엔티티가 읽어 들이는 대상이 로컬 파일이나 위험한 스킴인가(EPUB의 `XXE_LOCAL`과 같은 기준).
static XXE_LOCAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)["'](?:file|php|expect|jar|gopher):|/etc/(?:passwd|shadow|hosts)|[a-z]:[\\/]windows"#).unwrap());

fn preview(text: &str) -> String {
    sanitize_display(text, DETAIL_CHARS, Some('…'))
}

/// 같은 (코드, 위치) 지표는 한 번만 낸다.
fn push_once(report: &mut FileReport, severity: Severity, code: &str, location: &str, detail: &str) {
    if !report.findings.iter().any(|f| f.code == code && f.location == location) {
        report.push(Category::Security, severity, code, location, detail);
    }
}

/// 부분 이름만으로 정하는 지표: 매크로 프로젝트, 내장 OLE·패키지, ActiveX.
fn check_part_names(names: &[String], macro_enabled: bool, report: &mut FileReport) {
    let mut activex_done = false;
    for name in names {
        let lower = name.to_lowercase();
        let shown = preview(name);
        if lower.ends_with("vbaproject.bin") {
            // 매크로가 없는 형식(.docx·.dotx)에 매크로 프로젝트가 있으면 확장자로 위장한 것이다.
            let (severity, detail) = if macro_enabled { (Severity::High, "VBA macro project in a macro-enabled document") } else { (Severity::Critical, "VBA macro project inside a macro-free format (disguised macro document)") };
            push_once(report, severity, "OOXML_VBA_PROJECT", &shown, detail);
        } else if lower.starts_with("word/embeddings/") {
            let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
            match ext {
                "bin" => push_once(report, Severity::High, "OOXML_OLE_OBJECT", &shown, "Embedded OLE object"),
                "docx" | "docm" | "xlsx" | "xlsm" | "pptx" | "pptm" => push_once(report, Severity::Medium, "OOXML_EMBEDDED_PACKAGE", &shown, "Embedded Office package (not inspected recursively yet)"),
                _ => {}
            }
        } else if lower.starts_with("word/activex/") && !activex_done {
            activex_done = true;
            push_once(report, Severity::High, "OOXML_ACTIVEX", &shown, "ActiveX control part");
        }
    }
}

/// DTD·엔티티 공격. OOXML 부분에는 DOCTYPE이 있을 이유가 없다. 파서는 DTD를 해석하지 않으므로 원문에서 찾는다.
fn check_xml_attacks(text: &str, name: &str, report: &mut FileReport) {
    if !text.contains("<!") {
        return;
    }
    let shown = preview(name);
    if let Some(m) = ENTITY_EXTERNAL.find(text) {
        let severity = if XXE_LOCAL.is_match(m.as_str()) { Severity::Critical } else { Severity::High };
        push_once(report, severity, "XXE_ENTITY", &shown, "External entity declaration");
    } else if ENTITY_ANY.is_match(text) {
        push_once(report, Severity::Medium, "ENTITY_DECLARATION", &shown, "Entity declaration (expansion risk)");
    }
    if DOCTYPE_EXTERNAL.is_match(text) {
        push_once(report, Severity::High, "EXTERNAL_DTD", &shown, "DOCTYPE references an external DTD");
    }
}

fn parse(text: &str) -> Option<roxmltree::Document<'_>> {
    roxmltree::Document::parse_with_options(text.strip_prefix('\u{feff}').unwrap_or(text), roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).ok()
}

/// 대상이 UNC 경로(`\\host\share`)나 `file:`인가. 열면 NTLM 해시가 새거나 로컬 파일을 읽는다.
fn is_unc_or_file(target: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    t.starts_with("\\\\") || t.starts_with("file:")
}

/// 관계 파일(`.rels`)의 외부 참조를 본다. 일반 하이퍼링크는 지표를 내지 않는다.
fn check_relationships(text: &str, name: &str, report: &mut FileReport) {
    let Some(doc) = parse(text) else { return };
    let shown = preview(name);
    for rel in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Relationship") {
        let external = rel.attribute("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
        if !external {
            continue;
        }
        let ty = rel.attribute("Type").unwrap_or("").rsplit('/').next().unwrap_or("");
        let target = rel.attribute("Target").unwrap_or("");
        let detail = preview(&format!("{ty} -> {target}"));
        match ty {
            "attachedTemplate" => push_once(report, Severity::Critical, "OOXML_REMOTE_TEMPLATE", &shown, &detail),
            "oleObject" | "subDocument" | "frame" | "externalLinkPath" => push_once(report, Severity::High, "OOXML_EXTERNAL_OBJECT", &shown, &detail),
            "image" => push_once(report, Severity::Medium, "OOXML_EXTERNAL_IMAGE", &shown, &detail),
            _ => {}
        }
        if is_unc_or_file(target) {
            push_once(report, Severity::High, "OOXML_UNC_TARGET", &shown, &detail);
        }
    }
}

fn classify_field(instr: &str, name: &str, report: &mut FileReport) {
    let word = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    let shown = preview(name);
    match word.as_str() {
        "DDEAUTO" | "DDE" => push_once(report, Severity::Critical, "OOXML_DDE_FIELD", &shown, &preview(instr.trim())),
        "INCLUDETEXT" | "INCLUDEPICTURE" | "LINK" | "IMPORT" => {
            let lower = instr.to_ascii_lowercase();
            if lower.contains("://") || lower.contains("\\\\") || lower.contains("file:") {
                push_once(report, Severity::High, "OOXML_DANGEROUS_FIELD", &shown, &preview(instr.trim()));
            }
        }
        _ => {}
    }
}

/// 본문의 필드 지시문을 본다. 한 필드의 지시문은 여러 run에 나뉠 수 있어(`DDE` + `AUTO ...`) begin부터 end까지 이어 붙여 읽는다.
fn check_fields(text: &str, name: &str, report: &mut FileReport) {
    let Some(doc) = parse(text) else { return };
    let mut stack: Vec<String> = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "fldChar" => match node.attributes().find(|a| a.name() == "fldCharType").map(|a| a.value()) {
                Some("begin") => stack.push(String::new()),
                Some("end") => {
                    if let Some(instr) = stack.pop() {
                        classify_field(&instr, name, report);
                    }
                }
                _ => {}
            },
            "instrText" => {
                if let Some(top) = stack.last_mut() {
                    top.push_str(node.text().unwrap_or(""));
                }
            }
            "fldSimple" => {
                if let Some(instr) = node.attributes().find(|a| a.name() == "instr").map(|a| a.value()) {
                    classify_field(instr, name, report);
                }
            }
            _ => {}
        }
    }
    for instr in stack {
        classify_field(&instr, name, report);
    }
}

/// DOCX 계열의 보안 지표를 `report`에 더한다. 압축 해제는 남은 예산 안에서만 한다. 예산을 넘는 부분은 미완료로 남긴다.
pub fn inspect_security(path: &Path, limits: &Limits, macro_enabled: bool, report: &mut FileReport) {
    let Some(mut zip) = open_zip(path) else { return };
    let names: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
    check_part_names(&names, macro_enabled, report);
    let mut spent = 0u64;
    let mut scanned = 0usize;
    for (index, name) in names.iter().enumerate() {
        let is_rels = name.ends_with(".rels");
        let is_word_xml = name.starts_with("word/") && name.ends_with(".xml") && !name.contains("/_rels/");
        if !(is_rels || is_word_xml || name == "[Content_Types].xml") {
            continue;
        }
        scanned += 1;
        if scanned > MAX_PARTS_SCANNED {
            report.mark_incomplete(&format!("security scan stopped after {MAX_PARTS_SCANNED} XML parts"));
            break;
        }
        let remaining = limits.max_total.saturating_sub(spent);
        let cap = limits.max_member.min(remaining);
        if cap == 0 {
            report.mark_incomplete("security scan stopped: decompression budget exhausted");
            break;
        }
        // 읽기는 남은 예산 +1바이트에서 멈춘다. 선언 크기를 믿지 않는다.
        let Ok(data) = read_limited(&mut zip, index, cap) else { continue };
        if data.len() as u64 > cap {
            report.mark_incomplete(&format!("{}: part larger than the remaining inspection budget", preview(name)));
            continue;
        }
        spent += data.len() as u64;
        let text = String::from_utf8_lossy(&data);
        check_xml_attacks(&text, name, report);
        if is_rels {
            check_relationships(&text, name, report);
        } else if is_word_xml {
            check_fields(&text, name, report);
        }
    }
}
