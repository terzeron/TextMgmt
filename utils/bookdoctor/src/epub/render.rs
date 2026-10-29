//! EPUB 렌더링·손상 검사: 브라우저 뷰어가 책을 못 여는 이유와 ZIP 멤버 손상을 찾는다.
//!
//! 규칙의 근거는 브라우저 진단 `frontend/src/EpubDiagnose.js`와 뷰어 `frontend/src/ViewEPUB.jsx`다.
//! 같은 규칙을 한 번만 Rust로 옮겼고, 코드마다 근거 함수를 주석으로 남겼다.
//!
//! 읽기는 모두 한도 안에서만 한다. 멤버 하나는 `Limits.max_member`, 파일 전체는 `Limits.max_total`까지다.
//! 한도는 ZIP이 *선언한* 크기가 아니라 실제로 읽은 바이트로 센다. 한도를 넘어 못 본 부분은 결함이 아니므로
//! 지표를 만들지 않고 `incomplete`로만 표시한다.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use roxmltree::{Document, Node};
use scan_common::{Category, FileReport, Severity};

use corpus_common::reader::{Zip, open_zip};
use corpus_common::util::{posix_dirname, posix_join, posix_normpath, unquote};

use super::security::Limits;
use super::structure::{is_markup, parse_xml, rootfile_path, spine_members};

// EpubDiagnose.js 상단 상수.
const OPF_NS: &str = "http://www.idpf.org/2007/opf";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";
const NCX_NS: &str = "http://www.daisy.org/z3986/2005/ncx/";

const CONTAINER: &str = "META-INF/container.xml";
/// container.xml, OPF, CSS처럼 작아야 하는 멤버의 읽기 상한. structure.rs의 `MAX_XML_BYTES`와 같다.
const MAX_SMALL_BYTES: usize = 5 * 1024 * 1024;

/// 근거: backend/src/book_manager/preview.rs:26 `FONT_SIZE_LIMIT = 500 * 1024`. 이 크기를 넘는 폰트는
/// 미리보기에서 빼므로 책이 글꼴 없이 보인다(preview.rs:527은 선언 크기 `size() > FONT_SIZE_LIMIT`).
const FONT_SIZE_LIMIT: u64 = 500 * 1024;
// preview.rs:28-29의 `FONT_EXTENSIONS`, `FONT_MEDIA_TYPES`와 같다.
const FONT_EXTENSIONS: [&str; 4] = [".ttf", ".otf", ".woff", ".woff2"];
const FONT_MEDIA_TYPES: [&str; 8] = ["font/ttf", "font/otf", "font/woff", "font/woff2", "application/font-ttf", "application/font-woff", "application/font-woff2", "application/x-font-ttf"];

/// 난독화 임계값. 한 XHTML 안에서 크기 0 요소 또는 U+202E가 이만큼 이상이면 난독화로 본다.
/// 난독화 EPUB은 글자마다 숨은 더미 요소(`font-size:0%`, 메모리 노트의 eeḕ 요소)를 끼우고 RTL 오버라이드
/// (U+202E)로 순서를 뒤집는다. 정상 책에는 거의 없고 난독화 책은 쪽당 수백 개라 20은 보수적인 하한이다.
/// 실제 난독화 EPUB으로는 검증하지 않았다(scripts/epub_deobfuscate.py와 표본이 이 작업 트리에 없다).
const OBFUSCATION_THRESHOLD: usize = 20;
/// 문서 하나가 모을 선택자와 스타일시트의 상한. 악성 CSS가 선택자 × 요소 곱으로 시간을 쓰지 못하게 한다.
const MAX_SELECTORS_PER_DOC: usize = 10_000;
const MAX_STYLESHEETS_PER_DOC: usize = 64;

