//! `backend::book_manager::category_reloader` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;

#[path = "../common/book_fixtures.rs"]
mod fx;
use crate::common::shared_fixtures::{LoaderReadError, ParserTimeout, file_reader};
use crate::common::shared_fixtures::{asyncio_runner, deps, make_doc, make_manager, outcome, tmp_dir};
use backend::book_manager::BookManager;
use common::es_fake::FakeEsManager;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;

// ---- coverage: reload_category edge cases ----

#[test]
fn test_reload_category_empty_name() {
    // Line 1032: empty category name
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("", None)));
    let err = err.expect("err is not None");
    assert!(err.contains("비어있습니다"));
}

#[test]
fn test_reload_category_path_traversal() {
    // Line 1034: category with '..'
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("../etc", None)));
    assert!(err.is_some());
}

#[test]
fn test_reload_category_path_escape() {
    // Line 1038: resolved path escapes path_prefix
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let evil = tmp_path.join("evil");
    symlink("/tmp", &evil).unwrap();
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("evil", None)));
    assert!(err.is_some());
}

#[test]
fn test_reload_category_not_a_directory() {
    // Lines 1040-1041: path is not a directory
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::write(tmp_path.join("notadir"), "file").unwrap();
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("notadir", None)));
    let err = err.expect("err is not None");
    assert!(err.contains("디렉토리를 찾을 수 없습니다"));
}

#[test]
fn test_mismatch_and_reload_more_edge_cases() {
    // Lines 1780-1781, 1905-1906, 1909-1912, 1921, 1935, 1951-1952, 1964-1965, 1989, 1995, 2012, 2029-2030, 2042-2043, 2063, 2065, 2087-2088
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());

    // 1780-1781: unsafe category in get_category_mismatch_details
    let details = manager.get_category_mismatch_details("../unsafe");
    assert_eq!(details, json!({"es_only": [], "fs_only": [], "duplicates": [], "fs_count": 0}));

    // 1905-1906, 1909-1912, 1921, 1935: _bulk_index_files edge cases

    // non-existent file (1905-1906)
    let (_idx1, fail1) = asyncio_runner(manager._bulk_index_files(&["A/non_existent.txt"], true));
    assert_eq!(fail1.len(), 1);
    assert!(fail1[0]["error"].as_str().unwrap().contains("파일을 찾을 수 없습니다"));

    // ParserTimeout (1909-1912)
    let f_timeout = tmp_path.join("A").join("timeout.txt");
    fs::create_dir_all(f_timeout.parent().unwrap()).unwrap();
    fs::write(&f_timeout, "timeout").unwrap();

    // 단계마다 `with_file_reader`가 달라 매니저를 새로 만든다(구 전역 가드의 생명주기).
    let manager_timeout =
        BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_file_reader(file_reader(|_path| Err(LoaderReadError::Timeout(ParserTimeout { message: "read timed out".to_string() })))));
    let (_idx2, fail2) = asyncio_runner(manager_timeout._bulk_index_files(&["A/timeout.txt"], true));
    assert_eq!(fail2.len(), 1);
    assert!(fail2[0]["error"].as_str().unwrap().contains("파싱 시간 초과"));

    // merged is empty (1921)
    let manager_empty = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_file_reader(file_reader(|_path| Ok(json!({})))));
    let (idx3, _fail3) = asyncio_runner(manager_empty._bulk_index_files(&["A/timeout.txt"], true));
    assert!(idx3.is_empty());

    // ES insert failure (1935)
    let f_ok = tmp_path.join("A").join("ok.txt");
    fs::write(&f_ok, "ok").unwrap();

    let fail_insert_es = FakeEsManager::new();
    fail_insert_es.with(|s| s.insert_returns_empty = true); // 999 not in indexed_ids
    let manager_fail_ins = BookManager::with_deps(deps(tmp_path, &fail_insert_es).with_file_reader(file_reader(|_path| Ok(json!({"999": make_doc("A/ok.txt", ".txt")})))));
    let (_idx4, fail4) = asyncio_runner(manager_fail_ins._bulk_index_files(&["A/ok.txt"], true));
    assert_eq!(fail4.len(), 1);
    assert!(fail4[0]["error"].as_str().unwrap().contains("ES 적재 실패"));

    // 1951-1952, 1964-1965, 1989, 1995, 2012: _reload_category_mismatch_details branches
    let bad_details = json!({
        "fs_only": [{"file_path": ""}], // 1951-1952: empty file_path
        "duplicates": [
            {"file_path": "", "file_exists": true, "docs": []}, // 1964-1965: empty file_path
            {"file_path": "A/missing_idx.txt", "file_exists": true, "docs": [{"book_id": "not_an_int"}]}, // 1989: new_book_id is None, 1995: not int
        ],
        "es_only": [{"book_id": 888}],
    });

    let partial_del_es = FakeEsManager::new();
    partial_del_es.with(|s| s.delete_by_ids_returns = Some(0)); // deleted < len(ids_to_delete) -> 2012
    let manager_partial_del = make_manager(tmp_path, &partial_del_es);
    let res_bad = asyncio_runner(manager_partial_del._reload_category_mismatch_details("A", &bad_details));
    let failures = res_bad["failures"].as_array().unwrap();
    assert!(failures.iter().any(|f| f.get("error").and_then(Value::as_str).unwrap_or("").contains("file_path가 비어있습니다")));
    assert!(failures.iter().any(|f| f.get("error").and_then(Value::as_str).unwrap_or("").contains("삭제됨")));

    // 2029-2030, 2042-2043: reload_category_mismatches unsafe category and refresh exception
    let refresh_err_es = FakeEsManager::new();
    refresh_err_es.fail("refresh", "refresh error");
    // 가정한 seam: 인스턴스 메서드 바꿔치기(문서에 없는 seam, 파일 머리말 참고).
    let manager_ref_err = BookManager::with_deps(deps(tmp_path, &refresh_err_es).with_get_category_mismatches_override(Arc::new(|_force_refresh| json!({"mismatches": [{"category": "../unsafe"}]}))));
    let (res_mismatches, _) = outcome(asyncio_runner(manager_ref_err.reload_category_mismatches(None)));
    let failures = res_mismatches["failures"].as_array().unwrap();
    assert!(failures.iter().any(|f| f.to_string().contains("잘못된 카테고리 경로입니다")));
    assert!(failures.iter().any(|f| f.to_string().contains("ES refresh 실패")));

    // 2063, 2065, 2087-2088: reload_category_mismatch_files validation and refresh exception
    let (_, err_empty) = outcome(asyncio_runner(manager.reload_category_mismatch_files("", None)));
    assert_eq!(err_empty.as_deref(), Some("카테고리 이름이 비어있습니다"));
    let (_, err_unsafe) = outcome(asyncio_runner(manager.reload_category_mismatch_files("../unsafe", None)));
    assert_eq!(err_unsafe.as_deref(), Some("잘못된 카테고리 경로입니다"));

    let manager_ref_details =
        BookManager::with_deps(deps(tmp_path, &refresh_err_es).with_get_category_mismatch_details_override(Arc::new(|_cat| json!({"fs_only": [], "es_only": [], "duplicates": []}))));
    let (res_single_reload, _) = outcome(asyncio_runner(manager_ref_details.reload_category_mismatch_files("A", None)));
    assert!(res_single_reload["failures"].as_array().unwrap().iter().any(|f| f.to_string().contains("ES refresh 실패")));
}

