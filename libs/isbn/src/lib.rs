//! 파일에서 ISBN을 추출한다. 원본: `utils/isbn.py`.
//!
//! TXT는 앞뒤 8KB, EPUB은 OPF 메타데이터와 spine의 첫·끝 챕터, PDF는 앞뒤 5쪽, DJVU는 앞뒤 5쪽,
//! HWP는 `strings` 출력의 앞뒤만 본다. OCR이 망가뜨린 글자(l, I, B, O, Ⅹ)도 알아본다.
//!
//! 외부 도구(`djvused`, `djvutxt`, `strings`)와 PDF 추출기는 [`Backends`]로 주입한다. Python 원본의
//! monkeypatch 자리이고, 시간 상한([`parser_timeout::run_with_timeout`])이 걸리는 단계다.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Instant;

use corpus_common::reader::{decode_utf8_ignore, open_zip};
use htmlkit::{local_name, text_of, tokenize};
use parser_timeout::run_with_timeout;
use regex::Regex;
use stat::Stat;

/// 앞뒤로 읽을 바이트 크기 (8KB).
pub const HEAD_TAIL_SIZE: usize = 8 * 1024;
/// PDF ISBN은 앞/뒤 페이지에 있는 경우가 대부분이라 전체 문서를 열지 않는다.
pub const PDF_ISBN_PAGE_WINDOW: usize = 5;
/// PDF ISBN 추출 상한(초). 본문 추출을 포기한 손상 PDF도 여기서 다시 파싱되므로 상한이 없으면 멈춘다.
pub const PDF_ISBN_TIMEOUT: f64 = 30.0;

// ---------------------------------------------------------------- 앞뒤 읽기

/// 파일의 앞/뒤 부분만 읽는다(바이트 기준). 작은 파일은 전체를 읽는다.
pub fn read_head_tail_from_file(path: &Path, size: usize) -> io::Result<String> {
    let mut f = File::open(path)?;
    let file_size = f.seek(SeekFrom::End(0))? as usize;
    f.seek(SeekFrom::Start(0))?;
    if file_size <= size * 2 {
        let mut all = Vec::new();
        f.read_to_end(&mut all)?;
        return Ok(decode_utf8_ignore(&all));
    }
    let mut head = vec![0u8; size];
    f.read_exact(&mut head)?;
    f.seek(SeekFrom::End(-(size as i64)))?;
    let mut tail = vec![0u8; size];
    f.read_exact(&mut tail)?;
    Ok(decode_utf8_ignore(&head) + &decode_utf8_ignore(&tail))
}

/// 문자열의 앞/뒤 부분만 추출한다(문자 수 기준).
pub fn read_head_tail_from_content(content: &str, size: usize) -> String {
    let chars: Vec<char> = content.chars().collect();
    if chars.len() <= size * 2 {
        return content.to_owned();
    }
    chars[..size].iter().chain(&chars[chars.len() - size..]).collect()
}

// ---------------------------------------------------------------- 검증

pub fn validate_isbn10(isbn: &str) -> bool {
    if isbn == "1111111111" || isbn == "1100101101" {
        return false;
    }
    let chars: Vec<char> = isbn.chars().collect();
    if chars.len() != 10 || !chars.iter().all(|c| c.is_ascii_digit() || *c == 'X') {
        return false;
    }
    let mut total = 0;
    for (i, c) in chars[..9].iter().enumerate() {
        let Some(d) = c.to_digit(10) else { return false };
        total += d * (10 - i as u32);
    }
    total += if chars[9] == 'X' { 10 } else { chars[9].to_digit(10).unwrap_or(0) };
    total % 11 == 0
}

pub fn validate_isbn13(isbn: &str) -> bool {
    if isbn.chars().count() != 13 || !isbn.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let total: u32 = isbn.chars().enumerate().map(|(i, c)| c.to_digit(10).unwrap_or(0) * if i.is_multiple_of(2) { 1 } else { 3 }).sum();
    total.is_multiple_of(10)
}

pub fn validate_isbn(isbn: &str) -> bool {
    match isbn.chars().count() {
        10 => validate_isbn10(isbn),
        13 => validate_isbn13(isbn),
        _ => false,
    }
}

