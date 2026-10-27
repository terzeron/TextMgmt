//! PDF 읽기. 원본: `Loader.read_from_pdf`와 `_fast_pdf_page_count`.
//!
//! 원본은 단계를 순서대로 시도한다: pypdfium2 -> pdfplumber -> (쪽을 하나도 못 열었을 때만) pikepdf로 구조 복구 후 pypdfium2.
//! pdfium은 `pdfium-render`가 같은 libpdfium을 쓴다. pdfplumber와 pikepdf는 Rust 대응이 없어 `lopdf`로 대신한다.
//! 단계 실행은 [`StageRunner`] 뒤에 둬서, 멈춘 pdfium을 끊어야 하는 실행 파일이 자식 프로세스로 돌릴 수 있게 했다.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Instant;

use isbn::pdfium_render::prelude::PdfiumLibraryBindings;
use regex::bytes::Regex as BytesRegex;

use crate::TEXT_SIZE;
use crate::mojibake::{count_mojibake_runs, is_usable_text};
use crate::pytext::{head_chars, is_space, keep_word_chars};

/// 앞에서부터 이만큼의 쪽만 훑는다(TEXT_SIZE를 채우는 데 실측 최대 27쪽).
pub const PDF_PAGE_LIMIT: usize = 40;
/// 파서 단계 하나의 벽시계 상한(초).
pub const PDF_STAGE_TIMEOUT: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// pypdfium2에 대응
    Pdfium,
    /// pdfplumber에 대응(lopdf)
    Plumber,
    /// pikepdf 복구 후 pypdfium2에 대응(lopdf로 다시 저장한 뒤 pdfium)
    Repair,
}

impl Stage {
    /// 통계에 쓰는 이름. 원본의 함수 이름 그대로다.
    pub fn name(self) -> &'static str {
        match self {
            Stage::Pdfium => "_pdf_text_by_pypdfium2",
            Stage::Plumber => "_pdf_text_by_pdfplumber",
            Stage::Repair => "_pdf_text_by_pikepdf_repair",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    Timeout,
    Failed(String),
}

/// 단계 하나를 실행해 (텍스트, 쪽수)를 돌려준다.
pub trait StageRunner {
    fn run(&self, stage: Stage, path: &Path) -> Result<(String, usize), StageError>;

    /// 실행 파일이 제공하는 격리 worker. 외부 파서의 후속 처리도 같은 실행 파일에서 분리할 때 쓴다.
    fn worker_executable(&self) -> Option<&Path> {
        None
    }
}

/// 현재 프로세스에서 단계를 실행한다. 시간 상한은 걸지 않는다.
pub struct InProcess;

impl StageRunner for InProcess {
    fn run(&self, stage: Stage, path: &Path) -> Result<(String, usize), StageError> {
        run_stage(stage, path).map_err(StageError::Failed)
    }
}

/// 단계의 실제 작업.
pub fn run_stage(stage: Stage, path: &Path) -> Result<(String, usize), String> {
    match stage {
        Stage::Pdfium => pdfium_text_from_file(path),
        Stage::Plumber => lopdf_text(path),
        Stage::Repair => {
            let mut doc = lopdf::Document::load(path).map_err(|e| e.to_string())?;
            let mut buffer = Vec::new();
            doc.save_to(&mut buffer).map_err(|e| e.to_string())?;
            pdfium_text_from_bytes(buffer)
        }
    }
}

pub fn collect_pages<E>(page_count: usize, mut page_text: impl FnMut(usize) -> Result<String, E>) -> Result<(String, usize), E> {
    let mut out = String::new();
    let mut total = 0;
    for i in 0..page_count.min(PDF_PAGE_LIMIT) {
        let text = page_text(i)?;
        total += text.chars().count();
        out.push_str(&text);
        if total >= TEXT_SIZE {
            break;
        }
    }
    Ok((out, page_count))
}

/// pypdfium2 `PdfTextPage.get_text_range()`와 같은 방식으로 한 쪽의 글을 읽는다: `FPDFText_GetText`에 글자 수 전체를 준다.
/// 짝이 안 맞는 서로게이트는 버린다(`errors="ignore"`).
///
/// # Safety
/// `bindings`가 가리키는 라이브러리에서 얻은 유효한 `text_page` 핸들이어야 한다.
unsafe fn text_range(bindings: &dyn PdfiumLibraryBindings, text_page: isbn::pdfium_render::prelude::FPDF_TEXTPAGE) -> String {
    let count = unsafe { bindings.FPDFText_CountChars(text_page) };
    if count <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; count as usize + 1];
    let written = unsafe { bindings.FPDFText_GetText(text_page, 0, count, buffer.as_mut_ptr()) };
    if written <= 0 {
        return String::new();
    }
    // 반환값은 끝의 널 글자를 포함한 글자 수다.
    char::decode_utf16(buffer[..(written as usize - 1).min(buffer.len())].iter().copied()).filter_map(Result::ok).collect()
}

