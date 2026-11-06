//! `book_manager::category_editor` 단위 테스트.

#[path = "common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{block_on, make_manager, outcome_or_default, tmp_dir};
use book_manager::CategoryRename;
use common::es_fake::FakeEsManager;
use search_manager::DeleteByQueryResult;
use std::fs;
use std::os::unix::fs::symlink;

// ---- coverage: rename_category new_category path traversal ----

#[test]
fn test_rename_category_new_path_traversal() {
    // new_category resolved path escapes path_prefix
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("old", 1)]);
    let evil = tmp_path.join("evil");
    symlink("/tmp", &evil).unwrap();
    let (_result, err) = outcome_or_default(block_on(manager.rename_category("old", "evil")));
    assert!(err.is_some());
}

// ---- coverage: rename_category old/new count checks ----

#[test]
fn test_rename_category_old_count_zero() {
    // old_category has 0 documents
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 0), ("new", 0)]);
    let (_result, err) = outcome_or_default(block_on(manager.rename_category("old", "new")));
    assert!(err.expect("err").contains("문서가 없습니다"));
}

#[test]
fn test_rename_category_new_count_nonzero() {
    // new_category already has documents
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 1), ("new", 5)]);
    let (_result, err) = outcome_or_default(block_on(manager.rename_category("old", "new")));
    let err = err.expect("err");
    assert!(err.contains("이미"));
    assert!(err.contains('5'));
}

#[test]
fn test_rename_category_counts_subcategory_documents() {
    // 문서가 하위 카테고리에만 있어도 디렉토리는 존재하므로 이름을 바꿀 수 있어야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir(tmp_path.join("A")).unwrap();
    es.set_counts(&[("A/sub", 3)]);

    let result = block_on(manager.rename_category("A", "B")).unwrap();

    assert!(result.fs_renamed);
    assert!(tmp_path.join("B").is_dir());
}

#[test]
fn test_rename_category_blocks_target_with_subcategory_documents() {
    // 대상 카테고리가 하위에만 문서를 가져도 충돌이다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir(tmp_path.join("A")).unwrap();
    es.set_counts(&[("A", 2), ("B/sub", 5)]);

    let (result, err) = outcome_or_default(block_on(manager.rename_category("A", "B")));

    assert_eq!(result, CategoryRename::default());
    assert!(err.expect("err").contains("이미"));
    assert!(tmp_path.join("A").is_dir());
}

#[test]
fn test_delete_category_success() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir_all(tmp_path.join("test_cat")).unwrap();
    es.set_counts(&[("test_cat", 5)]);
    es.with(|s| s.deleted_by_category = DeleteByQueryResult { deleted: 5, failures: vec![] });
    let res = block_on(manager.delete_category("test_cat", false)).unwrap();
    assert_eq!(res.deleted_count, 5);
    assert_eq!(res.category, "test_cat");
}