// ---------------------------------------------------------------- 본문 검색

// 구분자를 쓰는 패턴과 구분자가 뚜렷하지 않은 패턴. 구분자가 있으면 그룹 숫자 사이에 공백이 낄 수 있다.
// Python 원본은 re.VERBOSE로 줄을 나눠 썼다. 여기서는 같은 조각을 이어 붙인다.
const ISBN_PATTERN: &str = concat!(
    r"(?:[:_]?|\b)",
    r"(",
    r"(?:9\s?7\s?[8B9]\s?[\-─–－‐]{1,3}\s?)?",
    r"(?:[8B]\s?9|[1Il]\s?[1Il])",
    r"\s?[\-─–－‐]{1,3}\s?",
    r"(?:[\dIOBlsⅩ]\s?){1,7}",
    r"\s?[\-─–－‐]{1,3}\s?",
    r"(?:[\dIOBlsⅩ]\s?){1,6}",
    r"\s?[\-─–－‐]{1,3}\s?",
    r"[\dIOBlsⅩX]",
    r"|",
    r"(?:97[8B9][\-\s─–－‐]{0,3})?",
    r"(?:[8B]9|11|[Il][Il])",
    r"[\-\s─–－‐]{0,3}",
    r"[\dIOBlsⅩ]{1,7}",
    r"[\-\s─–－‐]{0,3}",
    r"[\dIOBlsⅩ]{1,6}",
    r"[\-\s─–－‐]{0,3}",
    r"[\dIOBlsⅩ]{0,7}",
    r"[\-\s─–－‐]{0,3}",
    r"[\dIOBlsⅩ]{0,7}",
    r"[\-\s─–－‐]{0,3}",
    r"[\dIOBlsⅩX]",
    r")",
    r"\b"
);

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("잘못된 정규식 {pattern}: {e}"))
}

static ISBN_RE: LazyLock<Regex> = LazyLock::new(|| re(ISBN_PATTERN));
static OCR_ONE: LazyLock<Regex> = LazyLock::new(|| re(r"l|\bI\b"));
static OCR_EIGHT: LazyLock<Regex> = LazyLock::new(|| re(r"s|B"));
static NON_DIGIT_RUN: LazyLock<Regex> = LazyLock::new(|| re(r"[^\dX]+"));
static LEADING_OR_STRAY: LazyLock<Regex> = LazyLock::new(|| re(r"^ |[^\dX ]"));
static ISBN13_RE: LazyLock<Regex> = LazyLock::new(|| re(r"(97[89] ?(?:\d ?){10})\b(?:\d+)?"));
static ISBN10_RE: LazyLock<Regex> = LazyLock::new(|| re(r"^((?:8 ?9 ?(?:\d ?){7}|[01] ?(?:\d ?){8})[\dX])\b(?:\d)?"));

/// OCR이 읽은 글자를 숫자로 되돌린다: l·I→1, s·B→8, O→0, Ⅹ→X.
fn normalize_ocr(raw: &str) -> String {
    let s = OCR_ONE.replace_all(raw, "1");
    let s = OCR_EIGHT.replace_all(&s, "8");
    let s = s.replace('O', "0").replace('Ⅹ', "X");
    let s = NON_DIGIT_RUN.replace_all(&s, " ");
    LEADING_OR_STRAY.replace_all(&s, "").into_owned()
}

fn strip_to_isbn_chars(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit() || *c == 'X').collect()
}

pub fn search_in_content(content: &str) -> Vec<String> {
    let mut found = Vec::new();
    for m in ISBN_RE.captures_iter(content) {
        let result = normalize_ocr(&m[1]);
        let candidate = ISBN13_RE.captures(&result).or_else(|| ISBN10_RE.captures(&result)).map(|c| strip_to_isbn_chars(&c[1]));
        if let Some(isbn) = candidate
            && validate_isbn(&isbn)
        {
            found.push(isbn);
        }
    }
    found
}

pub fn search_in_content_bytes(content: &[u8]) -> Vec<String> {
    search_in_content(&decode_utf8_ignore(content))
}

// ---------------------------------------------------------------- 외부 명령

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdOutput {
    pub code: Option<i32>,
    pub stdout: String,
}

