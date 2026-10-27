//! Python `str` 동작을 따르는 작은 도구들. 정규식 `\s`, `isspace`, 코드포인트 단위 자르기.

/// Python `str.isspace()`/정규식 `\s`와 같은 공백 판정. `\x1c`~`\x1f`를 포함한다.
pub fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// 첫 `n`개 코드포인트.
pub fn head_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Python `str.isspace()`: 비어 있지 않고 모두 공백.
pub fn is_all_space(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_space)
}

/// `re.sub(r"\s+", " ", s).strip()`
pub fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if is_space(c) {
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
    out.trim_matches(is_space).to_owned()
}

/// Python `\w`(유니코드): 글자·숫자·밑줄.
pub fn is_word(c: char) -> bool {
    pyfmt::py_isalnum(c) || c == '_'
}

/// `re.sub(r"[^\w\sㄱ-힣]", " ", s)`: 글자·숫자·밑줄·공백·한글 호환 자모 이후 한글 범위만 남긴다.
pub fn keep_word_chars(s: &str) -> String {
    s.chars().map(|c| if is_word(c) || is_space(c) || ('\u{3131}'..='\u{d7a3}').contains(&c) { c } else { ' ' }).collect()
}

/// `re.sub(r"[^a-zA-Z0-9\sㄱ-힣]", " ", s)`: TXT용. 영문 숫자 공백 한글만 남긴다.
pub fn keep_ascii_hangul(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || is_space(c) || ('\u{3131}'..='\u{d7a3}').contains(&c) { c } else { ' ' }).collect()
}

/// 텍스트 모드 열기의 줄바꿈 번역: `\r\n`과 `\r`을 `\n`으로.
pub fn universal_newlines(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_owned();
    }
    s.replace("\r\n", "\n").replace('\r', "\n")
}
