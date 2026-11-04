//! `classifier::feature_extractor` 단위 테스트.

mod common;
use crate::common::shared_fixtures::approx;
use classifier::feature_extractor::{Dtype, FeatureSpace, FitOptions, make_vectorizer, shrink_sources};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

fn word_field(source: &str, weight: f64) -> Value {
    json!({"source": source, "weight": weight, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0})
}

/// 가중치, analyzer, 빈 필드가 섞여야 합치는 순서와 열 오프셋이 검증된다.
///
/// `text` 를 보는 필드를 셋 두되 뒤로 갈수록 요구 길이를 늘렸다. 원문을 줄일 때
/// '남은 필드 중 가장 긴 요구'가 아니라 '바로 다음 필드'만 보면 여기서 깨진다.
fn spill_fields() -> Value {
    json!({
        "body_word": {"source": "text", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 50, "max_chars": 0},
        "body_char": {"source": "text", "weight": 0.3, "analyzer": "char_wb", "ngram": [2, 3], "min_df": 1, "max_features": 80, "max_chars": 8},
        "body_tail": {"source": "text", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 2], "min_df": 1, "max_features": 60, "max_chars": 18},
        "title": {"source": "title", "weight": 3.0, "analyzer": "char_wb", "ngram": [2, 2], "min_df": 1, "max_features": 40, "max_chars": 0},
        "publisher": {"source": "publisher", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 10, "max_chars": 0},
    })
}

fn spill_docs() -> Vec<Value> {
    vec![
        json!({"text": "무협 강호 절세 고수", "title": "천룡팔부", "publisher": ""}),
        json!({"text": "마법 던전 용사 모험", "title": "던전 일지", "publisher": ""}),
        json!({"text": "", "title": "빈 본문", "publisher": ""}),
        json!({"text": "무협 마법 혼합 세계", "title": "혼합", "publisher": ""}),
    ]
}

