//! display 테스트: 출력용 문자열 정리와 터미널 색.

use bookdoctor::output_formatter::{is_hidden_format_char, sanitize, sanitize_display};

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

use bookdoctor::output_formatter::{ColorMode, Style, enabled, paint};

#[test]
fn paint_wraps_with_ansi_when_on() {
    assert_eq!(paint("x", Style::Red, true), "\u{1b}[31mx\u{1b}[0m");
    assert_eq!(paint("x", Style::Purple, true), "\u{1b}[35mx\u{1b}[0m");
    assert_eq!(paint("x", Style::Cyan, true), "\u{1b}[36mx\u{1b}[0m");
}

#[test]
fn paint_is_identity_when_off() {
    for style in [Style::Red, Style::Purple, Style::Cyan] {
        assert_eq!(paint("x\u{1b}[31m", style, false), "x\u{1b}[31m");
    }
}

#[test]
fn enabled_truth_table() {
    // (mode, is_tty, no_color) -> 색 사용
    let table = [
        (ColorMode::Auto, true, false, true),
        (ColorMode::Auto, true, true, false),
        (ColorMode::Auto, false, false, false),
        (ColorMode::Auto, false, true, false),
        (ColorMode::Always, false, true, true),
        (ColorMode::Always, true, false, true),
        (ColorMode::Never, true, false, false),
        (ColorMode::Never, false, false, false),
    ];
    for (mode, tty, no_color, want) in table {
        assert_eq!(enabled(mode, tty, no_color), want, "{mode:?} tty={tty} no_color={no_color}");
    }
}

#[test]
fn color_mode_parses_three_names() {
    assert_eq!(ColorMode::parse("auto"), Some(ColorMode::Auto));
    assert_eq!(ColorMode::parse("always"), Some(ColorMode::Always));
    assert_eq!(ColorMode::parse("never"), Some(ColorMode::Never));
    assert_eq!(ColorMode::parse("Always"), None);
    assert_eq!(ColorMode::parse(""), None);
}