/// EpubDiagnose.js `findUndeclaredPrefixes`의 두 정규식. JS의 `\w`는 ASCII라 같은 범위로 쓴다.
static DECLARED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"xmlns:([A-Za-z0-9_]+)\s*=").unwrap());
static USED_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[< ]\s*([A-Za-z0-9_]+):[A-Za-z0-9_]+").unwrap());
/// EpubDiagnose.js `checkGuide`: `<guide>…</guide>` 구간과 그 안의 `href="…"`.
static GUIDE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<guide>(.*?)</guide>").unwrap());
static HREF_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"href="([^"]+)""#).unwrap());
/// `font-size`를 0으로 만드는 선언(`0`, `0%`, `0px`, `0em`, `0rem`, `0pt`). `0.5em`이나 `10px`은 맞지 않는다.
static ZERO_SIZE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[\s;{])font-size\s*:\s*0(?:\.0+)?(?:%|px|rem|em|pt)?\s*(?:;|!|$)").unwrap());
static CSS_RULE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^{}]+)\{([^{}]*)\}").unwrap());

// ---------------------------------------------------------------- 결과 모으기

#[derive(Default)]
struct Sink {
    findings: Vec<(Category, Severity, &'static str, String, String)>,
    seen: HashSet<&'static str>,
    notes: Vec<String>,
}

impl Sink {
    /// 같은 코드는 한 번만, 첫 사례를 `location`에 남긴다.
    fn flag(&mut self, category: Category, severity: Severity, code: &'static str, location: &str, detail: &str) {
        if self.seen.insert(code) {
            self.findings.push((category, severity, code, location.to_string(), detail.to_string()));
        }
    }
}

/// 실제로 읽은 바이트로 센 전체 예산.
struct Budget {
    remaining: u64,
    exhausted: bool,
}

/// 읽은 바이트를 세고, 요청하면 내용을 담는 `Write`. 읽기가 중간에 실패해도 그때까지의 수가 남는다.
struct Capture {
    n: u64,
    buf: Option<Vec<u8>>,
}

impl Write for Capture {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.n += data.len() as u64;
        if let Some(buf) = &mut self.buf {
            buf.extend_from_slice(data);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- OPF 분석

/// OPF 분석에서 ZIP 전체 읽기 단계로 넘기는 정보.
#[derive(Default)]
struct OpfPlan {
    opf_dir: String,
    /// 본문(마크업) spine 멤버. ZIP에 있는 것만 담는다.
    spine_docs: HashSet<String>,
    ncx: Option<String>,
    fonts: HashSet<String>,
}

fn elements<'a, 'i: 'a>(doc: &'a Document<'i>, ns: &'static str, name: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    doc.descendants().filter(move |n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns))
}

/// OPF 디렉토리 기준으로 href를 ZIP 멤버 이름으로 바꾼다. 조각(`#…`)은 뗀다. 파일 부분이 없으면 None.
fn resolve(opf_dir: &str, href: &str) -> Option<String> {
    let target = unquote(href);
    let file = target.split('#').next().unwrap_or("");
    (!file.is_empty()).then(|| posix_normpath(&posix_join(opf_dir, file)))
}

/// EpubDiagnose.js `findUndeclaredPrefixes`: 쓰였지만 `xmlns:`로 선언되지 않은 접두사.
fn undeclared_prefixes(text: &str) -> Vec<String> {
    let mut declared: HashSet<&str> = HashSet::from(["xml", "xmlns"]);
    declared.extend(DECLARED_RE.captures_iter(text).filter_map(|c| c.get(1)).map(|m| m.as_str()));
    let mut used: Vec<String> = Vec::new();
    for c in USED_RE.captures_iter(text) {
        let prefix = &c[1];
        if !declared.contains(prefix) && !used.iter().any(|u| u == prefix) {
            used.push(prefix.to_string());
        }
    }
    used
}

/// OPF를 읽어 규칙을 적용하고, 이어질 ZIP 읽기에 필요한 계획을 돌려준다.
fn analyze_opf(opf_bytes: &[u8], opf_path: &str, name_set: &HashSet<&str>, sink: &mut Sink) -> OpfPlan {
    let mut plan = OpfPlan { opf_dir: posix_dirname(opf_path), ..OpfPlan::default() };
    let text = String::from_utf8_lossy(opf_bytes);

    // EpubDiagnose.js `checkOpf`(findUndeclaredPrefixes 호출): 파싱 성공 여부와 무관하게 적용한다.
    let undeclared = undeclared_prefixes(&text);
    if !undeclared.is_empty() {
        sink.flag(Category::Render, Severity::Medium, "UNDECLARED_PREFIX", opf_path, &format!("미선언 네임스페이스 접두사: {}", undeclared.join(", ")));
    }

    // 파싱 실패는 structure의 OPF_UNPARSABLE이 맡는다. 이후 규칙은 DOM이 있어야 한다.
    let Some(doc) = parse_xml(opf_bytes) else { return plan };

    let items: Vec<Node> = elements(&doc, OPF_NS, "item").collect();
    let itemrefs: Vec<Node> = elements(&doc, OPF_NS, "itemref").collect();

    // EpubDiagnose.js `checkOpf`: manifestCount/spineCount가 0이면 FATAL.
    if items.is_empty() {
        sink.flag(Category::Render, Severity::High, "MANIFEST_EMPTY", opf_path, "manifest에 item이 없음");
    }
    if itemrefs.is_empty() {
        sink.flag(Category::Render, Severity::High, "SPINE_EMPTY", opf_path, "spine에 itemref가 없음");
    }

    // EpubDiagnose.js `checkOpf`: dc:identifier, dc:language, dc:title(`inspectOpfDoc`의 title은 textContent라 빈 글자는 없음).
    let title_ok = elements(&doc, DC_NS, "title").next().is_some_and(|n| n.descendants().filter_map(|d| d.text()).any(|t| !t.is_empty()));
    let mut missing_dc = Vec::new();
    if elements(&doc, DC_NS, "identifier").next().is_none() {
        missing_dc.push("dc:identifier");
    }
    if !title_ok {
        missing_dc.push("dc:title");
    }
    if elements(&doc, DC_NS, "language").next().is_none() {
        missing_dc.push("dc:language");
    }
    if !missing_dc.is_empty() {
        sink.flag(Category::Render, Severity::Low, "DC_METADATA_MISSING", opf_path, &format!("없는 메타데이터: {}", missing_dc.join(", ")));
    }

    // EpubDiagnose.js `inspectOpfDoc`/`checkOpf`: version이 3으로 시작하고 properties에 nav가 든 item이 없다.
    // `inspectOpfDoc`처럼 properties 문자열에 "nav"가 들어 있으면 nav로 본다.
    let version = elements(&doc, OPF_NS, "package").next().and_then(|n| n.attribute("version")).unwrap_or("?");
    let has_nav = items.iter().any(|n| n.attribute("properties").unwrap_or("").contains("nav"));
    if version.starts_with('3') && !has_nav {
        sink.flag(Category::Render, Severity::Low, "NAV_MISSING", opf_path, "EPUB3인데 nav 문서가 없음");
    }

    // EpubDiagnose.js `checkSpineFiles`: idref가 manifest에 없다. (파일이 ZIP에 없는 경우는 같은 함수에서
    // 한 메시지로 합쳐 세지만, 정해진 대로 구조 검사의 SPINE_MEMBER_MISSING이 따로 맡는다.)
    let manifest: HashMap<&str, &str> = items.iter().map(|n| (n.attribute("id").unwrap_or(""), n.attribute("href").unwrap_or(""))).collect();
    for r in &itemrefs {
        let idref = r.attribute("idref").unwrap_or("");
        if manifest.get(idref).is_none_or(|h| h.is_empty()) {
            sink.flag(Category::Render, Severity::High, "SPINE_IDREF_MISSING", idref, &format!("spine idref=\"{idref}\"가 manifest에 없음"));
        }
    }

    // EpubDiagnose.js `checkNcx`: spine의 toc가 가리키는 item과 파일.
    if let Some(toc) = elements(&doc, OPF_NS, "spine").next().and_then(|n| n.attribute("toc")).filter(|t| !t.is_empty()) {
        match items.iter().find(|n| n.attribute("id") == Some(toc)).and_then(|n| n.attribute("href")) {
            None => sink.flag(Category::Render, Severity::Medium, "NCX_REF_MISSING", opf_path, &format!("toc=\"{toc}\"가 가리키는 manifest item이 없음")),
            Some(href) => match resolve(&plan.opf_dir, href) {
                Some(path) if name_set.contains(path.as_str()) => plan.ncx = Some(path),
                other => sink.flag(Category::Render, Severity::Medium, "NCX_REF_MISSING", other.as_deref().unwrap_or(opf_path), "NCX 파일이 ZIP에 없음"),
            },
        }
    }

    // EpubDiagnose.js `checkGuide`: guide의 href가 가리키는 파일. 파일 부분이 빈 href(`#x`)는 건너뛴다.
    if let Some(guide) = GUIDE_RE.captures(&text) {
        let missing: Vec<String> = HREF_RE.captures_iter(&guide[1]).filter_map(|c| resolve(&plan.opf_dir, &c[1])).filter(|p| !name_set.contains(p.as_str())).collect();
        if let Some(first) = missing.first() {
            sink.flag(Category::Render, Severity::Low, "GUIDE_REF_MISSING", first, &format!("guide 참조 {}건이 존재하지 않는 파일", missing.len()));
        }
    }

    // EpubDiagnose.js `checkContentDocuments`가 읽는 spine 문서. 마크업이 아닌 멤버(오디오 등)는 파싱하지 않는다.
    for (member, media) in spine_members(&doc, opf_path) {
        if name_set.contains(member.as_str()) && is_markup(&member, media.as_str()) {
            plan.spine_docs.insert(member);
        }
    }

    // preview.rs:524-525처럼 확장자나 manifest media-type이 폰트이면 폰트다.
    for n in &items {
        if FONT_MEDIA_TYPES.contains(&n.attribute("media-type").unwrap_or(""))
            && let Some(path) = n.attribute("href").and_then(|h| resolve(&plan.opf_dir, h))
        {
            plan.fonts.insert(path);
        }
    }
    plan
}

// ---------------------------------------------------------------- 본문 분석

/// EpubDiagnose.js `checkNcx`: navPoint의 content src가 ZIP에 없다.
fn analyze_ncx(bytes: &[u8], ncx_path: &str, opf_dir: &str, name_set: &HashSet<&str>, sink: &mut Sink) {
    // NCX 자체의 파싱 실패는 정해진 코드가 없어 다루지 않는다(EpubDiagnose.js는 ERROR로 보고한다).
    let Some(doc) = parse_xml(bytes) else { return };
    let mut missing = Vec::new();
    for point in elements(&doc, NCX_NS, "navPoint") {
        let Some(content) = point.descendants().find(|n| n.is_element() && n.tag_name().name() == "content" && n.tag_name().namespace() == Some(NCX_NS)) else { continue };
        if let Some(path) = resolve(opf_dir, content.attribute("src").unwrap_or("")) && !name_set.contains(path.as_str()) {
            missing.push(path);
        }
    }
    if let Some(first) = missing.first() {
        sink.flag(Category::Render, Severity::Medium, "NCX_REF_MISSING", ncx_path, &format!("NCX navPoint {}건이 존재하지 않는 파일 참조 (예: {first})", missing.len()));
    }
}

/// CSS 선택자 하나(마지막 복합 선택자의 태그 이름과 클래스들).
#[derive(Clone, Debug)]
pub struct Selector {
    pub tag: String,
    pub classes: Vec<String>,
}

/// CSS 본문에서 `font-size:0` 규칙의 선택자를 모은다. 속성·의사 클래스·id가 든 선택자는 건너뛴다.
pub fn zero_size_selectors(css: &str) -> Vec<Selector> {
    let mut out = Vec::new();
    for rule in CSS_RULE_RE.captures_iter(css) {
        if !ZERO_SIZE_RE.is_match(&rule[2]) {
            continue;
        }
        for sel in rule[1].split(',') {
            let last = sel.rsplit(|c: char| c.is_whitespace() || c == '>' || c == '+' || c == '~').find(|s| !s.is_empty()).unwrap_or("");
            if last.is_empty() || last.contains([':', '[', '#', '*']) {
                continue;
            }
            let mut parts = last.split('.');
            let tag = parts.next().unwrap_or("").to_string();
            let classes: Vec<String> = parts.map(str::to_string).collect();
            if !tag.is_empty() || !classes.is_empty() {
                out.push(Selector { tag, classes });
            }
        }
    }
    out
}

fn matches_selector(node: &Node, sel: &Selector) -> bool {
    if !sel.tag.is_empty() && node.tag_name().name() != sel.tag {
        return false;
    }
    let class_attr = node.attribute("class").unwrap_or("");
    sel.classes.iter().all(|c| class_attr.split_whitespace().any(|t| t == c))
}

/// 크기 0 요소(인라인 style 또는 선택자 일치)를 `stop_at`개까지만 센다. 임계값에 닿으면 더 보지 않는다.
pub fn count_hidden_elements(doc: &Document, selectors: &[Selector], stop_at: usize) -> usize {
    doc.descendants()
        .filter(|n| n.is_element())
        .filter(|n| n.attribute("style").is_some_and(|s| ZERO_SIZE_RE.is_match(s)) || selectors.iter().any(|s| matches_selector(n, s)))
        .take(stop_at)
        .count()
}

/// 읽기 오류의 종류. 체크섬은 지표, 손상된 스트림은 structure의 MEMBER_UNREADABLE 몫, 그 밖(I/O)은 incomplete다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadErrorKind {
    Checksum,
    Corrupt,
    Io,
}