/// 외부 명령 실행. 테스트가 `djvused`·`strings` 같은 도구 없이 확인할 수 있게 분리했다.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CmdOutput>;
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[String]) -> io::Result<CmdOutput> {
        let out = Command::new(program).args(args).output()?;
        Ok(CmdOutput { code: out.status.code(), stdout: decode_utf8_ignore(&out.stdout) })
    }
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// DJVU: 앞 5쪽과 뒤 5쪽의 텍스트만 본다. 도구가 없으면 빈 목록이다.
pub fn extract_from_djvu(path: &Path, runner: &dyn CommandRunner) -> Vec<String> {
    let file = path_arg(path);
    let Ok(pages) = runner.run("djvused", &[file.clone(), "-e".into(), "n".into()]) else { return Vec::new() };
    if pages.code != Some(0) {
        return Vec::new();
    }
    let Ok(total) = pages.stdout.trim().parse::<usize>() else { return Vec::new() };

    let head: Vec<String> = (1..=total.min(5)).map(|i| i.to_string()).collect();
    let tail: Vec<String> = (total.saturating_sub(4).max(1)..=total).map(|i| i.to_string()).collect();
    let mut content = String::new();
    for range in [head.join(","), tail.join(",")] {
        match runner.run("djvutxt", &[format!("--page={range}"), file.clone()]) {
            Ok(o) if o.code == Some(0) => content.push_str(&o.stdout),
            Ok(_) => {}
            Err(_) => return Vec::new(),
        }
    }
    if content.is_empty() { Vec::new() } else { search_in_content(&content) }
}

/// HWP: `strings` 출력의 앞뒤만 본다. 도구가 없으면 빈 목록이다.
pub fn extract_from_hwp(path: &Path, runner: &dyn CommandRunner) -> Vec<String> {
    let Ok(out) = runner.run("strings", &[path_arg(path)]) else { return Vec::new() };
    if out.code != Some(0) {
        return Vec::new();
    }
    let content = read_head_tail_from_content(&out.stdout, HEAD_TAIL_SIZE);
    if content.is_empty() { Vec::new() } else { search_in_content(&content) }
}

// ---------------------------------------------------------------- EPUB

fn read_member(zip: &mut corpus_common::reader::Zip, name: &str) -> Result<Option<String>, ()> {
    use std::io::Read as _;
    let Ok(mut entry) = zip.by_name(name) else { return Ok(None) };
    let mut data = Vec::new();
    entry.read_to_end(&mut data).map_err(|_| ())?;
    Ok(Some(decode_utf8_ignore(&data)))
}

/// EPUB: OPF 메타데이터를 먼저 보고, 없으면 spine의 첫 챕터와 끝 챕터 본문을 본다.
pub fn extract_from_epub(path: &Path) -> Vec<String> {
    epub_isbns(path).unwrap_or_default()
}

fn epub_isbns(path: &Path) -> Option<Vec<String>> {
    let mut zip = open_zip(path)?;
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    // 확장자 비교는 대소문자를 가리고 첫 번째 것만 쓴다(원본과 같다).
    let opf_path = names.iter().find(|n| n.ends_with(".opf"))?.clone();

    let opf = read_member(&mut zip, &opf_path).ok()??;
    let from_metadata = search_in_content(&opf);
    if !from_metadata.is_empty() {
        return Some(from_metadata);
    }

    let (tags, _) = tokenize(&opf);
    let mut manifest: Vec<(String, String)> = Vec::new();
    let mut spine: Vec<String> = Vec::new();
    for tag in tags.iter().filter(|t| !t.closing) {
        match tag.name.as_str() {
            "item" => {
                let (id, href, media) = (tag.attr("id"), tag.attr("href"), tag.attr("media-type").unwrap_or(""));
                if let (Some(id), Some(href)) = (id, href)
                    && !id.is_empty()
                    && !href.is_empty()
                    && media.contains("html")
                {
                    manifest.retain(|(k, _)| k != id);
                    manifest.push((id.to_owned(), href.to_owned()));
                }
            }
            "itemref" => spine.extend(tag.attr("idref").map(str::to_owned)),
            _ => {}
        }
    }
    let hrefs: Vec<&String> = spine.iter().filter_map(|id| manifest.iter().find(|(k, _)| k == id).map(|(_, h)| h)).collect();
    let (first, last) = (hrefs.first()?, hrefs.last()?);
    let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");

    let mut content = String::new();
    for href in [first, last] {
        let chapter = if opf_dir.is_empty() { (*href).clone() } else { format!("{opf_dir}/{href}") };
        // 챕터가 없으면 건너뛰고, 읽다가 깨진 경우는 전체를 포기한다.
        if let Some(html) = read_member(&mut zip, &chapter).ok()? {
            content.push_str(&text_of(&html));
            content.push('\n');
        }
    }
    let _ = local_name;
    Some(search_in_content(&content))
}

