//! OOXML(DOCX 계열) 검사. 구조(필수 부분, XML 파싱, 선언된 부분의 존재)와 보안 지표를 본다.
//! 압축 해제는 `epub::security::read_limited`로 상한에서 멈추고, 읽은 바이트는 모두 공유 예산(`spent`)에서 뺀다.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use corpus_common::reader::{Zip, open_zip};
use corpus_common::util::{posix_join, posix_normpath, unquote};
use regex::Regex;
use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};

use super::fast::read_failure;
use crate::epub::security::{Limits, TextEnc, decode_text, read_limited};

/// 주 문서 부분을 가리키는 관계가 없을 때 기본으로 보는 이름.
const DEFAULT_MAIN_PART: &str = "word/document.xml";

/// 부분 하나를 읽는다. 오류가 나도 그때까지 읽은 바이트를 돌려주고(CRC 오류는 본문이 온전하다) 오류 글도 돌려준다.
fn read_partial(zip: &mut Zip, index: usize, cap: u64) -> (Vec<u8>, Option<String>) {
    let mut entry = match zip.by_index(index) {
        Ok(e) => e,
        Err(e) => return (Vec::new(), Some(e.to_string())),
    };
    let mut data = Vec::new();
    let result = (&mut entry).take(cap.saturating_add(1)).read_to_end(&mut data);
    (data, result.err().map(|e| e.to_string()))
}

/// 부분 하나를 공유 예산 안에서 읽어 디코딩한다. 읽기는 남은 예산 +1바이트에서 멈추고 읽은 만큼 예산에서 뺀다(넘겨서 건너뛰는 부분도).
/// 읽기 오류(CRC 불일치, 선언 크기 위조, 손상)는 지표와 미완료로 남긴다. 기본 모드의 빠른 점검은 앞 4KiB만 풀어 보므로 그 뒤의 오류는 여기서만 드러난다.
pub(super) fn read_xml_part(zip: &mut Zip, index: usize, name: &str, limits: &Limits, spent: &mut u64, report: &mut FileReport) -> Option<String> {
    let cap = limits.max_member.min(limits.max_total.saturating_sub(*spent));
    if cap == 0 {
        report.mark_incomplete("security scan stopped: decompression budget exhausted");
        return None;
    }
    let (data, error) = read_partial(zip, index, cap);
    *spent += data.len() as u64;
    if let Some(e) = error {
        read_failure(report, &preview(name), &e);
        report.mark_incomplete(&format!("{}: read failed ({}); security checks for this part may be incomplete", preview(name), preview(&e)));
        if data.is_empty() {
            return None;
        }
    }
    if data.len() as u64 > cap {
        report.mark_incomplete(&format!("{}: part larger than the remaining inspection budget", preview(name)));
        return None;
    }
    decode_xml(&data, name, report)
}

/// `_rels/.rels`의 officeDocument 관계가 가리키는 주 문서 부분 이름(앞 `/` 없이). 관계가 없거나 읽지 못하면 `None`이다.
fn main_part_target(text: &str) -> Option<String> {
    let doc = parse(text)?;
    doc.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Relationship")
        .find(|n| n.attribute("Type").is_some_and(|t| t.ends_with("/officeDocument")))
        .and_then(|n| n.attribute("Target"))
        .map(|t| unquote(t).trim_start_matches('/').to_string())
}