pub fn classify_read_error(e: &io::Error) -> ReadErrorKind {
    match e.kind() {
        // zip 9의 Crc32Reader는 EOF에서 InvalidData "Invalid checksum"을 낸다(테스트로 확인).
        io::ErrorKind::InvalidData if e.to_string().contains("checksum") => ReadErrorKind::Checksum,
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput | io::ErrorKind::UnexpectedEof => ReadErrorKind::Corrupt,
        _ => ReadErrorKind::Io,
    }
}

fn classify_zip_error(e: &zip::result::ZipError) -> ReadErrorKind {
    match e {
        zip::result::ZipError::Io(io) => classify_read_error(io),
        _ => ReadErrorKind::Corrupt,
    }
}

/// 한 번의 검사가 공유하는 읽기 상태.
struct Ctx {
    budget: Budget,
    max_member: u64,
    css_cache: HashMap<String, Vec<Selector>>,
    /// 이미 한도 안에서 읽은 멤버(container, OPF, CSS). 전체 읽기 단계에서 다시 읽지 않는다.
    done: HashSet<String>,
    skipped: usize,
    io_errors: usize,
    css_failed: usize,
    selector_cap_hit: bool,
    sheet_cap_hit: bool,
}

impl Ctx {
    fn on_error(&mut self, kind: ReadErrorKind, name: &str, sink: &mut Sink) {
        match kind {
            ReadErrorKind::Checksum => sink.flag(Category::Defect, Severity::High, "ZIP_CRC_MISMATCH", name, "멤버를 끝까지 읽으면 CRC가 맞지 않음"),
            ReadErrorKind::Corrupt => {}
            ReadErrorKind::Io => self.io_errors += 1,
        }
    }

