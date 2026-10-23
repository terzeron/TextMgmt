//! 파일 이름으로 표지 검색어를 만드는 도우미. Python `filename_title`의 유니코드 처리를 외부 crate 없이 옮겼다.
//!
//! Python은 NFC로 정규화한 뒤 일반 범주가 P(구두점)나 S(기호)인 글자를 공백으로 바꾼다.
//! 이 crate 의존성에는 유니코드 표가 없어서 다음과 같이 근사한다.
//! - NFC: 한글 자모(L+V(+T))만 음절로 합친다(macOS가 만든 NFD 이름이 대표적이다). 라틴 결합 문자는 합치지 않는다.
//! - P/S: 글자·숫자·공백·제어 문자·결합 부호가 아니면 구두점·기호로 본다.

fn compose_hangul(text: &str) -> String {
    const S_BASE: u32 = 0xAC00;
    const L_BASE: u32 = 0x1100;
    const V_BASE: u32 = 0x1161;
    const T_BASE: u32 = 0x11A7;
    const L_COUNT: u32 = 19;
    const V_COUNT: u32 = 21;
    const T_COUNT: u32 = 28;
    let mut out = String::with_capacity(text.len());
    let mut last: Option<u32> = None;
    let flush = |out: &mut String, last: &mut Option<u32>| {
        if let Some(code) = last.take()
            && let Some(c) = char::from_u32(code)
        {
            out.push(c);
        }
    };
    for c in text.chars() {
        let code = c as u32;
        if let Some(prev) = last {
            // L + V
            if (L_BASE..L_BASE + L_COUNT).contains(&prev) && (V_BASE..V_BASE + V_COUNT).contains(&code) {
                last = Some(S_BASE + ((prev - L_BASE) * V_COUNT + (code - V_BASE)) * T_COUNT);
                continue;
            }
            // LV + T
            if (S_BASE..S_BASE + L_COUNT * V_COUNT * T_COUNT).contains(&prev) && (prev - S_BASE).is_multiple_of(T_COUNT) && (T_BASE + 1..T_BASE + T_COUNT).contains(&code) {
                last = Some(prev + (code - T_BASE));
                continue;
            }
        }
        flush(&mut out, &mut last);
        last = Some(code);
    }
    flush(&mut out, &mut last);
    out
}

fn is_combining_mark(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x064B..=0x065F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0x302A..=0x302F | 0x3099..=0x309A | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F)
}

fn is_punctuation_or_symbol(c: char) -> bool {
    !(c.is_alphanumeric() || c.is_whitespace() || c.is_control() || is_combining_mark(c))
}

/// 파일 이름(확장자 제외)을 검색어로 쓴다. 구두점과 기호는 공백으로, 연속 공백은 하나로 줄인다. 비면 "제목 없음"이다.
pub fn clean_title(stem: &str) -> String {
    let replaced: String = compose_hangul(stem).chars().map(|c| if is_punctuation_or_symbol(c) { ' ' } else { c }).collect();
    let title = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        "제목 없음".to_string()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_match_the_python_cases() {
        assert_eq!(clean_title("[랜들 개릿] 다아시 경의 모험"), "랜들 개릿 다아시 경의 모험");
        assert_eq!(clean_title("[작가] 구운몽—꿈(1) & 이야기!"), "작가 구운몽 꿈 1 이야기");
        assert_eq!(clean_title("제목.v2"), "제목 v2");
        assert_eq!(clean_title("  책\t 이름   "), "책 이름");
        assert_eq!(clean_title("【作者】中文_タイトル〈第2巻〉"), "作者 中文 タイトル 第2巻");
        assert_eq!(clean_title("★[!]★"), "제목 없음");
    }

    #[test]
    fn nfd_hangul_is_composed() {
        let nfd = "\u{1112}\u{1161}\u{11AB}\u{1100}\u{1173}\u{11AF}";
        assert_eq!(clean_title(nfd), "한글");
    }
}
