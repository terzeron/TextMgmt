//! `backend::main` 단위 테스트.
#![allow(dead_code)]

mod common;
use crate::common::shared_fixtures::{MockCat, mock_bm};
use backend::api_router::{CategoryMappingApi, route_table};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

#[tokio::test]
async fn test_apply_job_marks_state_failed_when_book_manager_returns_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bm = mock_bm();
    let cat = MockCat::with_store(Some(json!({"status": "applying", "source_category": "0_inbox", "apply_token": "tok"})));
    *bm.apply_result.lock().unwrap() = (json!({}), Some("적용 오류".into()));
    *bm.path_prefix.lock().unwrap() = tmp.path().to_path_buf();

    // content_type="book"인 라우터 안의 작업 함수. 가정한 seam이다.
    backend::api_router::run_classify_apply_job(
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

/// 상태 조회부터 실패하는 저장소. 기본 구현은 모두 오류다.
struct StatusUnreadable;
impl CategoryMappingApi for StatusUnreadable {}

/// 상태는 읽지만 저장은 실패하는 저장소.
struct StatusUnwritable;
impl CategoryMappingApi for StatusUnwritable {
    fn get_classify_proposal_status(&self, _content_type: &str, _stale_seconds: i64) -> Result<serde_json::Value, backend::api_router::MappingError> {
        Ok(json!({"status": "applying"}))
    }
}

#[tokio::test]
async fn test_apply_job_failure_logs_when_its_state_cannot_be_read_or_saved() {
    let bm = mock_bm();
    *bm.apply_result.lock().unwrap() = (json!({}), Some("적용 오류".into()));
    let logs = common::log_capture::LogCapture::start(log::Level::Error);
    let stores: [(&dyn CategoryMappingApi, &str); 2] = [(&StatusUnreadable, "상태 조회 실패"), (&StatusUnwritable, "상태 저장 실패")];
    for (store, expected) in stores {
        backend::api_router::run_classify_apply_job(bm.as_ref(), store, "book", vec![], HashSet::new(), false, "tok").await;
        assert!(logs.text().contains(expected), "expected={expected}: {}", logs.text());
    }
    // 적용이 성공하면 상태를 건드리지 않는다.
    *bm.apply_result.lock().unwrap() = (json!({"applied_count": 0, "failed_count": 0}), None);
    let cat = MockCat::with_store(Some(json!({"status": "applying"})));
    backend::api_router::run_classify_apply_job(bm.as_ref(), cat.as_ref(), "book", vec![], HashSet::new(), false, "tok").await;
    assert_eq!(cat.store.lock().unwrap()["book"]["status"], "applying");
}

fn normalize_path(method: &str, path: &str) -> String {
    let path = path.replace("{*", "{");
    if method != "GET" {
        return path;
    }
    ["/categories/rename", "/categories/delete"].into_iter().find_map(|reserved| path.strip_suffix(reserved).map(|prefix| format!("{prefix}/categories/{{category}}"))).unwrap_or(path)
}

#[test]
fn registers_every_openapi_route_and_method() {
    let expected: Vec<(String, String, Vec<String>)> = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/openapi_routes.json"))).unwrap();
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

/// 카테고리가 빈 문자열이면 false다.
#[cfg(any())] // hidden category 필터는 공개 라우트 통합 테스트에서 검증한다.
#[test]
fn test_category_matches_hidden_empty_category_returns_false() {
    assert!(!backend::api_router::category_matches_hidden("", &["cat1", "cat2"]));
}