enum Source<'a> {
    File(&'a Path),
    Bytes(&'a [u8]),
}

/// 문서를 열어 앞쪽부터 `TEXT_SIZE`만큼의 글을 모은다. 핸들은 이 함수가 소유하고 반드시 닫는다.
fn pdfium_text(source: Source) -> Result<(String, usize), String> {
    let bindings = isbn::pdfium_bindings().ok_or_else(|| "libpdfium을 찾지 못했다".to_owned())?;
    // SAFETY: 아래 핸들은 모두 이 라이브러리에서 얻어 이 함수 안에서만 쓰고, 얻은 순서의 역순으로 닫는다.
    unsafe {
        let doc = match source {
            Source::File(path) => bindings.FPDF_LoadDocument(path.to_str().ok_or("경로가 UTF-8이 아니다")?, None),
            Source::Bytes(bytes) => bindings.FPDF_LoadMemDocument(bytes, None),
        };
        if doc.is_null() {
            return Err(format!("PDF를 열지 못했다(오류 {})", bindings.FPDF_GetLastError()));
        }
        let page_count = bindings.FPDF_GetPageCount(doc).max(0) as usize;
        let result = collect_pages(page_count, |i| {
            let page = bindings.FPDF_LoadPage(doc, i as i32);
            if page.is_null() {
                return Err(format!("{i}쪽을 열지 못했다"));
            }
            let text_page = bindings.FPDFText_LoadPage(page);
            let text = if text_page.is_null() { Err(format!("{i}쪽의 글을 열지 못했다")) } else { Ok(text_range(bindings, text_page)) };
            if !text_page.is_null() {
                bindings.FPDFText_ClosePage(text_page);
            }
            bindings.FPDF_ClosePage(page);
            text
        });
        bindings.FPDF_CloseDocument(doc);
        result
    }
}

fn pdfium_text_from_file(path: &Path) -> Result<(String, usize), String> {
    pdfium_text(Source::File(path))
}

fn pdfium_text_from_bytes(bytes: Vec<u8>) -> Result<(String, usize), String> {
    pdfium_text(Source::Bytes(&bytes))
}

fn lopdf_text(path: &Path) -> Result<(String, usize), String> {
    let doc = lopdf::Document::load(path).map_err(|e| e.to_string())?;
    let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    collect_pages(numbers.len(), |i| Ok::<_, String>(doc.extract_text(&[numbers[i]]).unwrap_or_default()))
}

/// 어느 단계가 어떻게 끝났는지(통계 기록용).
#[derive(Debug, Clone, PartialEq)]
pub struct StageRecord {
    pub stage: Stage,
    pub seconds: f64,
    /// `ok`, `empty`, `damaged`, `timeout`, `error`
    pub outcome: &'static str,
}

/// PDF 읽기 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfRead {
    pub summary: String,
    pub page_count: usize,
    pub records: Vec<StageRecord>,
    /// 원본이 `LOGGER.error`/`warning`으로 남기는 메시지: (수준, 내용)
    pub logs: Vec<(&'static str, String)>,
}

fn blank(text: &str) -> bool {
    text.trim_matches(is_space).is_empty()
}

/// PDF에서 요약과 쪽수를 읽는다.
pub fn read_from_pdf(path: &Path, runner: &dyn StageRunner) -> PdfRead {
    let mut result = String::new();
    let mut page_count = 0;
    let mut best_damage: Option<usize> = None;
    let mut records = Vec::new();
    let mut logs: Vec<(&'static str, String)> = Vec::new();
    let mut timed_out: Vec<&'static str> = Vec::new();

    for (stage_no, stage) in [Stage::Pdfium, Stage::Plumber].into_iter().enumerate() {
        let started = Instant::now();
        let mut outcome = "error";
        let mut stop = false;
        match runner.run(stage, path) {
            Ok((text, pages)) => {
                if page_count == 0 {
                    page_count = pages;
                }
                if blank(&text) {
                    outcome = "empty";
                    // 1단계가 문서를 열었는데 텍스트가 없으면 스캔본이다. 다른 파서도 결과가 없다.
                    stop = stage_no == 0 && page_count > 0;
                } else {
                    let damage = count_mojibake_runs(&text);
                    if is_usable_text(&text, Some(damage)) {
                        outcome = "ok";
                        result = text;
                        stop = true;
                    } else {
                        outcome = "damaged";
                        if best_damage.is_none_or(|b| damage < b) {
                            best_damage = Some(damage);
                            result = text;
                        }
                    }
                }
            }
            Err(StageError::Timeout) => {
                outcome = "timeout";
                timed_out.push(stage.name());
                logs.push(("ERROR", path.display().to_string()));
                logs.push(("ERROR", format!("PDF 파서 타임아웃({PDF_STAGE_TIMEOUT}초 초과): {}", stage.name())));
            }
            Err(StageError::Failed(_)) => {}
        }
        records.push(StageRecord { stage, seconds: started.elapsed().as_secs_f64(), outcome });
        if stop {
            break;
        }
    }

    // 어느 파서도 문서를 열지 못했다면 구조 손상이다. 재구성한 뒤 한 번 더 시도한다.
    if blank(&result) && page_count == 0 {
        let stage = Stage::Repair;
        let started = Instant::now();
        let outcome = match runner.run(stage, path) {
            Ok((text, pages)) => {
                page_count = pages;
                if is_usable_text(&text, None) {
                    result = text;
                    logs.push(("WARNING", format!("PDF 구조 복구 후 적재 ({pages}쪽)")));
                    "ok"
                } else if !blank(&text) {
                    "damaged"
                } else {
                    "empty"
                }
            }
            Err(StageError::Timeout) => {
                timed_out.push(stage.name());
                logs.push(("ERROR", path.display().to_string()));
                logs.push(("ERROR", format!("PDF 구조 복구 타임아웃({PDF_STAGE_TIMEOUT}초 초과)")));
                "timeout"
            }
            Err(StageError::Failed(e)) => {
                logs.push(("ERROR", path.display().to_string()));
                logs.push(("ERROR", format!("PDF 구조 복구 실패(객체 소실 추정): {}", head_chars(&e, 120))));
                "error"
            }
        };
        records.push(StageRecord { stage, seconds: started.elapsed().as_secs_f64(), outcome });
    }

    if blank(&result) && !timed_out.is_empty() {
        logs.push(("ERROR", path.display().to_string()));
        logs.push(("ERROR", format!("PDF 텍스트 추출 실패: 모든 파서가 타임아웃 또는 오류 (타임아웃: {})", timed_out.join(", "))));
    } else if blank(&result) && page_count == 0 {
        logs.push(("ERROR", path.display().to_string()));
    }

    let cleaned = keep_word_chars(&result);
    PdfRead { summary: head_chars(&cleaned, TEXT_SIZE).to_owned(), page_count, records, logs }
}

pub type PdftotextWhich<'a> = dyn Fn() -> Option<PathBuf> + 'a;
pub type PdftotextRun<'a> = dyn Fn(&[String]) -> Option<(i32, Vec<u8>)> + 'a;

#[derive(Clone, Copy)]
pub struct PdftotextHooks<'a> {
    pub which: Option<&'a PdftotextWhich<'a>>,
    pub run: Option<&'a PdftotextRun<'a>>,
    /// Python `Loader` caches the binary path. Each Rust loader/test scope owns its cache.
    pub path_cache: &'a std::sync::OnceLock<Option<PathBuf>>,
}

/// 호환성 검증용 pdftotext 추출기. 운영 파싱은 `read_from_pdf`의 StageRunner 경로를 쓴다.
pub fn pdf_text_by_pdftotext_with(path: &Path, hooks: &PdftotextHooks<'_>) -> (String, usize) {
    let binary = hooks
        .path_cache
        .get_or_init(|| hooks.which.map_or_else(|| std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|dir| dir.join("pdftotext")).find(|p| p.is_file()), |find| find()));
    let Some(binary) = binary else { return (String::new(), 0) };
    let argv = vec![binary.to_string_lossy().into_owned(), "-enc".to_string(), "UTF-8".to_string(), "-l".to_string(), PDF_PAGE_LIMIT.to_string(), path.to_string_lossy().into_owned(), "-".to_string()];
    let output = hooks.run.map_or_else(|| std::process::Command::new(&argv[0]).args(&argv[1..]).output().ok().map(|o| (o.status.code().unwrap_or(1), o.stdout)), |run| run(&argv));
    let Some((0, bytes)) = output else { return (String::new(), 0) };
    let text = String::from_utf8_lossy(&bytes);
    let cleaned = keep_word_chars(&text);
    (head_chars(&cleaned, TEXT_SIZE).to_owned(), 0)
}

