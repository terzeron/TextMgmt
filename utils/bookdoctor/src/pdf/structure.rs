//! PDF 구조·렌더링 검사. lopdf로 구조를 보고, Poppler(`pdfinfo`, `pdftotext`)로 실제 열리는지 확인한다.
//!
//! Poppler 호출 정책: 비싼 외부 프로세스라서 파일당 `pdfinfo`는 최대 한 번이다. `--no-poppler`이거나 `%PDF-` 머리가 없으면
//! 부르지 않는다. lopdf가 못 읽은 파일도 부른다(lopdf는 못 읽어도 뷰어는 여는 파일을 가려내는 것이 목적이다).
//! 값싼 구조 검사를 모두 끝낸 뒤에 부르고, 시간 상한은 `sysdeps::poppler`의 30초다.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
const POPPLER_MISSING: &str = "poppler not installed";

/// PDF 32000-1:2008 9.6.2.2 "Standard Type 1 Fonts (Standard 14 Fonts)". 이 글꼴은 뷰어가 내장하므로 임베드하지 않아도 된다.
const BASE_14: [&str; 14] =
    ["Times-Roman", "Times-Bold", "Times-Italic", "Times-BoldItalic", "Helvetica", "Helvetica-Bold", "Helvetica-Oblique", "Helvetica-BoldOblique", "Courier", "Courier-Bold", "Courier-Oblique", "Courier-BoldOblique", "Symbol", "ZapfDingbats"];

/// `NotFound`를 기록하는 실행기 래퍼. `sysdeps::poppler`는 오류를 문자열로만 돌려주기 때문이다.
struct Watching<'a> {
    inner: &'a dyn CommandRunner,
    not_found: AtomicBool,
}

impl CommandRunner for Watching<'_> {
    fn run(&self, argv: &[String], timeout: Option<Duration>) -> Result<CommandOutput, CommandError> {
        let result = self.inner.run(argv, timeout);
        if matches!(result, Err(CommandError::NotFound)) {
            self.not_found.store(true, Ordering::Relaxed);
        }
        result
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        self.inner.which(program)
    }
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
        poppler_checks(path, opts, report, runner);
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
    if has_root && pages.is_empty() {
        report.push(Category::Defect, Severity::High, "ZERO_PAGES", "", "페이지가 0쪽이다");
    }
    tiny_pages(doc, &pages, report);
    unembedded_fonts(doc, report);
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
        if BASE_14.contains(&name) {
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

fn poppler_checks(path: &Path, opts: &Options, report: &mut FileReport, runner: &dyn CommandRunner) {
    if opts.no_poppler {
        return;
    }
    let watching = Watching { inner: runner, not_found: AtomicBool::new(false) };
    let adapter = &watching;
    let pages = match page_count(adapter, path) {
        Ok(n) => n,
        Err(e) => {
            if watching.not_found.load(Ordering::Relaxed) {
                report.mark_incomplete(POPPLER_MISSING);
            } else {
                report.push(Category::Render, Severity::High, "POPPLER_OPEN_FAILED", "", &sanitize(&e, 200, Some('…')));
            }
            return;
        }
    };
    if !opts.pdf_text || pages == 0 {
        return;
    }
    match extract_text(adapter, path, 1, pages.min(TEXT_PROBE_PAGES)) {
        Ok(text) if text.trim().is_empty() => {
            report.push(Category::Defect, Severity::Low, "NO_TEXT_LAYER", "", &format!("앞 {}쪽에서 추출한 텍스트가 없다", pages.min(TEXT_PROBE_PAGES)));
        }
        Ok(_) => {}
        Err(_) if watching.not_found.load(Ordering::Relaxed) => report.mark_incomplete(POPPLER_MISSING),
        Err(e) => report.mark_incomplete(&format!("pdftotext failed: {}", sanitize(&e, 200, Some('…')))),
    }
}