    /// 작은 멤버(container, OPF, CSS)를 min(max_member, 작은 멤버 상한, 남은 예산)까지만 읽는다.
    /// 읽은 바이트를 예산에서 빼고, 못 읽거나 상한을 넘으면 사유를 기록하고 None을 돌려준다.
    fn read_small(&mut self, zip: &mut Zip, name: &str, sink: &mut Sink) -> Option<Vec<u8>> {
        self.done.insert(name.to_string());
        let cap = self.max_member.min(MAX_SMALL_BYTES as u64);
        if self.budget.remaining == 0 {
            self.budget.exhausted = true;
            return None;
        }
        let limit = cap.min(self.budget.remaining);
        let mut entry = match zip.by_name(name) {
            Ok(e) => e,
            Err(e) => {
                let kind = classify_zip_error(&e);
                self.on_error(kind, name, sink);
                return None;
            }
        };
        let mut cap_w = Capture { n: 0, buf: Some(Vec::new()) };
        let result = io::copy(&mut (&mut entry).take(limit + 1), &mut cap_w);
        drop(entry);
        self.budget.remaining -= cap_w.n.min(limit);
        match result {
            Err(e) => {
                let kind = classify_read_error(&e);
                self.on_error(kind, name, sink);
                None
            }
            Ok(_) if cap_w.n > limit => {
                if limit < cap {
                    self.budget.exhausted = true;
                } else {
                    self.skipped += 1;
                }
                None
            }
            Ok(_) => cap_w.buf,
        }
    }
}

