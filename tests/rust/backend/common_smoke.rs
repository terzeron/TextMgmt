//! `common/` 공용 도구가 컴파일되고 기본 동작을 하는지 확인한다. 다른 테스트 binary가 같은 모듈을 쓰므로 여기서 먼저 깨진다.

mod common;

use backend::es_manager::{Docs, EsError, EsManagerApi};
use common::es_fake::FakeEsManager;
use common::{TestClient, setup_env};

#[test]
fn setup_env_sets_the_conftest_variables() {
    setup_env();
    assert_eq!(std::env::var("TM_FRONTEND_URL").unwrap(), "http://localhost:3000");
    assert_eq!(std::env::var("TM_ADMIN_EMAIL").unwrap(), "admin@test.com");
}

/// 메서드를 하나도 구현하지 않은 대역. trait 기본 구현이 `Unsupported`인지 확인한다.
struct Bare;
impl EsManagerApi for Bare {}

#[test]
fn trait_defaults_report_unsupported() {
    let bare = Bare;
    let docs = Docs::new();
    let fields = serde_json::json!({});
    let categories: Vec<String> = Vec::new();
    let paths: Vec<String> = Vec::new();
    let ids: Vec<i64> = Vec::new();
    let excluded: Vec<String> = Vec::new();
    let args = serde_json::json!({});
    let check = |result: Result<(), EsError>| assert_eq!(result.unwrap_err(), EsError::Unsupported);

    check(bare.create_index().map(|_| ()));
    check(bare.search_by_id(1).map(|_| ()));
    check(bare.update(1, &fields).map(|_| ()));
    check(bare.delete(1).map(|_| ()));
    check(bare.insert(&docs).map(|_| ()));
    check(bare.delete_by_file_paths(&paths, None).map(|_| ()));
    check(bare.delete_by_ids(&ids, 10).map(|_| ()));
    check(bare.refresh());
    check(bare.search_and_aggregate_by_category().map(|_| ()));
    check(bare.iter_all_category_file_paths(10).map(|_| ()));
    check(bare.search_categories_pdf_stats(&categories).map(|_| ()));
    check(bare.search_category_pdf_stats("cat").map(|_| ()));
    check(bare.get_by_id_source("1").map(|_| ()));
    check(bare.search_by_path_source("book.epub").map(|_| ()));
    check(bare.search_by_category("cat", 10).map(|_| ()));
    check(bare.search_by_category_paged("cat", 10, None).map(|_| ()));
    check(bare.count_by_categories(&categories, false).map(|_| ()));
    check(bare.count_by_category("cat", false).map(|_| ()));
    check(bare.rename_category("old", "new").map(|_| ()));
    check(bare.delete_by_category("cat", false).map(|_| ()));
    check(bare.search_by_keyword("word", 10).map(|_| ()));
    check(bare.search_by_keyword_paged(&args).map(|_| ()));
    check(bare.search_similar_docs(&args).map(|_| ()));
    check(bare.search_similar_docs_paged(&args).map(|_| ()));
    check(bare.search_latest_docs(10, None).map(|_| ()));
    check(bare.backfill_created_time(std::path::Path::new("/x")).map(|_| ()));
    check(bare.aggregate_cover_categories().map(|_| ()));
    check(bare.search_cover_files_paged(&categories, 10, None, &excluded).map(|_| ()));
    check(bare.search_group_representatives(&categories, &excluded).map(|_| ()));
}

#[test]
fn fake_es_manager_answers_the_methods_it_implements() {
    let es = FakeEsManager::new();
    assert!(es.refresh().is_ok());
    assert!(es.backfill_created_time(std::path::Path::new("/x")).is_ok());
}

#[tokio::test]
async fn test_client_reaches_the_app_router() {
    let resp = TestClient::new().get("/definitely-not-a-route").await;
    assert_eq!(resp.status, 404);
}
