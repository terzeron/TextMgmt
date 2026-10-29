//! TXT 검사. 인코딩을 먼저 판정하고, 유효한 UTF-8 본문에만 문자 단위 검사를 한다.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use scan_common::{Category, FileReport, Severity};

use crate::cli::Options;

/// 한 줄의 최대 글자 수. 프론트(`frontend/src/ViewTXT.jsx`)가 `\n`으로 줄을 나눠 줄마다 `<p>`를 그린다.
const MAX_LINE_CHARS: usize = 100_000;

/// 안전을 위해 읽는 최대 바이트 수.
const MAX_READ_BYTES: u64 = 64 * 1024 * 1024;

static HTML_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<\s*(script|iframe|object|embed|html)\b|javascript:").unwrap());

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}')
}

/// 탭·LF·CR·FF를 뺀 제어문자인가.
fn is_binary_control(c: char) -> bool {
    c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{c}')
}

/// 같은 코드의 첫 위치와 횟수를 모은다.
#[derive(Default)]
struct Tally {
    first: Option<String>,
    count: usize,
}

impl Tally {
    fn hit(&mut self, location: impl FnOnce() -> String) {
        if self.first.is_none() {
            self.first = Some(location());
        }
        self.count += 1;
    }

    fn report(self, report: &mut FileReport, category: Category, code: &str) {
        if let Some(location) = self.first {
            report.push(category, Severity::Medium, code, &location, &format!("count={}", self.count));
        }
    }
}

fn read_capped(path: &Path, report: &mut FileReport) -> Option<Vec<u8>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            report.mark_incomplete(&format!("read failed: {e}"));
            return None;
        }
    };
    let mut buf = Vec::new();
    if let Err(e) = file.take(MAX_READ_BYTES + 1).read_to_end(&mut buf) {
        report.mark_incomplete(&format!("read failed: {e}"));
        return None;
    }
    if buf.len() as u64 > MAX_READ_BYTES {
        buf.truncate(MAX_READ_BYTES as usize);
        report.mark_incomplete("truncated: only first 64 MiB scanned");
    }
    Some(buf)
}

/// 잘린 끝(멀티바이트 한가운데)은 인코딩 오류가 아니므로 유효한 앞부분까지만 본다.
fn trim_truncated_tail(bytes: &[u8]) -> &[u8] {
    match std::str::from_utf8(bytes) {
        Err(e) if e.error_len().is_none() => &bytes[..e.valid_up_to()],
        _ => bytes,
    }
}

/// TXT 파일 하나를 검사해 `report`에 더한다.
pub fn scan_txt_file(path: &Path, _opts: &Options, report: &mut FileReport) {
    let Some(mut data) = read_capped(path, report) else { return };
    let truncated = report.incomplete;

    if data.starts_with(b"\xEF\xBB\xBF") {
        data.drain(..3);
    } else if data.starts_with(b"\xFF\xFE") || data.starts_with(b"\xFE\xFF") {
        let enc = if data[0] == 0xFF { "utf-16le" } else { "utf-16be" };
        report.push(Category::Defect, Severity::High, "NOT_UTF8", "", &format!("encoding={enc}"));
        return;
    }

    let bytes = if truncated { trim_truncated_tail(&data) } else { &data[..] };
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => {
            let (_, _, had_errors) = encoding_rs::EUC_KR.decode(bytes);
            let enc = if had_errors { "unknown" } else { "cp949" };
            report.push(Category::Defect, Severity::High, "NOT_UTF8", "", &format!("encoding={enc}"));
            report.push(Category::Defect, Severity::High, "INVALID_UTF8", "", &format!("offset={}", e.valid_up_to()));
            return;
        }
    };

    check_text(text, report);
}

fn check_text(text: &str, report: &mut FileReport) {
    if text.trim().is_empty() {
        // 0바이트 파일은 공통 검사(EMPTY_FILE)가 맡는다. 여기는 BOM만 있거나 공백뿐인 파일이다.
        report.push(Category::Defect, Severity::Medium, "EMPTY_TEXT", "", "whitespace only");
        return;
    }

    let (mut chars, mut controls, mut nul) = (0usize, 0usize, false);
    let mut replacement = Tally::default();
    let mut bidi = Tally::default();
    let mut zero_width = Tally::default();
    for (i, c) in text.chars().enumerate() {
        chars += 1;
        match c {
            '\0' => nul = true,
            '\u{FFFD}' => replacement.hit(|| format!("char {i}")),
            '\u{FEFF}' if i != 0 => zero_width.hit(|| format!("char {i}")),
            _ if is_bidi(c) => bidi.hit(|| format!("char {i}")),
            _ if is_zero_width(c) => zero_width.hit(|| format!("char {i}")),
            _ => {}
        }
        if is_binary_control(c) {
            controls += 1;
        }
    }

    if let Some(location) = replacement.first.clone() {
        report.push(Category::Defect, Severity::Medium, "REPLACEMENT_CHAR", &location, &format!("count={}", replacement.count));
    }
    if nul || controls * 100 > chars {
        report.push(Category::Defect, Severity::High, "BINARY_CONTENT", "", &format!("nul={nul} controls={controls}"));
    }
    bidi.report(report, Category::Security, "BIDI_CONTROL");
    zero_width.report(report, Category::Security, "ZERO_WIDTH");

    let mut html_count = 0usize;
    let mut html_line = 0usize;
    let mut long_line = None;
    for (n, line) in text.split('\n').enumerate() {
        let hits = HTML_RE.find_iter(line).count();
        if hits > 0 {
            if html_count == 0 {
                html_line = n + 1;
            }
            html_count += hits;
        }
        // 바이트 수가 한도 이하면 글자 수도 한도 이하다.
        if long_line.is_none() && line.len() > MAX_LINE_CHARS && line.chars().count() > MAX_LINE_CHARS {
            long_line = Some(n + 1);
        }
    }
    if html_count > 0 {
        report.push(Category::Security, Severity::Medium, "HTML_IN_TEXT", &format!("line {html_line}"), &format!("count={html_count}"));
    }

    // 크기 상한은 두지 않는다: doc_reader::TEXT_SIZE는 요약 길이이며 렌더링 한도가 아니다

    let cr_total = text.matches('\r').count();
    let lf_total = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    let (bare_lf, bare_cr) = (lf_total - crlf, cr_total - crlf);
    if [crlf, bare_lf, bare_cr].iter().filter(|&&n| n > 0).count() >= 2 {
        report.push(Category::Render, Severity::Low, "MIXED_NEWLINES", "", &format!("crlf={crlf} lf={bare_lf} cr={bare_cr}"));
    }

    if let Some(n) = long_line {
        report.push(Category::Render, Severity::Low, "LONG_LINE", &format!("line {n}"), &format!("limit={MAX_LINE_CHARS}"));
    }
}
