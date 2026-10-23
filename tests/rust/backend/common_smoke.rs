//! `common/` 공용 도구가 컴파일되고 기본 동작을 하는지 확인한다. 다른 테스트 binary가 같은 모듈을 쓰므로 여기서 먼저 깨진다.

mod common;

use backend::es_manager::{EsError, EsManagerApi};
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
    assert_eq!(Bare.refresh().unwrap_err(), EsError::Unsupported);
    assert_eq!(Bare.backfill_created_time(std::path::Path::new("/x")).unwrap_err(), EsError::Unsupported);
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