pub trait PlumberDocument {
    fn page_count(&self) -> usize;
    fn extract_text(&self, idx: usize) -> Option<String>;
}

pub fn pdf_text_by_plumber_with(_path: &Path, document: &dyn PlumberDocument) -> Result<(String, usize), String> {
    let mut text = String::new();
    for idx in 0..document.page_count() {
        if let Some(page) = document.extract_text(idx) {
            text.push_str(&page);
        }
        if text.chars().count() >= TEXT_SIZE {
            break;
        }
    }
    Ok((text, document.page_count()))
}

// ---- 경량 쪽수 추출 -------------------------------------------------------------------------

fn bre(pattern: &str) -> BytesRegex {
    BytesRegex::new(pattern).unwrap_or_else(|e| panic!("잘못된 정규식 {pattern}: {e}"))
}

pub fn find_xref_offset(xref_data: &[u8], obj_num: i64) -> Option<i64> {
    let text: Vec<u8> = {
        let mut t = Vec::with_capacity(xref_data.len());
        let mut i = 0;
        while i < xref_data.len() {
            if xref_data[i] == b'\r' {
                t.push(b'\n');
                if xref_data.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            } else {
                t.push(xref_data[i]);
            }
            i += 1;
        }
        t
    };
    let lines: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    let trim = |l: &[u8]| -> Vec<u8> { String::from_utf8_lossy(l).trim().as_bytes().to_vec() };
    let mut i = 0;
    while i < lines.len() && trim(lines[i]) != b"xref" {
        i += 1;
    }
    i += 1;
    while i < lines.len() {
        let line = trim(lines[i]);
        if line.is_empty() {
            i += 1;
            continue;
        }
        if line.starts_with(b"trailer") {
            break;
        }
        let parts: Vec<&[u8]> = line.split(|b| b.is_ascii_whitespace()).filter(|p| !p.is_empty()).collect();
        let digits = |p: &[u8]| !p.is_empty() && p.iter().all(u8::is_ascii_digit);
        if parts.len() == 2 && digits(parts[0]) && digits(parts[1]) {
            let sub_start: i64 = std::str::from_utf8(parts[0]).ok()?.parse().ok()?;
            let sub_count: i64 = std::str::from_utf8(parts[1]).ok()?.parse().ok()?;
            if sub_start <= obj_num && obj_num < sub_start + sub_count {
                let entry_idx = i + 1 + (obj_num - sub_start) as usize;
                if entry_idx < lines.len() {
                    let entry = trim(lines[entry_idx]);
                    let entry_parts: Vec<&[u8]> = entry.split(|b| b.is_ascii_whitespace()).filter(|p| !p.is_empty()).collect();
                    if entry_parts.len() >= 3 && entry_parts[2] == b"n" {
                        return std::str::from_utf8(entry_parts[0]).ok()?.parse().ok();
                    }
                }
                return None;
            }
            i += 1 + sub_count as usize;
            continue;
        }
        i += 1;
    }
    None
}

