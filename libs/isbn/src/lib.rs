//! 파일에서 ISBN을 찾는다.
//!
//! TXT는 앞뒤 8KB, EPUB은 OPF 메타데이터를 먼저 보고 없으면 spine의 첫·끝 챕터, PDF는 앞뒤 5쪽, DJVU는 앞뒤 5쪽,
//! HWP는 `strings` 출력의 앞뒤만 본다. OCR이 망가뜨린 글자(l, I, B, O, Ⅹ)도 알아본다.
//!
//! 파일을 읽는 일은 [`doc_reader::head_tail_reader`]가 한다. 이 crate는 어느 부분을 어떤 순서로 볼지,
//! PDF 단계의 시간 상한과 통계 기록, 본문 검색과 검증을 맡는다.
//! 외부 도구와 PDF 추출기는 [`Backends`]로 주입한다. 테스트가 대역을 꽂는 자리다.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use doc_reader::head_tail_reader::{EpubPackage, pdf_text_auto, pdf_text_fallback, read_djvu_head_tail, read_head_tail_from_content, read_head_tail_from_file, read_hwp_strings};
use regex::Regex;
use stat::Stat;
use sysdeps::timeout_runner::{run_with_hard_timeout, run_with_timeout};
use sysdeps::{CommandRunner, SystemRunner};

/// 앞뒤로 읽을 바이트 크기 (8KB).
pub const HEAD_TAIL_SIZE: usize = 8 * 1024;
/// PDF ISBN 추출 상한(초). 본문 추출을 포기한 손상 PDF도 여기서 다시 파싱되므로 상한이 없으면 멈춘다.
pub const PDF_ISBN_TIMEOUT: f64 = 30.0;
/// ISBN PDF 파서를 자식 프로세스에서 실행하는 숨은 부명령.
pub const PDF_ISBN_WORKER_COMMAND: &str = "__isbn-pdf-stage";

// ---------------------------------------------------------------- 검증

pub fn validate_isbn10(isbn: &str) -> bool {
    if isbn == "1111111111" || isbn == "1100101101" {
        return false;
    }
    let chars: Vec<char> = isbn.chars().collect();
    if chars.len() != 10 || !chars[..9].iter().all(|c| decimal_digit_value(*c).is_some()) || !(chars[9] == 'X' || decimal_digit_value(chars[9]).is_some()) {
        return false;
    }
    let mut total = 0;
    for (i, c) in chars[..9].iter().enumerate() {
        let Some(d) = decimal_digit_value(*c) else { return false };
        total += d * (10 - i as u32);
    }
    total += if chars[9] == 'X' { 10 } else { decimal_digit_value(chars[9]).unwrap_or(0) };
    total % 11 == 0
}

pub fn validate_isbn13(isbn: &str) -> bool {
    if isbn.chars().count() != 13 {
        return false;
    }
    let Some(digits) = isbn.chars().map(decimal_digit_value).collect::<Option<Vec<_>>>() else { return false };
    let total: u32 = digits.iter().enumerate().map(|(i, d)| d * if i.is_multiple_of(2) { 1 } else { 3 }).sum();
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
// 정규식 조각을 줄마다 나눠 적고 이어 붙인다.
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
    s.chars().filter(|c| decimal_digit_value(*c).is_some() || *c == 'X').collect()
}

/// ASCII 숫자와 전각 숫자를 숫자값으로 바꾼다.
fn decimal_digit_value(c: char) -> Option<u32> {
    match c {
        '0'..='9' => c.to_digit(10),
        // 전각 숫자(０~９). 한국어 문서의 ISBN에 가끔 쓰인다.
        '０'..='９' => Some(c as u32 - '０' as u32),
        _ => None,
    }
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
    // UTF-8로 읽되 잘못된 바이트는 버린다.
    search_in_content(&content.utf8_chunks().map(|c| c.valid()).collect::<String>())
}
// ---------------------------------------------------------------- 외부 명령·EPUB

/// 외부 도구(`djvused`, `djvutxt`, `strings`)의 시간 상한. PDF ISBN 추출과 같은 [`PDF_ISBN_TIMEOUT`]이다.
fn tool_timeout() -> Duration {
    Duration::from_secs_f64(PDF_ISBN_TIMEOUT)
}

/// DJVU: 앞 5쪽과 뒤 5쪽의 텍스트만 본다. 도구가 없으면 빈 목록이다.
pub fn extract_from_djvu(path: &Path, runner: &dyn CommandRunner) -> Vec<String> {
    read_djvu_head_tail(path, runner, tool_timeout()).map(|content| search_in_content(&content)).unwrap_or_default()
}

/// HWP: `strings` 출력의 앞뒤만 본다. 도구가 없으면 빈 목록이다.
pub fn extract_from_hwp(path: &Path, runner: &dyn CommandRunner) -> Vec<String> {
    read_hwp_strings(path, runner, tool_timeout()).map(|out| search_in_content(&read_head_tail_from_content(&out, HEAD_TAIL_SIZE))).unwrap_or_default()
}

