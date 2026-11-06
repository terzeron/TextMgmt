//! `book_manager::category_reloader` 단위 테스트.

#[path = "common/mod.rs"]
mod common;

#[path = "../shared/book_fixtures.rs"]
mod fx;
use crate::common::shared_fixtures::{LoaderReadError, ParserTimeout, file_reader};
use crate::common::shared_fixtures::{block_on, deps, make_doc, make_manager, outcome_or_default, tmp_dir};
use book_manager::{BookManager, CategoryMismatch, DuplicateDoc, DuplicatePath, EsOnlyFile, FsOnlyFile, MismatchDetails, MismatchSummary};
use common::es_fake::FakeEsManager;
use serde_json::json;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;

// ---- coverage: reload_category edge cases ----

#[test]
fn test_reload_category_empty_name() {
    // empty category name
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (_result, err) = outcome_or_default(block_on(manager.reload_category("", None)));
    let err = err.expect("err is not None");
    assert!(err.contains("비어있습니다"));
}

#[test]
fn test_reload_category_path_traversal() {
    // category with '..'
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (_result, err) = outcome_or_default(block_on(manager.reload_category("../etc", None)));
    assert!(err.is_some());
}

#[test]
fn test_reload_category_path_escape() {
    // resolved path escapes path_prefix
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let evil = tmp_path.join("evil");
    symlink("/tmp", &evil).unwrap();
    let (_result, err) = outcome_or_default(block_on(manager.reload_category("evil", None)));
    assert!(err.is_some());
}

#[test]
fn test_reload_category_not_a_directory() {
    // path is not a directory
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::write(tmp_path.join("notadir"), "file").unwrap();
    let (_result, err) = outcome_or_default(block_on(manager.reload_category("notadir", None)));
    let err = err.expect("err is not None");
    assert!(err.contains("디렉토리를 찾을 수 없습니다"));
}

