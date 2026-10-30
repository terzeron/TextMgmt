//! 터미널 색. 색 코드는 `sanitize_display`를 거친 문자열 바깥에만 붙인다(공격자가 정한 문자열에 든 ESC가 색 코드가 되지 않게).

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
