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
