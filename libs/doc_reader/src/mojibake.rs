//! EUC-KR 바이트를 latin-1로 잘못 읽은 구간 세기. 원본: `Loader._count_mojibake_runs`.

use corpus_common::reader::{decode_korean_ignore, is_hangul_syllable};

use crate::TEXT_SIZE;
use crate::pytext::head_chars;

/// 이 개수 이상의 mojibake 구간이 나오면 추출 실패로 본다.
pub const PDF_MOJIBAKE_RUN_LIMIT: usize = 3;

/// `[\u0080-ÿ]{2,}` 구간 각각을 EUC-KR로 다시 읽어 한글이 절반 이상이면 센다.
/// 전체 비율은 부분 손상을 놓치므로 구간 수로 센다. 앞 `TEXT_SIZE`자만 본다.
pub fn count_mojibake_runs(text: &str) -> usize {
    let mut runs = 0;
    let mut current: Vec<u8> = Vec::new();
    let flush = |current: &mut Vec<u8>, runs: &mut usize| {
        if current.len() >= 2 {
            let decoded = decode_korean_ignore(current, true);
            let total = decoded.chars().count();
            if total > 0 && decoded.chars().filter(|&c| is_hangul_syllable(c)).count() as f64 / total as f64 >= 0.5 {
                *runs += 1;
            }
        }
        current.clear();
    };
    for c in head_chars(text, TEXT_SIZE).chars() {
        if ('\u{80}'..='\u{ff}').contains(&c) {
            current.push(c as u32 as u8);
        } else {
            flush(&mut current, &mut runs);
        }
    }
    flush(&mut current, &mut runs);
    runs
}

/// 정규식으로 지정한 구간만 mojibake 후보로 검사한다.
pub fn count_mojibake_runs_with(text: &str, pattern: &str) -> usize {
    let Ok(regex) = regex::Regex::new(pattern) else { return 0 };
    let mut count = 0;
    for m in regex.find_iter(head_chars(text, TEXT_SIZE)) {
        let bytes: Vec<u8> = m.as_str().chars().filter_map(|c| (c as u32 <= 255).then_some(c as u8)).collect();
        if bytes.len() < 2 || bytes.len() != m.as_str().chars().count() {
            continue;
        }
        let decoded = decode_korean_ignore(&bytes, true);
        let total = decoded.chars().count();
        if total > 0 && decoded.chars().filter(|&c| is_hangul_syllable(c)).count() as f64 / total as f64 >= 0.5 {
            count += 1;
        }
    }
    count
}

/// 비어 있거나 mojibake면 쓸 수 없다. `mojibake_runs`를 넘기면 같은 텍스트를 두 번 훑지 않는다.
pub fn is_usable_text(text: &str, mojibake_runs: Option<usize>) -> bool {
    if head_chars(text, TEXT_SIZE).trim_matches(crate::pytext::is_space).is_empty() {
        return false;
    }
    mojibake_runs.unwrap_or_else(|| count_mojibake_runs(text)) < PDF_MOJIBAKE_RUN_LIMIT
}