/// 본문 XHTML 하나를 검사한다. 읽은 CSS는 `ctx.css_cache`에 두고 다시 읽지 않는다.
fn analyze_xhtml(zip: &mut Zip, bytes: &[u8], member: &str, name_set: &HashSet<&str>, ctx: &mut Ctx, sink: &mut Sink) {
    let text = String::from_utf8_lossy(bytes);
    let doc = parse_xml(bytes);

    match &doc {
        // EpubDiagnose.js `checkContentDocuments`: 브라우저 DOMParser 파싱 실패.
        None => sink.flag(Category::Render, Severity::High, "XHTML_PARSE_FAILED", member, "XML 파싱 실패"),
        Some(doc) => {
            // frontend/src/ViewEPUB.jsx:61-71 `secureEpubDocument`: `stripElementPrefixes`(:61) 뒤에
            // `doc.querySelector("head")`가 없으면 "EPUB 본문에 head가 없습니다."로 렌더링을 거부한다.
            // 접두사를 먼저 벗기므로 `<xhtml:head>`도 head다. 로컬 이름만 비교하면 같은 결과다.
            if !doc.descendants().any(|n| n.is_element() && n.tag_name().name() == "head") {
                sink.flag(Category::Render, Severity::High, "HEAD_MISSING", member, "head 요소가 없어 뷰어가 본문을 렌더링하지 못함");
            }
        }
    }

    // 난독화 (scripts/epub_deobfuscate.py가 푸는 패턴): 크기 0 요소와 RTL 오버라이드(U+202E).
    let lower = text.to_lowercase();
    let rtl = text.matches('\u{202e}').count() + lower.matches("&#x202e;").count() + text.matches("&#8238;").count();
    let mut hidden = 0;
    if let Some(doc) = &doc {
        let dir = posix_dirname(member);
        let mut selectors: Vec<Selector> = Vec::new();
        // 같은 스타일시트는 문서마다 한 번만 더한다.
        let mut seen_css: HashSet<String> = HashSet::new();
        let push_capped = |selectors: &mut Vec<Selector>, new: Vec<Selector>, hit: &mut bool| {
            for s in new {
                if selectors.len() >= MAX_SELECTORS_PER_DOC {
                    *hit = true;
                    break;
                }
                selectors.push(s);
            }
        };
        for n in doc.descendants().filter(|n| n.is_element()) {
            match n.tag_name().name() {
                "style" => {
                    let css: String = n.descendants().filter_map(|d| d.text()).collect();
                    push_capped(&mut selectors, zero_size_selectors(&css), &mut ctx.selector_cap_hit);
                }
                "link" if n.attribute("rel").is_some_and(|r| r.to_lowercase().contains("stylesheet")) => {
                    let Some(path) = n.attribute("href").and_then(|h| resolve(&dir, h)).filter(|p| name_set.contains(p.as_str())) else { continue };
                    if !seen_css.insert(path.clone()) {
                        continue;
                    }
                    if seen_css.len() > MAX_STYLESHEETS_PER_DOC {
                        ctx.sheet_cap_hit = true;
                        continue;
                    }
                    if !ctx.css_cache.contains_key(&path) {
                        let parsed = match ctx.read_small(zip, &path, sink) {
                            Some(css) => zero_size_selectors(&String::from_utf8_lossy(&css)),
                            None => {
                                ctx.css_failed += 1;
                                Vec::new()
                            }
                        };
                        ctx.css_cache.insert(path.clone(), parsed);
                    }
                    push_capped(&mut selectors, ctx.css_cache[&path].clone(), &mut ctx.selector_cap_hit);
                }
                _ => {}
            }
        }
        hidden = count_hidden_elements(doc, &selectors, OBFUSCATION_THRESHOLD);
    }
    if hidden >= OBFUSCATION_THRESHOLD || rtl >= OBFUSCATION_THRESHOLD {
        sink.flag(Category::Render, Severity::Medium, "OBFUSCATED_TEXT", member, &format!("크기 0 요소 {hidden}개 이상 또는 U+202E {rtl}개 (임계 {OBFUSCATION_THRESHOLD})"));
    }
}

