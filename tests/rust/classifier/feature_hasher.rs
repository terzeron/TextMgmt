//! 특징 해시: ES 문서 필드를 고정 크기 희소 벡터로 바꾼다.

use classifier::feature_hasher::{fnv1a64, hash_features, BookFields, SUMMARY_MAX_CHARS};
use serde_json::json;

fn fields(summary: &str, title: &str, author: &str) -> BookFields {
    BookFields { summary: summary.to_string(), title: title.to_string(), author: author.to_string() }
}

#[test]
fn same_input_gives_identical_vector() {
    let f = fields("헌터가 던전에 들어갔다", "나 혼자 레벨업", "추공");
    assert_eq!(hash_features(&f, 17), hash_features(&f, 17));
}

#[test]
fn namespaces_keep_title_and_summary_apart() {
    let in_title = hash_features(&fields("", "헌터", ""), 17);
    let in_summary = hash_features(&fields("헌터", "", ""), 17);
    assert!(!in_title.is_empty());
    assert_ne!(in_title, in_summary);
}

#[test]
fn summary_is_cut_at_1500_chars() {
    assert_eq!(SUMMARY_MAX_CHARS, 1500);
    let head = "가".repeat(SUMMARY_MAX_CHARS);
    let longer = format!("{head}뒤에만있는글자");
    assert_eq!(hash_features(&fields(&head, "", ""), 17), hash_features(&fields(&longer, "", ""), 17));
}

#[test]
fn each_field_is_l2_normalized() {
    let v = hash_features(&fields("검과 마법의 세계에서 검을 든다", "", ""), 17);
    let norm: f32 = v.iter().map(|(_, x)| x * x).sum();
    assert!((norm - 1.0).abs() < 1e-5, "norm={norm}");
}

#[test]
fn indices_are_sorted_unique_and_below_bucket_count() {
    let v = hash_features(&fields("어느 날 갑자기 세상이 바뀌었다. 탑이 나타났다.", "탑의 주인", "가, 나"), 10);
    assert!(v.windows(2).all(|w| w[0].0 < w[1].0));
    assert!(v.iter().all(|(i, _)| *i < 1024));
}

#[test]
fn author_array_and_string_hash_the_same() {
    let from_array = BookFields::from_es_source(&json!({"author": ["가", "나"]}));
    let from_string = BookFields::from_es_source(&json!({"author": "가, 나"}));
    assert_eq!(hash_features(&from_array, 17), hash_features(&from_string, 17));
}

#[test]
fn from_es_source_fills_missing_fields_with_empty() {
    let f = BookFields::from_es_source(&json!({"title": "제목"}));
    assert_eq!(f, fields("", "제목", ""));
}

#[test]
fn fnv1a_matches_known_vector() {
    assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
}

#[test]
fn empty_title_and_summary_is_empty() {
    assert!(fields("  ", "", "작가").is_empty());
    assert!(!fields("", "제목", "").is_empty());
}
