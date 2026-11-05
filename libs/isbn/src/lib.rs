//! 파일에서 ISBN을 찾는다.
//!
//! TXT는 앞뒤 8KB, EPUB은 OPF 메타데이터를 먼저 보고 없으면 spine의 첫·끝 챕터, PDF는 앞뒤 5쪽, DJVU는 앞뒤 5쪽,
//! HWP는 `strings` 출력의 앞뒤만 본다. OCR이 망가뜨린 글자(l, I, B, O, Ⅹ)도 알아본다.
//!
//! 파일을 읽는 일은 [`doc_reader::head_tail_reader`]가 한다. 이 crate는 어느 부분을 어떤 순서로 볼지,
//! PDF 단계의 통계 기록, 본문 검색과 검증을 맡는다.
//! PDF 단계는 본문 읽기와 같은 [`StageRunner`]로 실행한다. 시간 상한과 종료는 실행기가 맡는다.
//! 외부 도구와 PDF 실행기는 [`Backends`]로 주입한다. 테스트가 대역을 꽂는 자리다.

use std::path::Path;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use doc_reader::head_tail_reader::{EpubPackage, read_djvu_head_tail, read_head_tail_from_content, read_head_tail_from_file, read_hwp_strings};
use doc_reader::pdf::{PDF_STAGE_TIMEOUT, Stage, StageError, StageRunner};
use regex::Regex;
use stat::Stat;
use sysdeps::{CommandRunner, SystemRunner};

/// 앞뒤로 읽을 바이트 크기 (8KB).
pub const HEAD_TAIL_SIZE: usize = 8 * 1024;
/// 외부 도구(`djvused`, `djvutxt`, `strings`)의 시간 상한(초). PDF 단계 상한과 같다.
pub const TOOL_TIMEOUT: u64 = PDF_STAGE_TIMEOUT;

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

fn tool_timeout() -> Duration {
    Duration::from_secs(TOOL_TIMEOUT)
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

/// 외부 의존을 한곳에 모은 묶음.
pub struct Backends<'a> {
    /// PDF 단계 실행기. 본문 읽기에 쓴 실행기를 그대로 넘긴다.
    pub pdf_runner: &'a dyn StageRunner,
    pub runner: Rc<dyn CommandRunner>,
    /// PDF 단계 통계를 쌓을 곳. 본문 읽기와 같은 통계를 넘긴다.
    pub stat: &'a Stat,
}

impl<'a> Backends<'a> {
    /// 실제 외부 도구에 `pdf_runner`와 통계 `stat`을 묶는다.
    pub fn real(pdf_runner: &'a dyn StageRunner, stat: &'a Stat) -> Self {
        Backends { pdf_runner, runner: Rc::new(SystemRunner), stat }
    }
}

/// PDF 단계 하나를 실행하고 결과를 통계에 남긴다.
fn run_pdf_stage(b: &Backends<'_>, stage: Stage, path: &Path) -> Result<(String, usize), StageError> {
    let start = Instant::now();
    let result = b.pdf_runner.run(stage, path);
    let outcome = match &result {
        Ok((text, _)) if text.is_empty() => "empty",
        Ok(_) => "ok",
        Err(StageError::Timeout) => "timeout",
        Err(StageError::Failed(_)) => "error",
    };
    b.stat.record_pdf_stage(stage.name(), start.elapsed().as_secs_f64(), outcome);
    result
}

fn extract_pdf(path: &Path, content: Option<&str>, b: &Backends<'_>) -> Vec<String> {
    if let Some(c) = content.filter(|c| !c.is_empty()) {
        let found = search_in_content(c);
        if !found.is_empty() {
            return found;
        }
    }

    let (text, pages) = match run_pdf_stage(b, Stage::HeadTail, path) {
        Ok(read) => read,
        Err(StageError::Timeout) => return Vec::new(),
        Err(StageError::Failed(_)) => (String::new(), 0),
    };
    if !text.is_empty() {
        return search_in_content(&text);
    }
    if pages > 0 {
        return Vec::new();
    }

    // 문서 구조를 아예 못 열었을 때만 스트림을 직접 훑는다.
    match run_pdf_stage(b, Stage::HeadTailScan, path) {
        Ok((text, _)) => search_in_content(&text),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- 진입점

/// 파일에서 ISBN을 추출한다. `content`는 이미 읽은 본문이다(중복 I/O를 피하려는 용도).
pub fn extract(path: &Path, content: Option<&str>, backends: &Backends<'_>) -> Vec<String> {
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
