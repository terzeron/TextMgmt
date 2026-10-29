//! PDF 구조·렌더링 검사. lopdf로 구조를 보고, Poppler(`pdfinfo`, `pdftotext`)로 실제 열리는지 확인한다.
//!
//! Poppler 호출 정책: 비싼 외부 프로세스라서 파일당 `pdfinfo`는 최대 한 번이다. `--no-poppler`이거나 `%PDF-` 머리가 없으면
//! 부르지 않는다. lopdf가 못 읽은 파일도 부른다(lopdf는 못 읽어도 뷰어는 여는 파일을 가려내는 것이 목적이다).
//! 값싼 구조 검사를 모두 끝낸 뒤에 부르고, 시간 상한은 `sysdeps::poppler`의 30초다.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use lopdf::{Dictionary, Document, Object};
use scan_common::text::sanitize;
use scan_common::{Category, FileReport, Severity};
use sysdeps::poppler::{extract_text, page_count};
use sysdeps::{CommandError, CommandOutput, CommandRunner, SystemRunner};

use crate::cli::Options;
use crate::epub::security::guarded;
use crate::pdf::security::has_header;

/// 머리·꼬리 표지를 찾는 구간. 뷰어도 앞 1024바이트 안에서 `%PDF-`를 찾는다(pdf.js `find("%PDF-", 1024)`).
const MARKER_WINDOW: usize = 1024;
/// 이보다 작은 쪽 크기(pt)는 뷰어가 canvas를 0x0으로 만든다. 0.09pt 만화 PDF가 백지가 된 사례:
/// frontend/src/ViewPDF.jsx 35행 주석, frontend/src/pdfRenderScale.js 21행 주석.
const MIN_PAGE_POINTS: f32 = 1.0;
/// `NO_TEXT_LAYER`는 앞 몇 쪽만 본다. 표지만 그림인 책을 결함으로 몰지 않는다.
const TEXT_PROBE_PAGES: usize = 5;
/// 상속 사슬(Parent)을 따라가는 최대 깊이. 순환 PDF에서 멈추기 위한 상한이다.
const MAX_PARENT_DEPTH: usize = 64;
/// 지표에 이름을 나열할 최대 개수.
const LIST_LIMIT: usize = 5;
/// `--pdf-text`인데 쪽수를 몰라 본문(NO_TEXT_LAYER·MOJIBAKE 검사)을 뽑지 못했다.
const PAGE_COUNT_UNKNOWN: &str = "pdf text not extracted: page count unknown";
const POPPLER_MISSING: &str = "poppler not installed";

/// PDF 32000-1:2008 9.6.2.2 "Standard Type 1 Fonts (Standard 14 Fonts)". 이 글꼴은 뷰어가 내장하므로 임베드하지 않아도 된다.
const BASE_14: [&str; 14] =
    ["Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic", "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique", "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique", "Symbol", "ZapfDingbats"];

/// 실행기가 낸 `CommandError`의 종류. `sysdeps::poppler`는 오류를 문자열로 뭉개므로 래퍼가 직접 기록한다.
/// 0이 아닌 종료 코드는 `CommandError`가 아니므로 여기에 남지 않는다(그것만 `POPPLER_OPEN_FAILED`다).
enum Failure {
    NotFound,
    Timeout,
    Spawn(String),
}

/// `CommandError`를 기록하는 실행기 래퍼.
struct Watching<'a> {
    inner: &'a dyn CommandRunner,
    failure: Mutex<Option<Failure>>,
}

impl CommandRunner for Watching<'_> {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError> {
        let result = self.inner.run(argv, timeout);
        if let Err(e) = &result {
            let failure = match e {
                CommandError::NotFound => Failure::NotFound,
                CommandError::Timeout => Failure::Timeout,
                CommandError::Failed(m) => Failure::Spawn(m.clone()),
            };
            if let Ok(mut slot) = self.failure.lock() {
                *slot = Some(failure);
            }
        }
        result
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        self.inner.which(program)
    }
}

impl Watching<'_> {
    fn take(&self) -> Option<Failure> {
        self.failure.lock().ok().and_then(|mut slot| slot.take())
    }
}

