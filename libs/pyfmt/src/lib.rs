//! Python과 같은 모양으로 값을 쓴다. Python 원본의 출력과 맞춰야 하는 도구들이 함께 쓴다.
//!
//! - [`py_repr`]: JSON 값을 `repr(dict)` 모양으로 (`print("item:", item)`)
//! - [`py_float`]: float을 `repr(float)` 모양으로 (정수값에도 `.0`)
//! - [`py_round`]: `round(x, ndigits)`와 같은 결과(이진 값을 십진 전개해 짝수 쪽으로)

use serde_json::Value;

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

fn py_float_repr(v: f64) -> String {
    let abs = v.abs();
    if abs != 0.0 && !(1e-4..1e16).contains(&abs) {
        // Python은 지수를 두 자리 이상으로 쓴다(1e-05).
        let s = format!("{v:e}");
        let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!("{mant}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    if v.fract() == 0.0 { format!("{v:.1}") } else { format!("{v}") }
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

/// Python의 float 표기. 정수값에도 `.0`이 붙는다.
pub fn py_float(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 { format!("{v:.1}") } else { format!("{v}") }
}

/// Python `round(x, ndigits)`와 같은 결과를 낸다. 곱셈 뒤 반올림은 `0.35` 같은 값에서 어긋나므로,
/// 이진 값을 정확히 십진 전개해 짝수 쪽으로 맞추는 Rust 포맷터를 거쳐 되읽는다.
pub fn py_round(x: f64, ndigits: usize) -> f64 {
    format!("{x:.ndigits$}").parse().unwrap_or(x)
}