pub fn apply_png_predictor(data: &[u8], columns: usize) -> Vec<u8> {
    let stride = 1 + columns;
    let rows = data.len() / stride;
    let mut result = Vec::with_capacity(rows * columns);
    let mut prev_row = vec![0u8; columns];
    for i in 0..rows {
        let offset = i * stride;
        let fb = data[offset];
        let mut row = data[offset + 1..offset + 1 + columns].to_vec();
        match fb {
            1 => {
                for j in 1..columns {
                    row[j] = row[j].wrapping_add(row[j - 1]);
                }
            }
            2 => {
                for j in 0..columns {
                    row[j] = row[j].wrapping_add(prev_row[j]);
                }
            }
            _ => {}
        }
        result.extend_from_slice(&row);
        prev_row = row;
    }
    result
}

pub fn xref_stream_find_entry(data: &[u8], w: [usize; 3], index_ranges: &[i64], obj_num: i64) -> Option<(i64, i64, i64)> {
    let entry_size: usize = w.iter().sum();
    if entry_size == 0 {
        return None;
    }
    let big_endian = |bytes: &[u8]| bytes.iter().fold(0i64, |acc, &b| (acc << 8) | i64::from(b));
    let mut pos = 0;
    for pair in index_ranges.chunks_exact(2) {
        let (start_obj, count) = (pair[0], pair[1]);
        for j in 0..count {
            if pos + entry_size > data.len() {
                return None;
            }
            if start_obj + j == obj_num {
                let entry = &data[pos..pos + entry_size];
                let type_val = if w[0] > 0 { big_endian(&entry[..w[0]]) } else { 1 };
                let field2 = if w[1] > 0 { big_endian(&entry[w[0]..w[0] + w[1]]) } else { 0 };
                let field3 = if w[2] > 0 { big_endian(&entry[w[0] + w[1]..]) } else { 0 };
                return Some((type_val, field2, field3));
            }
            pos += entry_size;
        }
    }
    None
}

