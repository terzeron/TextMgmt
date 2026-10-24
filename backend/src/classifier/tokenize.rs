//! sklearn `TfidfVectorizer` 의 분석기 3종(`word`, `char`, `char_wb`)을 그대로 옮긴 토크나이저.
//!
//! 글자는 코드포인트 단위로 센다(Python `str` 과 같다). 어휘에 없는 항을 거르는 일은 호출한 쪽 몫이다.
//! 기준은 sklearn 1.8 의 `_VectorizerMixin` 이다. `strip_accents` 는 지원하지 않는다.

use super::pyunicode_table::{DIGIT_RANGES, SPACE_RANGES, WORD_RANGES};

/// 분석기 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzerKind {
    /// 토큰 패턴 `(?u)\b\w+\b` 의 토큰과 그 n-gram(" " 로 결합).
    Word,
    /// 공백 정규화 뒤 모든 글자 n-gram.
    Char,
    /// 공백으로 나눈 단어마다 앞뒤에 공백을 붙여 단어 안에서만 만든 글자 n-gram.
    CharWb,
}

impl AnalyzerKind {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "word" => Some(Self::Word),
            "char" => Some(Self::Char),
            "char_wb" => Some(Self::CharWb),
            _ => None,
        }
    }
}

/// 분석기 하나의 옵션. `ngram_lo <= ngram_hi`, `ngram_lo >= 1` 이어야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalyzerSpec {
    pub kind: AnalyzerKind,
    pub ngram_lo: usize,
    pub ngram_hi: usize,
    pub lowercase: bool,
}

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let cp = c as u32;
    let i = table.partition_point(|&(_, hi)| hi < cp);
    i < table.len() && table[i].0 <= cp
}

/// Python `str.isspace()`, 정규식 `\s` 와 같은 집합.
pub fn is_py_space(c: char) -> bool {
    if c.is_ascii() {
        return matches!(c, '\t'..='\r' | '\x1c'..='\x20');
    }
    in_ranges(SPACE_RANGES, c)
}

/// 정규식 `\w` (유니코드 모드). Python 의 `isalnum() or '_'` 이다.
pub fn is_py_word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    in_ranges(WORD_RANGES, c)
}

/// 정규식 `\d` (유니코드 모드). 십진수 숫자(Nd)다.
pub fn is_py_digit(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_digit();
    }
    in_ranges(DIGIT_RANGES, c)
}

/// 정규식 `\s\s+` → `" "`. 공백이 연달아 둘 이상일 때만 한 칸으로 줄이고, 하나뿐이면 그대로 둔다.
pub fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_py_space(c) && chars.peek().is_some_and(|&n| is_py_space(n)) {
            while chars.peek().is_some_and(|&n| is_py_space(n)) {
                chars.next();
            }
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// `text` 에서 분석기가 만드는 항을 차례로 `emit` 에 넘긴다. 순서와 중복은 sklearn 과 같다.
pub fn for_each_term<F: FnMut(&str)>(text: &str, spec: &AnalyzerSpec, mut emit: F) {
    let lowered;
    let text = if spec.lowercase {
        lowered = text.to_lowercase();
        lowered.as_str()
    } else {
        text
    };
    match spec.kind {
        AnalyzerKind::Word => word_terms(text, spec, &mut emit),
        AnalyzerKind::Char => char_terms(&collapse_whitespace(text), spec, &mut emit),
        AnalyzerKind::CharWb => char_wb_terms(text, spec, &mut emit),
    }
}

/// `for_each_term` 의 결과를 모은다. 시험과 점검용이다.
pub fn analyze(text: &str, spec: &AnalyzerSpec) -> Vec<String> {
    let mut out = Vec::new();
    for_each_term(text, spec, |t| out.push(t.to_string()));
    out
}

/// `\b\w+\b` 의 `findall` 은 `\w` 가 이어지는 구간 하나하나와 같다.
fn word_terms<F: FnMut(&str)>(text: &str, spec: &AnalyzerSpec, emit: &mut F) {
    let mut tokens: Vec<&str> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match (is_py_word(c), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                tokens.push(&text[s..i]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        tokens.push(&text[s..]);
    }
    let (lo, hi) = (spec.ngram_lo, spec.ngram_hi);
    // sklearn: max_n == 1 이면 min_n 과 상관없이 토큰만 돌려준다.
    if hi == 1 {
        tokens.iter().for_each(|t| emit(t));
        return;
    }
    let mut min_n = lo;
    if min_n == 1 {
        tokens.iter().for_each(|t| emit(t));
        min_n += 1;
    }
    let mut joined = String::new();
    for n in min_n..(hi + 1).min(tokens.len() + 1) {
        for window in tokens.windows(n) {
            joined.clear();
            for (k, t) in window.iter().enumerate() {
                if k > 0 {
                    joined.push(' ');
                }
                joined.push_str(t);
            }
            emit(&joined);
        }
    }
}

/// 글자 경계의 바이트 위치. 길이는 글자 수 + 1 이라 `bounds[a]..bounds[b]` 가 글자 구간이다.
fn char_bounds(text: &str) -> Vec<usize> {
    let mut bounds: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    bounds.push(text.len());
    bounds
}

fn char_terms<F: FnMut(&str)>(text: &str, spec: &AnalyzerSpec, emit: &mut F) {
    let bounds = char_bounds(text);
    let text_len = bounds.len() - 1;
    let mut min_n = spec.ngram_lo;
    if min_n == 1 {
        for i in 0..text_len {
            emit(&text[bounds[i]..bounds[i + 1]]);
        }
        min_n += 1;
    }
    for n in min_n..(spec.ngram_hi + 1).min(text_len + 1) {
        for i in 0..=(text_len - n) {
            emit(&text[bounds[i]..bounds[i + n]]);
        }
    }
}

fn char_wb_terms<F: FnMut(&str)>(text: &str, spec: &AnalyzerSpec, emit: &mut F) {
    let mut padded = String::new();
    for word in text.split(is_py_space).filter(|w| !w.is_empty()) {
        padded.clear();
        padded.push(' ');
        padded.push_str(word);
        padded.push(' ');
        let bounds = char_bounds(&padded);
        let w_len = bounds.len() - 1;
        for n in spec.ngram_lo..=spec.ngram_hi {
            let mut offset = 0usize;
            // `w[0:n]` 은 단어가 n 보다 짧으면 단어 전체다.
            emit(&padded[..bounds[n.min(w_len)]]);
            while offset + n < w_len {
                offset += 1;
                emit(&padded[bounds[offset]..bounds[offset + n]]);
            }
            if offset == 0 {
                break;
            }
        }
    }
}