/// DOCX의 구조를 검사해 `report`에 더한다. ZIP으로 열리지 않으면 아무것도 하지 않는다(빠른 점검이 `NOT_ZIP`을 낸다).
/// 읽은 바이트는 `spent`(보안 검사와 공유하는 예산)에서 뺀다. 부분 이름은 대소문자를 가리지 않고, 주 문서는 관계로 찾는다.
pub fn inspect_structure(path: &Path, limits: &Limits, spent: &mut u64, report: &mut FileReport) {
    let Some(mut zip) = open_zip(path) else { return };
    let names: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
    let set: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut by_lower: HashMap<String, usize> = HashMap::new();
    for (i, n) in names.iter().enumerate() {
        by_lower.entry(n.to_lowercase()).or_insert(i);
    }

    // 주 문서 이름은 `_rels/.rels`가 정한다.
    let rels_index = names.iter().position(|n| n == "_rels/.rels");
    let rels_text = rels_index.and_then(|i| read_xml_part(&mut zip, i, "_rels/.rels", limits, spent, report));
    let main_name = rels_text.as_deref().and_then(main_part_target).unwrap_or_else(|| DEFAULT_MAIN_PART.to_string());
    let main_index = by_lower.get(&main_name.to_lowercase()).copied();

    for part in ["[Content_Types].xml", "_rels/.rels"] {
        if !set.contains(part) {
            report.push(Category::Render, Severity::High, "OOXML_NOT_DOCX", part, "필수 부분이 없다");
        }
    }
    if main_index.is_none() {
        report.push(Category::Render, Severity::High, "OOXML_NOT_DOCX", &preview(&main_name), "필수 부분이 없다");
    }

    let mut texts: Vec<(String, Option<String>)> = Vec::new();
    texts.push(("_rels/.rels".to_string(), rels_text));
    for (label, index) in [("[Content_Types].xml".to_string(), names.iter().position(|n| n == "[Content_Types].xml")), (main_name.clone(), main_index)] {
        let text = index.and_then(|i| read_xml_part(&mut zip, i, &label, limits, spent, report));
        texts.push((label, text));
    }
    for (part, text) in texts {
        let Some(text) = text else { continue };
        match roxmltree::Document::parse_with_options(&text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }) {
            Err(e) => report.push(Category::Render, Severity::High, "MALFORMED_MARKUP", &preview(&part), &format!("XML이 올바르지 않다: {e}")),
            Ok(doc) if part == "[Content_Types].xml" => {
                for node in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Override") {
                    let Some(declared) = node.attribute("PartName") else { continue };
                    let declared = declared.trim_start_matches('/');
                    if !by_lower.contains_key(&declared.to_lowercase()) {
                        report.push(Category::Render, Severity::Medium, "OOXML_PART_MISSING", &preview(declared), "[Content_Types].xml이 선언한 부분이 ZIP에 없다");
                    }
                }
            }
            Ok(_) => {}
        };
    }
}

/// XML 부분의 바이트를 디코딩한다. UTF-16은 BOM·NUL 교차로 알아봐 변환하고(Word 같은 XML 파서가 받아들이는 인코딩이라 그대로 읽으면 검사를 피한다),
/// UTF-32는 변환하지 못하므로 미완료로 남긴다. UTF-8이 아닌 바이트나 선언이 다른 인코딩은 검사는 계속하되 미완료로 남긴다.
pub(super) fn decode_xml(data: &[u8], name: &str, report: &mut FileReport) -> Option<String> {
    let (text, enc) = decode_text(data);
    match enc {
        TextEnc::Utf32 => {
            report.mark_incomplete(&format!("{}: UTF-32 content cannot be inspected", preview(name)));
            return None;
        }
        TextEnc::Invalid => report.mark_incomplete(&format!("{}: non-UTF-8 content cannot be reliably inspected", preview(name))),
        _ => check_declaration(&text, enc == TextEnc::Utf16, name, report),
    }
    Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

static XML_ENCODING_ATTR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)\bencoding\s*=\s*["']([^"']+)["']"#).unwrap());
/// XML 선언을 찾는 최대 글자 수. 선언이 이 안에 끝나지 않으면 읽을 수 없는 것으로 본다.
const DECLARATION_CHARS: usize = 1024;

/// XML 선언(`<?xml ... ?>`) 전체를 읽어 인코딩 선언을 확인한다. 고정 창만 보면 속성을 뒤로 밀어 피할 수 있다.
fn check_declaration(text: &str, bytes_are_utf16: bool, name: &str, report: &mut FileReport) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text).trim_start();
    if !t.starts_with("<?xml") {
        return;
    }
    let head: String = t.chars().take(DECLARATION_CHARS).collect();
    let Some(end) = head.find("?>") else {
        report.mark_incomplete(&format!("{}: XML declaration is not terminated within {DECLARATION_CHARS} characters", preview(name)));
        return;
    };
    let Some(caps) = XML_ENCODING_ATTR.captures(&head[..end]) else { return };
    let declared = caps[1].trim().to_ascii_lowercase();
    match declared.as_str() {
        "utf-8" | "utf8" | "us-ascii" | "ascii" => {}
        "utf-16" | "utf16" | "utf-16le" | "utf-16be" => {
            if !bytes_are_utf16 {
                report.mark_incomplete(&format!("{}: declares UTF-16 but the bytes are not UTF-16", preview(name)));
            }
        }
        other => report.mark_incomplete(&format!("{}: declared encoding {} cannot be reliably inspected", preview(name), preview(other))),
    }
}

