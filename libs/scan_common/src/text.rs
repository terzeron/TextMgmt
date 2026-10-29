//! 사람이 읽는 출력에 들어가는 문자열 정리.

/// 공격자가 정한 문자열을 터미널에 안전하게 찍는다: 제어문자(C0·C1)는 `.`로 바꾸고 `max_chars`글자에서 자른다.
/// 잘렸다면 `ellipsis`를 붙인다(없으면 그냥 자른다).
pub fn sanitize(text: &str, max_chars: usize, ellipsis: Option<char>) -> String {
    let mut out = String::new();
    for (n, c) in text.chars().enumerate() {
        if n >= max_chars {
            out.extend(ellipsis);
            break;
        }
        out.push(if c.is_control() { '.' } else { c });
    }
    out
}

/// 글자 방향을 바꾸거나 보이지 않는 서식 문자인가: 방향 지정(U+202A-202E, U+2066-2069), LRM/RLM(U+200E/F), ALM(U+061C),
/// 폭 없는 문자(U+200B-200D, U+2060), BOM(U+FEFF). 터미널에서 파일명 뒤쪽을 뒤집거나 숨겨 다른 이름처럼 보이게 한다.
pub fn is_hidden_format_char(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

/// `sanitize`에 더해 방향·서식 문자도 `.`로 바꾼다. 사람이 읽는 텍스트 출력 전용이다. JSON에는 원문을 쓴다(serde가 이스케이프한다).
pub fn sanitize_display(text: &str, max_chars: usize, ellipsis: Option<char>) -> String {
    sanitize(text, max_chars, ellipsis).chars().map(|c| if is_hidden_format_char(c) { '.' } else { c }).collect()
}
