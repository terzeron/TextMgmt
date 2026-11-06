//! `book_manager::api_adapter` 단위 테스트. 라우트가 쓰는 `BookManagerApi`가 inherent 메서드에 그대로 이어지는지 본다.

#[path = "common/mod.rs"]
mod common;

use std::path::Path;
use std::sync::Arc;

use book_manager::BookManagerApi;
use book_manager::{BookManager, BookManagerDeps};
use common::es_fake::FakeEsManager;

fn manager(root: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    common::setup_env();
    BookManager::with_deps(BookManagerDeps::new(root.to_path_buf(), es.clone()))
}

#[tokio::test]
async fn the_facade_exposes_the_manager_root() {
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let manager = manager(tmp.path(), &es);
    assert_eq!(BookManagerApi::path_prefix(&manager), manager.path_prefix());
}

#[tokio::test]
async fn facade_lookups_return_what_the_inherent_methods_return() {
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let manager = manager(tmp.path(), &es);
    let via_facade = BookManagerApi::get_book(&manager, 404).await;
    let direct = manager.get_book(404).await;
    assert_eq!(via_facade.is_err(), direct.is_err());
    assert_eq!(format!("{:?}", via_facade.err()), format!("{:?}", direct.err()));
    let categories = BookManagerApi::get_categories(&manager).await.unwrap();
    assert_eq!(categories, serde_json::json!(manager.get_categories().await.unwrap()));
}

#[tokio::test]
async fn facade_reloads_return_what_the_inherent_methods_return() {
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let manager = manager(tmp.path(), &es);

    let via = BookManagerApi::reload_category(&manager, "", "book").await;
    let direct = manager.reload_category("", Some("book")).await;
    assert_eq!(format!("{:?}", via.as_ref().err()), format!("{:?}", direct.as_ref().err()));
    assert!(via.is_err(), "빈 카테고리 이름은 오류다");

    let via = BookManagerApi::reload_category_mismatch_files(&manager, "", "book", None).await;
    let direct = manager.reload_category_mismatch_files_for("", "book", None).await;
    assert_eq!(format!("{:?}", via.as_ref().err()), format!("{:?}", direct.as_ref().err()));
    assert!(via.is_err());

    let via = BookManagerApi::reload_category_mismatches(&manager, "book", None).await;
    let direct = manager.reload_category_mismatches_for("book", None).await;
    assert_eq!(via.is_ok(), direct.is_ok());
    assert_eq!(format!("{:?}", via.as_ref().err()), format!("{:?}", direct.as_ref().err()));
}
