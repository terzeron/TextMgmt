#[path = "../common/mod.rs"]
mod common;

use backend::es_manager::EsManagerApi;
use common::es_fake::FakeEsManager;
use common::{TestClient, setup_env};

#[test]
fn setup_env_sets_the_test_variables() {
    setup_env();
    assert_eq!(std::env::var("TM_FRONTEND_URL").unwrap(), "http://localhost:3000");
    assert_eq!(std::env::var("TM_ADMIN_EMAIL").unwrap(), "admin@test.com");
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
