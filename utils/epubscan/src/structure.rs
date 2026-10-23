//! EPUB 구조 검사: epubscan이 하는 일 중 "뷰어가 렌더링하지 못하는 구조"를 찾는 부분.
//! (원래 `detect_mojibake epub-structure`였고 `epubscan structure`로 옮겼다.)
//!
//! epubcheck는 XML 규격만 보므로 `<html:html>`처럼 접두사가 붙은 본문도 통과시키지만 뷰어의
//! 정제기는 이를 거부한다. 단 bad_mimetype은 표준 위반일 뿐 뷰어를 깨뜨리지 않는다.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use indexmap::IndexMap;
use regex::bytes::Regex;
use serde::{Deserialize, Serialize};
use zip::CompressionMethod;

use detect_mojibake::reader::{Zip, open_zip};
use detect_mojibake::util::{category_of, posix_dirname, posix_join, posix_normpath, unquote};
use detect_mojibake::{Log, limited, list_library_files, now_iso, pool, round1, thousands};
use rayon::prelude::*;

pub const EPUB_MIMETYPE: &[u8] = b"application/epub+zip";
pub const ISSUE_NOT_ZIP: &str = "not_zip";
pub const ISSUE_BAD_MIMETYPE: &str = "bad_mimetype";
pub const ISSUE_NO_CONTAINER: &str = "no_container";
pub const ISSUE_OPF_MISSING: &str = "opf_missing";
pub const ISSUE_MEMBER_UNREADABLE: &str = "member_unreadable";
pub const ISSUE_SCAN_ERROR: &str = "scan_error";
pub const ISSUE_OPF_UNPARSABLE: &str = "opf_unparsable";
pub const ISSUE_SPINE_MEMBER_MISSING: &str = "spine_member_missing";
pub const ISSUE_PREFIXED_ROOT: &str = "prefixed_root";
pub const EPUB_ISSUES: [&str; 9] =
    [ISSUE_NOT_ZIP, ISSUE_BAD_MIMETYPE, ISSUE_NO_CONTAINER, ISSUE_OPF_MISSING, ISSUE_MEMBER_UNREADABLE, ISSUE_SCAN_ERROR, ISSUE_OPF_UNPARSABLE, ISSUE_SPINE_MEMBER_MISSING, ISSUE_PREFIXED_ROOT];

/// 루트 태그는 파일 앞부분에 있다. DOCTYPE 내부 선언(<!ELEMENT)과 주석은 건너뛴다.
const ROOT_TAG_HEAD_BYTES: usize = 4096;
/// container.xml, OPF는 작다. 비정상적으로 큰 멤버는 읽지 않는다.
const MAX_XML_BYTES: usize = 5 * 1024 * 1024;

// Python 원본은 `<(?![?!])([^\s/>]+)`. Rust regex에는 lookahead가 없어 첫 글자 조건으로 푼다.
static ROOT_TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?-u)<([^?!\s/>][^\s/>]*)").unwrap());
static COMMENT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s-u)<!--.*?-->").unwrap());

// 루트 요소 검사는 마크업 문서에만 한다. spine에 오디오 같은 바이너리가 들어 있는 EPUB이 있고,
// 그 앞부분에서 우연히 찾은 '<x:y'를 접두사 루트로 오탐한 적이 있다.
const MARKUP_MEDIA_TYPES: [&str; 5] = ["application/xhtml+xml", "text/html", "application/xml", "text/xml", "image/svg+xml"];
const MARKUP_EXTENSIONS: [&str; 5] = [".xhtml", ".html", ".htm", ".xml", ".svg"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EpubResult {
    pub path: String,
    pub category: String,
    pub issues: Vec<String>,
    pub examples: IndexMap<String, String>,
}

// ---- posixpath / urllib 대응 ----

// ---- ZIP 읽기 ----

/// ZIP 멤버를 읽는다. 없거나 깨졌거나 limit를 넘으면 None. 구분은 호출자가 이름 목록으로 한다.
fn read_member(zip: &mut Zip, name: &str, limit: usize) -> Option<Vec<u8>> {
    let entry = zip.by_name(name).ok()?;
    let mut data = Vec::new();
    entry.take(limit as u64 + 1).read_to_end(&mut data).ok()?;
    (data.len() <= limit).then_some(data)
}

/// 멤버 앞부분만 읽는다. 읽을 수 없으면 None(Bad magic number, Overlapped entries 등).
fn read_head(zip: &mut Zip, name: &str, size: usize) -> Option<Vec<u8>> {
    let entry = zip.by_name(name).ok()?;
    let mut data = Vec::new();
    entry.take(size as u64).read_to_end(&mut data).ok()?;
    Some(data)
}

fn parse_xml(bytes: &[u8]) -> Option<roxmltree::Document<'_>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let opts = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(text, opts).ok()
}

