//! Python과 같은 모양으로 값을 쓴다. Python 원본의 출력과 맞춰야 하는 도구들이 함께 쓴다.
//!
//! - [`py_repr`]: JSON 값을 `repr(dict)` 모양으로 (`print("item:", item)`)
//! - [`py_round`]: `round(x, ndigits)`와 같은 결과(이진 값을 십진 전개해 짝수 쪽으로)
//! - [`py_float_repr`]: float를 `repr(float)` 모양으로
//! - [`py_truthy`]: JSON 값의 `bool(v)`

use serde_json::Value;

mod alnum_table;
mod decimal_table;
mod assigned_table;

mod casefold_table;
use assigned_table::py_is_assigned;

/// Python `repr(str)`: 작은따옴표를 기본으로 쓰되 `'`만 있으면 큰따옴표로 감싼다.
fn py_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control()
                || matches!(c, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{e000}'..='\u{f8ff}' | '\u{e0000}'..='\u{e007f}' | '\u{f0000}'..='\u{10ffff}') =>
            {
                let n = c as u32;
                if n <= 0xff {
                    out.push_str(&format!("\\x{n:02x}"))
                } else if n <= 0xffff {
                    out.push_str(&format!("\\u{n:04x}"))
                } else {
                    out.push_str(&format!("\\U{n:08x}"))
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python `repr(float)`: 1e-4 미만·1e16 이상은 두 자리 이상 지수(`1e-05`), 정수 값은 `.0`을 붙인다.
pub fn py_float_repr(v: f64) -> String {
    let abs = v.abs();
    if abs != 0.0 && !(1e-4..1e16).contains(&abs) {
        // Python은 지수를 두 자리 이상으로 쓴다(1e-05).
        let s = format!("{v:e}");
        let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!("{mant}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// JSON 값을 Python `repr(dict)`과 같은 모양으로 쓴다(`print("item:", item)`과 같은 출력).
pub fn py_repr(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(b) => if *b { "True" } else { "False" }.into(),
        Value::Number(n) => n.as_f64().filter(|_| n.is_f64()).map_or_else(|| n.to_string(), py_float_repr),
        Value::String(s) => py_str_repr(s),
        Value::Array(a) => format!("[{}]", a.iter().map(py_repr).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!("{{{}}}", o.iter().map(|(k, v)| format!("{}: {}", py_str_repr(k), py_repr(v))).collect::<Vec<_>>().join(", ")),
    }
}

/// Python `bool(v)`: `None`, `False`, 0, 빈 문자열·리스트·dict만 거짓이다.
pub fn py_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python `round(x, ndigits)`와 같은 결과를 낸다. 곱셈 뒤 반올림은 `0.35` 같은 값에서 어긋나므로,
/// 이진 값을 정확히 십진 전개해 짝수 쪽으로 맞추는 Rust 포맷터를 거쳐 되읽는다.
pub fn py_round(x: f64, ndigits: usize) -> f64 {
    format!("{x:.ndigits$}").parse().unwrap_or(x)
}

/// Python `str.isalnum()`: 글자(Lu, Ll, Lt, Lm, Lo)나 숫자(Nd, Nl, No)다.
/// Rust `char::is_alphanumeric`은 결합 모음 기호(Indic 등)까지 글자로 보므로 다르다.
pub fn py_isalnum(c: char) -> bool {
    let codepoint = c as u32;
    let index = alnum_table::PY_ALNUM_RANGES.partition_point(|(_, end)| *end < codepoint);
    alnum_table::PY_ALNUM_RANGES.get(index).is_some_and(|(start, end)| codepoint >= *start && codepoint <= *end)
}

/// Python 3.13 `int(str)`가 받는 Unicode 15.1 decimal digit의 값.
pub fn py_decimal_digit(c: char) -> Option<u8> {
    let codepoint = c as u32;
    let index = decimal_table::PY_DECIMAL_RANGES.partition_point(|(_, end)| *end < codepoint);
    let (start, end) = decimal_table::PY_DECIMAL_RANGES.get(index)?;
    if codepoint >= *start && codepoint <= *end {
        Some(((codepoint - start) % 10) as u8)
    } else {
        None
    }
}

// ---- Python 3.13 대소문자 변환: `str.casefold()`(생성 표 기반)와 `str.lower()`. Unicode 15.1 기준이다. ----
pub fn py_casefold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for character in value.chars() {
        match casefold_table::CASE_FOLD.binary_search_by_key(&(character as u32), |(codepoint, _)| *codepoint) {
            Ok(index) => {
                for &codepoint in casefold_table::CASE_FOLD[index].1 {
                    folded.push(char::from_u32(codepoint).expect("generated casefold data contains Unicode scalar values"));
                }
            }
            Err(_) => folded.push(character),
        }
    }
    folded
}

pub fn py_lowercase(value: &str) -> String {
    let mut lowered = String::with_capacity(value.len());
    let mut assigned = String::new();
    for character in value.chars() {
        if py_is_assigned(character) {
            assigned.push(character);
        } else {
            lowered.push_str(&assigned.to_lowercase());
            assigned.clear();
            lowered.push(character);
        }
    }
    lowered.push_str(&assigned.to_lowercase());
    lowered
}
