//! `book_manager::mismatch_reporter` 단위 테스트.

#[path = "common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{deps, make_doc, make_manager, tmp_dir};
use book_manager::BookManager;
use book_manager::BookManagerApi;
use common::es_fake::{FakeEsManager, Hit};
use common::log_capture::LogCapture;
use std::fs;
use std::os::unix::fs::MetadataExt;

#[test]
fn test_get_category_mismatch_details_reads_all_pages() {
    // 상한 크기의 첫 페이지에서 멈추지 않고 다음 페이지까지 비교한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let first = tmp_path.join("A").join("first.txt");
    let second = tmp_path.join("A").join("second.txt");
    fs::create_dir_all(first.parent().unwrap()).unwrap();
    fs::write(&first, "x").unwrap();
    fs::write(&second, "y").unwrap();
    es.set_category_docs(vec![
        Hit::new(fs::metadata(&first).unwrap().ino() as i64, make_doc("A/first.txt", ".txt"), 1.0),
        Hit::new(fs::metadata(&second).unwrap().ino() as i64, make_doc("A/second.txt", ".txt"), 1.0),
    ]);
    // 상한을 1로 줄인다(문서에 없는 seam, 파일 머리말 참고).
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_max_category_result_count(1));

    let details = manager.get_category_mismatch_details("A");

    assert!(details.es_only.is_empty());
    assert!(details.fs_only.is_empty());
}

#[test]
fn test_get_category_mismatches_root_file() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_aggregate(&[]);
    fs::write(tmp_path.join("root_file.txt"), "sample").unwrap();
    let result = BookManagerApi::get_category_mismatches(&manager, false).expect("get_category_mismatches");
    assert!(result.fs_only.iter().any(|m| m.category == "_root"));
}

mod mismatch_summary {
    use super::common;
    use crate::common::dyn_es::make_manager;
    use crate::common::shared_fixtures::category_with_files;

    use common::es_fake::FakeEsManager;

    use tempfile::TempDir;

    // ---- 불일치 집계 ----

    #[test]
    fn test_category_mismatches_omit_categories_without_anomalies() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        es.with(|s| s.all_file_paths = vec![("A".to_string(), "A/ok.txt".to_string())]);
        let manager = make_manager(tmp.path(), es);
        category_with_files(tmp.path(), "A", &["ok.txt"]);

        let result = manager.get_category_mismatches(true);

        assert!(result.mismatches.iter().all(|item| item.category != "A"));
    }

    #[test]
    fn test_category_mismatches_list_a_category_whose_directory_is_gone_as_es_only() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        es.with(|s| s.all_file_paths = vec![("Gone".to_string(), "Gone/a.txt".to_string()), ("Gone".to_string(), "Gone/b.txt".to_string())]);
        let manager = make_manager(tmp.path(), es);

        let result = manager.get_category_mismatches(true);

        assert_eq!(result.es_only.len(), 1, "{:?}", result.es_only);
        let item = &result.es_only[0];
        assert_eq!((item.category.as_str(), item.es_count, item.fs_count, item.es_only_count), ("Gone", 2, 0, 2));
    }

    #[test]
    fn test_category_mismatches_ignore_symlinks_in_a_category_directory() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        es.with(|s| s.all_file_paths = vec![("A".to_string(), "A/real.txt".to_string())]);
        let manager = make_manager(tmp.path(), es);
        category_with_files(tmp.path(), "A", &["real.txt"]);
        std::os::unix::fs::symlink(tmp.path().join("A/real.txt"), tmp.path().join("A/link.txt")).unwrap();

        let result = manager.get_category_mismatches(true);

        assert!(result.mismatches.is_empty() && result.fs_only.is_empty() && result.es_only.is_empty(), "링크는 파일로 세지 않는다: {result:?}");
    }

    #[test]
    fn test_category_mismatches_report_an_es_failure_as_an_error_in_an_empty_summary() {
        let tmp = TempDir::new().unwrap();
        let es = FakeEsManager::new();
        es.fail("iter_all_category_file_paths", "down");
        let manager = make_manager(tmp.path(), es);

        let result = manager.get_category_mismatches(true);

        assert!(result.error.as_deref().is_some_and(|e| e.contains("down")), "{:?}", result.error);
        assert!(result.mismatches.is_empty() && result.es_only.is_empty() && result.fs_only.is_empty());
    }
}

#[test]
fn es_failure_in_mismatch_details_returns_an_empty_result_with_the_error_and_logs_it() {
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    fs::create_dir_all(tmp.path().join("A")).unwrap();
    es.fail("search_by_category_paged", "es down");
    let manager = make_manager(tmp.path(), &es);
    let logs = LogCapture::start(log::Level::Error);

    let details = manager.get_category_mismatch_details("A");

    assert!(details.es_only.is_empty());
    assert_eq!(details.fs_count, 0);
    assert!(details.error.as_deref().is_some_and(|e| e.contains("es down")), "{details:?}");
    assert!(logs.text().contains("get_category_mismatch_details 실패: A"), "{}", logs.text());
}
