//! `backend::book_manager::mismatch_reporter` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{deps, make_doc, make_manager, tmp_dir};
use backend::book_manager::BookManager;
use backend::main::BookManagerApi;
use common::es_fake::{FakeEsManager, Hit};
use common::log_capture::LogCapture;
use serde_json::json;
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

    assert_eq!(details["es_only"], json!([]));
    assert_eq!(details["fs_only"], json!([]));
}

#[test]
fn test_get_category_mismatches_root_file() {
    // Line 1722
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_aggregate(&[]);
    fs::write(tmp_path.join("root_file.txt"), "sample").unwrap();
    // 3.2 `BookManagerApi`가 `Result<Value, String>`을 돌려주므로 풀어서 읽는다.
    let result = BookManagerApi::get_category_mismatches(&manager, false).expect("get_category_mismatches");
    assert!(result["fs_only"].as_array().unwrap().iter().any(|m| m["category"] == "_root"));
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

        assert!(result["mismatches"].as_array().unwrap().iter().all(|item| item["category"] != "A"));
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

    assert_eq!(details["es_only"], json!([]));
    assert_eq!(details["fs_count"], json!(0));
    assert!(details["error"].as_str().is_some_and(|e| e.contains("es down")), "{details}");
    assert!(logs.text().contains("get_category_mismatch_details 실패: A"), "{}", logs.text());
}