fn inflate(raw: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(raw).read_to_end(&mut out).ok().map(|_| out)
}

fn read_at(file: &mut std::fs::File, offset: u64, len: usize) -> Option<Vec<u8>> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        let n = file.read(&mut buf[filled..]).ok()?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Some(buf)
}

pub fn read_from_obj_stream(file: &mut std::fs::File, stream_offset: i64, target: i64) -> Option<Vec<u8>> {
    static FIRST: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/First\s+(\d+)"));
    static LENGTH: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Length\s+(\d+)"));
    static LENGTH_REF: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Length\s+\d+\s+\d+\s+R"));
    static STREAM: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"stream\r?\n"));
    static FILTER: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Filter\s*/(\w+)"));
    let header = read_at(file, stream_offset as u64, 4096)?;
    let first_offset: usize = std::str::from_utf8(&FIRST.captures(&header)?[1]).ok()?.parse().ok()?;
    let stream_length: usize = std::str::from_utf8(&LENGTH.captures(&header)?[1]).ok()?.parse().ok()?;
    if LENGTH_REF.is_match(&header) {
        return None;
    }
    let stream_start = STREAM.find(&header)?.end();
    let mut raw = read_at(file, stream_offset as u64 + stream_start as u64, stream_length)?;
    if let Some(f) = FILTER.captures(&header) {
        if &f[1] != b"FlateDecode" {
            return None;
        }
        raw = inflate(&raw)?;
    }
    let header_text = String::from_utf8_lossy(&raw[..first_offset.min(raw.len())]).into_owned();
    let tokens: Vec<&str> = header_text.split_whitespace().collect();
    let mut target_offset = None;
    let mut next_offset = raw.len();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if tokens[i].parse::<i64>().ok()? == target {
            target_offset = Some(tokens[i + 1].parse::<usize>().ok()? + first_offset);
            if i + 3 < tokens.len() {
                next_offset = tokens[i + 3].parse::<usize>().ok()? + first_offset;
            }
            break;
        }
        i += 2;
    }
    let start = target_offset?;
    raw.get(start..next_offset.min(raw.len())).map(<[u8]>::to_vec)
}

pub type XrefStream = (Vec<u8>, [usize; 3], Vec<i64>, Option<i64>);

