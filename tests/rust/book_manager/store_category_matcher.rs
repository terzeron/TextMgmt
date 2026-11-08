//! 서점 카테고리 → 내부 카테고리 대응. 프런트엔드 JS(`Actions.jsx`, `Bookstore.jsx`)에서 옮긴 로직이라
//! JS가 만든 fixture(`fixtures/store_category_scores.json`)와 같은 값을 내야 한다.

use std::collections::BTreeMap;

use book_manager::store_category_matcher::{calculate_similarity, category_scores, multi_path_categories, similarity_debug, top_similar_categories, two_level_category};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/store_category_scores.json")).unwrap()
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn pairs(value: &Value) -> Vec<(String, String)> {
    value.as_array().unwrap().iter().map(|p| (p[0].as_str().unwrap().to_string(), p[1].as_str().unwrap().to_string())).collect()
}

fn mappings(f: &Value) -> BTreeMap<String, Vec<String>> {
    f["mappings"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), strings(v))).collect()
}

/// 숫자는 1e-12 안에서, 나머지는 정확히 같아야 한다. 객체 키 순서도 본다(화면이 이 순서로 그린다).
fn assert_same(actual: &Value, expected: &Value, at: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(e)) => assert!((a.as_f64().unwrap() - e.as_f64().unwrap()).abs() < 1e-12, "{at}: {a} != {e}"),
        (Value::Array(a), Value::Array(e)) => {
            assert_eq!(a.len(), e.len(), "{at}: 길이");
            for (i, (x, y)) in a.iter().zip(e).enumerate() {
                assert_same(x, y, &format!("{at}[{i}]"));
            }
        }
        (Value::Object(a), Value::Object(e)) => {
            assert_eq!(a.keys().collect::<Vec<_>>(), e.keys().collect::<Vec<_>>(), "{at}: 키");
            for (k, y) in e {
                assert_same(&a[k], y, &format!("{at}.{k}"));
            }
        }
        _ => assert_eq!(actual, expected, "{at}"),
    }
}

#[test]
fn similarity_matches_js() {
    for case in fixture()["similarity"].as_array().unwrap() {
        let (a, b) = (case["a"].as_str().unwrap(), case["b"].as_str().unwrap());
        assert_same(&serde_json::json!(calculate_similarity(a, b)), &case["score"], &format!("{a}~{b}"));
    }
}

#[test]
fn category_path_extraction_matches_js() {
    let f = fixture();
    for case in f["two_level"].as_array().unwrap() {
        assert_eq!(two_level_category(case["input"].as_str().unwrap()), case["output"].as_str().unwrap(), "{}", case["input"]);
    }
    for case in f["multi_path"].as_array().unwrap() {
        assert_eq!(multi_path_categories(case["input"].as_str().unwrap()), strings(&case["output"]), "{}", case["input"]);
    }
}

#[test]
fn category_scores_match_js() {
    let f = fixture();
    let (categories, mappings) = (strings(&f["categories"]), mappings(&f));
    for case in f["scores"].as_array().unwrap() {
        let store = case["store_category"].as_str().unwrap();
        let actual = serde_json::to_value(category_scores(store, &categories, &mappings)).unwrap();
        assert_same(&actual, &case["scores"], store);
    }
}

#[test]
fn top_similar_categories_match_js() {
    let f = fixture();
    let (categories, mappings) = (strings(&f["categories"]), mappings(&f));
    for case in f["top"].as_array().unwrap() {
        let top_n = case["top_n"].as_u64().unwrap() as usize;
        assert_eq!(top_similar_categories(&pairs(&case["suggested"]), &categories, &mappings, top_n), strings(&case["result"]), "{}", case["suggested"]);
    }
}

#[test]
fn similarity_debug_matches_js() {
    let f = fixture();
    let (categories, mappings) = (strings(&f["categories"]), mappings(&f));
    for case in f["debug"].as_array().unwrap() {
        let top_n = case["top_n"].as_u64().unwrap() as usize;
        let actual = serde_json::to_value(similarity_debug(&pairs(&case["suggested"]), &categories, &mappings, top_n)).unwrap();
        assert_same(&actual, &case["result"], &case["suggested"].to_string());
    }
}
