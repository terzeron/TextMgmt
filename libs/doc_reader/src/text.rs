//! TXT 읽기.
//!
//! 원본은 chardet으로 후보 순서를 정한다. chardet은 옮기지 않았고, 같은 목적의 간단한 판별
//! (`text_decoder::encoding_guesser::detect_hint`)으로 대신한다. 엄격 UTF-8로 읽히는 파일(코퍼스 표본 거의 전부)에서는 같은 길을 탄다.
//! 인코딩 후보 디코딩은 `body_decoder`, 인코딩 이름과 코덱은 `text_decoder`, UTF-8 재인코딩(교정)은 `doc_repairer::encoding_repairer`에 있다.

use std::path::Path;
use std::{fs, io};

use text_decoder::encoding_guesser::TextDetector;
use text_decoder::text_transcoder::Candidate;

use crate::body_decoder::decode_body;

use crate::TEXT_SIZE;
use crate::text_normalizer::{collapse_spaces, head_chars, keep_ascii_hangul};

/// 읽기 결과: 요약, 줄 수, 쪽 수(TXT는 0), 전체 본문.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextRead {
    pub summary: String,
    pub line_count: usize,
    pub page_count: usize,
    pub raw_content: String,
}

pub type TextReader<'a> = dyn Fn(&Path) -> io::Result<Vec<u8>> + 'a;

pub use crate::body_decoder::ReplaceReader;

/// TXT 판별과 읽기 I/O의 주입 경계.
#[derive(Clone, Default)]
pub struct TextHooks<'a> {
    pub detect: Option<&'a TextDetector<'a>>,
    pub read_bytes: Option<&'a TextReader<'a>>,
    pub encodings: Option<Vec<Candidate<'a>>>,
    pub read_replace: Option<&'a ReplaceReader<'a>>,
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
    let raw_content = decode_body(path, &raw, hooks.detect, hooks.encodings.as_deref(), hooks.read_replace).map_err(|e| error(&e))?;
    let (summary, line_count) = summarize(&raw_content);
    Ok(TextRead { summary, line_count, page_count: 0, raw_content })
}