// ---------------------------------------------------------------- PDF

/// 앞 `window`쪽과 뒤 `window`쪽의 0부터 시작하는 번호. 겹치면 한 번만 넣는다.
pub fn head_tail_page_indices(total_pages: usize, window: usize) -> Vec<usize> {
    let mut seen = Vec::new();
    for idx in (0..window.min(total_pages)).chain(total_pages.saturating_sub(window)..total_pages) {
        if !seen.contains(&idx) {
            seen.push(idx);
        }
    }
    seen
}

/// PDF 앞뒤 쪽의 텍스트와 전체 쪽수. 문서를 열지 못하면 오류다(pypdfium2 대응).
pub fn pdf_text_primary(path: &Path) -> Result<(String, usize), String> {
    let doc = lopdf::Document::load(path).map_err(|e| e.to_string())?;
    let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    let total = numbers.len();
    let mut text = String::new();
    for idx in head_tail_page_indices(total, PDF_ISBN_PAGE_WINDOW) {
        // 한 쪽을 못 읽어도 나머지 쪽은 계속 읽는다.
        if let Ok(page) = doc.extract_text(&[numbers[idx]]) {
            text.push_str(&page);
        }
    }
    Ok((text, total))
}

fn pdfium() -> Option<&'static pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::Pdfium;
    static PDFIUM: OnceLock<Option<Pdfium>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            // PDFIUM_LIB_PATH는 라이브러리 파일이거나 그것이 든 디렉토리다.
            if let Some(configured) = std::env::var_os("PDFIUM_LIB_PATH").map(std::path::PathBuf::from) {
                let lib = if configured.is_dir() { Pdfium::pdfium_platform_library_name_at_path(&configured) } else { configured };
                if let Ok(bindings) = Pdfium::bind_to_library(lib) {
                    return Some(Pdfium::new(bindings));
                }
            }
            Pdfium::bind_to_system_library().ok().map(Pdfium::new)
        })
        .as_ref()
}

/// pdfium으로 앞뒤 쪽의 텍스트와 전체 쪽수를 읽는다(pypdfium2와 같은 엔진). 라이브러리가 없거나
/// 문서를 열지 못하면 오류다. 쪽 하나라도 못 읽으면 전체가 실패한다(원본과 같다).
pub fn pdf_text_pdfium(path: &Path) -> Result<(String, usize), String> {
    let pdfium = pdfium().ok_or_else(|| "libpdfium을 찾지 못했다".to_owned())?;
    let doc = pdfium.load_pdf_from_file(path, None).map_err(|e| e.to_string())?;
    let pages = doc.pages();
    let total = pages.len() as usize;
    let mut text = String::new();
    for idx in head_tail_page_indices(total, PDF_ISBN_PAGE_WINDOW) {
        let page = pages.get(idx as i32).map_err(|e| e.to_string())?;
        // `all()`은 페이지 경계 안의 글자만 돌려준다. pypdfium2의 `get_text_range()`처럼 모든 글자를
        // (줄바꿈으로 만들어 넣는 글자까지) 순서대로 읽는다.
        let page_text = page.text().map_err(|e| e.to_string())?;
        text.extend(page_text.chars().iter().filter_map(|c| c.unicode_char()));
    }
    Ok((text, total))
}

/// 기본 추출기: pdfium이 있으면 그것을, 없으면 순수 Rust(lopdf)를 쓴다.
pub fn pdf_text_auto(path: &Path) -> Result<(String, usize), String> {
    if pdfium().is_some() { pdf_text_pdfium(path) } else { pdf_text_primary(path) }
}