/// 실행기 자체의 실패(검사 불능)를 `incomplete` 사유로 바꾼다. 실행기 실패가 아니면 `false`다.
fn note_runner_failure(failure: Option<Failure>, report: &mut FileReport) -> bool {
    match failure {
        None => false,
        Some(Failure::NotFound) => {
            report.mark_incomplete(POPPLER_MISSING);
            true
        }
        // 시간 초과와 실행 실패는 파일 결함의 증거가 아니다. 부하가 큰 디스크에서 멀쩡한 파일도 시간을 넘길 수 있다.
        Some(Failure::Timeout) => {
            report.mark_incomplete("poppler timed out");
            true
        }
        Some(Failure::Spawn(m)) => {
            report.mark_incomplete(&format!("poppler failed to run: {}", sanitize(&m, 200, Some('…'))));
            true
        }
    }
}

/// pdfinfo가 암호 때문에 못 열 때 stderr에 쓰는 문구. 실제 출력(qpdf 256비트로 암호화한 파일):
/// `Command Line Error: Incorrect password`.
fn mentions_password(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("password") || lower.contains("encrypted")
}

/// 운영용 진입점. `SystemRunner`로 Poppler를 부른다.
pub fn inspect(data: &[u8], doc: Option<&Document>, parse_error: Option<&str>, path: &Path, opts: &Options, report: &mut FileReport) {
    inspect_with(data, doc, parse_error, path, opts, report, &SystemRunner);
}

/// `inspect`에 Poppler 실행기를 주입한다.
pub fn inspect_with(data: &[u8], doc: Option<&Document>, parse_error: Option<&str>, path: &Path, opts: &Options, report: &mut FileReport, runner: &dyn CommandRunner) {
    let header = has_header(data);
    if !header {
        report.push(Category::Defect, Severity::High, "NO_PDF_HEADER", "", "앞 1024바이트에 %PDF-가 없다");
    }
    let tail = &data[data.len().saturating_sub(MARKER_WINDOW)..];
    if !tail.windows(5).any(|w| w == b"%%EOF") {
        report.push(Category::Defect, Severity::Medium, "NO_EOF_MARKER", "", "끝 1024바이트에 %%EOF가 없다 (잘린 파일)");
    }
    if let Some(e) = parse_error {
        report.push(Category::Defect, Severity::High, "XREF_BROKEN", "", &format!("구문 분석 실패: {e}"));
        report.mark_incomplete(&format!("parse error: {e}"));
    }
    if let Some(doc) = doc {
        guarded(report, |r| document_checks(doc, r));
    }
    if header {
        poppler_checks(path, opts, doc, report, runner);
    }
}

fn document_checks(doc: &Document, report: &mut FileReport) {
    let has_root = doc.trailer.get(b"Root").is_ok();
    if !has_root {
        report.push(Category::Defect, Severity::High, "NO_ROOT", "trailer", "trailer에 /Root가 없다");
    }
    if doc.trailer.has(b"Encrypt") {
        // pdf.js는 암호화 파일을 비밀번호 요청으로 처리해 본문을 그리지 못한다.
        report.push(Category::Render, Severity::High, "ENCRYPTED", "trailer", "trailer에 /Encrypt가 있다");
    }
    let pages = doc.get_pages();
    // 암호화 문서는 객체를 읽지 못해 쪽 트리가 비어 보인다. ENCRYPTED만 알린다.
    if has_root && pages.is_empty() && !doc.trailer.has(b"Encrypt") {
        // 읽기 훅이 참조 배열을 비웠거나 /Pages의 /Count가 0보다 크면 쪽 트리가 잘렸을 뿐 0쪽이라는 증거가 아니다.
        // 그 경우 High 결함이 아니라 "끝까지 보지 못함"으로만 알린다.
        if crate::pdf::security::emptied_ref_array() || declared_page_count(doc) > 0 {
            report.mark_incomplete("page tree could not be read (reference array emptied or /Count > 0)");
        } else {
            report.push(Category::Defect, Severity::High, "ZERO_PAGES", "", "페이지가 0쪽이다");
        }
    }
    tiny_pages(doc, &pages, report);
    unembedded_fonts(doc, report);
}

/// 카탈로그 `/Pages`의 `/Count`. 없거나 읽을 수 없으면 0이다.
fn declared_page_count(doc: &Document) -> i64 {
    doc.catalog().ok().and_then(|c| c.get(b"Pages").ok()).and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok()).and_then(|d| d.get(b"Count").ok()).and_then(|c| c.as_i64().ok()).unwrap_or(0)
}