/// XML 문서 루트 요소의 원래 이름(접두사 포함).
pub fn root_tag_name(head: &[u8]) -> Option<String> {
    let stripped = COMMENT_RE.replace_all(head, &b""[..]);
    ROOT_TAG_RE.captures(&stripped).map(|m| String::from_utf8_lossy(&m[1]).into_owned())
}

fn is_markup(member: &str, media_type: &str) -> bool {
    if !media_type.is_empty() {
        return MARKUP_MEDIA_TYPES.contains(&media_type.to_lowercase().as_str());
    }
    // `posixpath.splitext`: 파일명 앞의 점은 확장자가 아니고, 디렉토리 구간의 점도 세지 않는다.
    let base = member.rsplit('/').next().unwrap_or("").trim_start_matches('.');
    let ext = base.rfind('.').map(|i| base[i..].to_lowercase()).unwrap_or_default();
    MARKUP_EXTENSIONS.contains(&ext.as_str())
}

/// spine 순서대로 (본문 문서의 ZIP 멤버 경로, media-type)을 돌려준다.
fn spine_members(opf: &roxmltree::Document, opf_path: &str) -> Vec<(String, String)> {
    let mut manifest: HashMap<Option<String>, (String, String)> = HashMap::new();
    for el in opf.descendants().filter(|n| n.is_element() && n.tag_name().name() == "item") {
        if let Some(href) = el.attribute("href").filter(|h| !h.is_empty()) {
            manifest.insert(el.attribute("id").map(str::to_owned), (href.to_owned(), el.attribute("media-type").unwrap_or("").to_owned()));
        }
    }
    let base = posix_dirname(opf_path);
    let mut members = Vec::new();
    for el in opf.descendants().filter(|n| n.is_element() && n.tag_name().name() == "itemref") {
        if let Some((href, media)) = manifest.get(&el.attribute("idref").map(str::to_owned)) {
            let target = unquote(href);
            let target = target.split('#').next().unwrap_or("");
            members.push((posix_normpath(&posix_join(&base, target)), media.clone()));
        }
    }
    members
}