/// 문서 구조를 못 여는 손상 PDF용 대비책(pypdf 대응): 스트림을 직접 훑어 글자열을 모은다.
pub fn pdf_text_fallback(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let blocks = stream_blocks(&bytes);
    let total = blocks.len();
    let mut text = String::new();
    for idx in head_tail_page_indices(total, PDF_ISBN_PAGE_WINDOW) {
        let raw = blocks[idx];
        let mut inflated = Vec::new();
        let content = match flate2::read::ZlibDecoder::new(raw).read_to_end(&mut inflated) {
            Ok(_) => &inflated[..],
            Err(_) => raw,
        };
        text.push_str(&strings_of(content));
    }
    Ok(text)
}

fn stream_blocks(bytes: &[u8]) -> Vec<&[u8]> {
    let find = |needle: &[u8], from: usize| bytes[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from);
    let mut blocks = Vec::new();
    let mut at = 0;
    while let Some(s) = find(b"stream", at) {
        let mut start = s + 6;
        if bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        match find(b"endstream", start) {
            Some(end) => {
                // "endstream" 안의 "stream"을 다시 시작점으로 잡지 않게 그 뒤부터 이어 찾는다.
                blocks.push(&bytes[start..end]);
                at = end + 9;
            }
            None => break,
        }
    }
    blocks
}

/// 콘텐츠 스트림의 `(...)`와 `<...>` 문자열을 모은다. `[...] TJ` 배열 안의 조각은 이어 붙이고 나머지는 공백으로 나눈다.
fn strings_of(content: &[u8]) -> String {
    let mut out = String::new();
    let (mut i, mut in_array) = (0, false);
    while i < content.len() {
        match content[i] {
            b'[' => in_array = true,
            b']' => {
                in_array = false;
                out.push(' ');
            }
            b'(' => {
                let (s, next) = literal_string(content, i);
                out.push_str(&s);
                if !in_array {
                    out.push(' ');
                }
                i = next;
                continue;
            }
            b'<' if content.get(i + 1) != Some(&b'<') => {
                if let Some(close) = content[i..].iter().position(|b| *b == b'>') {
                    let hex: Vec<u8> = content[i + 1..i + close].iter().copied().filter(u8::is_ascii_hexdigit).collect();
                    let decoded: String = hex.chunks(2).map(|p| u8::from_str_radix(&String::from_utf8_lossy(p), 16).unwrap_or(b'?') as char).collect();
                    out.push_str(&decoded);
                    if !in_array {
                        out.push(' ');
                    }
                    i += close;
                }
            }
            b'<' => i += 1, // 딕셔너리 `<<`
            _ => {}
        }
        i += 1;
    }
    out
}

/// PDF 리터럴 문자열을 읽는다. 중첩 괄호와 `\` 이스케이프(`\n`, `\(`, 8진수)를 푼다.
fn literal_string(content: &[u8], start: usize) -> (String, usize) {
    let (mut depth, mut i, mut out) = (0usize, start, String::new());
    while i < content.len() {
        match content[i] {
            b'(' => {
                depth += 1;
                if depth > 1 {
                    out.push('(');
                }
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return (out, i + 1);
                }
                out.push(')');
            }
            b'\\' => {
                i += 1;
                match content.get(i) {
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(d @ b'0'..=b'7') => {
                        let mut value = u32::from(d - b'0');
                        for _ in 0..2 {
                            match content.get(i + 1) {
                                Some(n @ b'0'..=b'7') => {
                                    value = value * 8 + u32::from(n - b'0');
                                    i += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(char::from_u32(value & 0xff).unwrap_or('?'));
                    }
                    Some(other) => out.push(*other as char),
                    None => break,
                }
            }
            b => out.push(b as char),
        }
        i += 1;
    }
    (out, content.len())
}

type PdfPrimary = Arc<dyn Fn(&Path) -> Result<(String, usize), String> + Send + Sync>;
type PdfFallback = Arc<dyn Fn(&Path) -> Result<String, String> + Send + Sync>;

/// 외부 의존을 한곳에 모은 묶음. 기본값은 실제 도구와 lopdf 추출기다.
pub struct Backends {
    pub pdf_primary: PdfPrimary,
    pub pdf_fallback: PdfFallback,
    pub runner: Rc<dyn CommandRunner>,
    pub stat: Arc<Stat>,
    pub pdf_timeout: f64,
}

impl Backends {
    pub fn real() -> Self {
        Backends { pdf_primary: Arc::new(pdf_text_auto), pdf_fallback: Arc::new(pdf_text_fallback), runner: Rc::new(SystemRunner), stat: Arc::clone(Stat::global()), pdf_timeout: PDF_ISBN_TIMEOUT }
    }
}

enum StageError {
    Timeout,
    Failed,
}

/// 한 추출 단계를 시간 상한 안에서 실행한다. panic도 실패로 본다(원본의 `except Exception`).
fn run_stage<T: Send + 'static>(seconds: f64, message: &str, f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, StageError> {
    match run_with_timeout(seconds, message, move || catch_unwind(AssertUnwindSafe(f))) {
        Err(_) => Err(StageError::Timeout),
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(_) => Err(StageError::Failed),
    }
}

fn outcome(text: &str) -> &'static str {
    if text.is_empty() { "empty" } else { "ok" }
}

