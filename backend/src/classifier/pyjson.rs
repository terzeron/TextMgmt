//! Python `json.dumps` 와 바이트 호환인 직렬화. 설정 해시(`config_hash`), JSONL 코퍼스, publisher 캐시가
//! Python 이 쓴 파일과 같은 바이트여야 해서 필요하다. `ensure_ascii=False` 만 지원한다.

use serde_json::Value;

/// Python `repr(float)`. 최단 왕복 자릿수를 쓰고, 지수가 -4 미만이거나 16 이상이면 과학 표기다.
pub fn float_repr(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let sign = if f < 0.0 { "-" } else { "" };
    let decpt = exp + 1;
    let body = if !(-3..=16).contains(&decpt) {
        let (first, rest) = digits.split_at(1);
        let m = if rest.is_empty() { first.to_string() } else { format!("{first}.{rest}") };
        format!("{m}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    } else if decpt <= 0 {
        format!("0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt as usize >= digits.len() {
        format!("{digits}{}.0", "0".repeat(decpt as usize - digits.len()))
    } else {
        format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
    };
    format!("{sign}{body}")
}

fn write_value(v: &Value, sort_keys: bool, indent: Option<usize>, level: usize, out: &mut String) {
    let (item_sep, key_sep) = if indent.is_some() { (",", ": ") } else { (", ", ": ") };
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => out.push_str(&i.to_string()),
            (_, Some(u)) => out.push_str(&u.to_string()),
            _ => out.push_str(&float_repr(n.as_f64().unwrap_or(0.0))),
        },
        Value::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                newline(indent, level + 1, out);
                write_value(item, sort_keys, indent, level + 1, out);
            }
            newline(indent, level, out);
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if sort_keys {
                // Python 은 키를 코드포인트 순으로 정렬한다. UTF-8 바이트 순서와 같다.
                entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
            }
            out.push('{');
            for (i, (k, val)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(item_sep);
                }
                newline(indent, level + 1, out);
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push_str(key_sep);
                write_value(val, sort_keys, indent, level + 1, out);
            }
            newline(indent, level, out);
            out.push('}');
        }
    }
}

fn newline(indent: Option<usize>, level: usize, out: &mut String) {
    if let Some(width) = indent {
        out.push('\n');
        out.push_str(&" ".repeat(width * level));
    }
}

/// `json.dumps(v, ensure_ascii=False, sort_keys=sort_keys)`.
pub fn dumps(v: &Value, sort_keys: bool) -> String {
    let mut out = String::new();
    write_value(v, sort_keys, None, 0, &mut out);
    out
}

/// `json.dumps(v, ensure_ascii=False, indent=2)`.
pub fn dumps_indent2(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, false, Some(2), 0, &mut out);
    out
}
