//! 인코딩 추정 휴리스틱 두 벌과 한국어 판정.
//!
//! - [`detect_hint`]: TXT 읽기와 재인코딩이 후보 순서를 정할 때 쓴다(chardet 대신).
//! - [`decode_best`]: 본문 표본 추출(`doc_reader::head_tail_reader`)이 쓴다.
//! - [`korean_likeness`]: 글이 정상 한국어로 읽히는 정도. `decode_best`와 인코딩 손상 판정이 쓴다.
//!
//! 두 휴리스틱은 서로 다른 오판 사례로 조정했다. 합치면 판정이 바뀐다.
//!
//! `decode_best`의 인코딩 판별은 실제 코퍼스의 오판 사례로 네 번 고쳐 잡은 휴리스틱이다. 다시 쓰지 말 것:
//!   - "한글이 하나라도 나오면 그 인코딩 채택"은 정상 CP949 소설 1,856건을 오판했다
//!   - 한글 비율 하한이 없으면 영문 OCR 문서를 한글 잡음 60자로 판정한다
//!   - UTF-16을 빼면 97건을 못 읽는다
//!   - EPUB은 첫 문서가 목차인 경우가 있어 건너뛰어야 한다

use crate::text_transcoder::{Enc, valid_korean_bytes};
use crate::{decode_cp949_strict, decode_korean_ignore, decode_utf8_ignore, is_hangul_syllable};

/// txt 디코딩 시도 순서.
pub const TEXT_ENCODINGS: [Enc; 7] = [Enc::Utf8, Enc::Cp949, Enc::EucKr, Enc::Utf16, Enc::Utf16Le, Enc::Utf16Be, Enc::Utf8Sig];

/// 상위바이트가 고립되어 나타나면 서양어 단일바이트로 본다. 인접 비율 기준.
pub const HIGH_BYTE_PAIRED_MIN: f64 = 0.5;

/// 판별에 쓰는 앞부분 바이트 수.
pub const DETECT_PREFIX: usize = 8 * 1024;

/// [`detect_hint`]를 대신하는 판별기(테스트·호출자 주입용).
pub type TextDetector<'a> = dyn Fn(&[u8]) -> Option<Enc> + 'a;

/// 상위바이트(0x80~0xFF) 연속 구간으로 (고립 아닌 바이트 수 / 전체 상위바이트 수)를 센다. 상위바이트가 없으면 `None`.
pub fn high_byte_paired_ratio(raw: &[u8]) -> Option<f64> {
    let (mut total, mut paired, mut run) = (0usize, 0usize, 0usize);
    for &b in raw.iter().chain(std::iter::once(&0u8)) {
        if b >= 0x80 {
            run += 1;
        } else if run > 0 {
            total += run;
            if run >= 2 {
                paired += run;
            }
            run = 0;
        }
    }
    (total > 0).then(|| paired as f64 / total as f64)
}

/// 상위바이트가 모두 KS X 1001 영역(두 바이트 모두 0xA1~0xFE)의 쌍인가. 잘린 끝 바이트는 무시한다.
fn is_ksx1001_only(raw: &[u8]) -> bool {
    let mut i = 0;
    while i < raw.len() {
        if raw[i] < 0x80 {
            i += 1;
        } else if let Some(&d) = raw.get(i + 1) {
            if !(0xa1..=0xfe).contains(&raw[i]) || !(0xa1..=0xfe).contains(&d) {
                return false;
            }
            i += 2;
        } else {
            break;
        }
    }
    true
}

/// chardet 대신 쓰는 후보 힌트. BOM, 상위바이트 인접 비율로 가장 그럴듯한 인코딩 하나를 고른다.
pub fn detect_hint(prefix: &[u8]) -> Option<Enc> {
    if prefix.starts_with(b"\xef\xbb\xbf") {
        return Some(Enc::Utf8Sig);
    }
    if prefix.starts_with(&[0xff, 0xfe]) || prefix.starts_with(&[0xfe, 0xff]) {
        return Some(Enc::Utf16);
    }
    match high_byte_paired_ratio(prefix) {
        None => None,
        // chardet은 KS X 1001 안에 머무르면 EUC-KR, 확장 완성형(UHC)이 섞이면 CP949라고 답한다.
        Some(r) if r >= HIGH_BYTE_PAIRED_MIN => {
            if valid_korean_bytes(prefix, true) && is_ksx1001_only(prefix) {
                Some(Enc::EucKr)
            } else if valid_korean_bytes(prefix, false) {
                Some(Enc::Cp949)
            } else {
                None
            }
        }
        Some(_) => Some(Enc::Cp1252),
    }
}

