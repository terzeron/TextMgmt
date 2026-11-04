//! `backend::comics_manager::api_adapter` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use backend::book_manager::BookManagerDeps;
use backend::comics_manager::ComicsManager;
use backend::main::{ApplyCategoryChanges, BookManagerApi, ProposeCategoryChanges};
use crate::common::es_fake::FakeEsManager;
use serde_json::json;

/// `Result<Value, _>`를 Python 원본의 `(결과, 오류)`로 펼친다. 실패하면 원본처럼 빈 객체다.
fn outcome(result: Result<serde_json::Value, String>) -> (serde_json::Value, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::json!({}), Some(error)),
    }
}

#[tokio::test]
async fn test_category_proposals_delegate_to_shared_comics_logic() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("0_inbox")).unwrap();
    let es = FakeEsManager::new();
    let manager = ComicsManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es));
    let mappings = serde_json::json!({});

    let (result, error) = outcome(
        BookManagerApi::propose_category_changes(
            &manager,
            ProposeCategoryChanges { category: "0_inbox", mappings: &mappings, content_type: "comic", use_bookstore: false, use_content_meta: false, delay: 0.0, on_progress: None },
        )
        .await,
    );

    assert_eq!(error, None);
    assert_ne!(result, serde_json::Value::Null);
}

#[tokio::test]
async fn test_apply_category_changes_preserves_comics_cancellation_callback() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("1_source")).unwrap();
    std::fs::create_dir(tmp.path().join("2_target")).unwrap();
    std::fs::write(tmp.path().join("1_source/book.txt"), "comic").unwrap();
    let es = FakeEsManager::new();
    let manager = ComicsManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es));
    let items = [serde_json::json!({"file_path": "1_source/book.txt", "target_category": "2_target"})];
    let allowed = ["1_source/book.txt".to_string()].into();
    let callback_calls = std::sync::atomic::AtomicUsize::new(0);
    let should_continue = || {
        callback_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        false
    };

    let (result, error) = outcome(
        BookManagerApi::apply_category_changes_with(
            &manager,
            ApplyCategoryChanges {
                items: &items,
                allowed_file_paths: &allowed,
                content_type: "comic",
                clean_existing: false,
                on_progress: None,
                on_item_done: None,
                should_continue: Some(&should_continue),
            },
        )
        .await,
    );

    assert_eq!(error, None);
    assert_eq!(result["applied_count"], 0);
    assert_eq!(callback_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(tmp.path().join("1_source/book.txt").is_file());
}

#[tokio::test]
async fn comics_book_manager_api_forwards_shared_operations_to_its_inner_manager() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = ComicsManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), FakeEsManager::new()));
    let mappings = json!({});
    let items: [serde_json::Value; 0] = [];
    let allowed = std::collections::HashSet::new();
    let exclusions: Vec<String> = Vec::new();
    let path = tmp.path().join("missing.pdf");

    let _ = manager.path_prefix();
    let _ = <ComicsManager as BookManagerApi>::es_manager(&manager);
    let _ = BookManagerApi::propose_category_changes(
        &manager,
        ProposeCategoryChanges { category: "missing", mappings: &mappings, content_type: "book", use_bookstore: false, use_content_meta: false, delay: 0.0, on_progress: None },
    )
    .await;
    let _ = BookManagerApi::index_single_file(&manager, "missing.txt").await;
    let _ = BookManagerApi::delete_file(&manager, "missing.txt").await;
    let _ = BookManagerApi::reload_category(&manager, "missing", "book").await;
    let _ = BookManagerApi::reload_category_mismatch_files(&manager, "missing", "book", None).await;
    let _ = BookManagerApi::reload_category_mismatches(&manager, "book", None).await;
    let _ = BookManagerApi::apply_category_changes(&manager, &items, &allowed, false).await;
    let _ = BookManagerApi::apply_category_changes_with(
        &manager,
        ApplyCategoryChanges { items: &items, allowed_file_paths: &allowed, content_type: "book", clean_existing: false, on_progress: None, on_item_done: None, should_continue: None },
    )
    .await;
    let _ = BookManagerApi::delete_book(&manager, -1).await;
    let _ = BookManagerApi::rename_category(&manager, "missing", "renamed").await;
    let _ = BookManagerApi::delete_category(&manager, "missing", false).await;
    let _ = BookManagerApi::preview_delete_category(&manager, "missing").await;
    let _ = BookManagerApi::preview_insert_image(&manager, "not a URL").await;
    let _ = BookManagerApi::preview_insert_upload(&manager, b"not an image").await;
    let _ = BookManagerApi::delete_pdf_pages(&manager, -1, &[], None, None, None, None).await;
    let _ = BookManagerApi::get_book(&manager, -1).await;
    let _ = BookManagerApi::get_categories(&manager).await;
    let _ = BookManagerApi::get_books_in_category_paged(&manager, "missing", Some(1), None).await;
    let _ = BookManagerApi::get_book_content(&manager, -1).await;
    let _ = BookManagerApi::get_book_preview(&manager, -1, Some(1), None).await;
    let _ = BookManagerApi::get_book_preview_with_pages(&manager, -1, Some(1), Some(1), None).await;
    let _ = BookManagerApi::get_pdf_pages(&manager, -1, 0, 1).await;
    let _ = BookManagerApi::get_html_resource(&manager, -1, "missing.html").await;
    let _ = BookManagerApi::get_cover(&manager, -1, Some(100)).await;
    let _ = BookManagerApi::validate_epub(&manager, -1).await;
    let _ = BookManagerApi::validate_pdf(&manager, -1).await;
    let _ = BookManagerApi::get_books_in_category(&manager, "missing").await;
    let _ = BookManagerApi::get_latest_books(&manager, 1, Some(&exclusions)).await;
    let _ = BookManagerApi::search_by_keyword_paged(&manager, "keyword", 1, 0, Some(&exclusions), None).await;
    let _ = BookManagerApi::search_similar_books_paged(&manager, -1, 1, 0).await;
    let _ = BookManagerApi::get_cover_books_paged(&manager, "missing", 1, "", Some(&exclusions)).await;
    let _ = BookManagerApi::get_uncovered_books_paged(&manager, "missing", 1, "", Some(&exclusions)).await;
    let _ = BookManagerApi::get_category_mismatches(&manager, false);
    let _ = BookManagerApi::get_category_mismatch_details(&manager, "missing");
    let _ = BookManagerApi::set_epub_cover(&manager, -1, None, None).await;
    let _ = BookManagerApi::edit_epub_images(&manager, -1, &[], &[], &[], &[]).await;
    let _ = BookManagerApi::remove_epub_cover(&manager, -1).await;
    let _ = BookManagerApi::insert_epub_illustration(&manager, -1, "before", None, None).await;
    let _ = BookManagerApi::update_book(&manager, -1, "missing", "title", "author", &path, "pdf", false).await;
}
