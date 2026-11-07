//! 본문 요약용 텍스트 정리 도구: 코드포인트 단위 자르기, 공백 접기, 글자 거르기, 줄바꿈 통일.

/// 첫 `n`개 코드포인트.
pub fn head_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// 비어 있지 않고 모두 공백인가.
pub fn is_all_space(s: &str) -> bool {
    !s.is_empty() && s.chars().all(char::is_whitespace)
}

/// 연속 공백을 공백 하나로 접고 앞뒤 공백을 지운다.
pub fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            in_space = true;
        } else {
            if in_space {
                out.push(' ');
                in_space = false;
            }
            out.push(c);
        }
    }
    if in_space {
        out.push(' ');
    }
    out.trim_matches(char::is_whitespace).to_owned()
}

/// 단어 글자: 유니코드 글자·숫자·밑줄.
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// 글자·숫자·밑줄·공백·한글(호환 자모~음절)만 남기고 나머지는 공백으로 바꾼다.
pub fn keep_word_chars(s: &str) -> String {
    s.chars().map(|c| if is_word(c) || c.is_whitespace() || ('\u{3131}'..='\u{d7a3}').contains(&c) { c } else { ' ' }).collect()
}

/// TXT용. 영문·숫자·공백·한글만 남기고 나머지는 공백으로 바꾼다.
pub fn keep_ascii_hangul(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c.is_whitespace() || ('\u{3131}'..='\u{d7a3}').contains(&c) { c } else { ' ' }).collect()
}

/// 줄바꿈을 통일한다: `\r\n`과 `\r`을 `\n`으로.
pub fn universal_newlines(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_owned();
    }
    s.replace("\r\n", "\n").replace('\r', "\n")
}
