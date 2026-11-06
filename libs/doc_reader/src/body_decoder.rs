//! 본문 바이트를 글로 푼다. 인코딩 후보를 차례로 시도하고(엄격 디코딩 → 사용자 정의 후보 → 대체 읽기 → 대체 문자 디코딩),
//! 줄바꿈을 통일한다. TXT 읽기(`text`)와 EPUB 장의 UTF-8이 아닌 마크업(`epub`)이 함께 쓴다.
//!
//! 후보 순서는 `text_decoder::encoding_guesser::detect_hint`의 판별을 앞에 둔다. 엄격 UTF-8로 읽히는 바이트는 판별을 건너뛴다.

use std::io;
use std::path::Path;

use text_decoder::encoding_guesser::{DETECT_PREFIX, TEXT_ENCODINGS, TextDetector, detect_hint};
use text_decoder::text_transcoder::{Candidate, Enc, decode};

use crate::text_normalizer::{is_all_space, universal_newlines};

/// 디코딩 후보가 모두 실패했을 때 파일을 다른 방법으로 읽는 대역. 테스트가 꽂는다.
pub type ReplaceReader<'a> = dyn Fn(&Path) -> io::Result<String> + 'a;

/// 디코딩한 글에 글자가 있는지 확인하는 인코딩. 엄격 디코딩이 우연히 성공해도 글자가 없으면 다음 후보로 넘어간다.
fn is_checked(enc: Enc) -> bool {
    matches!(enc, Enc::Cp949 | Enc::EucKr | Enc::Utf16 | Enc::Utf16Le | Enc::Utf16Be)
}

fn has_text_char(s: &str) -> bool {
    s.chars().any(|c| ('\u{ac00}'..='\u{d7a3}').contains(&c) || c.is_ascii_alphanumeric())
}

/// 인코딩 후보를 순서대로 시도해 본문을 읽는다. 읽을 수 없으면 `Err`(원본은 빈 본문과 오류 로그).
pub(crate) fn decode_body(path: &Path, raw: &[u8], detect: Option<&TextDetector<'_>>, encodings: Option<&[Candidate<'_>]>, read_replace: Option<&ReplaceReader<'_>>) -> Result<String, String> {
    let mut order: Vec<Enc> = encodings.map(|cs| cs.iter().filter_map(|c| if let Candidate::Builtin(e) = c { Some(*e) } else { None }).collect()).unwrap_or_else(|| TEXT_ENCODINGS.to_vec());
    let skip_detect = std::str::from_utf8(raw).is_ok() && !raw.starts_with(b"\xef\xbb\xbf");
    if !skip_detect
        && encodings.is_none()
        && let Some(hint) = detect.map_or_else(|| detect_hint(&raw[..raw.len().min(DETECT_PREFIX)]), |f| f(&raw[..raw.len().min(DETECT_PREFIX)]))
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
    if let Some(candidates) = encodings {
        for candidate in candidates {
            if let Candidate::Custom { decode, .. } = candidate {
                let content = universal_newlines(&decode(raw));
                if !content.is_empty() {
                    return Ok(content);
                }
            }
        }
    }
    if let Some(reader) = read_replace
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

/// 인코딩 선언 없이 들어온 마크업 바이트를 글로 푼다. 읽을 수 없으면 `None`이다.
pub(crate) fn decode_markup_bytes(raw: &[u8]) -> Option<String> {
    decode_body(Path::new(""), raw, None, None, None).ok()
}
