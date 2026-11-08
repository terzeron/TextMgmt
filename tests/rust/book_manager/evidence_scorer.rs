//! 근거별 카테고리 점수와 가중치 합산.

use book_manager::evidence_scorer::{EVIDENCE_WEIGHTS, Evidence, combine, detail_of, isbn_scores, keyword_scores};

fn s(pairs: &[(&str, f64)]) -> Vec<(String, f64)> {
    pairs.iter().map(|(c, p)| (c.to_string(), *p)).collect()
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn weights_follow_isbn_keyword_model_order() {
    assert_eq!(EVIDENCE_WEIGHTS, [(Evidence::Isbn, 0.6), (Evidence::Keyword, 0.25), (Evidence::Model, 0.15)]);
}

#[test]
fn isbn_scores_are_category_shares() {
    let scores = isbn_scores(&["A/x.epub".into(), "A/y.epub".into(), "B/sub/z.epub".into()]).unwrap();
    assert_eq!(scores.len(), 2);
    assert!(approx(scores.iter().find(|(c, _)| c == "A").unwrap().1, 2.0 / 3.0));
    // 하위 디렉토리가 아니라 최상위 카테고리로 센다. 승인은 최상위 카테고리만 목적지로 받는다.
    assert!(approx(scores.iter().find(|(c, _)| c == "B").unwrap().1, 1.0 / 3.0));
}

#[test]
fn isbn_scores_merge_subdirectories_of_one_category() {
    let scores = isbn_scores(&["A/x/1.epub".into(), "A/y/2.epub".into(), "B/3.epub".into()]).unwrap();
    assert_eq!(scores, s(&[("A", 2.0 / 3.0), ("B", 1.0 / 3.0)]));
}

#[test]
fn isbn_scores_none_when_empty_or_at_limit() {
    assert!(isbn_scores(&[]).is_none());
    let many: Vec<String> = (0..search_manager::ISBN_MATCH_LIMIT).map(|i| format!("A/{i}.epub")).collect();
    assert!(isbn_scores(&many).is_none());
    assert!(isbn_scores(&many[..many.len() - 1]).is_some());
}

#[test]
fn isbn_scores_skip_root_files() {
    assert!(isbn_scores(&["x.epub".into()]).is_none());
}

#[test]
fn keyword_scores_are_keyword_shares() {
    let scores = keyword_scores(&[("A".into(), vec!["k1".into(), "k2".into(), "k3".into()]), ("B".into(), vec!["k4".into()])]).unwrap();
    assert_eq!(scores, s(&[("A", 0.75), ("B", 0.25)]));
    assert!(keyword_scores(&[]).is_none());
}

#[test]
fn combine_normalizes_by_present_weights() {
    let ranked = combine(&[(Evidence::Model, s(&[("A", 0.7), ("B", 0.3)]))]);
    assert_eq!(ranked[0].category, "A");
    assert!(approx(ranked[0].score, 0.7));
    assert!(approx(ranked[1].score, 0.3));
}

#[test]
fn combine_isbn_outweighs_keyword_and_model_together() {
    let ranked = combine(&[(Evidence::Isbn, s(&[("A", 1.0)])), (Evidence::Keyword, s(&[("B", 1.0)])), (Evidence::Model, s(&[("B", 1.0)]))]);
    assert_eq!(ranked[0].category, "A");
    assert!(approx(ranked[0].score, 0.6));
    assert!(approx(ranked[1].score, 0.4));
}

#[test]
fn combine_records_each_contribution() {
    let ranked = combine(&[(Evidence::Keyword, s(&[("A", 1.0)])), (Evidence::Model, s(&[("A", 0.5)]))]);
    assert_eq!(ranked[0].contributions, vec![(Evidence::Keyword, 1.0), (Evidence::Model, 0.5)]);
    assert!(approx(ranked[0].score, (0.25 + 0.075) / 0.4));
}

#[test]
fn combine_tie_breaks_by_name() {
    let ranked = combine(&[(Evidence::Model, s(&[("B", 0.5), ("A", 0.5)]))]);
    assert_eq!(ranked.iter().map(|c| c.category.as_str()).collect::<Vec<_>>(), vec!["A", "B"]);
}

#[test]
fn combine_with_no_evidence_is_empty() {
    assert!(combine(&[]).is_empty());
}

#[test]
fn detail_lists_each_evidence_and_dominant_source() {
    let ranked = combine(&[(Evidence::Isbn, s(&[("A", 0.5)])), (Evidence::Model, s(&[("A", 0.41)]))]);
    assert_eq!(detail_of(&ranked[0]), "ISBN 0.50 · 모델 0.41");
    assert_eq!(ranked[0].dominant().name(), "isbn");
}
