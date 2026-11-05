//! 사람이 읽는 출력: 숫자 형식, 터미널 색. 공격자가 정한 문자열 정리는 `doc_checker::text_sanitizer`에 있다.

use doc_checker::text_sanitizer::sanitize_display;

/// 정수를 세 자리마다 쉼표로 끊는다(`1,234,567`).
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---- 터미널 색. 색 코드는 `sanitize_display`를 거친 문자열 바깥에만 붙인다(공격자가 정한 문자열에 든 ESC가 색 코드가 되지 않게). ----

/// 입힐 색. 보라는 ANSI magenta(35)다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Red,
    Purple,
    Cyan,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Style::Red => "\u{1b}[31m",
            Style::Purple => "\u{1b}[35m",
            Style::Cyan => "\u{1b}[36m",
        }
    }
}

const RESET: &str = "\u{1b}[0m";

/// `on`이면 `text`를 `style` 색으로 감싼다. 아니면 그대로 돌려준다.
pub fn paint(text: &str, style: Style, on: bool) -> String {
    if on { format!("{}{text}{RESET}", style.code()) } else { text.to_string() }
}

/// `--color` 값.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    /// 소문자 이름(`auto`·`always`·`never`)으로 읽는다. 그 밖의 문자열은 `None`이다.
    pub fn parse(name: &str) -> Option<ColorMode> {
        match name {
            "auto" => Some(ColorMode::Auto),
            "always" => Some(ColorMode::Always),
            "never" => Some(ColorMode::Never),
            _ => None,
        }
    }
}

/// 색을 쓸지 정한다. `Auto`는 터미널이고 `NO_COLOR`가 없을 때만 쓴다.
pub fn enabled(mode: ColorMode, is_tty: bool, no_color: bool) -> bool {
    match mode {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => is_tty && !no_color,
    }
}

/// 터미널에 찍을 문자열에서 제어문자(ESC 등)와 방향·서식 문자를 지운다. 경로·멤버 이름·오류 메시지는 파일이 정한 값이다.
pub(crate) fn show(text: &str) -> String {
    sanitize_display(text, usize::MAX, None)
}