#[test]
fn test_mismatch_and_reload_more_edge_cases() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());

    // 1780-1781: unsafe category in get_category_mismatch_details
    let details = manager.get_category_mismatch_details("../unsafe");
    assert_eq!(details, MismatchDetails::default());

    // 1905-1906, 1909-1912, 1921, 1935: bulk_index_files edge cases

    // non-existent file (1905-1906)
    let (_idx1, fail1) = block_on(manager.bulk_index_files(&["A/non_existent.txt"], true));
    assert_eq!(fail1.len(), 1);
    assert!(fail1[0].error.contains("파일을 찾을 수 없습니다"));

    // ParserTimeout (1909-1912)
    let f_timeout = tmp_path.join("A").join("timeout.txt");
    fs::create_dir_all(f_timeout.parent().unwrap()).unwrap();
    fs::write(&f_timeout, "timeout").unwrap();

    // 단계마다 `with_file_reader`가 달라 매니저를 새로 만든다(구 전역 가드의 생명주기).
    let manager_timeout =
        BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_file_reader(file_reader(|_path| Err(LoaderReadError::Timeout(ParserTimeout { message: "read timed out".to_string() })))));
    let (_idx2, fail2) = block_on(manager_timeout.bulk_index_files(&["A/timeout.txt"], true));
    assert_eq!(fail2.len(), 1);
    assert!(fail2[0].error.contains("파싱 시간 초과"));

    // merged is empty (1921)
    let manager_empty = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_file_reader(file_reader(|_path| Ok(json!({})))));
    let (idx3, _fail3) = block_on(manager_empty.bulk_index_files(&["A/timeout.txt"], true));
    assert!(idx3.is_empty());

    // ES insert failure (1935)
    let f_ok = tmp_path.join("A").join("ok.txt");
    fs::write(&f_ok, "ok").unwrap();

    let fail_insert_es = FakeEsManager::new();
    fail_insert_es.with(|s| s.insert_returns_empty = true); // 999 not in indexed_ids
    let manager_fail_ins = BookManager::with_deps(deps(tmp_path, &fail_insert_es).with_file_reader(file_reader(|_path| Ok(json!({"999": make_doc("A/ok.txt", ".txt")})))));
    let (_idx4, fail4) = block_on(manager_fail_ins.bulk_index_files(&["A/ok.txt"], true));
    assert_eq!(fail4.len(), 1);
    assert!(fail4[0].error.contains("ES 적재 실패"));

    // 1951-1952, 1964-1965, 1989, 1995, 2012: reload_category_mismatch_details branches
    let bad_details = MismatchDetails {
        fs_only: vec![FsOnlyFile::default()], // empty file_path
        duplicates: vec![
            DuplicatePath { file_path: String::new(), file_exists: true, docs: vec![] }, // empty file_path
            // 색인하지 못한 경로: 새 book_id가 없다
            DuplicatePath { file_path: "A/missing_idx.txt".into(), file_exists: true, docs: vec![DuplicateDoc { book_id: 777, ..DuplicateDoc::default() }] },
        ],
        es_only: vec![EsOnlyFile { book_id: 888, ..EsOnlyFile::default() }],
        ..MismatchDetails::default()
    };

    let partial_del_es = FakeEsManager::new();
    partial_del_es.with(|s| s.delete_by_ids_returns = Some(0)); // deleted < len(ids_to_delete) -> 2012
    let manager_partial_del = make_manager(tmp_path, &partial_del_es);
    let res_bad = block_on(manager_partial_del.reload_category_mismatch_details("A", &bad_details));
    let failures = &res_bad.failures;
    assert!(failures.iter().any(|f| f.error.contains("file_path가 비어있습니다")));
    assert!(failures.iter().any(|f| f.error.contains("삭제됨")));

    // 2029-2030, 2042-2043: reload_category_mismatches unsafe category and refresh exception
    let refresh_err_es = FakeEsManager::new();
    refresh_err_es.fail("refresh", "refresh error");
    // 가정한 seam: 인스턴스 메서드 바꿔치기(문서에 없는 seam, 파일 머리말 참고).
    let manager_ref_err = BookManager::with_deps(deps(tmp_path, &refresh_err_es).with_get_category_mismatches_override(Arc::new(|_force_refresh| MismatchSummary {
        mismatches: vec![CategoryMismatch { category: "../unsafe".into(), ..CategoryMismatch::default() }],
        ..MismatchSummary::default()
    })));
    let (res_mismatches, _) = outcome_or_default(block_on(manager_ref_err.reload_category_mismatches(None)));
    let failures = &res_mismatches.failures;
    assert!(failures.iter().any(|f| f.error.contains("잘못된 카테고리 경로입니다")));
    assert!(failures.iter().any(|f| f.error.contains("ES refresh 실패")));

    // 2063, 2065, 2087-2088: reload_category_mismatch_files validation and refresh exception
    let (_, err_empty) = outcome_or_default(block_on(manager.reload_category_mismatch_files("", None)));
    assert_eq!(err_empty.as_deref(), Some("카테고리 이름이 비어있습니다"));
    let (_, err_unsafe) = outcome_or_default(block_on(manager.reload_category_mismatch_files("../unsafe", None)));
    assert_eq!(err_unsafe.as_deref(), Some("잘못된 카테고리 경로입니다"));

    let manager_ref_details = BookManager::with_deps(deps(tmp_path, &refresh_err_es).with_get_category_mismatch_details_override(Arc::new(|_cat| MismatchDetails::default())));
    let (res_single_reload, _) = outcome_or_default(block_on(manager_ref_details.reload_category_mismatch_files("A", None)));
    assert!(res_single_reload.failures.iter().any(|f| f.error.contains("ES refresh 실패")));
}

#[test]
fn test_reload_category_mismatch_files_handles_duplicate_of_missing_file() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // 가정한 seam: 인스턴스 메서드 바꿔치기(문서에 없는 seam, 파일 머리말 참고).
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_get_category_mismatch_details_override(Arc::new(|_cat| MismatchDetails {
        duplicates: vec![DuplicatePath { file_path: "cat/dup.txt".into(), file_exists: false, docs: vec![DuplicateDoc { book_id: 9, ..DuplicateDoc::default() }] }],
        ..MismatchDetails::default()
    })));
    fs::create_dir_all(tmp_path.join("cat")).unwrap();
    block_on(manager.reload_category_mismatch_files("cat", None)).unwrap();
}

