//! `backend::book_manager::category_editor` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
use backend::book_manager::{BookManager, BookManagerDeps};
use common::es_fake::FakeEsManager;
use serde_json::json;
use tempfile::TempDir;

/// `Result<Value, _>`를 Python 원본의 `(결과, 오류)`로 펼친다. 실패하면 원본처럼 빈 객체다.
fn outcome(result: Result<serde_json::Value, String>) -> (serde_json::Value, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::json!({}), Some(error)),
    }
}

// ---------------------------------------------------------------------------
// 공용 헬퍼 (전반부 정의를 복사)
// ---------------------------------------------------------------------------

/// pytest `tmp_path`에 대응한다. 반환값이 drop되면 디렉토리가 지워진다.
fn tmp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `asyncio.run(coro)`에 대응한다. 동기 테스트 안에서 future를 끝까지 돌린다.
fn asyncio_runner<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

/// 기본 구성의 `BookManagerDeps`. `with_*`를 더 붙인 뒤 `BookManager::with_deps`로 만든다.
fn deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    common::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(deps(tmp_path, es))
}

// ---- coverage: rename_category new_category path traversal ----

#[test]
fn test_rename_category_new_path_traversal() {
    // Line 1149: new_category resolved path escapes path_prefix
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("old", 1)]);
    let evil = tmp_path.join("evil");
    symlink("/tmp", &evil).unwrap();
    let (_result, err) = outcome(asyncio_runner(manager.rename_category("old", "evil")));
    assert!(err.is_some());
}

// ---- coverage: rename_category old/new count checks ----

#[test]
fn test_rename_category_old_count_zero() {
    // Line 1155: old_category has 0 documents
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 0), ("new", 0)]);
    let (_result, err) = outcome(asyncio_runner(manager.rename_category("old", "new")));
    assert!(err.expect("err").contains("문서가 없습니다"));
}

#[test]
fn test_rename_category_new_count_nonzero() {
    // Line 1159: new_category already has documents
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 1), ("new", 5)]);
    let (_result, err) = outcome(asyncio_runner(manager.rename_category("old", "new")));
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

    let result = asyncio_runner(manager.rename_category("A", "B")).unwrap();

    assert_eq!(result["fs_renamed"], json!(true));
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

    let (result, err) = outcome(asyncio_runner(manager.rename_category("A", "B")));

    assert_eq!(result, json!({}));
    assert!(err.expect("err").contains("이미"));
    assert!(tmp_path.join("A").is_dir());
}

#[test]
fn test_delete_category_success() {
    // Lines 2326-2328
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir_all(tmp_path.join("test_cat")).unwrap();
    es.set_counts(&[("test_cat", 5)]);
    es.with(|s| s.deleted_by_category = json!({"deleted": 5, "failures": []}));
    let res = asyncio_runner(manager.delete_category("test_cat", false)).unwrap();
    assert_eq!(res["deleted_count"], 5);
    assert_eq!(res["category"], "test_cat");
}