#[test]
fn test_make_vectorizer_keeps_single_character_word_tokens() {
    let mut vec = make_vectorizer(&json!({"analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100}));
    // 기본 token_pattern 은 한 글자를 버린다. 한자 한 글자가 뜻을 갖는 문서가 있어 살려 둔다.
    assert!(vec.fit(&["天 地 人 무협"]).vocabulary().contains_key("天"));
}

#[test]
fn test_feature_space_applies_per_field_weight() {
    let fields = json!({"body_word": word_field("text", 1.0)});
    let docs = vec![json!({"text": "무협 강호"}), json!({"text": "마법 던전"})];

    let plain = FeatureSpace::new(&fields).fit_transform(&docs).unwrap();
    let heavy_fields = json!({"body_word": word_field("text", 3.0)});
    let heavy = FeatureSpace::new(&heavy_fields).fit_transform(&docs).unwrap();

    assert_eq!(heavy.shape(), plain.shape());
    let (h, p) = (heavy.to_dense(), plain.to_dense());
    for (hr, pr) in h.iter().zip(&p) {
        for (a, b) in hr.iter().zip(pr) {
            assert!(approx(*a, b * 3.0, 1e-8), "heavy={a} plain*3={}", b * 3.0);
        }
    }
}

#[test]
fn test_feature_space_skips_fields_without_any_vocabulary() {
    let fields = json!({"body_word": word_field("text", 1.0), "publisher": word_field("publisher", 3.0)});
    let mut space = FeatureSpace::new(&fields);
    space.fit_transform(&[json!({"text": "무협", "publisher": ""}), json!({"text": "마법", "publisher": ""})]).unwrap();
    assert!(!space.has_vectorizer("publisher"));
    assert!(space.describe().get("body_word").is_some());
}

#[test]
fn test_feature_space_raises_when_no_field_yields_features() {
    let fields = json!({"publisher": word_field("publisher", 1.0)});
    assert!(FeatureSpace::new(&fields).fit_transform(&[json!({"publisher": ""}), json!({"publisher": ""})]).is_err());
}

#[test]
fn test_feature_space_transform_requires_fitted_vectorizers() {
    assert!(FeatureSpace::new(&json!({})).transform(&[json!({"text": "무협"})]).is_err());
}

#[test]
fn test_feature_space_spill_matches_in_memory_matrix() {
    // 디스크로 흘려보내고 다시 합친 행렬은 한 번에 만든 것과 같아야 한다.
    //
    // 합치기를 직접 구현했다. 열 오프셋이나 행 경계가 한 칸만 어긋나도 모델이
    // 조용히 엉뚱한 특징을 학습한다. 같은 자리 같은 값인지 값으로 못 박는다.
    let tmp = TempDir::new().unwrap();
    let (fields, docs) = (spill_fields(), spill_docs());
    let plain = FeatureSpace::new(&fields).fit_transform_with(&docs, FitOptions { spill_dir: None, dtype: Dtype::F64 }).unwrap();
    let spilled = FeatureSpace::new(&fields).fit_transform_with(&docs, FitOptions { spill_dir: Some(tmp.path().to_path_buf()), dtype: Dtype::F64 }).unwrap();

    assert_eq!(spilled.shape(), plain.shape());
    assert_eq!(spilled.dtype(), Dtype::F64);
    assert_eq!(spilled.nnz(), plain.nnz());
    assert_eq!(spilled.to_dense(), plain.to_dense());
    assert!(fs::read_dir(tmp.path()).unwrap().next().is_some(), "spill 행렬은 사용 중 파일로 유지돼야 한다");
    drop(spilled);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0, "spill 파일은 행렬 수명 종료 때 제거한다");
}

#[test]
fn test_feature_space_lazy_rows_match_materialized_rows() {
    let tmp = TempDir::new().unwrap();
    let (fields, docs) = (spill_fields(), spill_docs());
    let selected = [0usize, 1, 3];
    let materialized: Vec<Value> = selected.iter().map(|&index| docs[index].clone()).collect();
    let expected = FeatureSpace::new(&fields).fit_transform_with(&materialized, FitOptions { spill_dir: Some(tmp.path().join("expected")), dtype: Dtype::F64 }).unwrap();
    let actual =
        FeatureSpace::new(&fields).fit_transform_selected_by(&selected, FitOptions { spill_dir: Some(tmp.path().join("actual")), dtype: Dtype::F64 }, |index| Ok(docs[index].clone())).unwrap();

    assert_eq!(actual.shape(), expected.shape());
    assert_eq!(actual.nnz(), expected.nnz());
    assert_eq!(actual.to_dense(), expected.to_dense());
}

#[test]
fn test_feature_space_spill_keeps_same_block_layout() {
    // 스필 경로도 벡터라이저와 블록 크기를 똑같이 남겨야 판정 때 transform 이 맞는다.
    let tmp = TempDir::new().unwrap();
    let (fields, docs) = (spill_fields(), spill_docs());
    let (mut plain, mut spilled) = (FeatureSpace::new(&fields), FeatureSpace::new(&fields));
    plain.fit_transform(&docs).unwrap();
    spilled.fit_transform_with(&docs, FitOptions { spill_dir: Some(tmp.path().to_path_buf()), ..Default::default() }).unwrap();

    assert_eq!(spilled.vectorizer_names(), plain.vectorizer_names());
    assert_eq!(spilled.block_sizes(), plain.block_sizes());
    // publisher 는 전부 빈 문자열이라 양쪽 모두에서 빠져야 한다.
    assert!(!spilled.has_vectorizer("publisher"));
}

#[test]
fn test_feature_space_spill_raises_when_no_field_yields_features() {
    let tmp = TempDir::new().unwrap();
    let fields = json!({"publisher": word_field("publisher", 1.0)});
    let result = FeatureSpace::new(&fields).fit_transform_with(&[json!({"publisher": ""}), json!({"publisher": ""})], FitOptions { spill_dir: Some(tmp.path().to_path_buf()), ..Default::default() });
    assert!(result.is_err());
}

#[test]
fn test_shrink_sources_keeps_the_longest_remaining_need() {
    // 남은 필드 중 가장 긴 요구만큼은 남겨야 한다. 짧은 쪽에 맞추면 특징이 사라진다.
    let mut docs = vec![json!({"text": "가나다라마바사아자차카타파하", "title": "제목", "cat": "3_무협"})];
    let remaining = [("body_char", json!({"source": "text", "max_chars": 5})), ("body_tail", json!({"source": "text", "max_chars": 9})), ("title", json!({"source": "title", "max_chars": 0}))];
    shrink_sources(&mut docs, &["text", "title"], &remaining);
    assert_eq!(docs[0]["text"], "가나다라마바사아자");
    assert_eq!(docs[0]["title"], "제목", "max_chars 0 은 전부 필요하다는 뜻이다");
    assert_eq!(docs[0]["cat"], "3_무협", "원문이 아닌 키는 건드리면 안 된다");
}

#[test]
fn test_shrink_sources_drops_a_source_no_field_reads_any_more() {
    let mut docs = vec![json!({"text": "본문", "title": "제목", "cat": "3_무협"})];
    shrink_sources(&mut docs, &["text", "title"], &[("title", json!({"source": "title", "max_chars": 0}))]);
    assert!(docs[0].get("text").is_none());
    assert_eq!(docs[0]["title"], "제목");
}

#[test]
fn test_shrink_sources_leaves_the_filename_alone() {
    // `field_text` 는 숫자를 지운 뒤에 자른다. 먼저 자르면 결과가 달라진다.
    let mut docs = vec![json!({"name": "무협 1234 강호 5678 협객.epub"})];
    shrink_sources(&mut docs, &["name"], &[("filename", json!({"source": "name", "max_chars": 5}))]);
    assert_eq!(docs[0]["name"], "무협 1234 강호 5678 협객.epub");
}

#[test]
fn test_feature_space_names_follow_field_order() {
    let space = FeatureSpace::new(&json!({"title": {}, "body": {}}));
    assert_eq!(space.names(), vec!["title", "body"]);
}