fn check_epub_members(zip: &mut Zip, root_tag: &dyn Fn(&[u8]) -> Option<String>, flag: &mut dyn FnMut(&str, &str)) {
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let name_set: HashSet<&str> = names.iter().map(String::as_str).collect();

    let first_name = names.first().cloned();
    let first_stored = if zip.is_empty() { None } else { zip.by_index(0).ok().map(|f| f.compression() == CompressionMethod::Stored) };
    let mimetype = if first_name.as_deref() == Some("mimetype") { read_member(zip, "mimetype", 64) } else { None };
    let ok_mimetype = first_stored == Some(true) && mimetype.as_deref().map(<[u8]>::trim_ascii) == Some(EPUB_MIMETYPE);
    if !ok_mimetype {
        flag(ISSUE_BAD_MIMETYPE, first_name.as_deref().unwrap_or(""));
    }

    // 없는 것(고칠 수 있다)과 있지만 읽을 수 없는 것(ZIP 내부 손상이나 비정상적 크기)을 구분한다.
    const CONTAINER: &str = "META-INF/container.xml";
    let Some(container) = read_member(zip, CONTAINER, MAX_XML_BYTES) else {
        flag(if name_set.contains(CONTAINER) { ISSUE_MEMBER_UNREADABLE } else { ISSUE_NO_CONTAINER }, CONTAINER);
        return;
    };
    let opf_path = parse_xml(&container).and_then(|doc| {
        let rootfile = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == "rootfile")?;
        Some(posix_normpath(&unquote(rootfile.attribute("full-path").unwrap_or(""))))
    });
    let Some(opf_path) = opf_path else {
        flag(ISSUE_NO_CONTAINER, CONTAINER);
        return;
    };

    let Some(opf_bytes) = read_member(zip, &opf_path, MAX_XML_BYTES) else {
        flag(if name_set.contains(opf_path.as_str()) { ISSUE_MEMBER_UNREADABLE } else { ISSUE_OPF_MISSING }, &opf_path);
        return;
    };
    let Some(opf) = parse_xml(&opf_bytes) else {
        flag(ISSUE_OPF_UNPARSABLE, &opf_path);
        return;
    };

    for (member, media_type) in spine_members(&opf, &opf_path) {
        if !name_set.contains(member.as_str()) {
            flag(ISSUE_SPINE_MEMBER_MISSING, &member);
            continue;
        }
        if !is_markup(&member, &media_type) {
            continue;
        }
        let Some(head) = read_head(zip, &member, ROOT_TAG_HEAD_BYTES) else {
            flag(ISSUE_MEMBER_UNREADABLE, &member);
            continue;
        };
        if root_tag(&head).is_some_and(|n| n.contains(':')) {
            flag(ISSUE_PREFIXED_ROOT, &member);
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown".into())
}

/// 루트 태그 추출기를 바꿔 끼울 수 있는 검사. 테스트가 예상 못 한 실패를 주입하는 데 쓴다.
pub fn inspect_epub_structure_with(path_str: &str, root_tag: &dyn Fn(&[u8]) -> Option<String>) -> EpubResult {
    let path = Path::new(path_str);
    let mut issues: Vec<String> = Vec::new();
    let mut examples: IndexMap<String, String> = IndexMap::new();
    let mut flag = |code: &str, member: &str| {
        if !issues.iter().any(|i| i == code) {
            issues.push(code.to_owned());
            examples.insert(code.to_owned(), member.to_owned());
        }
    };

    match open_zip(path) {
        None => flag(ISSUE_NOT_ZIP, ""),
        Some(mut zip) => {
            // 한 파일 때문에 수천 건 스캔이 중단되면 안 된다.
            let outcome = catch_unwind(AssertUnwindSafe(|| check_epub_members(&mut zip, root_tag, &mut flag)));
            if let Err(payload) = outcome {
                let msg: String = format!("panic: {}", panic_message(payload.as_ref())).chars().take(120).collect();
                flag(ISSUE_SCAN_ERROR, &msg);
            }
        }
    }
    EpubResult { path: path_str.to_owned(), category: category_of(path), issues, examples }
}

/// EPUB 한 건의 구조를 검사한다. issues는 중복 없는 코드 목록이고 examples는 코드별 첫 사례 멤버다.
pub fn inspect_epub_structure(path_str: &str) -> EpubResult {
    inspect_epub_structure_with(path_str, &root_tag_name)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpubReport {
    pub scanned_at: String,
    pub root: String,
    pub total: usize,
    pub flagged_count: usize,
    pub counts: IndexMap<String, usize>,
    pub elapsed_sec: f64,
    pub flagged: Vec<EpubResult>,
}

/// 라이브러리의 EPUB 구조를 훑어 문제가 있는 파일 목록을 모은다.
pub fn scan_epub_structure(root: &Path, workers: usize, limit: usize, log: Log) -> EpubReport {
    let files = limited(list_library_files(root).into_iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub"))).collect(), limit);
    log(&format!("INFO EPUB {}건, 워커 {}개", thousands(files.len()), workers));

    let started = Instant::now();
    let results: Vec<EpubResult> = pool(workers).install(|| files.par_iter().map(|p| inspect_epub_structure(&p.to_string_lossy())).collect());

    let flagged: Vec<EpubResult> = results.iter().filter(|r| !r.issues.is_empty()).cloned().collect();
    let counts = EPUB_ISSUES.iter().map(|code| (code.to_string(), flagged.iter().filter(|r| r.issues.iter().any(|i| i == code)).count())).collect();
    EpubReport {
        scanned_at: now_iso(),
        root: root.to_string_lossy().into_owned(),
        total: results.len(),
        flagged_count: flagged.len(),
        counts,
        elapsed_sec: round1(started.elapsed().as_secs_f64()),
        flagged,
    }
}

pub fn print_epub_structure_summary(report: &EpubReport, out: &mut dyn Write) -> io::Result<()> {
    let total = report.total.max(1) as f64;
    writeln!(out, "\nEPUB {}건 검사 / {:.1}분, 문제 {}건 ({:.2}%)", thousands(report.total), report.elapsed_sec / 60.0, thousands(report.flagged_count), report.flagged_count as f64 / total * 100.0)?;
    for (code, n) in &report.counts {
        if *n > 0 {
            writeln!(out, "  {code:<22}{:>8}", thousands(*n))?;
        }
    }
    Ok(())
}