// ---------------------------------------------------------------- ZIP 전체 읽기

fn run(zip: &mut Zip, limits: &Limits, sink: &mut Sink) {
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let name_set: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut ctx = Ctx {
        budget: Budget { remaining: limits.max_total, exhausted: false },
        max_member: limits.max_member,
        css_cache: HashMap::new(),
        done: HashSet::new(),
        skipped: 0,
        io_errors: 0,
        css_failed: 0,
        selector_cap_hit: false,
        sheet_cap_hit: false,
    };

    // container → OPF. 못 찾거나 못 읽는 것은 structure가 보고하므로 여기서는 OPF 규칙만 건너뛴다.
    // 한 번만 읽고 예산에서 한 번만 뺀다(전체 읽기 단계는 `done`에 든 멤버를 건너뛴다).
    let mut plan = OpfPlan::default();
    if name_set.contains(CONTAINER)
        && let Some(container) = ctx.read_small(zip, CONTAINER, sink)
        && let Some(opf_path) = rootfile_path(&container)
        && name_set.contains(opf_path.as_str())
        && let Some(opf) = ctx.read_small(zip, &opf_path, sink)
    {
        plan = analyze_opf(&opf, &opf_path, &name_set, sink);
    }

    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                let kind = classify_zip_error(&e);
                ctx.on_error(kind, "", sink);
                continue;
            }
        };
        if entry.is_dir() {
            continue;
        }
        let Ok(name) = entry.name().map(|n| n.into_owned()) else { continue };
        if ctx.done.contains(&name) {
            continue;
        }
        let declared = entry.size();

        // preview.rs:527: 폰트 크기는 선언 크기로 잰다(읽기 한도에서 건너뛰는 큰 폰트도 잡아야 한다).
        let lower = name.to_lowercase();
        if declared > FONT_SIZE_LIMIT && (FONT_EXTENSIONS.iter().any(|e| lower.ends_with(e)) || plan.fonts.contains(&name)) {
            sink.flag(Category::Render, Severity::Low, "FONT_TOO_LARGE", &name, &format!("폰트 {declared}바이트 > {FONT_SIZE_LIMIT}바이트"));
        }

        if declared > limits.max_member {
            ctx.skipped += 1;
            continue;
        }
        if ctx.budget.remaining == 0 {
            ctx.budget.exhausted = true;
            break;
        }
        // 선언 크기를 믿지 않는다: 읽기 자체를 min(멤버 한도, 남은 예산)+1바이트로 자른다.
        let limit = limits.max_member.min(ctx.budget.remaining);
        let keep = plan.spine_docs.contains(&name) || plan.ncx.as_deref() == Some(name.as_str());
        let mut cap = Capture { n: 0, buf: keep.then(Vec::new) };
        // ZIP은 끝까지 읽어야 CRC를 검증한다. limit+1바이트까지 읽으면 limit 이하 멤버는 항상 끝에 닿는다.
        let result = io::copy(&mut (&mut entry).take(limit + 1), &mut cap);
        drop(entry); // 이어서 CSS를 읽으려면 zip의 빌림을 풀어야 한다.
        ctx.budget.remaining -= cap.n.min(limit);

        match result {
            // 체크섬은 지표, 손상된 스트림은 structure의 MEMBER_UNREADABLE, 그 밖의 오류는 incomplete.
            Err(e) => {
                let kind = classify_read_error(&e);
                ctx.on_error(kind, &name, sink);
            }
            Ok(_) if cap.n > limit => {
                if limit < limits.max_member {
                    ctx.budget.exhausted = true;
                    break;
                }
                ctx.skipped += 1; // 선언 크기보다 실제로 큰 멤버.
            }
            Ok(_) => {
                if let Some(bytes) = cap.buf {
                    if plan.ncx.as_deref() == Some(name.as_str()) {
                        analyze_ncx(&bytes, &name, &plan.opf_dir, &name_set, sink);
                    } else {
                        analyze_xhtml(zip, &bytes, &name, &name_set, &mut ctx, sink);
                    }
                }
            }
        }
    }

    // 한도 때문에 못 본 멤버는 결함이 아니다. 검사가 불완전하다고만 한 번씩 알린다.
    if ctx.skipped > 0 {
        sink.notes.push(format!("render: {} member(s) larger than read cap (max_member {} bytes) skipped, CRC and content unchecked", ctx.skipped, limits.max_member));
    }
    if ctx.budget.exhausted {
        sink.notes.push(format!("render: decompression budget {} bytes reached, remaining members unchecked", limits.max_total));
    }
    if ctx.io_errors > 0 {
        sink.notes.push(format!("render: {} member(s) failed to read (I/O error), checks skipped", ctx.io_errors));
    }
    if ctx.css_failed > 0 {
        sink.notes.push(format!("render: {} stylesheet(s) unread (over cap, budget or error), OBFUSCATED_TEXT may be missed", ctx.css_failed));
    }
    if ctx.sheet_cap_hit {
        sink.notes.push(format!("render: more than {MAX_STYLESHEETS_PER_DOC} stylesheets in one document, extra stylesheets ignored"));
    }
    if ctx.selector_cap_hit {
        sink.notes.push(format!("render: more than {MAX_SELECTORS_PER_DOC} selectors in one document, extra selectors ignored"));
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown".into())
}

/// 한도를 바꿔 끼울 수 있는 검사. 테스트가 작은 한도로 예산과 건너뛰기를 확인하는 데 쓴다.
pub fn inspect_with(path: &Path, report: &mut FileReport, limits: &Limits) {
    let Some(mut zip) = open_zip(path) else { return };
    let mut sink = Sink::default();
    // 한 파일 때문에 수천 건 스캔이 중단되면 안 된다. panic 전에 모은 지표는 남긴다.
    let outcome = catch_unwind(AssertUnwindSafe(|| run(&mut zip, limits, &mut sink)));
    for (category, severity, code, location, detail) in &sink.findings {
        report.push(*category, *severity, code, location, detail);
    }
    for note in &sink.notes {
        report.mark_incomplete(note);
    }
    if let Err(payload) = outcome {
        let msg: String = format!("panic: {}", panic_message(payload.as_ref())).chars().take(120).collect();
        report.mark_incomplete(&msg);
    }
}

/// EPUB 한 건의 렌더링·손상을 검사해 `report`에 더한다.
pub fn inspect(path: &Path, report: &mut FileReport) {
    inspect_with(path, report, &Limits::default());
}