/// 이름이 `.xml`·`.rels`·`.hpf`가 아니어도 내용이 XML처럼 보이는가. OPC는 부분의 종류를 확장자가 아니라 content type으로 정하므로
/// 확장자만 믿으면 `.dat`나 확장자 없는 이름에 숨긴 XML을 놓친다. 앞부분만 본다: UTF-8 BOM과 공백 뒤 `<`, UTF-16 BOM,
/// BOM 없는 UTF-16의 `3C 00`·`00 3C`(XML 부록 F). 앞부분이 전부 공백이면 `<`가 더 뒤에 있을 수 있으므로 후보로 본다.
pub(super) fn looks_like_xml(prefix: &[u8]) -> bool {
    if prefix.starts_with(&[0xFF, 0xFE]) || prefix.starts_with(&[0xFE, 0xFF]) || prefix.starts_with(&[0x3C, 0x00]) || prefix.starts_with(&[0x00, 0x3C]) {
        return true;
    }
    let body = prefix.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(prefix);
    match body.iter().find(|b| !b.is_ascii_whitespace()) {
        Some(b) => *b == b'<',
        None => !body.is_empty(),
    }
}

/// 부분의 종류. 확장자가 XML 계열이거나 내용이 XML처럼 보이면 `Xml`, OLE 복합 파일 매직이면 `Ole`이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Member {
    Xml,
    Ole,
    Other,
}

/// 부분의 종류를 정한다. 이름이 XML 계열이 아니면 앞 256바이트를 풀어 내용으로 정한다(읽기 오류는 빠른 점검이 이미 알린 것으로 보고 `Other`다).
pub(super) fn classify_member(zip: &mut Zip, index: usize, name: &str) -> Member {
    if is_xml_part(name) {
        return Member::Xml;
    }
    if name.ends_with('/') {
        return Member::Other;
    }
    match read_limited(zip, index, 256) {
        Ok(prefix) if sniff(&prefix) == Sniffed::Ole => Member::Ole,
        Ok(prefix) if looks_like_xml(&prefix) => Member::Xml,
        _ => Member::Other,
    }
}

/// 이름이 XML 부분(`.xml`·`.rels`·`.hpf`, 대소문자 무시)인가.
pub(super) fn is_xml_part(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".xml") || lower.ends_with(".rels") || lower.ends_with(".hpf")
}

/// 파싱하지 못하면 이 부분의 검사를 건너뛴 것을 미완료로 남긴다(조용히 넘기지 않는다).
fn parse_or_note<'a>(text: &'a str, name: &str, report: &mut FileReport) -> Option<roxmltree::Document<'a>> {
    let doc = parse(text);
    if doc.is_none() {
        report.mark_incomplete(&format!("{}: XML could not be parsed; security checks for this part were skipped", preview(name)));
    }
    doc
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

pub(super) fn preview(text: &str) -> String {
    sanitize_display(text, DETAIL_CHARS, Some('…'))
}

/// 같은 (코드, 위치) 지표는 한 번만 낸다.
pub(super) fn push_once(report: &mut FileReport, severity: Severity, code: &str, location: &str, detail: &str) {
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
pub(super) fn check_xml_attacks(text: &str, name: &str, report: &mut FileReport) {
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

pub(super) fn parse(text: &str) -> Option<roxmltree::Document<'_>> {
    roxmltree::Document::parse_with_options(text.strip_prefix('\u{feff}').unwrap_or(text), roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).ok()
}

/// 관계·필드 대상이 어디를 가리키는가.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// 원격: URL(`http:` 등), UNC(`\\\\host\\share`), 호스트가 있는 `file://host/...`. 열면 네트워크에 닿고 UNC·file 호스트는 NTLM 해시가 샌다.
    Remote { unc: bool },
    /// 로컬 `file:` 경로(`file:///C:/...`).
    LocalFile,
    /// 상대 경로나 그 밖의 것.
    Other,
}

static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^([a-z][a-z0-9+.-]*):").unwrap());

