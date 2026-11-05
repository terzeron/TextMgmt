//! 인코딩 손상(mojibake) 판정. 두 가지 기법이 있다.
//!
//! - 파일 판정(`check_file`, `classify_text`): 본문 표본의 `korean_likeness`로 파일 전체의 손상을 판정한다.
//! - 구간 세기(`count_mojibake_runs`): 추출한 글에서 EUC-KR 바이트를 latin-1로 잘못 읽은 구간을 센다.
//!   PDF 추출 실패 판정과 TXT 재인코딩 검증이 쓴다.
//!
//! 손상 유형은 원본 텍스트를 잘못된 코드페이지로 읽고 다시 UTF-8로 저장한 것이다. 파일 자체는 유효한 UTF-8이라
//! 디코딩으로는 걸러지지 않는다. 판별은 `korean_likeness` 하나로 한다(손상 0.006 이하, 정상 0.271 이상).
//! 한글이 적은 문서(한문 원전, 중국어, 일본어, 영문)는 판정 대상이 아니라 `Undetermined`다. 오진 이력이 있다.

use std::path::Path;

use crate::text_sampler::{TEXT_HEAD_CHARS, TEXT_TAIL_CHARS, extract_epub_text_limited, extract_txt_text, head_chars};
use crate::{Category, FileReport, Severity};
use text_decoder::encoding_guesser::korean_likeness;
use text_decoder::{decode_korean_ignore, is_hangul_syllable};

/// 손상 0.006 이하, 정상 0.271 이상. 사이가 비어 있어 경계를 넉넉히 잡는다.
/// 값 출처: 옛 인코딩 검사 도구 소스 28행.
pub const MOJIBAKE_LIKENESS_THRESHOLD: f64 = 0.10;

/// 한글이 전체의 이 비율에 못 미치면 판정하지 않는다. 영문 OCR 문서의 한글 잡음 오탐(1,971건)을 막는다.
/// 값 출처: 옛 인코딩 검사 도구 소스 34행.
pub const MIN_HANGUL_RATIO: f64 = 0.10;

/// 표본이 이보다 짧으면 판독할 수 없다.
const MIN_SAMPLE_CHARS: usize = 200;

/// EPUB 표본을 위해 읽는 멤버 수 상한(건너뛴 것 포함). 표본은 앞 20,000자와 뒤 10,000자이고 본문 문서 하나가 보통 수천 자 이상이라
/// 정상 EPUB은 몇 개만 읽어도 채워진다. 태그뿐인 문서(50자 미만)를 건너뛰며 읽는 경우에도 64개면 충분하다.
/// 보안 검사의 `max_files`(10,000)와 따로 정한다. 그 값은 파일 전체 검사용이라 표본에는 너무 크다.
pub const SAMPLE_MAX_MEMBERS: usize = 64;

/// 판정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Corrupted,
    Clean,
    Undetermined,
    Unreadable,
}

/// 본문 표본으로 손상 여부를 판정한다.
pub fn classify_text(text: &str) -> (Verdict, Option<f64>) {
    let len = text.chars().count();
    if len < MIN_SAMPLE_CHARS {
        return (Verdict::Unreadable, None);
    }
    let hangul = text.chars().filter(|&c| is_hangul_syllable(c)).count();
    if (hangul as f64) / (len as f64) < MIN_HANGUL_RATIO {
        // 한국어 문서가 아니다(한문 원전, 중국어, 일본어, 영문, 악보). 판정 대상이 아니다.
        return (Verdict::Undetermined, None);
    }
    match korean_likeness(text) {
        None => (Verdict::Undetermined, None),
        Some(l) if l < MOJIBAKE_LIKENESS_THRESHOLD => (Verdict::Corrupted, Some(l)),
        Some(l) => (Verdict::Clean, Some(l)),
    }
}

/// 소수 넷째 자리 반올림. `{:.4}`는 정확한 이진 값을 짝수 쪽으로 반올림한다.
fn round4(x: f64) -> f64 {
    format!("{x:.4}").parse().unwrap_or(x)
}

/// 표본 텍스트를 판정해 손상이면 지표를 더한다. 그 밖의 판정(`Unreadable` 포함)은 발견이 아니다.
pub fn check_text(report: &mut FileReport, text: &str) {
    if let (Verdict::Corrupted, Some(l)) = classify_text(text) {
        let l = round4(l);
        report.push(Category::Defect, Severity::High, "MOJIBAKE", "", &format!("likeness={l}"));
        report.likeness = Some(l);
    }
}

/// 표본을 읽는 방식. 확장자가 아니라 호출한 검사기(내용으로 정한 포맷)가 정한다.
/// `extract_document_text`는 확장자로 분기하므로 `.pdf`라는 이름의 EPUB에는 빈 문자열을 돌려준다(검토 I1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSource {
    Txt,
    Epub,
}

/// 파일에서 앞·뒤 표본을 읽어 판정한다. `extract_document_text`가 확장자별로 부르는 읽기 함수를 `source`로 직접 고르므로
/// `.txt`·`.epub` 이름 파일의 표본을 읽는다.
/// 추출한 본문이 없으면 검사하지 못한 것이므로 `incomplete`다. 너무 짧은 본문은 발견도 `incomplete`도 아니다.
/// EPUB 표본은 압축 해제 총량 상한 없이 읽는다. 신뢰할 수 없는 파일은 `check_file_limited`를 쓴다.
pub fn check_file(path: &Path, report: &mut FileReport, source: TextSource) {
    check_file_limited(path, report, source, u64::MAX, usize::MAX);
}

/// `check_file`에 EPUB 표본의 압축 해제 총량(`max_total`)과 읽는 멤버 수(`max_members`) 상한을 둔다. TXT에는 영향이 없다.
/// 예산이 바닥나 표본이 잘렸으면 `incomplete`로 표시한다.
pub fn check_file_limited(path: &Path, report: &mut FileReport, source: TextSource, max_total: u64, max_members: usize) {
    let (text, cut) = match source {
        TextSource::Txt => (extract_txt_text(path, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS), false),
        TextSource::Epub => extract_epub_text_limited(path, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS, max_total, max_members),
    };
    if cut {
        report.mark_incomplete("mojibake sampling: decompression budget reached, sample truncated");
    }
    if text.is_empty() {
        if !cut {
            report.mark_incomplete("no text extracted");
        }
        return;
    }
    check_text(report, &text);
}

// ---- 구간 세기 ----

/// 구간 세기가 보는 앞부분 글자 수. `doc_reader::TEXT_SIZE`(요약 길이)와 같다.
pub const RUN_SCAN_CHARS: usize = 4096;

/// 이 개수 이상의 mojibake 구간이 나오면 추출 실패로 본다.
pub const PDF_MOJIBAKE_RUN_LIMIT: usize = 3;

/// `[\u0080-ÿ]{2,}` 구간 각각을 EUC-KR로 다시 읽어 한글이 절반 이상이면 센다.
/// 전체 비율은 부분 손상을 놓치므로 구간 수로 센다. 앞 `RUN_SCAN_CHARS`자만 본다.
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
    for c in head_chars(text, RUN_SCAN_CHARS).chars() {
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
    for m in regex.find_iter(head_chars(text, RUN_SCAN_CHARS)) {
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
    if head_chars(text, RUN_SCAN_CHARS).trim_matches(char::is_whitespace).is_empty() {
        return false;
    }
    mojibake_runs.unwrap_or_else(|| count_mojibake_runs(text)) < PDF_MOJIBAKE_RUN_LIMIT
}
