mod common;
use book_manager::comics_manager::ComicsManager;
use common::es_fake::FakeEsManager;
use search_manager::EsManagerApi;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    move |key: &str| map.get(key).cloned()
}

fn build(pairs: &[(&str, &str)], es: &Arc<FakeEsManager>) -> Result<ComicsManager, book_manager::comics_manager::BookManagerError> {
    let env = env_of(pairs);
    let es = es.clone();
    ComicsManager::from_env(&env, &move |_| es.clone() as Arc<dyn EsManagerApi>)
}

const BASE: [(&str, &str); 2] = [("TM_COMICS_DIR", "/data/comics"), ("TM_ES_COMICS_INDEX", "idx")];

fn with_flag(flag: &str) -> Vec<(&str, &str)> {
    let mut v = BASE.to_vec();
    v.push(("TM_BACKFILL_CREATED_TIME_ON_STARTUP", flag));
    v
}

#[test]
fn test_from_env_sets_root_and_index_and_starts_with_empty_mismatch_cache() {
    let es = FakeEsManager::new();
    let manager = build(&BASE, &es).unwrap();
    assert_eq!(manager.path_prefix(), std::path::Path::new("/data/comics"));
    assert_eq!(manager.es_index_name(), "idx");
    assert_eq!(es.create_index_count(), 1);
    assert!(manager.mismatch_cache().is_none());
    assert_eq!(manager.mismatch_cache_time(), 0.0);
}

#[test]
fn test_missing_comics_dir_does_not_touch_es() {
    let es = FakeEsManager::new();
    let err = build(&[("TM_ES_COMICS_INDEX", "idx")], &es).err().expect("실패해야 한다");
    assert!(err.to_string().contains("TM_COMICS_DIR"), "{err}");
    assert_eq!(es.create_index_count(), 0);
}

#[test]
fn test_missing_index_env_is_error_before_es_calls() {
    let es = FakeEsManager::new();
    let err = build(&[("TM_COMICS_DIR", "/data/comics")], &es).err().expect("실패해야 한다");
    assert!(err.to_string().contains("TM_ES_COMICS_INDEX"), "{err}");
    assert_eq!(es.create_index_count(), 0);
}

#[test]
fn test_create_index_failure_is_error() {
    let es = FakeEsManager::new();
    es.fail("create_index", "boom");
    let err = build(&BASE, &es).err().expect("실패해야 한다");
    assert!(err.to_string().contains("boom"), "{err}");
}

#[test]
fn test_backfill_is_off_by_default() {
    let es = FakeEsManager::new();
    let m = build(&BASE, &es).unwrap();
    m.join_backfill();
    assert_eq!(es.call_count("backfill_created_time"), 0);
}

#[test]
fn test_backfill_runs_once_with_path_prefix_when_enabled() {
    for flag in ["1", "true", "YES", " On "] {
        let es = FakeEsManager::new();
        let m = build(&with_flag(flag), &es).unwrap();
        m.join_backfill();
        assert_eq!(es.calls_of("backfill_created_time"), [json!(["/data/comics"])], "flag={flag:?}");
    }
}

#[test]
fn test_backfill_ignores_falsy_flag_values() {
    for flag in ["0", "", "no", "off", "2"] {
        let es = FakeEsManager::new();
        let m = build(&with_flag(flag), &es).unwrap();
        m.join_backfill();
        assert_eq!(es.call_count("backfill_created_time"), 0, "flag={flag:?}");
    }
}

#[test]
fn test_backfill_failure_is_swallowed() {
    let es = FakeEsManager::new();
    es.fail("backfill_created_time", "backfill boom");
    let m = build(&with_flag("1"), &es).unwrap();
    m.join_backfill();
    assert_eq!(es.call_count("backfill_created_time"), 1);
}

#[test]
fn test_backfill_runs_after_create_index() {
    let es = FakeEsManager::new();
    let m = build(&with_flag("1"), &es).unwrap();
    m.join_backfill();
    let names: Vec<String> = es.calls().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["create_index", "backfill_created_time"]);
}

#[test]
fn test_mismatch_cache_set_stamps_time_and_clear_resets() {
    let es = FakeEsManager::new();
    let mut m = build(&BASE, &es).unwrap();
    m.set_mismatch_cache(json!({"n": 1}));
    assert_eq!(m.mismatch_cache(), Some(&json!({"n": 1})));
    assert!(m.mismatch_cache_time() > 0.0);
    m.clear_mismatch_cache();
    assert!(m.mismatch_cache().is_none());
    assert_eq!(m.mismatch_cache_time(), 0.0);
}

#[test]
fn test_mismatch_locks_are_independent() {
    let es = FakeEsManager::new();
    let m = build(&BASE, &es).unwrap();
    let _scan = m.mismatch_scan_lock().lock().unwrap();
    // scan lock을 잡은 채로 state lock을 잡을 수 있어야 한다.
    assert!(m.mismatch_state_lock().try_lock().is_ok());
}

#[test]
fn test_es_factory_receives_index_name_once() {
    let es = FakeEsManager::new();
    let names = Arc::new(Mutex::new(Vec::<String>::new()));
    let (es2, names2) = (es.clone(), names.clone());
    let env = env_of(&BASE);
    ComicsManager::from_env(&env, &move |n: &str| {
        names2.lock().unwrap().push(n.to_string());
        es2.clone() as Arc<dyn EsManagerApi>
    })
    .unwrap();
    assert_eq!(*names.lock().unwrap(), ["idx"]);
}

#[test]
fn test_from_env_with_applies_manager_customization() {
    let es = FakeEsManager::new();
    let env = env_of(&BASE);
    let mut customized = false;
    let manager = ComicsManager::from_env_with(&env, &|_| es.clone() as Arc<dyn EsManagerApi>, |deps| {
        customized = true;
        deps.with_bulk_reindex_batch_size(17)
    })
    .unwrap();
    assert_eq!(manager.path_prefix(), std::path::Path::new("/data/comics"));
    assert!(customized);
}

#[test]
fn test_from_env_with_es_applies_manager_customization() {
    let es = FakeEsManager::new();
    let env = env_of(&BASE);
    let mut customized = false;
    let manager = ComicsManager::from_env_with_es(&env, es.clone(), |deps| {
        customized = true;
        deps.with_bulk_reindex_batch_size(17)
    })
    .unwrap();
    assert_eq!(manager.path_prefix(), std::path::Path::new("/data/comics"));
    assert!(customized);
    assert_eq!(es.create_index_count(), 1);
}

// ---- api_adapter (하위 모듈을 합치며 옮긴 테스트) ----

mod api_adapter {
    //! `book_manager::comics_manager` 단위 테스트.

    use crate::common::es_fake::FakeEsManager;
    use crate::common::shared_fixtures::outcome;
    use book_manager::BookManagerDeps;
    use book_manager::comics_manager::ComicsManager;
    use book_manager::{ApplyCategoryChanges, BookManagerApi, ProposeCategoryChanges};
    use serde_json::json;

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
}