/// 상속을 풀어 쪽의 MediaBox를 찾는다(PDF 32000-1:2008 7.7.3.4: 쪽에 없으면 /Parent 사슬에서 물려받는다).
fn inherited_media_box(doc: &Document, page: &Dictionary) -> Option<[f32; 4]> {
    let mut node = page;
    for _ in 0..MAX_PARENT_DEPTH {
        if let Some(raw) = node.get(b"MediaBox").ok().and_then(|o| doc.dereference(o).ok()).map(|(_, o)| o) {
            let items = raw.as_array().ok()?;
            let mut v = [0f32; 4];
            if items.len() != 4 {
                return None;
            }
            for (slot, item) in v.iter_mut().zip(items) {
                *slot = doc.dereference(item).ok()?.1.as_float().ok()?;
            }
            return Some(v);
        }
        node = node.get(b"Parent").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_dict().ok())?;
    }
    None
}

fn tiny_pages(doc: &Document, pages: &std::collections::BTreeMap<u32, lopdf::ObjectId>, report: &mut FileReport) {
    let mut tiny: Vec<(u32, f32, f32)> = Vec::new();
    for (&number, &id) in pages {
        let Some(b) = doc.get_dictionary(id).ok().and_then(|p| inherited_media_box(doc, p)) else { continue };
        let (w, h) = ((b[2] - b[0]).abs(), (b[3] - b[1]).abs());
        if w < MIN_PAGE_POINTS || h < MIN_PAGE_POINTS {
            tiny.push((number, w, h));
        }
    }
    if let Some(&(first, w, h)) = tiny.first() {
        // 만화 PDF는 모든 쪽이 작을 수 있으므로 한 건으로 묶는다.
        report.push(Category::Render, Severity::High, "TINY_PAGE", &format!("page {first}"), &format!("{}쪽의 MediaBox가 {w}x{h}pt다 (총 {}쪽)", first, tiny.len()));
    }
}

/// Base-14 이름과 흔한 별칭인가. 별칭은 PDF 32000-1:2008 9.6.2.2 표 중 Arial=Helvetica, TimesNewRoman=Times,
/// CourierNew=Courier 대응(글꼴 대체 표)과 Windows가 쓰는 철자(`Arial,Bold`, `Arial-BoldMT`, `TimesNewRomanPSMT`,
/// `TimesNewRomanPS-BoldMT`, `CourierNewPSMT`)를 따른다. 구분 문자를 지우고 소문자로 만들어 가족 접두와 스타일 꼬리로 나눠 비교한다.
fn is_standard_font(name: &str) -> bool {
    if BASE_14.contains(&name) {
        return true;
    }
    let norm: String = name.chars().filter(|c| !matches!(c, ',' | '-' | ' ' | '_')).flat_map(char::to_lowercase).collect();
    // 긴 접두를 먼저 본다(`timesnewroman`이 `times`보다 앞).
    let Some(rest) = ["timesnewroman", "couriernew", "helvetica", "arial", "courier", "times"].iter().find_map(|f| norm.strip_prefix(f)) else { return false };
    let style = rest.trim_start_matches("ps").trim_end_matches("mt");
    ["", "roman", "regular", "bold", "italic", "oblique", "bolditalic", "boldoblique"].contains(&style)
}

fn font_file_missing(d: &Dictionary) -> bool {
    !(d.has(b"FontFile") || d.has(b"FontFile2") || d.has(b"FontFile3"))
}

fn unembedded_fonts(doc: &Document, report: &mut FileReport) {
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut first: Option<lopdf::ObjectId> = None;
    for (&id, obj) in &doc.objects {
        let Object::Dictionary(d) = obj else { continue };
        if d.get(b"Type").ok().and_then(|t| t.as_name().ok()) != Some(b"FontDescriptor") || !font_file_missing(d) {
            continue;
        }
        let raw = d.get(b"FontName").ok().and_then(|n| n.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned()).unwrap_or_default();
        // 부분 집합 글꼴은 `ABCDEF+Name`이다.
        let name = raw.split_once('+').filter(|(tag, _)| tag.len() == 6).map_or(raw.as_str(), |(_, n)| n);
        if is_standard_font(name) {
            continue;
        }
        first.get_or_insert(id);
        names.insert(sanitize(if name.is_empty() { "<unnamed>" } else { name }, 60, Some('…')));
    }
    if let Some(id) = first {
        let listed: Vec<&str> = names.iter().take(LIST_LIMIT).map(String::as_str).collect();
        let more = if names.len() > LIST_LIMIT { ", …" } else { "" };
        report.push(Category::Defect, Severity::Low, "FONT_NOT_EMBEDDED", &format!("obj {} {}", id.0, id.1), &format!("임베드되지 않은 글꼴 {}종: {}{more}", names.len(), listed.join(", ")));
    }
}