mod root_file_reload {
    use super::common;
    use crate::common::shared_fixtures::{make_deps, outcome_or_default};

    use std::fs;

    use std::path::Path;
    use std::sync::Arc;

    use book_manager::{BookManager, ReloadFailure, ReloadedFiles};

    use common::es_fake::FakeEsManager;
    use common::faulty_fs::FaultyFs;

    use tempfile::TempDir;

    // ---- _root 재적재 ----

    #[tokio::test]
    async fn test_reload_root_files_reports_unreadable_root() {
        let tmp = TempDir::new().unwrap();
        // 디렉토리 열거(scandir)가 PermissionError로 실패하는 파일 시스템
        let fs_ops = FaultyFs::new();
        fs_ops.set_read_dir(|_p: &Path| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "읽기 거부")));
        let manager = BookManager::with_deps(make_deps(tmp.path(), FakeEsManager::new()).with_fs(fs_ops));

        let (result, err) = outcome_or_default(manager.reload_book_root_files().await);

        assert_eq!(result, ReloadedFiles::default());
        assert!(err.unwrap().contains("최상위 디렉토리를 읽을 수 없습니다"));
    }

    #[tokio::test]
    async fn test_reload_root_files_reports_refresh_and_batch_failures() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        fs::write(tmp.path().join("top.txt"), "x").unwrap();
        es.fail("refresh", "refresh 불가");

        // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 색인 함수를 교체하는 훅
        let deps = make_deps(tmp.path(), es).with_bulk_index_files_hook(Arc::new(|_names: &[String], _clean_existing: bool| {
            (std::collections::BTreeMap::new(), vec![ReloadFailure { error: "색인 실패".into(), ..ReloadFailure::default() }])
        }));
        let manager = BookManager::with_deps(deps);

        let (result, err) = outcome_or_default(manager.reload_book_root_files().await);

        assert_eq!(result, ReloadedFiles::default());
        assert_eq!(err.as_deref(), Some("최상위 파일 재적재 실패 2건: 색인 실패"));
    }
}

// ---- ES·파서 실패 분기 ----

/// `A/ok.txt`를 만들고, 파일 읽기 대역이 `reader`를 쓰는 매니저를 만든다.
fn manager_with_reader(tmp_path: &std::path::Path, es: &Arc<FakeEsManager>, reader: impl Fn(&std::path::Path) -> Result<serde_json::Value, LoaderReadError> + Send + Sync + 'static) -> BookManager {
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    fs::write(tmp_path.join("A").join("ok.txt"), "ok").unwrap();
    BookManager::with_deps(deps(tmp_path, es).with_file_reader(file_reader(reader)))
}

#[test]
fn test_bulk_index_files_reports_a_parse_failure_per_file() {
    let tmp = tmp_dir();
    let manager = manager_with_reader(tmp.path(), &FakeEsManager::new(), |_| Err(LoaderReadError::Other("broken".to_string())));
    let (indexed, failures) = block_on(manager.bulk_index_files(&["A/ok.txt"], true));
    assert!(indexed.is_empty());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error, "파일 읽기 실패: A/ok.txt");
}

#[test]
fn test_bulk_index_files_still_indexes_when_cleaning_stale_documents_fails() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    es.fail("delete_by_file_paths", "down");
    let manager = manager_with_reader(tmp.path(), &es, |_| Ok(json!({"999": make_doc("A/ok.txt", ".txt")})));
    let (indexed, failures) = block_on(manager.bulk_index_files(&["A/ok.txt"], true));
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(indexed.len(), 1);
}

#[test]
fn test_bulk_index_files_reports_an_es_insert_error_as_a_load_failure() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    es.fail("insert", "down");
    let manager = manager_with_reader(tmp.path(), &es, |_| Ok(json!({"999": make_doc("A/ok.txt", ".txt")})));
    let (indexed, failures) = block_on(manager.bulk_index_files(&["A/ok.txt"], true));
    assert!(indexed.is_empty());
    assert!(failures.iter().any(|f| f.error.contains("ES 적재 실패")), "{failures:?}");
}