/// `file:` 뒤의 문자열(역슬래시는 `/`로, 소문자로 바꾼 것)이 원격을 가리키는가. 호스트가 있으면(`file://host/...`) 원격이고,
/// 호스트가 비었거나 `localhost`여도 경로가 `//`로 시작하면(`file:////host/share`, `file://localhost//host/share`) UNC다.
/// 드라이브 경로(`file:///C:/...`, `file:/C:/...`)와 POSIX 절대 경로만 로컬이다.
/// `file:` 뒤에서 보는 최대 바이트 수.
const TAIL_LOOKAHEAD: usize = 512;

fn file_tail_is_remote(tail: &str) -> bool {
    // 앞 `TAIL_LOOKAHEAD`바이트만 본다. 경로의 시작(`//`, 호스트)만 필요하고, `file:`이 수십만 번 되풀이되는 입력에서 일치마다 끝까지 훑으면 제곱 시간이다.
    let mut end = tail.len().min(TAIL_LOOKAHEAD);
    while !tail.is_char_boundary(end) {
        end -= 1;
    }
    let tail = &tail[..end];
    let tail = tail.split(|c: char| c.is_whitespace() || c == '"' || c == '\'').next().unwrap_or("");
    let (authority, path) = match tail.strip_prefix("//") {
        Some(after) => {
            let end = after.find('/').unwrap_or(after.len());
            (&after[..end], &after[end..])
        }
        None => ("", tail),
    };
    (!authority.is_empty() && authority != "localhost") || path.starts_with("//")
}

fn target_kind(target: &str) -> Target {
    let t = target.trim();
    if t.starts_with("\\\\") {
        return Target::Remote { unc: true };
    }
    // 역슬래시를 `/`로 바꿔 `file:\\host\share`와 `file://host/share`를 같게 본다.
    let normalized = t.replace('\\', "/").to_ascii_lowercase();
    if let Some(rest) = normalized.strip_prefix("file:") {
        return if file_tail_is_remote(rest) { Target::Remote { unc: true } } else { Target::LocalFile };
    }
    if SCHEME.is_match(&normalized) && normalized.contains("://") { Target::Remote { unc: false } } else { Target::Other }
}

/// 관계 파일 이름과 `Target`으로 가리키는 부분의 이름을 구한다(내부 관계). `word/_rels/document.xml.rels`의 기준 디렉터리는 `word`다.
fn resolve_target(rels_name: &str, target: &str) -> String {
    // 소문자 변환은 바이트 길이를 바꿀 수 있으므로(`İ` 등) 길이를 보존하는 ASCII 변환으로 찾고, 원본은 `get`으로 자른다.
    let base = rels_name.to_ascii_lowercase().rfind("/_rels/").and_then(|i| rels_name.get(..i)).map_or(String::new(), str::to_string);
    let target = unquote(target);
    if let Some(rooted) = target.strip_prefix('/') {
        return posix_normpath(rooted);
    }
    posix_normpath(&posix_join(&base, &target))
}

/// 관계 파일(`.rels`)을 본다. 외부 참조(원격 템플릿, 외부 객체, UNC)와 내부 관계의 종류(매크로 프로젝트, OLE, ActiveX, 패키지)를 종류로 정한다.
/// Word는 부분 이름이 아니라 관계 종류와 content type으로 부분을 해석하므로 이름만 보면 이름을 바꿔 피할 수 있다. 일반 하이퍼링크는 지표를 내지 않는다.
fn check_relationships(doc: &roxmltree::Document, name: &str, macro_enabled: bool, report: &mut FileReport) {
    let shown = preview(name);
    for rel in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Relationship") {
        let external = rel.attribute("TargetMode").is_some_and(|m| m.eq_ignore_ascii_case("External"));
        let ty = rel.attribute("Type").unwrap_or("").rsplit('/').next().unwrap_or("");
        let target = rel.attribute("Target").unwrap_or("");
        let detail = preview(&format!("{ty} -> {target}"));
        if !external {
            let part = preview(&resolve_target(name, target));
            match ty {
                "vbaProject" => {
                    let (severity, why) = if macro_enabled { (Severity::High, "VBA macro project in a macro-enabled document") } else { (Severity::Critical, "VBA macro project inside a macro-free format (disguised macro document)") };
                    push_once(report, severity, "OOXML_VBA_PROJECT", &part, why);
                }
                "oleObject" => push_once(report, Severity::High, "OOXML_OLE_OBJECT", &part, "Embedded OLE object"),
                "control" => push_once(report, Severity::High, "OOXML_ACTIVEX", &part, "ActiveX control part"),
                "package" => push_once(report, Severity::Medium, "OOXML_EMBEDDED_PACKAGE", &part, "Embedded Office package (not inspected recursively yet)"),
                _ => {}
            }
            continue;
        }
        let kind = target_kind(target);
        match ty {
            // 원격 템플릿은 열 때 가져온다(T1221). 로컬 경로의 템플릿은 사용자가 만든 문서에 흔하다.
            "attachedTemplate" if matches!(kind, Target::Remote { .. }) => push_once(report, Severity::Critical, "OOXML_REMOTE_TEMPLATE", &shown, &detail),
            "attachedTemplate" => push_once(report, Severity::Low, "OOXML_LOCAL_TEMPLATE", &shown, &detail),
            "oleObject" | "subDocument" | "frame" | "externalLinkPath" => push_once(report, Severity::High, "OOXML_EXTERNAL_OBJECT", &shown, &detail),
            "image" => push_once(report, Severity::Medium, "OOXML_EXTERNAL_IMAGE", &shown, &detail),
            _ => {}
        }
        match kind {
            Target::Remote { unc: true } => push_once(report, Severity::High, "OOXML_UNC_TARGET", &shown, &detail),
            Target::LocalFile if ty != "attachedTemplate" => push_once(report, Severity::Low, "OOXML_LOCAL_FILE_TARGET", &shown, &detail),
            _ => {}
        }
    }
}

