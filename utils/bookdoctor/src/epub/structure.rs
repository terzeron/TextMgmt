//! EPUB 구조 검사: epubscan이 하는 일 중 "뷰어가 렌더링하지 못하는 구조"를 찾는 부분.
//! (원래 `detect_mojibake epub-structure`였고 `epubscan structure`로 옮겼다.)
//!
//! epubcheck는 XML 규격만 보므로 `<html:html>`처럼 접두사가 붙은 본문도 통과시키지만 뷰어의
//! 정제기는 이를 거부한다. 단 bad_mimetype은 표준 위반일 뿐 뷰어를 깨뜨리지 않는다.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::LazyLock;

use regex::bytes::Regex;
use scan_common::{Category, FileReport, Severity};
use zip::CompressionMethod;

use corpus_common::reader::{Zip, open_zip};
use corpus_common::util::{posix_dirname, posix_join, posix_normpath, unquote};

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

/// 옛 epubscan 이슈 이름 → (새 코드, 분류, 심각도). Task 16이 "옛 이슈 ⊆ 새 코드"를 증명할 때 쓴다.
///
/// `scan_error`는 지표가 아니라 "검사 불완전"(`mark_incomplete("panic: …")`)으로 보고한다. 타입이 "불완전"을
/// 표현하지 못하므로 코드를 빈 문자열로 두고 분류·심각도는 자리표시(`Defect`/`High`)다. 소비자는 빈 코드를
/// 지표 코드가 아니라 `report.incomplete`로 대응시켜야 한다.
pub const ISSUE_TO_CODE: [(&str, &str, Category, Severity); 9] = [
    (ISSUE_NOT_ZIP, "NOT_ZIP", Category::Defect, Severity::High),
    (ISSUE_BAD_MIMETYPE, "BAD_MIMETYPE", Category::Render, Severity::Low),
    (ISSUE_NO_CONTAINER, "NO_CONTAINER", Category::Render, Severity::High),
    (ISSUE_OPF_MISSING, "OPF_MISSING", Category::Render, Severity::High),
    (ISSUE_MEMBER_UNREADABLE, "MEMBER_UNREADABLE", Category::Defect, Severity::High),
    (ISSUE_SCAN_ERROR, "", Category::Defect, Severity::High),
    (ISSUE_OPF_UNPARSABLE, "OPF_UNPARSABLE", Category::Render, Severity::High),
    (ISSUE_SPINE_MEMBER_MISSING, "SPINE_MEMBER_MISSING", Category::Render, Severity::High),
    // Low: 뷰어(frontend/src/ViewEPUB.jsx:43 `stripElementPrefixes`, :61에서 렌더 전에 호출)가 접두사를 벗겨 렌더링한다.
    // 백엔드 미리보기는 대신 처리하지 않지만 Medium으로 올리지 않는다. `declare_unbound_prefixes`
    // (backend/src/book_manager/epub.rs:61)는 OPF의 선언 안 된 접두사만 고치고(호출은 :245, :458, 둘 다 OPF),
    // 본문 문서나 선언된(bound) 접두사 루트 `<html:html xmlns:html=…>`는 건드리지 않는다. 그러나 백엔드는 본문을
    // 렌더링하지 않고 뷰어로 그대로 보내므로 막히는 곳이 없다.
    (ISSUE_PREFIXED_ROOT, "PREFIXED_ROOT", Category::Render, Severity::Low),
];

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

// ---- ZIP 읽기 ----

/// ZIP 멤버를 읽는다. 없거나 깨졌거나 limit를 넘으면 None. 구분은 호출자가 이름 목록으로 한다.
pub(crate) fn read_member(zip: &mut Zip, name: &str, limit: usize) -> Option<Vec<u8>> {
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

/// container.xml의 첫 rootfile `full-path`(정규화). render 검사도 같은 규칙으로 OPF를 찾는다.
pub(crate) fn rootfile_path(container: &[u8]) -> Option<String> {
    let doc = parse_xml(container)?;
    let rootfile = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == "rootfile")?;
    Some(posix_normpath(&unquote(rootfile.attribute("full-path").unwrap_or(""))))
}

pub fn parse_xml(bytes: &[u8]) -> Option<roxmltree::Document<'_>> {
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

pub(crate) fn is_markup(member: &str, media_type: &str) -> bool {
    if !media_type.is_empty() {
        return MARKUP_MEDIA_TYPES.contains(&media_type.to_lowercase().as_str());
    }
    // `posixpath.splitext`: 파일명 앞의 점은 확장자가 아니고, 디렉토리 구간의 점도 세지 않는다.
    let base = member.rsplit('/').next().unwrap_or("").trim_start_matches('.');
    let ext = base.rfind('.').map(|i| base[i..].to_lowercase()).unwrap_or_default();
    MARKUP_EXTENSIONS.contains(&ext.as_str())
}

/// spine 순서대로 (본문 문서의 ZIP 멤버 경로, media-type)을 돌려준다.
pub fn spine_members(opf: &roxmltree::Document, opf_path: &str) -> Vec<(String, String)> {
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
    let opf_path = rootfile_path(&container);
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
pub fn inspect_with(path: &Path, report: &mut FileReport, root_tag: &dyn Fn(&[u8]) -> Option<String>) {
    // 같은 이슈는 한 번만, 첫 사례 멤버로 남긴다.
    let mut seen: HashSet<&'static str> = HashSet::new();
    let mut flag = |issue: &str, member: &str| {
        let Some(&(old, code, category, severity)) = ISSUE_TO_CODE.iter().find(|row| row.0 == issue) else { return };
        if !seen.insert(old) {
            return;
        }
        report.push(category, severity, code, member, &format!("EPUB 구조: {old}"));
    };

    match open_zip(path) {
        None => flag(ISSUE_NOT_ZIP, ""),
        Some(mut zip) => {
            // 한 파일 때문에 수천 건 스캔이 중단되면 안 된다.
            let outcome = catch_unwind(AssertUnwindSafe(|| check_epub_members(&mut zip, root_tag, &mut flag)));
            if let Err(payload) = outcome {
                let msg: String = format!("panic: {}", panic_message(payload.as_ref())).chars().take(120).collect();
                report.mark_incomplete(&msg);
            }
        }
    }
}

/// EPUB 한 건의 구조를 검사해 `report`에 더한다. 이슈마다 첫 사례 멤버를 `location`에 남긴다.
pub fn inspect(path: &Path, report: &mut FileReport) {
    inspect_with(path, report, &root_tag_name);
}
