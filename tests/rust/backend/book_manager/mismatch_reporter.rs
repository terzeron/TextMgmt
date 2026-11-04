//! `backend::book_manager::mismatch_reporter` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;
use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::{BookManagerApi};
use common::es_fake::FakeEsManager;
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// 공용 헬퍼 (전반부 정의를 복사)
// ---------------------------------------------------------------------------

/// pytest `tmp_path`에 대응한다. 반환값이 drop되면 디렉토리가 지워진다.
fn tmp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// 기본 구성의 `BookManagerDeps`. `with_*`를 더 붙인 뒤 `BookManager::with_deps`로 만든다.
fn deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    common::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(deps(tmp_path, es))
}

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
        (fs::metadata(&first).unwrap().ino() as i64, make_doc("A/first.txt", ".txt"), 1.0),
        (fs::metadata(&second).unwrap().ino() as i64, make_doc("A/second.txt", ".txt"), 1.0),
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

    use std::fs;

    use std::path::Path;
    use std::sync::Arc;

    use backend::book_manager::{BookManager, BookManagerDeps};

    use backend::es_manager::EsManagerApi;
    use common::es_fake::FakeEsManager;




    use tempfile::TempDir;


    fn category_with_files(root: &Path, category: &str, names: &[&str]) {
        fs::create_dir_all(root.join(category)).unwrap();
        for name in names {
            fs::write(root.join(category).join(name), "본문").unwrap();
        }
    }

    /// ES 대역을 쓰는 `BookManagerDeps`. 대역(`with_*`)을 꽂을 테스트는 이걸로 시작한다.
    fn make_deps(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManagerDeps {
        BookManagerDeps::new(tmp_path.to_path_buf(), es)
    }

    /// Python `make_manager(tmp_path, es)`.
    fn make_manager(tmp_path: &Path, es: Arc<dyn EsManagerApi>) -> BookManager {
        BookManager::with_deps(make_deps(tmp_path, es))
    }

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