/// UNC 경로가 토큰 맨 앞에 오는가. Word 지시문은 역슬래시를 두 배로 쓰므로 `"\\\\host`와 `"\\host`를 모두 본다.
/// 로컬 경로의 `c:\\dir`는 `:` 뒤에 오므로 맞지 않는다.
static FIELD_UNC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?:^|["'\s])\\{2,4}[A-Za-z0-9_.$-]"#).unwrap());
static FIELD_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\b([a-z][a-z0-9+.-]*)://([^\s"'/]*)"#).unwrap());

/// 지시문이 가리키는 원격 대상. 원격이 아니면 `None`이다. 값은 UNC·file 호스트(NTLM 해시가 새는 대상)인지다.
/// `file:`은 역슬래시를 `/`로 바꿔 본다. 그 변환을 URL 검사에 쓰면 두 배 역슬래시의 로컬 경로(`c:\\dir`)가 `c://dir`이 되어 URL로 오인된다.
fn field_remote(instr: &str) -> Option<bool> {
    if FIELD_UNC.is_match(instr) {
        return Some(true);
    }
    let normalized = instr.replace('\\', "/").to_ascii_lowercase();
    if normalized.match_indices("file:").any(|(i, _)| file_tail_is_remote(&normalized[i + 5..])) {
        return Some(true);
    }
    FIELD_URL.captures_iter(instr).any(|c| !c[1].eq_ignore_ascii_case("file")).then_some(false)
}

/// `s`를 `MAX_FIELD_TEXT`바이트까지만 남긴다(글자 경계에서 자른다). 잘렸으면 `true`다.
fn cap_text(s: &str) -> (&str, bool) {
    if s.len() <= MAX_FIELD_TEXT {
        return (s, false);
    }
    let mut end = MAX_FIELD_TEXT;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

fn classify_field(instr: &str, name: &str, report: &mut FileReport) {
    // 어떤 경로로 들어와도 분석하는 길이를 상한으로 막는다(`fldSimple`의 `instr` 속성은 `push_capped`를 거치지 않는다).
    let (instr, _) = cap_text(instr);
    let word = instr.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
    let shown = preview(name);
    match word.as_str() {
        "DDEAUTO" | "DDE" => push_once(report, Severity::Critical, "OOXML_DDE_FIELD", &shown, &preview(instr.trim())),
        "INCLUDETEXT" | "INCLUDEPICTURE" | "LINK" | "IMPORT" => {
            let Some(unc) = field_remote(instr) else { return };
            // 브라우저에서 붙여 넣은 그림은 Word가 `INCLUDEPICTURE "https://..."`로 쓴다. http(s)는 MEDIUM이고 UNC·file 호스트와 텍스트·링크 포함은 HIGH다.
            let severity = if word == "INCLUDEPICTURE" && !unc { Severity::Medium } else { Severity::High };
            push_once(report, severity, "OOXML_DANGEROUS_FIELD", &shown, &preview(instr.trim()));
        }
        _ => {}
    }
}

/// 중첩 필드 추적 깊이와 이어 붙이는 지시문 길이(바이트)의 상한. 깊게 중첩된 필드가 안쪽 텍스트를 바깥마다 복사하면 제곱 시간이 든다.
/// 정상 문서의 긴 지시문(참고문헌 관리자의 `ADDIN ... CSL_CITATION {...}`은 수십 KB까지 간다)을 담을 만큼 크게 잡는다.
const MAX_FIELD_DEPTH: usize = 32;
const MAX_FIELD_TEXT: usize = 64 * 1024;

/// `dst`에 `src`를 `MAX_FIELD_TEXT`바이트까지만 이어 붙인다. 잘렸으면 `true`다(글자 경계에서 자른다).
fn push_capped(dst: &mut String, src: &str) -> bool {
    let room = MAX_FIELD_TEXT.saturating_sub(dst.len());
    if src.len() <= room {
        dst.push_str(src);
        return false;
    }
    let mut end = room;
    while !src.is_char_boundary(end) {
        end -= 1;
    }
    dst.push_str(&src[..end]);
    true
}

/// 열려 있는 필드 하나. 지시문은 여러 run에 나뉘고 다른 필드를 품을 수 있다.
#[derive(Default)]
struct OpenField {
    /// 이 필드 자신의 지시문.
    own: String,
    /// 품은 필드의 지시문까지 읽은 순서대로 이어 붙인 것.
    all: String,
    nested: bool,
    /// 지시문 키워드 자리(맨 앞)에서 안쪽 필드가 시작했다: 키워드를 계산해서 숨기는 방식이다.
    nested_at_keyword: bool,
    /// 지시문이 상한에서 잘렸다. 뒤쪽은 분석하지 못했다.
    truncated: bool,
}

fn finish_field(field: OpenField, name: &str, report: &mut FileReport) {
    classify_field(&field.own, name, report);
    if field.nested {
        // 안쪽 필드의 지시문까지 이어 붙여 다시 본다(`INCLUDETEXT` + 중첩 `QUOTE "http://..."` 같은 조립).
        classify_field(&field.all, name, report);
        if field.nested_at_keyword {
            push_once(report, Severity::High, "OOXML_COMPUTED_FIELD", &preview(name), "Field type is computed by a nested field (DDE obfuscation pattern)");
        }
    }
    if field.truncated {
        report.mark_incomplete(&format!("{}: field instruction truncated at {MAX_FIELD_TEXT} bytes; the rest was not analyzed", preview(name)));
    }
}

/// 본문의 필드 지시문을 본다. 한 필드의 지시문은 여러 run에 나뉠 수 있어(`DDE` + `AUTO ...`) begin부터 end까지 이어 붙여 읽는다.
fn check_fields(doc: &roxmltree::Document, name: &str, report: &mut FileReport) {
    let mut stack: Vec<OpenField> = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "fldChar" => match node.attributes().find(|a| a.name() == "fldCharType").map(|a| a.value()) {
                Some("begin") => {
                    if stack.len() >= MAX_FIELD_DEPTH {
                        report.mark_incomplete(&format!("{}: field nesting deeper than {MAX_FIELD_DEPTH}; the remaining fields were not analyzed", preview(name)));
                        break;
                    }
                    if let Some(parent) = stack.last_mut() {
                        parent.nested = true;
                        parent.nested_at_keyword |= parent.own.trim().is_empty();
                    }
                    stack.push(OpenField::default());
                }
                Some("end") => {
                    if let Some(field) = stack.pop() {
                        if let Some(parent) = stack.last_mut() {
                            parent.truncated |= push_capped(&mut parent.all, &field.all);
                        }
                        finish_field(field, name, report);
                    }
                }
                _ => {}
            },
            "instrText" => {
                if let Some(top) = stack.last_mut() {
                    let t = node.text().unwrap_or("");
                    top.truncated |= push_capped(&mut top.own, t);
                    top.truncated |= push_capped(&mut top.all, t);
                }
            }
            "fldSimple" => {
                if let Some(instr) = node.attributes().find(|a| a.name() == "instr").map(|a| a.value()) {
                    if cap_text(instr).1 {
                        report.mark_incomplete(&format!("{}: field instruction truncated at {MAX_FIELD_TEXT} bytes; the rest was not analyzed", preview(name)));
                    }
                    classify_field(instr, name, report);
                }
            }
            _ => {}
        }
    }
    while let Some(field) = stack.pop() {
        finish_field(field, name, report);
    }
}