fn poppler_checks(path: &Path, opts: &Options, doc: Option<&Document>, report: &mut FileReport, runner: &dyn CommandRunner) {
    if opts.no_poppler {
        if opts.pdf_text {
            report.mark_incomplete("mojibake check skipped: poppler disabled");
        }
        return;
    }
    let watching = Watching { inner: runner, failure: Mutex::new(None) };
    let pages = match page_count(&watching, path) {
        Ok(n) => n,
        Err(e) => {
            if note_runner_failure(watching.take(), report) {
                return;
            }
            // 암호화 파일은 pdfinfo가 비밀번호를 요구하며 실패한다. 이미 ENCRYPTED로 알렸으므로 중복 지표를 내지 않는다.
            if doc.is_some_and(|d| d.trailer.has(b"Encrypt")) && mentions_password(&e) {
                return;
            }
            report.push(Category::Render, Severity::High, "POPPLER_OPEN_FAILED", "", &sanitize(&e, 200, Some('…')));
            if opts.pdf_text {
                report.mark_incomplete(PAGE_COUNT_UNKNOWN);
            }
            return;
        }
    };
    if !opts.pdf_text {
        return;
    }
    if pages == 0 {
        report.mark_incomplete(PAGE_COUNT_UNKNOWN);
        return;
    }
    let head_last = pages.min(TEXT_PROBE_PAGES);
    let head = match extract_text(&watching, path, 1, head_last) {
        Ok(text) => text,
        Err(e) => {
            if !note_runner_failure(watching.take(), report) {
                report.mark_incomplete(&format!("pdftotext failed: {}", sanitize(&e, 200, Some('…'))));
            }
            return;
        }
    };
    if head.trim().is_empty() {
        report.push(Category::Defect, Severity::Low, "NO_TEXT_LAYER", "", &format!("앞 {head_last}쪽에서 추출한 텍스트가 없다"));
    }
    // 인코딩 손상 판정 표본은 앞 5쪽과 끝 5쪽이다. 앞 5쪽은 위에서 이미 뽑았고, 겹치지 않는 뒤쪽만 더 뽑는다.
    let mut sample = head;
    if pages > head_last {
        let tail_first = (pages - TEXT_PROBE_PAGES + 1).max(head_last + 1);
        match extract_text(&watching, path, tail_first, pages) {
            Ok(text) => {
                sample.push(' ');
                sample.push_str(&text);
            }
            Err(e) => {
                if !note_runner_failure(watching.take(), report) {
                    report.mark_incomplete(&format!("pdftotext failed: {}", sanitize(&e, 200, Some('…'))));
                }
                return;
            }
        }
    }
    crate::mojibake::check_text(report, &sample);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::security;

    fn empty_tree_doc(count: i64) -> Document {
        let mut doc = Document::with_version("1.7");
        let mut cat = Dictionary::new();
        cat.set("Type", "Catalog");
        cat.set("Pages", Object::Reference((2, 0)));
        doc.objects.insert((1, 0), Object::Dictionary(cat));
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Count", count);
        pages.set("Kids", Object::Array(vec![]));
        doc.objects.insert((2, 0), Object::Dictionary(pages));
        doc.trailer.set("Root", Object::Reference((1, 0)));
        doc
    }

    fn check(doc: &Document) -> FileReport {
        let mut r = FileReport::new(Path::new("x.pdf"), None);
        document_checks(doc, &mut r);
        r
    }

    #[test]
    fn zero_pages_is_high_only_when_the_tree_is_really_empty() {
        let r = check(&empty_tree_doc(0));
        assert!(r.findings.iter().any(|f| f.code == "ZERO_PAGES") && !r.incomplete);
    }

    #[test]
    fn positive_count_with_empty_page_list_is_incomplete_not_zero_pages() {
        let r = check(&empty_tree_doc(40));
        assert!(!r.findings.iter().any(|f| f.code == "ZERO_PAGES"));
        assert!(r.incomplete);
    }

    #[test]
    fn emptied_reference_array_makes_zero_pages_incomplete() {
        security::mark_emptied_ref_array_for_test();
        let r = check(&empty_tree_doc(0));
        assert!(!r.findings.iter().any(|f| f.code == "ZERO_PAGES"));
        assert!(r.incomplete);
    }
}