#[test]
fn test_reload_mismatch_details_counts_nothing_deleted_when_es_delete_fails() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    es.fail("delete_by_ids", "down");
    let manager = make_manager(tmp.path(), &es);
    let details = MismatchDetails { es_only: vec![EsOnlyFile { book_id: 888, ..EsOnlyFile::default() }], ..MismatchDetails::default() };
    let result = block_on(manager.reload_category_mismatch_details("A", &details));
    assert_eq!(result.deleted_count, 0);
    assert!(result.failures.iter().any(|f| f.error == "ES 문서 1건 중 0건만 삭제됨"), "{:?}", result.failures);
}

#[test]
fn test_reload_category_mismatches_reports_a_summary_failure() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    es.fail("iter_all_category_file_paths", "down");
    let (_, err) = outcome_or_default(block_on(make_manager(tmp.path(), &es).reload_category_mismatches(None)));
    assert!(err.as_deref().is_some_and(|e| e.starts_with("불일치 조회 실패")), "{err:?}");
}

#[test]
fn test_reload_category_mismatches_rejects_a_category_whose_details_failed() {
    let tmp = tmp_dir();
    fs::create_dir_all(tmp.path().join("A")).unwrap();
    let manager = BookManager::with_deps(
        deps(tmp.path(), &FakeEsManager::new())
            .with_get_category_mismatches_override(Arc::new(|_| MismatchSummary {
                mismatches: vec![CategoryMismatch { category: "A".into(), ..CategoryMismatch::default() }],
                ..MismatchSummary::default()
            }))
            .with_get_category_mismatch_details_override(Arc::new(|_| MismatchDetails { error: Some("down".into()), ..MismatchDetails::default() })),
    );
    let logs = common::log_capture::LogCapture::start(log::Level::Info);
    let (result, err) = outcome_or_default(block_on(manager.reload_category_mismatches(None)));
    assert!(err.is_none(), "{err:?}");
    assert!(result.failures.iter().any(|f| f.error == "불일치 상세 조회 실패: down"), "{result:?}");
    assert!(logs.text().contains("reload_category_mismatches 완료"), "완료 요약을 남긴다: {}", logs.text());
    assert_eq!(result.failed_count, result.failures.len() as i64);
}

#[test]
fn test_reload_category_mismatch_files_reports_a_details_failure() {
    let tmp = tmp_dir();
    fs::create_dir_all(tmp.path().join("A")).unwrap();
    let manager = BookManager::with_deps(
        deps(tmp.path(), &FakeEsManager::new()).with_get_category_mismatch_details_override(Arc::new(|_| MismatchDetails { error: Some("down".into()), ..MismatchDetails::default() })),
    );
    let (_, err) = outcome_or_default(block_on(manager.reload_category_mismatch_files("A", None)));
    assert_eq!(err.as_deref(), Some("불일치 상세 조회 실패: down"));
}

#[test]
fn test_reload_category_logs_loader_stderr_only_when_there_is_some() {
    use common::runner::{FakeRunner, output};
    let tmp = tmp_dir();
    fs::create_dir_all(tmp.path().join("A")).unwrap();
    let es = FakeEsManager::new();
    let logs = common::log_capture::LogCapture::start(log::Level::Warn);

    let noisy = BookManager::with_deps(deps(tmp.path(), &es).with_runner(FakeRunner::returning(Ok(output(0, "총 3개 파일 처리됨", "slow disk")))));
    assert!(block_on(noisy.reload_category("A", None)).is_ok());
    assert!(logs.text().contains("stderr 출력 - slow disk"), "{}", logs.text());

    logs.clear();
    let quiet = BookManager::with_deps(deps(tmp.path(), &es).with_runner(FakeRunner::returning(Ok(output(0, "총 3개 파일 처리됨", "")))));
    assert!(block_on(quiet.reload_category("A", None)).is_ok());
    assert!(!logs.text().contains("stderr 출력"), "{}", logs.text());
}