/// 검사 우선순위: 관계 파일, `word/` 아래 XML, 그 밖의 XML, 이름으로는 XML이 아닌 부분 순이다. 부분 수 상한에 걸려도 주 문서와 관계가 먼저 검사되도록 한다.
fn scan_priority(name: &str) -> u8 {
    let lower = name.to_lowercase();
    if lower.ends_with(".rels") {
        0
    } else if lower.starts_with("word/") && is_xml_part(name) {
        1
    } else if is_xml_part(name) {
        2
    } else {
        3
    }
}

/// DOCX 계열의 보안 지표를 `report`에 더한다. 압축 해제는 공유 예산(`spent`) 안에서만 하고, 읽을 항목은 `limits.max_files`개까지다.
/// 상한을 넘는 항목과 예산을 넘는 부분은 미완료로 남긴다.
pub fn inspect_security(path: &Path, limits: &Limits, macro_enabled: bool, spent: &mut u64, report: &mut FileReport) {
    let Some(mut zip) = open_zip(path) else { return };
    let all: Vec<String> = zip.file_names().map(|r| r.map(|c| c.into_owned()).unwrap_or_else(|_| "<invalid-name>".into())).collect();
    if all.len() > limits.max_files {
        report.mark_incomplete(&format!("security scan limited to the first {} of {} entries", limits.max_files, all.len()));
    }
    let names: Vec<String> = all.into_iter().take(limits.max_files).collect();
    check_part_names(&names, macro_enabled, report);
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by_key(|&i| scan_priority(&names[i]));
    let mut scanned = 0usize;
    for index in order {
        let name = &names[index];
        match classify_member(&mut zip, index, name) {
            Member::Other => continue,
            Member::Ole => {
                // 매크로 프로젝트는 위의 이름 규칙·관계 규칙이 이미 `OOXML_VBA_PROJECT`로 낸다.
                if !name.to_lowercase().ends_with("vbaproject.bin") {
                    push_once(report, Severity::High, "OOXML_OLE_OBJECT", &preview(name), "OLE compound file content");
                }
                continue;
            }
            Member::Xml => {}
        }
        scanned += 1;
        if scanned > MAX_PARTS_SCANNED {
            report.mark_incomplete(&format!("security scan stopped after {MAX_PARTS_SCANNED} XML parts"));
            break;
        }
        let Some(text) = read_xml_part(&mut zip, index, name, limits, spent, report) else { continue };
        check_xml_attacks(&text, name, report);
        // 모든 XML 부분을 파싱하고, 파싱하지 못하면 미완료로 남긴다(필드 표지가 없어도 건너뛰지 않는다).
        let Some(doc) = parse_or_note(&text, name, report) else { continue };
        // 이름이 `.rels`가 아니어도 루트가 `Relationships`면 관계 파일로 읽는다(이름으로 숨긴 경우).
        if name.to_lowercase().ends_with(".rels") || doc.root_element().tag_name().name() == "Relationships" {
            check_relationships(&doc, name, macro_enabled, report);
        } else {
            check_fields(&doc, name, report);
        }
    }
}

#[cfg(test)]
mod budget_tests {
    use std::io::Write;

    use super::*;

    /// 예산을 넘겨 건너뛰는 부분도 읽은 만큼 `spent`에서 빠진다(안 빼면 부분마다 같은 만큼 다시 풀 수 있다).
    #[test]
    fn oversized_part_is_charged_to_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        w.start_file("big.xml", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        w.write_all(&vec![b' '; 600 * 1024]).unwrap();
        w.finish().unwrap();
        let mut zip = open_zip(&path).unwrap();
        let limits = Limits { max_total: 1024 * 1024, ..Limits::default() };
        let mut spent = 600 * 1024;
        let mut report = FileReport::new(&path, None);
        assert!(read_xml_part(&mut zip, 0, "big.xml", &limits, &mut spent, &mut report).is_none());
        assert!(report.incomplete);
        assert!(spent > 600 * 1024 + 400 * 1024, "읽은 바이트가 예산에서 빠지지 않았다: {spent}");
    }
}