pub const COMMON_HANGUL_SYLLABLES: &str = "이다는을가지에그고하의아은어도로리나를한었기서들게니있라자사시만으해마보했수대것";

/// 이 수보다 음절이 적으면 비율이 흔들려 판정하지 않는다.
pub const MIN_SYLLABLES_FOR_LIKENESS: usize = 50;

/// 이 글이 정상 한국어로 읽히는 정도. 잴 음절이 모자라면 None.
pub fn korean_likeness(text: &str) -> Option<f64> {
    let (mut total, mut hits) = (0usize, 0usize);
    for c in text.chars().filter(|&c| is_hangul_syllable(c)) {
        total += 1;
        if COMMON_HANGUL_SYLLABLES.contains(c) {
            hits += 1;
        }
    }
    (total >= MIN_SYLLABLES_FOR_LIKENESS).then(|| hits as f64 / total as f64)
}

/// UTF-16을 `errors="ignore"`로 푼다. 짝이 안 맞는 서로게이트와 홀수 바이트 꼬리는 버린다.
fn decode_utf16_ignore(raw: &[u8], big_endian: bool) -> String {
    let units = raw.chunks_exact(2).map(|b| if big_endian { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) });
    char::decode_utf16(units).filter_map(Result::ok).collect()
}

/// 바이트 열을 올바른 인코딩으로 디코딩한다.
///
/// UTF-8은 자기검증 인코딩이다. CP949로 저장된 한국어는 유효한 UTF-8이 되는 일이 거의 없어
/// 엄격 디코딩이 실패한다. 그래서 "엄격 UTF-8 성공 여부"가 가장 믿을 만한 판별이다.
/// "likeness가 가장 높은 인코딩 채택"은 이미 손상된 파일을 더 한국어처럼 보이는 쓰레기로
/// 되살려 손상을 놓치므로 마지막 수단으로만 쓴다.
pub fn decode_best(raw: &[u8]) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // UTF-16은 BOM이나 널 바이트 밀도로 먼저 알아본다.
    if raw.starts_with(&[0xff, 0xfe]) {
        return decode_utf16_ignore(&raw[2..], false);
    }
    if raw.starts_with(&[0xfe, 0xff]) {
        return decode_utf16_ignore(&raw[2..], true);
    }
    let nulls = raw.iter().filter(|&&b| b == 0).count();
    if raw.len() >= 200 && nulls * 4 > raw.len() {
        let even_nulls = raw.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
        let odd_nulls = raw.iter().step_by(2).filter(|&&b| b == 0).count();
        return decode_utf16_ignore(raw, even_nulls <= odd_nulls);
    }

    // 잘린 멀티바이트 문자 때문에 엄격 디코딩이 헛되이 실패하지 않도록 꼬리를 다듬는다.
    for trim in 0..4 {
        if let Some(chunk) = raw.len().checked_sub(trim).map(|n| &raw[..n])
            && let Ok(text) = std::str::from_utf8(chunk)
        {
            return text.to_owned();
        }
    }

    // cp949는 euc-kr의 상위 집합이라 엄격 디코딩은 cp949 하나로 충분하다. 실패하는 입력은 둘 다 실패한다.
    for trim in 0..2 {
        if let Some(chunk) = raw.len().checked_sub(trim).map(|n| &raw[..n])
            && let Some(text) = decode_cp949_strict(chunk)
        {
            return text;
        }
    }

    // 어느 것으로도 깨끗이 안 읽히면 한국어로 가장 잘 읽히는 쪽을 쓴다. 동점이면 앞선 후보.
    let mut best: (String, f64) = (String::new(), -1.0);
    for text in [decode_utf8_ignore(raw), decode_korean_ignore(raw, false), decode_korean_ignore(raw, true)] {
        let score = korean_likeness(&text).unwrap_or(0.001);
        if score > best.1 {
            best = (text, score);
        }
    }
    best.0
}
