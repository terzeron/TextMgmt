//! `doc_checker::text_sanitizer` 테스트.

use doc_checker::text_sanitizer::{is_hidden_format_char, sanitize, sanitize_display};

#[test]
fn sanitize_replaces_c0_and_c1_controls_and_cuts_at_the_limit() {
    assert_eq!(sanitize("a\u{1b}[2Jb\u{85}c\u{7f}", 100, None), "a.[2Jb.c.");
    assert_eq!(sanitize("가나다라마", 3, None), "가나다");
    assert_eq!(sanitize("가나다라마", 3, Some('…')), "가나다…");
    // 딱 맞으면 자르지 않았으므로 말줄임표도 없다.
    assert_eq!(sanitize("가나다", 3, Some('…')), "가나다");
    assert_eq!(sanitize("", 3, Some('…')), "");
}

#[test]
fn sanitize_display_also_replaces_bidi_and_zero_width_characters() {
    for c in [
        '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}',
        '\u{FEFF}',
    ] {
        assert!(is_hidden_format_char(c), "{c:?}");
        assert_eq!(sanitize_display(&format!("a{c}b"), 100, None), "a.b", "{c:?}");
    }
    // 일반 글자와 인접 코드 포인트는 그대로 둔다.
    assert!(!is_hidden_format_char('가') && !is_hidden_format_char('\u{2029}') && !is_hidden_format_char('\u{2070}'));
    assert_eq!(sanitize_display("가\u{1b}나\u{202e}다", 100, None), "가.나.다");
    // `sanitize` 자체는 방향 문자를 건드리지 않는다.
    assert_eq!(sanitize("a\u{202e}b", 100, None), "a\u{202e}b");
}
