//! scan_common::color 테스트: ANSI 색 입히기와 색 사용 여부 판단.

use scan_common::color::{ColorMode, Style, enabled, paint};

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
