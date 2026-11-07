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

#[cfg(test)]
mod tests {
    use text_decoder::text_transcoder::encode;

    use super::*;

    fn path() -> &'static Path {
        Path::new("a.txt")
    }

    fn cp949(text: &str) -> Vec<u8> {
        encode(Enc::Cp949, text).unwrap()
    }

    #[test]
    fn strict_utf8_skips_detection_and_unifies_newlines() {
        let never: &TextDetector = &|_| panic!("엄격 UTF-8로 읽히면 판별하지 않는다");
        let text = decode_body(path(), "안녕\r\n하세요\r".as_bytes(), Some(never), None, None).unwrap();
        assert_eq!(text, "안녕\n하세요\n");
    }

    #[test]
    fn a_utf8_bom_runs_detection_and_the_hint_goes_first() {
        let raw = [b"\xef\xbb\xbf".as_slice(), "안녕".as_bytes()].concat();
        assert_eq!(decode_body(path(), &raw, None, None, None).unwrap(), "안녕");
    }

    #[test]
    fn the_detector_hint_is_tried_before_the_default_order() {
        let raw = cp949("안녕하세요 독자 여러분");
        let hint: &TextDetector = &|_| Some(Enc::Cp949);
        assert_eq!(decode_body(path(), &raw, Some(hint), None, None).unwrap(), "안녕하세요 독자 여러분");
    }

    #[test]
    fn explicit_encodings_skip_detection() {
        let never: &TextDetector = &|_| panic!("후보를 지정하면 판별하지 않는다");
        let encodings = [Candidate::Builtin(Enc::Cp949)];
        assert_eq!(decode_body(path(), &cp949("안녕하세요"), Some(never), Some(&encodings[..]), None).unwrap(), "안녕하세요");
    }

    #[test]
    fn a_custom_candidate_runs_after_the_builtin_ones_fail() {
        let encodings = [Candidate::Builtin(Enc::Utf8), Candidate::Custom { name: "x", decode: |_| "사용자\r\n정의".to_string(), encode: |_| None }];
        assert_eq!(decode_body(path(), &[0xff, 0xff], None, Some(&encodings[..]), None).unwrap(), "사용자\n정의");
    }

    #[test]
    fn an_empty_custom_result_falls_through_to_the_replace_reader() {
        let encodings = [Candidate::Builtin(Enc::Utf8), Candidate::Custom { name: "x", decode: |_| String::new(), encode: |_| None }];
        let reader: &ReplaceReader = &|p| {
            assert_eq!(p, Path::new("a.txt"));
            Ok("대체\r\n본문".to_string())
        };
        assert_eq!(decode_body(path(), &[0xff, 0xff], None, Some(&encodings[..]), Some(reader)).unwrap(), "대체\n본문");
    }

    #[test]
    fn a_failing_or_empty_replace_reader_falls_through_to_lossy_decoding() {
        let encodings = [Candidate::Builtin(Enc::Utf8)];
        let failing: &ReplaceReader = &|_| Err(io::Error::other("읽기 실패"));
        let empty: &ReplaceReader = &|_| Ok(String::new());
        for reader in [failing, empty] {
            assert_eq!(decode_body(path(), b"ab\xffcd", None, Some(&encodings[..]), Some(reader)).unwrap(), "ab\u{fffd}cd");
        }
    }

    #[test]
    fn undecodable_bytes_report_the_last_codec_error() {
        let encodings = [Candidate::Builtin(Enc::Cp949)];
        let error = decode_body(path(), &[0xff], None, Some(&encodings[..]), None).unwrap_err();
        assert!(error.contains("cp949"), "{error}");
    }

    #[test]
    fn a_checked_encoding_without_any_text_char_is_skipped() {
        // UTF-16LE로는 "!!"로 풀리지만 글자가 없다. 다음 후보 UTF-8의 해석을 쓴다.
        let encodings = [Candidate::Builtin(Enc::Utf16Le), Candidate::Builtin(Enc::Utf8)];
        assert_eq!(decode_body(path(), &[0x21, 0, 0x21, 0], None, Some(&encodings[..]), None).unwrap(), "!\0!\0");
    }

    #[test]
    fn a_checked_encoding_with_only_spaces_is_accepted() {
        let encodings = [Candidate::Builtin(Enc::Utf16Le)];
        assert_eq!(decode_body(path(), &[0x20, 0, 0x20, 0], None, Some(&encodings[..]), None).unwrap(), "  ");
    }

    #[test]
    fn markup_bytes_decode_by_their_bom() {
        assert_eq!(decode_markup_bytes(&encode(Enc::Utf16, "<p>가</p>").unwrap()).as_deref(), Some("<p>가</p>"));
        assert_eq!(decode_markup_bytes("<p>나</p>".as_bytes()).as_deref(), Some("<p>나</p>"));
        assert_eq!(decode_markup_bytes(b"").as_deref(), Some(""));
    }
}
