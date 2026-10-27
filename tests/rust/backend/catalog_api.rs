//! Python main.py의 조회 API 계약을 실제 BookManager/ComicsManager와 검증한다.
mod common;

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::{AppDeps, AuthMode, CategoryMappingApi, MappingError, app_with};
use common::{TestClient, es_fake::FakeEsManager};
use serde_json::{Value, json};
use std::sync::Arc;

fn doc(category: &str) -> Value {
    json!({"category":category,"title":"책","author":"저자","file_path":"public/book.pdf","file_type":"pdf","file_size":100,"updated_time":"2026-10-25T00:00:00"})
}
struct Policy;
impl CategoryMappingApi for Policy {
    fn get_hidden_categories(&self, kind: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec![format!("hidden-{kind}")])
    }
    fn get_latest_excluded_categories(&self, kind: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec![format!("hidden-{kind}"), "old".into()])
    }
}
fn client(es: Arc<FakeEsManager>, role: &str, comics: bool) -> TestClient {
    common::setup_env();
    let managers = BookManagerDeps::new("/tmp/catalog-library".into(), es);
    let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":role}))).with_category_mapping(Arc::new(Policy));
    let deps =
        if comics { deps.with_comics_manager(Arc::new(backend::comics_manager::ComicsManager::with_deps(managers))) } else { deps.with_book_manager(Arc::new(BookManager::with_deps(managers))) };
    TestClient::with_app(app_with(deps))
}

#[tokio::test]
async fn metadata_returns_relative_path_and_manager_errors() {
    for (prefix, comics) in [("", false), ("/comics", true)] {
        let es = FakeEsManager::with_doc(doc("public"));
        let client = client(es.clone(), "viewer", comics);
        let response = client.get(&format!("{prefix}/books/7")).await;
        assert_eq!(response.status, 200, "{}", response.text());
        let body = response.json();
        assert_eq!(body["status"], "success");
        assert_eq!(body["result"]["book_id"], 7);
        assert_eq!(body["result"]["file_path"], "public/book.pdf");
        assert!(body["result"].get("summary").is_none());
        assert_eq!(es.calls_of("search_by_id").len(), 1);
        es.fail("search_by_id", "lookup failed");
        assert_eq!(client.get(&format!("{prefix}/books/7")).await.json(), json!({"status":"failure","error":"lookup failed"}));
        assert_eq!(client.get(&format!("{prefix}/books/invalid")).await.status, 422);
    }
}

#[tokio::test]
async fn hidden_book_and_category_descendants_are_denied() {
    for (prefix, kind, comics) in [("", "book", false), ("/comics", "comic", true)] {
        let es = FakeEsManager::with_doc(doc(&format!("hidden-{kind}/child")));
        let client = client(es.clone(), "viewer", comics);
        for path in ["books/7".to_string(), "similar/7".into(), format!("categories/hidden-{kind}/child")] {
            assert_eq!(client.get(&format!("{prefix}/{path}")).await.status, 403, "{path}");
        }
        assert!(es.calls_of("search_similar_docs_paged").is_empty());
        assert!(es.calls_of("search_by_category").is_empty());
    }
}

#[tokio::test]
async fn categories_filter_only_hidden_trees_for_viewers() {
    for role in ["viewer", "admin"] {
        let es = FakeEsManager::new();
        es.with(|s| s.aggregate = [("hidden-book".into(), 2), ("hidden-book/child".into(), 3), ("hidden-bookish".into(), 4), ("public".into(), 1)].into());
        let client = client(es.clone(), role, false);
        let response = client.get("/categories").await;
        assert_eq!(response.status, 200);
        let result = response.json()["result"].clone();
        assert_eq!(result["public"], 1);
        assert_eq!(result["hidden-bookish"], 4);
        assert_eq!(result.get("hidden-book").is_some(), role == "admin");
        es.fail("search_and_aggregate_by_category", "aggregate failed");
        assert_eq!(client.get("/categories").await.json(), json!({"status":"failure","error":"aggregate failed"}));
    }
}

