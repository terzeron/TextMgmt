use doc_reader::mojibake_counter::{count_mojibake_runs, is_usable_text};

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