fn extract_pdf(path: &Path, content: Option<&str>, b: &Backends) -> Vec<String> {
    if let Some(c) = content.filter(|c| !c.is_empty()) {
        let found = search_in_content(c);
        if !found.is_empty() {
            return found;
        }
    }

    let start = Instant::now();
    let primary = Arc::clone(&b.pdf_primary);
    let owned = path.to_path_buf();
    let (mut pages, mut text) = (0, String::new());
    match run_stage(b.pdf_timeout, "isbn/pdfium", move || primary(&owned)) {
        Ok((t, n)) => {
            b.stat.record_pdf_stage("isbn_pdfium", start.elapsed().as_secs_f64(), outcome(&t));
            (text, pages) = (t, n);
        }
        Err(StageError::Timeout) => {
            b.stat.record_pdf_stage("isbn_pdfium", start.elapsed().as_secs_f64(), "timeout");
            return Vec::new();
        }
        Err(StageError::Failed) => b.stat.record_pdf_stage("isbn_pdfium", start.elapsed().as_secs_f64(), "error"),
    }
    if !text.is_empty() {
        return search_in_content(&text);
    }
    if pages > 0 {
        return Vec::new();
    }

    // 문서 구조를 아예 못 열었을 때만 대비책을 쓴다. 손상 PDF는 안에서 무한히 돌 수 있어 상한이 필요하다.
    let start = Instant::now();
    let fallback = Arc::clone(&b.pdf_fallback);
    let owned = path.to_path_buf();
    match run_stage(b.pdf_timeout, "isbn/pypdf", move || fallback(&owned)) {
        Ok(t) => {
            b.stat.record_pdf_stage("isbn_pypdf", start.elapsed().as_secs_f64(), outcome(&t));
            search_in_content(&t)
        }
        Err(StageError::Timeout) => {
            b.stat.record_pdf_stage("isbn_pypdf", start.elapsed().as_secs_f64(), "timeout");
            Vec::new()
        }
        Err(StageError::Failed) => {
            b.stat.record_pdf_stage("isbn_pypdf", start.elapsed().as_secs_f64(), "error");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------- 진입점

/// 파일에서 ISBN을 추출한다. `content`는 이미 읽은 본문이다(중복 I/O를 피하려는 용도).
pub fn extract(path: &Path, content: Option<&str>, backends: &Backends) -> Vec<String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" => {
            let search = match content.filter(|c| !c.is_empty()) {
                Some(c) => read_head_tail_from_content(c, HEAD_TAIL_SIZE),
                None => read_head_tail_from_file(path, HEAD_TAIL_SIZE).unwrap_or_default(),
            };
            search_in_content(&search)
        }
        "epub" => extract_from_epub(path),
        "pdf" => extract_pdf(path, content, backends),
        "djvu" => extract_from_djvu(path, backends.runner.as_ref()),
        "hwp" => extract_from_hwp(path, backends.runner.as_ref()),
        _ => Vec::new(),
    }
}

/// 실제 도구로 추출한다.
pub fn extract_default(path: &Path, content: Option<&str>) -> Vec<String> {
    extract(path, content, &Backends::real())
}
