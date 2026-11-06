//! `text_decoder::encoding_guesser` 단위 테스트.

use text_decoder::encoding_guesser::*;
use text_decoder::text_transcoder::Enc;

const KOREAN: &str = "그는 아무 말도 하지 않았다 그리고 다시 길을 걸었다 바람이 불었다 ";

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    text.encode_utf16().flat_map(|u| if big_endian { u.to_be_bytes() } else { u.to_le_bytes() }).collect()
}

fn cp949(text: &str) -> Vec<u8> {
    let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode(text);
    assert!(!had_errors);
    bytes.into_owned()
}

#[test]
fn detect_hint_uses_bom_and_high_byte_pairing() {
    assert_eq!(detect_hint(b"\xef\xbb\xbfx"), Some(Enc::Utf8Sig));
    assert_eq!(detect_hint(b"\xff\xfex\0"), Some(Enc::Utf16));
    assert_eq!(detect_hint(b"plain ascii"), None);
    let (korean, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    assert_eq!(detect_hint(&korean), Some(Enc::EucKr));
    assert_eq!(detect_hint(b"caf\xe9 au lait"), Some(Enc::Cp1252));
    assert_eq!(high_byte_paired_ratio(b"abc"), None);
    assert_eq!(high_byte_paired_ratio(b"a\xe9b\xe9\xe9"), Some(2.0 / 3.0));
    assert_eq!(high_byte_paired_ratio(b"\xe9"), Some(0.0), "끝에 오는 구간도 센다");
}

#[test]
fn korean_likeness_needs_enough_syllables() {
    assert_eq!(korean_likeness("짧은 글"), None);
    assert_eq!(korean_likeness(""), None);
    let text = KOREAN.repeat(10);
    assert!(text.chars().filter(|c| ('가'..='힣').contains(c)).count() >= MIN_SYLLABLES_FOR_LIKENESS);
    let score = korean_likeness(&text).unwrap();
    assert!(score > 0.0 && score <= 1.0);
}

#[test]
fn korean_likeness_is_low_for_uncommon_syllables() {
    assert_eq!(korean_likeness(&"쀍".repeat(60)), Some(0.0));
}

#[test]
fn decode_best_empty() {
    assert_eq!(decode_best(b""), "");
}

#[test]
fn decode_best_utf16_by_bom() {
    let mut le = vec![0xff, 0xfe];
    le.extend(utf16(KOREAN, false));
    assert!(decode_best(&le).contains("바람이"));
    let mut be = vec![0xfe, 0xff];
    be.extend(utf16(KOREAN, true));
    assert!(decode_best(&be).contains("바람이"));
}

#[test]
fn decode_best_utf16_by_null_density() {
    for big in [false, true] {
        let raw = utf16(&"hello world ".repeat(40), big);
        let nulls = raw.iter().filter(|&&b| b == 0).count();
        assert!(raw.len() >= 200 && nulls * 4 > raw.len());
        assert!(decode_best(&raw).contains("hello world"));
    }
}

#[test]
fn decode_best_plain_utf8() {
    assert_eq!(decode_best(KOREAN.as_bytes()), KOREAN);
}

#[test]
fn decode_best_trims_truncated_utf8_tail() {
    let raw = KOREAN.repeat(5).into_bytes();
    assert!(decode_best(&raw[..raw.len() - 1]).contains("바람이"));
}

#[test]
fn decode_best_cp949() {
    let raw = cp949(&KOREAN.repeat(5));
    assert!(std::str::from_utf8(&raw).is_err());
    assert!(decode_best(&raw).contains("바람이"));
}

#[test]
fn decode_best_cp949_with_truncated_tail() {
    let raw = cp949(&KOREAN.repeat(5));
    assert!(decode_best(&raw[..raw.len() - 1]).contains("바람이"));
}

#[test]
fn decode_best_falls_back_to_most_korean_reading() {
    let encoded = cp949(&KOREAN.repeat(5));
    let mut raw = encoded[..40].to_vec();
    raw.push(0xff);
    raw.extend(&encoded[40..]);
    assert!(std::str::from_utf8(&raw).is_err());
    assert!(encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(&raw).is_none());
    assert!(decode_best(&raw).contains("바람이"));
}

#[test]
fn decode_best_all_garbage_returns_something() {
    assert_eq!(decode_best(&[0xff; 300]), "");
}