pub fn parse_one_xref_stream(file: &mut std::fs::File, xref_offset: i64) -> Option<XrefStream> {
    static W: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/W\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s*\]"));
    static SIZE: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Size\s+(\d+)"));
    static INDEX: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Index\s*\[([^\]]+)\]"));
    static LENGTH_REF: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Length\s+\d+\s+\d+\s+R"));
    static LENGTH: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Length\s+(\d+)"));
    static STREAM: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"stream\r?\n"));
    static FILTER: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Filter\s*/(\w+)"));
    static PARMS: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/DecodeParms\s*<<([^>]*)>>"));
    static PREDICTOR: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Predictor\s+(\d+)"));
    static COLUMNS: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Columns\s+(\d+)"));
    static PREV: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Prev\s+(\d+)"));
    let num = |b: &[u8]| -> Option<i64> { std::str::from_utf8(b).ok()?.parse().ok() };
    let data = read_at(file, xref_offset as u64, 4096)?;
    let wc = W.captures(&data)?;
    let w = [num(&wc[1])? as usize, num(&wc[2])? as usize, num(&wc[3])? as usize];
    let xref_size = num(&SIZE.captures(&data)?[1])?;
    let index_ranges: Vec<i64> = match INDEX.captures(&data) {
        Some(c) => String::from_utf8_lossy(&c[1]).split_whitespace().map(|x| x.parse::<i64>()).collect::<Result<_, _>>().ok()?,
        None => vec![0, xref_size],
    };
    if LENGTH_REF.is_match(&data) {
        return None;
    }
    let stream_length = num(&LENGTH.captures(&data)?[1])? as usize;
    let stream_start = STREAM.find(&data)?.end();
    let raw = read_at(file, xref_offset as u64 + stream_start as u64, stream_length)?;
    let mut decompressed = match FILTER.captures(&data) {
        Some(f) => {
            if &f[1] != b"FlateDecode" {
                return None;
            }
            inflate(&raw)?
        }
        None => raw,
    };
    if let Some(parms) = PARMS.captures(&data) {
        let inner = parms[1].to_vec();
        if let Some(p) = PREDICTOR.captures(&inner)
            && num(&p[1])? >= 10
        {
            let columns = match COLUMNS.captures(&inner) {
                Some(c) => num(&c[1])? as usize,
                None => w.iter().sum(),
            };
            decompressed = apply_png_predictor(&decompressed, columns);
        }
    }
    let prev = PREV.captures(&data).and_then(|c| num(&c[1]));
    Some((decompressed, w, index_ranges, prev))
}

enum Lookup {
    Traditional(Vec<u8>),
    Stream(Vec<u8>, [usize; 3], Vec<i64>),
}

/// PDF 끝의 xref와 trailer에서 쪽수만 읽는다(전체 파싱 없이). 실패하면 `None`.
pub type XrefStreamParser<'a> = dyn Fn(&mut std::fs::File, u64) -> Option<XrefStream> + 'a;
pub type XrefEntryFinder<'a> = dyn Fn(&[u8], [usize; 3], &[i64], i64) -> Option<(i64, i64, i64)> + 'a;
pub type ObjectStreamReader<'a> = dyn Fn(&mut std::fs::File, u64, i64) -> Option<Vec<u8>> + 'a;
pub type XrefOffsetFinder<'a> = dyn Fn(&[u8], i64) -> Option<i64> + 'a;

#[derive(Clone, Copy, Default)]
pub struct FastCountHooks<'a> {
    pub find_xref_offset: Option<&'a XrefOffsetFinder<'a>>,
    pub parse_one_xref_stream: Option<&'a XrefStreamParser<'a>>,
    pub xref_stream_find_entry: Option<&'a XrefEntryFinder<'a>>,
    pub read_from_obj_stream: Option<&'a ObjectStreamReader<'a>>,
}

pub fn fast_page_count(path: &Path) -> Option<usize> {
    fast_page_count_with(path, &FastCountHooks::default())
}

