//! 필드 하나의 TF-IDF 벡터화와 문서 필드 추출. Python `features.py` 의 `field_text`, `FeatureSpace.transform` 대응.

use std::collections::HashMap;

use serde_json::Value;

use super::export_format::ModelError;
use super::tokenize::{AnalyzerSpec, for_each_term, is_py_digit};

/// 문서에서 한 필드의 원문을 꺼낸다. Python `field_text` 와 같다.
///
/// 거짓값(null, false, 0, 빈 문자열·목록)은 빈 문자열이다. 목록은 거짓 원소를 빼고 공백으로 잇는다.
/// `name` 소스는 두 자리 이상 숫자열을 공백 하나로 바꾼 뒤에 `max_chars` 글자로 자른다.
pub fn field_text(doc: &Value, source: &str, max_chars: usize) -> String {
    let raw = doc.get(source).unwrap_or(&Value::Null);
    let mut text = match raw {
        Value::Array(items) => items.iter().filter(|v| is_truthy(v)).map(py_str).collect::<Vec<_>>().join(" "),
        v if is_truthy(v) => py_str(v),
        _ => String::new(),
    };
    if source == "name" {
        text = replace_digit_runs(&text);
    }
    if max_chars > 0
        && let Some((byte, _)) = text.char_indices().nth(max_chars)
    {
        text.truncate(byte);
    }
    text
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python `str(x)`. 숫자·불리언·문자열만 정확히 맞춘다. 중첩 값은 JSON 문자열로 대신한다.
fn py_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => py_float_repr(n.as_f64().unwrap_or(0.0)),
        },
        other => other.to_string(),
    }
}

fn py_float_repr(f: f64) -> String {
    let s = format!("{f:?}");
    match s.split_once('e') {
        Some((mantissa, exp)) => {
            let e: i32 = exp.parse().unwrap_or(0);
            format!("{mantissa}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
        }
        None => s,
    }
}

/// 정규식 `\d{2,}` → `" "`.
fn replace_digit_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_py_digit(c) && chars.peek().is_some_and(|&n| is_py_digit(n)) {
            while chars.peek().is_some_and(|&n| is_py_digit(n)) {
                chars.next();
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// 필드 하나의 벡터라이저. 열 번호는 필드 안에서 0부터, 전체 열은 `offset` 을 더한 값이다.
#[derive(Debug)]
pub struct FieldVectorizer {
    pub name: String,
    pub source: String,
    pub weight: f32,
    pub max_chars: usize,
    pub spec: AnalyzerSpec,
    pub sublinear_tf: bool,
    pub offset: u32,
    vocab: HashMap<String, u32>,
    idf: Vec<f32>,
}

impl FieldVectorizer {
    /// 어휘 항목이 중복이거나 idf 길이가 어휘와 다르면 오류다.
    #[allow(clippy::too_many_arguments)]
    pub fn new(name: &str, source: &str, weight: f32, max_chars: usize, spec: AnalyzerSpec, sublinear_tf: bool, offset: u32, vocab: Vec<String>, idf: Vec<f32>) -> Result<Self, ModelError> {
        if vocab.len() != idf.len() {
            return Err(ModelError::Invalid(format!("필드 '{name}': 어휘 {}개와 idf {}개의 길이가 다르다", vocab.len(), idf.len())));
        }
        let mut map = HashMap::with_capacity(vocab.len());
        for (i, term) in vocab.into_iter().enumerate() {
            if map.insert(term, i as u32).is_some() {
                return Err(ModelError::DuplicateTerm { field: name.to_string() });
            }
        }
        Ok(Self { name: name.to_string(), source: source.to_string(), weight, max_chars, spec, sublinear_tf, offset, vocab: map, idf })
    }

    pub fn vocab_size(&self) -> usize {
        self.idf.len()
    }

    /// 원문 하나를 희소 벡터(전체 열 번호 오름차순)로 바꿔 `out` 에 덧붙인다.
    ///
    /// 순서는 sklearn 과 같다. 빈도 → `1 + ln(tf)` → `× idf` → 행 L2 정규화(float32) → `× weight`.
    pub fn transform_into(&self, text: &str, out: &mut Vec<(u32, f32)>) {
        let mut cols: Vec<u32> = Vec::new();
        for_each_term(text, &self.spec, |term| {
            if let Some(&col) = self.vocab.get(term) {
                cols.push(col);
            }
        });
        if cols.is_empty() {
            return;
        }
        cols.sort_unstable();
        let start = out.len();
        let mut i = 0;
        while i < cols.len() {
            let mut j = i;
            while j < cols.len() && cols[j] == cols[i] {
                j += 1;
            }
            let count = (j - i) as f32;
            let tf = if self.sublinear_tf { count.ln() + 1.0 } else { count };
            out.push((cols[i], tf * self.idf[cols[i] as usize]));
            i = j;
        }
        let mut sum = 0.0f32;
        for &(_, v) in &out[start..] {
            sum += v * v;
        }
        let norm = sum.sqrt();
        for entry in &mut out[start..] {
            if norm != 0.0 {
                entry.1 /= norm;
            }
            entry.1 *= self.weight;
            entry.0 += self.offset;
        }
    }
}
