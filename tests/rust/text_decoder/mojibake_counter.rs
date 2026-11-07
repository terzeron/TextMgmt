//! `text_decoder::mojibake_counter` 단위 테스트.

use text_decoder::mojibake_counter::{count_mojibake_runs, count_mojibake_runs_with, is_usable_text};

fn latin1_misread(text: &str) -> String {
    let (bytes, _, had_errors) = encoding_rs::EUC_KR.encode(text);
    assert!(!had_errors);
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

#[test]
fn regex_mojibake_counter_handles_invalid_short_non_korean_and_valid_matches() {
    assert_eq!(count_mojibake_runs_with(&latin1_misread("한글본문"), "[\\x{80}-\\x{ff}]{2,}"), 1);
    assert_eq!(count_mojibake_runs_with("abc", "["), 0);
    assert_eq!(count_mojibake_runs_with("abc 漢字", "."), 0);
    assert_eq!(count_mojibake_runs_with("éé", ".+"), 0);
}

#[test]
fn mojibake_runs_count_latin1_misreads_of_euc_kr() {
    // "안녕하세요"를 EUC-KR 바이트로 만든 뒤 latin-1로 잘못 읽은 글자들
    let (bytes, _, _) = encoding_rs::EUC_KR.encode("안녕하세요");
    let mojibake: String = bytes.iter().map(|&b| b as char).collect();
    assert_eq!(count_mojibake_runs(&mojibake), 1);
    assert_eq!(count_mojibake_runs(&format!("{mojibake} 중간 {mojibake}")), 2);
    assert_eq!(count_mojibake_runs("café résumé"), 0);
    assert_eq!(count_mojibake_runs("정상 한국어"), 0);
    assert!(is_usable_text("정상 한국어", None));
    assert!(!is_usable_text("   ", None));
    assert!(!is_usable_text("x", Some(3)));
    assert!(is_usable_text("x", Some(2)));
}
