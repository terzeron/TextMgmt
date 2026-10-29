//! 인코딩 손상(mojibake) 판정. 원본: 옛 인코딩 검사 도구의 `classify_text`와 `inspect_file`.
//!
//! 손상 유형은 원본 텍스트를 잘못된 코드페이지로 읽고 다시 UTF-8로 저장한 것이다. 파일 자체는 유효한 UTF-8이라
//! 디코딩으로는 걸러지지 않는다. 판별은 `korean_likeness` 하나로 한다(손상 0.006 이하, 정상 0.271 이상).
//! 한글이 적은 문서(한문 원전, 중국어, 일본어, 영문)는 판정 대상이 아니라 `Undetermined`다. 오진 이력이 있다.

use std::path::Path;

use corpus_common::reader::{TEXT_HEAD_CHARS, TEXT_TAIL_CHARS, is_hangul_syllable, korean_likeness, read_epub_text, read_txt_text};
use scan_common::{Category, FileReport, Severity};

/// 손상 0.006 이하, 정상 0.271 이상. 사이가 비어 있어 경계를 넉넉히 잡는다.
/// 값 출처: 옛 인코딩 검사 도구 소스 28행.
pub const MOJIBAKE_LIKENESS_THRESHOLD: f64 = 0.10;

/// 한글이 전체의 이 비율에 못 미치면 판정하지 않는다. 영문 OCR 문서의 한글 잡음 오탐(1,971건)을 막는다.
/// 값 출처: 옛 인코딩 검사 도구 소스 34행.
pub const MIN_HANGUL_RATIO: f64 = 0.10;

/// 표본이 이보다 짧으면 판독할 수 없다. 원본 lib.rs 60행.
const MIN_SAMPLE_CHARS: usize = 200;

/// 원본의 `VERDICT_*` 문자열 상수에 대응한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Corrupted,
    Clean,
    Undetermined,
    Unreadable,
}

/// 본문 표본으로 손상 여부를 판정한다. 원본 `classify_text`(lib.rs 57-89행)와 규칙·순서가 같다.
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

/// 소수 넷째 자리 반올림. Rust의 `{:.4}`는 정확한 이진 값을 짝수 쪽으로 반올림하므로 Python `round(x, 4)`와 같다.
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
/// `read_document_text`는 확장자로 분기하므로 `.pdf`라는 이름의 EPUB에는 빈 문자열을 돌려준다(검토 I1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSource {
    Txt,
    Epub,
}

/// 파일에서 앞·뒤 표본을 읽어 판정한다. `read_document_text`가 확장자별로 부르는 읽기 함수를 `source`로 직접 고르므로
/// `.txt`·`.epub` 이름 파일의 표본은 원본 `inspect_file`(lib.rs 99행~)과 같다.
/// 추출한 본문이 없으면 검사하지 못한 것이므로 `incomplete`다. 너무 짧은 본문은 발견도 `incomplete`도 아니다.
pub fn check_file(path: &Path, report: &mut FileReport, source: TextSource) {
    let text = match source {
        TextSource::Txt => read_txt_text(path, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS),
        TextSource::Epub => read_epub_text(path, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS),
    };
    if text.is_empty() {
        report.mark_incomplete("no text extracted");
        return;
    }
    check_text(report, &text);
}
