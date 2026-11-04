//! `backend::main` 단위 테스트.
#![allow(dead_code)]

mod common;
use std::collections::{HashSet};
use std::sync::{Arc};
use backend::main::{CategoryMappingApi, route_table};
use serde_json::{json};
use crate::common::shared_fixtures::{MockCat, mock_bm};

#[tokio::test]
async fn test_apply_job_marks_state_failed_when_book_manager_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bm = mock_bm();
    let cat = MockCat::with_store(Some(json!({"status": "applying", "source_category": "0_inbox", "apply_token": "tok"})));
    *bm.apply_result.lock().unwrap() = (json!({}), Some("적용 오류".into()));
    *bm.path_prefix.lock().unwrap() = tmp.path().to_path_buf();

    // content_type="book"인 라우터 안의 작업 함수. 가정한 seam이다.
    backend::main::run_classify_apply_job(
        bm.as_ref(),
        cat.as_ref(),
        "book",
        vec![json!({"file_path": "0_inbox/a.epub", "target_category": "3_SF"})],
        HashSet::from(["0_inbox/a.epub".to_string()]),
        false,
        "tok",
    )
    .await;

    let state = cat.store.lock().unwrap().get("book").cloned().unwrap();
    assert_eq!(state["status"], "failed");
    assert_eq!(state["error"], "적용 오류");
}

fn normalize_path(method: &str, path: &str) -> String {
    let path = path.replace("{*", "{");
    if method != "GET" {
        return path;
    }
    ["/categories/rename", "/categories/delete"].into_iter().find_map(|reserved| path.strip_suffix(reserved).map(|prefix| format!("{prefix}/categories/{{category}}"))).unwrap_or(path)
}

#[test]
fn rust_registers_every_python_openapi_route_and_method() {
    let expected: Vec<(String, String, Vec<String>)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/python_openapi_routes.json"))).unwrap();
    let actual: HashSet<(String, String, Vec<String>)> = route_table()
        .into_iter()
        .flat_map(|route| {
            route.methods.into_iter().map(move |method| {
                let dependencies = route.dependencies.clone();
                (method.clone(), normalize_path(&method, &route.path), dependencies)
            })
        })
        .collect();
    let expected: HashSet<_> = expected.into_iter().collect();
    let missing: Vec<_> = expected.difference(&actual).collect();
    let extra: Vec<_> = actual.difference(&expected).collect();
    assert!(missing.is_empty() && extra.is_empty(), "Rust missing routes: {missing:?}; Rust-only routes: {extra:?}");
}

#[test]
fn real_mapping_facade_propagates_mapping_read_database_failures() {
    use db::{DbError, category_mapping::CategoryMapping};
    let mapping = CategoryMapping::with_connection_provider(Arc::new(|| Err(DbError::Other("offline".into()))));
    assert!(CategoryMappingApi::get_all_mappings(&mapping, "book").is_err());
    assert!(CategoryMappingApi::get_keywords(&mapping, "fiction", "book").is_err());
    assert!(!CategoryMappingApi::set_keywords(&mapping, "fiction", &["one".into()], "book").unwrap());
    assert!(CategoryMappingApi::add_keyword(&mapping, "fiction", "one", "book").is_err());
    assert!(CategoryMappingApi::remove_keyword(&mapping, "fiction", "one", "book").is_err());
    assert!(CategoryMappingApi::delete_category(&mapping, "fiction", "book", false).is_err());
    assert!(!CategoryMappingApi::update_all_mappings(&mapping, &json!({"fiction":["one"]}), "book").unwrap());
    assert!(CategoryMappingApi::get_hidden_categories(&mapping, "book").is_err());
    assert!(CategoryMappingApi::get_latest_excluded_categories(&mapping, "book").is_err());
    assert!(!CategoryMappingApi::set_hidden(&mapping, "fiction", true, "book").unwrap());
    assert!(!CategoryMappingApi::set_latest_excluded(&mapping, "fiction", true, "book").unwrap());
}

/// 카테고리가 빈 문자열이면 False (main.py:252).
#[cfg(any())] // hidden category 필터는 공개 라우트 통합 테스트에서 검증한다.
#[test]
fn test_category_matches_hidden_empty_category_returns_false() {
    assert!(!backend::main::category_matches_hidden("", &["cat1", "cat2"]));
}