/// EPUB: OPF 메타데이터를 먼저 보고, 없으면 spine의 첫 챕터와 끝 챕터 본문을 본다.
pub fn extract_from_epub(path: &Path) -> Vec<String> {
    epub_isbns(path).unwrap_or_default()
}

fn epub_isbns(path: &Path) -> Option<Vec<String>> {
    let mut package = EpubPackage::open(path)?;
    let from_metadata = search_in_content(&package.opf);
    if !from_metadata.is_empty() {
        return Some(from_metadata);
    }
    // OPF에 없을 때만 챕터를 읽는다.
    Some(search_in_content(&package.edge_chapters_text()?))
}

// ---------------------------------------------------------------- PDF

type PdfPrimary = Arc<dyn Fn(&Path) -> Result<(String, usize), String> + Send + Sync>;
type PdfFallback = Arc<dyn Fn(&Path) -> Result<String, String> + Send + Sync>;

/// 외부 의존을 한곳에 모은 묶음. 기본값은 실제 도구와 lopdf 추출기다.
pub struct Backends {
    pub pdf_primary: PdfPrimary,
    pub pdf_fallback: PdfFallback,
    pub runner: Rc<dyn CommandRunner>,
    pub stat: Arc<Stat>,
    pub pdf_timeout: f64,
    /// 지정하면 Poppler·대비책 단계를 별도 프로세스에서 실행해 멈춘 작업을 종료한다.
    pub pdf_worker_executable: Option<std::path::PathBuf>,
}

impl Backends {
    pub fn real() -> Self {
        Backends {
            pdf_primary: Arc::new(pdf_text_auto),
            pdf_fallback: Arc::new(pdf_text_fallback),
            runner: Rc::new(SystemRunner),
            stat: Arc::clone(Stat::global()),
            pdf_timeout: PDF_ISBN_TIMEOUT,
            pdf_worker_executable: None,
        }
    }
}

enum StageError {
    Timeout,
    Failed,
}

/// 한 추출 단계를 시간 상한 안에서 실행한다. panic도 실패로 본다.
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

fn run_pdf_worker(executable: &Path, stage: &str, path: &Path, seconds: f64) -> Result<(String, usize), StageError> {
    let mut command = Command::new(executable);
    command.arg(PDF_ISBN_WORKER_COMMAND).arg(stage).arg(path);
    let output = run_with_hard_timeout(command, seconds).map_err(|_| StageError::Timeout)?;
    if !output.status.success() {
        return Err(StageError::Failed);
    }
    let output = String::from_utf8(output.stdout).map_err(|_| StageError::Failed)?;
    let (pages, text) = output.split_once('\n').ok_or(StageError::Failed)?;
    let pages = pages.parse::<usize>().map_err(|_| StageError::Failed)?;
    Ok((text.to_owned(), pages))
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
    let primary_result = match b.pdf_worker_executable.as_deref() {
        Some(executable) => run_pdf_worker(executable, "poppler", path, b.pdf_timeout),
        None => run_stage(b.pdf_timeout, "isbn/poppler", move || primary(&owned)),
    };
    match primary_result {
        Ok((t, n)) => {
            b.stat.record_pdf_stage("isbn_poppler", start.elapsed().as_secs_f64(), outcome(&t));
            (text, pages) = (t, n);
        }
        Err(StageError::Timeout) => {
            b.stat.record_pdf_stage("isbn_poppler", start.elapsed().as_secs_f64(), "timeout");
            return Vec::new();
        }
        Err(StageError::Failed) => b.stat.record_pdf_stage("isbn_poppler", start.elapsed().as_secs_f64(), "error"),
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
    let fallback_result = match b.pdf_worker_executable.as_deref() {
        Some(executable) => run_pdf_worker(executable, "fallback", path, b.pdf_timeout).map(|(text, _)| text),
        None => run_stage(b.pdf_timeout, "isbn/fallback", move || fallback(&owned)),
    };
    match fallback_result {
        Ok(t) => {
            b.stat.record_pdf_stage("isbn_fallback", start.elapsed().as_secs_f64(), outcome(&t));
            search_in_content(&t)
        }
        Err(StageError::Timeout) => {
            b.stat.record_pdf_stage("isbn_fallback", start.elapsed().as_secs_f64(), "timeout");
            Vec::new()
        }
        Err(StageError::Failed) => {
            b.stat.record_pdf_stage("isbn_fallback", start.elapsed().as_secs_f64(), "error");
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

/// 호출자가 제공한 hidden worker로 PDF 파싱을 격리해 ISBN을 추출한다.
pub fn extract_default_with_worker(path: &Path, content: Option<&str>, executable: &Path) -> Vec<String> {
    let mut backends = Backends::real();
    backends.pdf_worker_executable = Some(executable.to_path_buf());
    extract(path, content, &backends)
}