pub fn fast_page_count_with(path: &Path, hooks: &FastCountHooks<'_>) -> Option<usize> {
    static STARTXREF: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"startxref\s+(\d+)"));
    static XREF_HDR: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"xref\s+\d+\s+(\d+)"));
    static ROOT: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Root\s+(\d+)\s+\d+\s+R"));
    static PREV: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Prev\s+(\d+)"));
    static PAGES_REF: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Pages\s+(\d+)\s+\d+\s+R"));
    static COUNT: LazyLock<BytesRegex> = LazyLock::new(|| bre(r"/Count\s+(\d+)"));
    let num = |b: &[u8]| -> Option<i64> { std::str::from_utf8(b).ok()?.parse().ok() };
    let mut file = std::fs::File::open(path).ok()?;
    let file_size = file.metadata().ok()?.len() as i64;
    let tail_size = file_size.min(4096);
    let tail = read_at(&mut file, (file_size - tail_size) as u64, tail_size as usize)?;
    let mut xref_offset: Option<i64> = Some(num(&STARTXREF.captures(&tail)?[1])?);
    let mut root_obj_num: Option<i64> = None;
    let mut lookups: Vec<Lookup> = Vec::new();
    let mut max_depth = 10;
    while let Some(offset) = xref_offset {
        if max_depth == 0 {
            break;
        }
        max_depth -= 1;
        let xref_header = read_at(&mut file, offset as u64, 256)?;
        let trimmed_start = xref_header.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(xref_header.len());
        if xref_header[trimmed_start..].starts_with(b"xref") {
            let entry_count = XREF_HDR.captures(&xref_header).and_then(|c| num(&c[1])).unwrap_or(5000);
            let read_size = (entry_count * 20 + 4096).min(file_size - offset);
            let xref_data = read_at(&mut file, offset as u64, read_size.max(0) as usize)?;
            if root_obj_num.is_none() {
                root_obj_num = ROOT.captures(&xref_data).and_then(|c| num(&c[1]));
            }
            xref_offset = PREV.captures(&xref_data).and_then(|c| num(&c[1]));
            lookups.push(Lookup::Traditional(xref_data));
        } else {
            let peek = read_at(&mut file, offset as u64, 512)?;
            if root_obj_num.is_none() {
                root_obj_num = ROOT.captures(&peek).and_then(|c| num(&c[1]));
            }
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match hooks.parse_one_xref_stream {
                Some(parse) => parse(&mut file, offset as u64),
                None => parse_one_xref_stream(&mut file, offset),
            }))
            .ok()
            .flatten();
            let Some((decompressed, w, index_ranges, prev)) = parsed else { break };
            lookups.push(Lookup::Stream(decompressed, w, index_ranges));
            xref_offset = prev;
        }
    }
    let root_obj_num = root_obj_num?;

    let find_file_offset = |obj_num: i64| -> Option<i64> {
        for lookup in &lookups {
            match lookup {
                Lookup::Traditional(data) => {
                    if let Some(o) = hooks.find_xref_offset.map_or_else(|| find_xref_offset(data, obj_num), |find| find(data, obj_num)) {
                        return Some(o);
                    }
                }
                Lookup::Stream(data, w, ranges) => {
                    if let Some((1, offset, _)) = hooks.xref_stream_find_entry.map_or_else(|| xref_stream_find_entry(data, *w, ranges, obj_num), |find| find(data, *w, ranges, obj_num)) {
                        return Some(offset);
                    }
                }
            }
        }
        None
    };
    let read_object_data = |obj_num: i64, size: usize, file: &mut std::fs::File| -> Option<Vec<u8>> {
        if let Some(offset) = find_file_offset(obj_num) {
            return read_at(file, offset as u64, size);
        }
        for lookup in &lookups {
            if let Lookup::Stream(data, w, ranges) = lookup
                && let Some((2, stream_obj, _)) = hooks.xref_stream_find_entry.map_or_else(|| xref_stream_find_entry(data, *w, ranges, obj_num), |find| find(data, *w, ranges, obj_num))
                && let Some(stream_offset) = find_file_offset(stream_obj)
            {
                return match hooks.read_from_obj_stream {
                    Some(read) => read(file, stream_offset as u64, obj_num),
                    None => read_from_obj_stream(file, stream_offset, obj_num),
                };
            }
        }
        None
    };
    let root_data = read_object_data(root_obj_num, 1024, &mut file)?;
    let pages_obj = num(&PAGES_REF.captures(&root_data)?[1])?;
    let pages_data = read_object_data(pages_obj, 4096, &mut file)?;
    COUNT.captures(&pages_data).and_then(|c| num(&c[1])).map(|n| n as usize)
}
