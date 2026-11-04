//! HNC 2바이트 글자 코드를 유니코드로 바꾼다.

use crate::tables::{FILL, HNC_L1, HNC_L2, HNC_T1, HNC_T2, HNC_V1, HNC_V2, KSC5601_TO_UNI, L_MAP, NONE, T_MAP, V_MAP, hnc2uni};

/// Python은 현대 음절 조합 계산(`chr(0xAC00 + ...)`)에서 lone surrogate 문자를 만든다. 이것은 run을 끊지 않은 채
/// 마지막에 일괄 제거된다. Rust의 `String`은 surrogate를 담을 수 없어서 같은 자리를 이 표시 문자로 채우고
/// `extract_text_from_hwp3`가 마지막에 지운다. 매핑 표의 어떤 값과도 겹치지 않는 U+10FFFF를 쓴다.
pub const LONE_SURROGATE: char = '\u{10FFFF}';

/// surrogate 범위(U+D800~U+DFFF)와 유니코드 밖의 값은 빈 문자열로 바꾼다.
pub fn safe_chr(code: u32) -> String {
    char::from_u32(code).map(String::from).unwrap_or_default()
}

fn mapped(uni: Option<u32>) -> String {
    // 0은 "대응 없음"이다(Python의 `if uni`).
    uni.filter(|u| *u != 0).map(safe_chr).unwrap_or_default()
}

fn is_blank(v: u16) -> bool {
    v == FILL || v == NONE
}

/// HNC 2바이트 코드를 유니코드 문자열로 바꾼다.
pub fn hnc_to_unicode(c: u16) -> String {
    match c {
        0 => String::new(),
        0x0020..=0x007E => char::from(c as u8).to_string(),
        // 특수문자·확장
        0x007F..=0x3FFF => mapped(hnc2uni(c)),
        // 1수준 한자 (4888자)
        0x4000..=0x5317 => mapped(Some(u32::from(KSC5601_TO_UNI[usize::from(c - 0x4000)]))),
        // 2수준 한자
        0x5318..=0x7FFF => mapped(hnc2uni(c)),
        0x8000..=0xFFFF => hangul(c),
        _ => String::new(),
    }
}

fn hangul(c: u16) -> String {
    let (cho, jung, jong) = (usize::from((c & 0x7C00) >> 10), usize::from((c & 0x03E0) >> 5), usize::from(c & 0x001F));
    let (l_idx, v_idx, t_idx) = (L_MAP[cho], V_MAP[jung], T_MAP[jong]);
    // 현대 한글 완성형 음절
    if l_idx != NONE && v_idx != NONE && t_idx != NONE {
        let code = 0xAC00 + (u32::from(l_idx) * 21 + u32::from(v_idx)) * 28 + u32::from(t_idx);
        // 음절 범위를 넘는 값은 Python에서 surrogate 문자가 된다(`safe_chr`를 거치지 않는다).
        return char::from_u32(code).unwrap_or(LONE_SURROGATE).to_string();
    }
    let (l1, v1, t1) = (HNC_L1[cho], HNC_V1[jung], HNC_T1[jong]);
    let no_vowel = is_blank(v1);
    match (l1 != FILL, !no_vowel, t1 != FILL) {
        (true, false, false) => safe_chr(u32::from(l1)),                                                                                  // 초성만
        (false, true, false) => safe_chr(u32::from(v1)),                                                                                  // 중성만
        (false, false, true) => safe_chr(u32::from(t1)),                                                                                  // 종성만
        (true, true, false) => safe_chr(u32::from(HNC_L2[cho])) + &safe_chr(u32::from(HNC_V2[jung])),                                     // 옛한글 초성+중성
        (true, true, true) => safe_chr(u32::from(HNC_L2[cho])) + &safe_chr(u32::from(HNC_V2[jung])) + &safe_chr(u32::from(HNC_T2[jong])), // 옛한글 3벌
        // 완성형 옛한글 fallback
        _ if jung == 0 => mapped(hnc2uni(c)),
        _ => String::new(),
    }
}
