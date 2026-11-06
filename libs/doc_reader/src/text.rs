//! TXT 읽기.
//!
//! 원본은 chardet으로 후보 순서를 정한다. chardet은 옮기지 않았고, 같은 목적의 간단한 판별
//! (`text_decoder::encoding_guesser::detect_hint`)으로 대신한다. 엄격 UTF-8로 읽히는 파일(코퍼스 표본 거의 전부)에서는 같은 길을 탄다.
//! 인코딩 이름과 코덱은 `text_decoder`, UTF-8 재인코딩(교정)은 `doc_repairer::encoding_repairer`에 있다.

use std::path::Path;
use std::{fs, io};

use text_decoder::encoding_guesser::{DETECT_PREFIX, TEXT_ENCODINGS, TextDetector, detect_hint};
use text_decoder::text_transcoder::{Candidate, Enc, decode};

use crate::TEXT_SIZE;
use crate::text_normalizer::{collapse_spaces, head_chars, is_all_space, keep_ascii_hangul, universal_newlines};

/// 디코딩한 글에 글자가 있는지 확인하는 인코딩. 엄격 디코딩이 우연히 성공해도 글자가 없으면 다음 후보로 넘어간다.
fn is_checked(enc: Enc) -> bool {
    matches!(enc, Enc::Cp949 | Enc::EucKr | Enc::Utf16 | Enc::Utf16Le | Enc::Utf16Be)
}

pub(crate) fn decode_markup_bytes(raw: &[u8]) -> Option<String> {
    decode_text_with(Path::new(""), raw, &TextHooks::default()).ok()
}

fn has_text_char(s: &str) -> bool {
    s.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphanumeric())
}

/// 읽기 결과: 요약, 줄 수, 쪽 수(TXT는 0), 전체 본문.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextRead {
    pub summary: String,
    pub line_count: usize,
    pub page_count: usize,
    pub raw_content: String,
}

pub type TextReader<'a> = dyn Fn(&Path) -> io::Result<Vec<u8>> + 'a;

pub type ReplaceReader<'a> = dyn Fn(&Path) -> io::Result<String> + 'a;

/// TXT 판별과 읽기 I/O의 주입 경계.
#[derive(Clone, Default)]
pub struct TextHooks<'a> {
    pub detect: Option<&'a TextDetector<'a>>,
    pub read_bytes: Option<&'a TextReader<'a>>,
    pub encodings: Option<Vec<Candidate<'a>>>,
    pub read_replace: Option<&'a ReplaceReader<'a>>,
}

/// 인코딩 후보를 순서대로 시도해 본문을 읽는다. 읽을 수 없으면 `Err`(원본은 빈 본문과 오류 로그).
fn decode_text_with(path: &Path, raw: &[u8], hooks: &TextHooks<'_>) -> Result<String, String> {
    let mut order: Vec<Enc> =
        hooks.encodings.as_ref().map(|cs| cs.iter().filter_map(|c| if let Candidate::Builtin(e) = c { Some(*e) } else { None }).collect()).unwrap_or_else(|| TEXT_ENCODINGS.to_vec());
    let skip_detect = std::str::from_utf8(raw).is_ok() && !raw.starts_with(b"\xef\xbb\xbf");
    if !skip_detect
        && hooks.encodings.is_none()
        && let Some(hint) = hooks.detect.map_or_else(|| detect_hint(&raw[..raw.len().min(DETECT_PREFIX)]), |f| f(&raw[..raw.len().min(DETECT_PREFIX)]))
    {
        order.retain(|e| *e != hint);
        order.insert(0, hint);
    }

    let mut last_error = None;
    for enc in &order {
        match decode(*enc, raw, false) {
            Some(content) => {
                let content = universal_newlines(&content);
                if is_checked(*enc) && !content.is_empty() && !has_text_char(&content) && !is_all_space(&content) {
                    continue;
                }
                return Ok(content);
            }
            None => last_error = Some(format!("'{}' codec can't decode bytes", enc.name())),
        }
    }
    if let Some(candidates) = &hooks.encodings {
        for candidate in candidates {
            if let Candidate::Custom { decode, .. } = candidate {
                let content = universal_newlines(&decode(raw));
                if !content.is_empty() {
                    return Ok(content);
                }
            }
        }
    }
    if let Some(reader) = hooks.read_replace
        && let Ok(content) = reader(path)
        && !content.is_empty()
    {
        return Ok(universal_newlines(&content));
    }
    for enc in &order {
        if let Some(content) = decode(*enc, raw, true) {
            let content = universal_newlines(&content);
            if is_checked(*enc) && !content.is_empty() && !has_text_char(&content) && !is_all_space(&content) {
                continue;
            }
            if !content.is_empty() {
                return Ok(content);
            }
        }
    }
    Err(last_error.unwrap_or_default())
}

/// 요약(앞 `TEXT_SIZE`자를 영문·숫자·공백·한글만 남겨 정리)과 줄 수를 만든다.
pub fn summarize(raw_content: &str) -> (String, usize) {
    if raw_content.is_empty() {
        return (String::new(), 0);
    }
    let line_count = raw_content.matches('\n').count() + 1;
    let head = head_chars(raw_content, TEXT_SIZE).replace('\u{feff}', "");
    (collapse_spaces(&keep_ascii_hangul(&head)), line_count)
}

/// TXT 파일을 읽는다. 읽지 못하면 오류 메시지를 돌려준다(원본은 빈 결과와 오류를 함께 돌려줬다).
pub fn read_from_text(path: &Path) -> Result<TextRead, String> {
    read_from_text_with(path, &TextHooks::default())
}

pub fn read_from_text_with(path: &Path, hooks: &TextHooks<'_>) -> Result<TextRead, String> {
    let error = |e: &dyn std::fmt::Display| format!("can't read unicode text from file '{}', {e}", path.display());
    let raw = hooks.read_bytes.map_or_else(|| fs::read(path), |read| read(path)).map_err(|e| error(&e))?;
    let raw_content = decode_text_with(path, &raw, hooks).map_err(|e| error(&e))?;
    let (summary, line_count) = summarize(&raw_content);
    Ok(TextRead { summary, line_count, page_count: 0, raw_content })
}