#[test]
fn test_reload_category_mismatch_files_non_int_book_id() {
    // Line 1995
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // 가정한 seam: 인스턴스 메서드 바꿔치기(문서에 없는 seam, 파일 머리말 참고).
    let manager = BookManager::with_deps(
        deps(tmp_path, &es)
            .with_get_category_mismatch_details_override(Arc::new(|_cat| json!({"duplicates": [{"file_path": "cat/dup.txt", "docs": [{"book_id": "bad_id"}]}], "fs_only": [], "es_only": []}))),
    );
    fs::create_dir_all(tmp_path.join("cat")).unwrap();
    asyncio_runner(manager.reload_category_mismatch_files("cat", None)).unwrap();
}

mod root_file_reload {
    use super::common;
    use crate::common::shared_fixtures::{make_deps, outcome};

    use std::fs;

    use std::path::Path;
    use std::sync::Arc;

    use backend::book_manager::BookManager;

    use common::es_fake::FakeEsManager;
    use common::faulty_fs::FaultyFs;

    use serde_json::json;
    use tempfile::TempDir;

    // ---- _root 재적재 ----

    #[tokio::test]
    async fn test_reload_root_files_reports_unreadable_root() {
        let tmp = TempDir::new().unwrap();
        // 디렉토리 열거(scandir)가 PermissionError로 실패하는 파일 시스템
        let fs_ops = FaultyFs::new();
        fs_ops.set_read_dir(|_p: &Path| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "읽기 거부")));
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));

        let (result, err) = outcome(manager._reload_root_files().await);

        assert_eq!(result, json!({}));
        assert!(err.unwrap().contains("최상위 디렉토리를 읽을 수 없습니다"));
    }

    #[tokio::test]
    async fn test_reload_root_files_reports_refresh_and_batch_failures() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        fs::write(tmp.path().join("top.txt"), "x").unwrap();
        es.fail("refresh", "refresh 불가");

        // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 색인 함수를 교체하는 훅
        let deps = make_deps(tmp.path(), es).with_bulk_index_files_hook(Arc::new(|_names: &[String], _clean_existing: bool| (std::collections::BTreeMap::new(), vec![json!({"error": "색인 실패"})])));
        let manager = BookManager::with_deps(deps);

        let (result, err) = outcome(manager._reload_root_files().await);

        assert_eq!(result, json!({}));
        assert_eq!(err.as_deref(), Some("최상위 파일 재적재 실패 2건: 색인 실패"));
    }
}
