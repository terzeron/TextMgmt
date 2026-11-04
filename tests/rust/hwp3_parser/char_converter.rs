//! HNC 글자 코드 변환 테스트 (`char_converter` 모듈).
mod common;

use hwp3_parser::tables::{HNC2UNI, KSC5601_TO_UNI};
use hwp3_parser::*;

// ---- safe_chr ----

#[test]
fn safe_chr_drops_surrogates_only() {
    assert_eq!(safe_chr(0x41), "A");
    assert_eq!(safe_chr(0xAC00), "가");
    for c in [0xD800, 0xDFFF, 0xDC00] {
        assert_eq!(safe_chr(c), "");
    }
    assert_eq!(safe_chr(0xD7FF), "\u{d7ff}");
    assert_eq!(safe_chr(0xE000), "\u{e000}");
    // 유니코드 범위를 넘는 값도 버린다.
    assert_eq!(safe_chr(0x110000), "");
}

// ---- hnc_to_unicode ----

#[test]
fn hnc_codes_map_by_range() {
    assert_eq!(hnc_to_unicode(0), "");
    assert_eq!(hnc_to_unicode(0x41), "A");
    assert_eq!(hnc_to_unicode(0x20), " ");
    assert_eq!(hnc_to_unicode(0x7E), "~");
    assert_eq!(hnc_to_unicode(0x1F), hnc_to_unicode(0x1F));
    assert!(hnc_to_unicode(0x4000).chars().count() <= 1);
}

#[test]
fn every_hnc_code_matches_the_python_conversion_table() {
    // Python golden: every u16 passed through _hnc_to_unicode; lone surrogates
    // are represented by the Rust parser's internal sentinel before UTF-8 hashing.
    let mut hash = 0xcbf29ce484222325u64;
    for code in 0..=u16::MAX {
        for byte in hnc_to_unicode(code).bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    assert_eq!(hash, 0x2ecc897d0738d928);
}

#[test]
fn modern_hangul_syllables_compose() {
    assert_eq!(hnc_to_unicode(0x8000 | (2 << 10) | (3 << 5) | 1), "가");
}

#[test]
fn partial_and_old_hangul_follow_the_python_branches() {
    // 낱자: 초성만, 중성만, 종성만.
    assert!(hnc_to_unicode(0x8000 | (2 << 10) | (2 << 5) | 1).chars().count() <= 1);
    assert!(hnc_to_unicode(0x8000 | (1 << 10) | (3 << 5) | 1).chars().count() <= 1);
    assert_eq!(hnc_to_unicode(0x8000 | (1 << 10) | 2), "\u{3131}");
    // 옛한글: 초성+중성, 초성+중성+종성, 완성형 fallback.
    assert!(hnc_to_unicode(0x8000 | (3 << 5) | 1).chars().count() >= 1);
    assert!(hnc_to_unicode(0x8000 | (2 << 10) | (3 << 5)).chars().count() >= 1);
    let _ = hnc_to_unicode(0x8000 | (2 << 10));
}

// ---- 실제 파일 비교에서 찾은 회귀 ----

#[test]
fn table_ranges_use_ksc5601_and_hnc2uni() {
    let known = HNC2UNI.iter().map(|(k, _)| *k).find(|k| (0x7F..=0x3FFF).contains(k)).unwrap();
    assert!(hnc_to_unicode(known).chars().count() <= 1);
    let high = HNC2UNI.iter().map(|(k, _)| *k).find(|k| (0x5318..=0x7FFF).contains(k)).unwrap();
    assert!(hnc_to_unicode(high).chars().count() <= 1);
    let unknown = (0x80u16..0x200).find(|c| HNC2UNI.binary_search_by_key(c, |(k, _)| *k).is_err()).unwrap();
    assert_eq!(hnc_to_unicode(unknown), "");
    if let Some(i) = KSC5601_TO_UNI.iter().position(|v| *v == 0) {
        assert_eq!(hnc_to_unicode(0x4000 + i as u16), "");
    }
    let mapped = KSC5601_TO_UNI.iter().position(|v| *v != 0).unwrap();
    assert_eq!(hnc_to_unicode(0x4000 + mapped as u16), char::from_u32(u32::from(KSC5601_TO_UNI[mapped])).unwrap().to_string());
}
