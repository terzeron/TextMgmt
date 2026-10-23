//! ES 제목·내용 검색 점검 도구. 원본: `utils/search.py`.
//!
//! 제목(`<title>`, `<ext>`, `<size>`)으로 찾고, 내용(`<content>`)이 있으면 요약이 비슷한 문서도 찾는다.

use std::io::Write;

use es_client::{BookSearch, EsError, Hit};
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
            c if c.is_control() || matches!(c, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{feff}') => {
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
    if !v.is_finite() {
        return if v.is_nan() {
            "nan".into()
        } else if v > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
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

fn print_usage(program: &str, out: &mut dyn Write) {
    let _ = writeln!(out, "Usage:\t{program}\t<title> [ <content> [ <ext> [ <size> ] ] ]\n");
    let _ = writeln!(out, "\tIf you specify <title> argument, this program will do searching with <title>, <ext>, and <size>.");
    let _ = writeln!(out, "\tIf you specify <content> argument, this program will do searching with <content> only.");
}

fn print_hits(hits: &[Hit], separator: bool, out: &mut dyn Write) -> std::io::Result<()> {
    for hit in hits {
        if separator {
            writeln!(out, "----------------------------------------")?;
        }
        writeln!(out, "item: {}", py_repr(&hit.source))?;
        // 목록 응답에는 summary 필드가 없을 수 있다. 원본은 여기서 KeyError로 죽는다.
        let summary = hit.source.get("summary").and_then(Value::as_str).unwrap_or("");
        let head: String = summary.chars().take(200).collect();
        writeln!(out, "{}", head.replace('\n', " "))?;
    }
    Ok(())
}

/// 진입점. `connect`는 ES 접속을 만든다. 제목 인자가 없으면 접속하지 않고 사용법만 보인다.
/// 종료 코드: 정상 0, 환경변수 누락 255(원본 `sys.exit(-1)`), 그 밖의 실패 1.
pub fn run<'a>(program: &str, args: &[String], out: &mut dyn Write, connect: &dyn Fn() -> Result<&'a dyn BookSearch, EsError>) -> i32 {
    let Some(title) = args.first() else {
        print_usage(program, out);
        return 0;
    };
    let content = args.get(1).map_or("", String::as_str);
    let ext = args.get(2).map_or("", String::as_str);
    let size: i64 = match args.get(3).map(|s| s.parse()) {
        None => 0,
        Some(Ok(n)) => n,
        Some(Err(_)) => {
            eprintln!("ERROR <size>는 정수여야 한다: {}", args[3]);
            return 1;
        }
    };

    let es = match connect() {
        Ok(es) => es,
        Err(e @ EsError::MissingEnv(_)) => {
            eprintln!("ERROR {e}");
            return 255;
        }
        Err(e) => {
            eprintln!("ERROR {e}");
            return 1;
        }
    };

    if !title.is_empty() {
        match es.search_by_title(title, ext, size) {
            Ok(hits) => drop(print_hits(&hits, true, out)),
            Err(e) => {
                eprintln!("ERROR {e}");
                return 1;
            }
        }
    }
    if !content.is_empty() {
        match es.search_similar_docs(content) {
            Ok(hits) => drop(print_hits(&hits, false, out)),
            Err(e) => {
                eprintln!("ERROR {e}");
                return 1;
            }
        }
    }
    0
}