#[tokio::test]
async fn category_list_supports_legacy_and_cursor_contracts() {
    let es = FakeEsManager::new();
    es.with(|s| s.category_docs = vec![(1, doc("public/sub"), 0.0)]);
    let client = client(es.clone(), "viewer", false);
    let legacy = client.get("/categories/public/sub").await;
    assert_eq!(legacy.status, 200);
    assert_eq!(legacy.json()["result"][0]["book_id"], 1);
    assert!(legacy.json().get("total").is_none());
    let paged = client.get("/categories/public/sub?limit=5").await;
    assert_eq!(paged.json()["status"], "success");
    assert!(paged.json().get("total").is_some());
    assert!(paged.json().get("next_cursor").is_some());
    assert_eq!(es.calls_of("search_by_category_paged"), vec![json!(["public/sub", 5, null])]);
    assert_eq!(client.get("/categories/public?limit=5&cursor=bad").await.json(), json!({"status":"failure","error":"invalid cursor"}));
}

#[tokio::test]
async fn latest_merges_hidden_and_latest_exclusions_and_validates_limit() {
    for (prefix, kind, comics) in [("", "book", false), ("/comics", "comic", true)] {
        let es = FakeEsManager::new();
        es.with(|s| s.latest_docs = (vec![(1, doc("public"), 0.0)], 12));
        let client = client(es.clone(), "viewer", comics);
        let response = client.get(&format!("{prefix}/latest?limit=20")).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["total"], 12);
        assert_eq!(es.calls_of("search_latest_docs"), vec![json!([20, [format!("hidden-{kind}"), "old"]])]);
        for limit in ["0", "2001", "bad"] {
            assert_eq!(client.get(&format!("{prefix}/latest?limit={limit}")).await.status, 422);
        }
    }
}

#[tokio::test]
async fn search_forwards_filters_and_preserves_total_score_and_errors() {
    for (prefix, kind, comics) in [("", "book", false), ("/comics", "comic", true)] {
        let es = FakeEsManager::new();
        es.with(|s| s.keyword_paged = (vec![(1, doc("public"), 2.5)], 42));
        let client = client(es.clone(), "viewer", comics);
        let response = client.get(&format!("{prefix}/search/hello?limit=7&offset=14&category=public&exclude_categories=old,%20hidden-{kind},,")).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["total"], 42);
        assert_eq!(es.calls_of("search_by_keyword_paged"), vec![json!({"keyword":"hello","size":7,"offset":14,"category":"public","exclude_categories":["old",format!("hidden-{kind}")]})]);
        es.fail("search_by_keyword_paged", "search failed");
        assert_eq!(client.get(&format!("{prefix}/search/hello")).await.json(), json!({"status":"failure","error":"search failed"}));
    }
}

#[tokio::test]
async fn similar_filters_hidden_results_and_falls_back_to_original() {
    let es = FakeEsManager::with_doc(doc("public"));
    es.with(|s| s.similar_paged = (vec![(2, doc("hidden-book/child"), 1.0), (3, doc("public"), 1.0)], 99));
    let client = client(es.clone(), "viewer", false);
    let response = client.get("/similar/1?offset=10&limit=5").await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["total"], 1);
    assert_eq!(response.json()["result"][0]["book_id"], 3);
    let args = es.calls_of("search_similar_docs_paged");
    assert_eq!(args[0]["size"], 5);
    assert_eq!(args[0]["offset"], 10);
    es.with(|s| s.similar_paged = (vec![], 0));
    let response = client.get("/similar/1").await;
    assert_eq!(response.json()["status"], "success");
    assert_eq!(response.json()["total"], 1);
    assert_eq!(response.json()["result"][0]["book_id"], 1);
}

#[tokio::test]
async fn all_catalog_routes_require_auth() {
    let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies)));
    for prefix in ["", "/comics"] {
        for path in ["/books/1", "/categories", "/categories/public", "/latest", "/similar/1", "/search/hello"] {
            assert_eq!(client.get(&format!("{prefix}{path}")).await.status, 401, "{prefix}{path}");
        }
    }
}
