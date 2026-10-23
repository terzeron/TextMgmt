//! 원본: tests/test_main_bulk_rename.py
//!
//! `POST /categories/bulk-rename-files` 라우트 테스트. ES/MySQL 없이 manager를 가짜로 바꾼다.
//!
//! 가정한 API (아직 구현 전이다. Python과 이름·시그니처가 같다고 본다):
//! - `backend::main::{app_with(deps: AppDeps) -> axum::Router, AppDeps, AuthMode}`
//!   - `AppDeps::new(auth: AuthMode) -> AppDeps`, `.with_book_manager(Arc<dyn BookManagerApi>) -> AppDeps`, `.with_fs(Arc<dyn sysdeps::FsOps>) -> AppDeps` (소비형 빌더, 설계 3.2)
//!   - `AuthMode::Fixed(serde_json::Value)`: `require_auth`/`require_admin` 의존성 override에 대응한다.
//! - `backend::book::Book { book_id: i64, category: String, author: String, file_path: PathBuf, file_type: String, .. }` (`Default + Clone`)
//! - `#[async_trait] backend::book_manager::BookManagerApi` (모든 메서드에 기본 구현이 있다)
//!   - `async fn get_categories(&self) -> (Value, Option<String>)`
//!   - `async fn get_books_in_category_paged(&self, category: &str, size: Option<i64>, cursor: Option<&str>) -> (Vec<Book>, i64, Option<String>, Option<String>)`
//!     (목록, 전체 수, 다음 커서, 오류. 설계 3.2의 통일 시그니처)
//!   - `async fn update_book(&self, book_id: i64, new_category: &str, new_title: &str, new_author: &str, new_path: &Path, new_type: &str, force: bool) -> (Value, Option<String>)`
//!   - `fn es_manager(&self) -> Arc<dyn EsManagerApi>`
//! - `backend::es_manager::EsManagerApi::refresh(&self) -> Result<(), EsError>` (대역은 `common::es_fake::FakeEsManager`)
//! - `sysdeps::FsOps::read_dir`: `Path.iterdir` 실패 주입은 `common::faulty_fs::FaultyFs`를 `AppDeps::with_fs`로 넣는다.
//!   (구 `backend::main::testing::fail_next_iterdir` 전역 가드는 없다)

mod common;

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use backend::book::Book;
use backend::book_manager::BookManagerApi;
use backend::es_manager::EsManagerApi;
use backend::main::{AppDeps, AuthMode, app_with};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::faulty_fs::{FaultyFs, fail_on_nth_path};
use serde_json::{Value, json};
use tempfile::TempDir;

const URL: &str = "/categories/bulk-rename-files";

/// (목록, 전체 수, 다음 커서, 오류)
type Page = (Vec<Book>, i64, Option<String>, Option<String>);

/// `AsyncMock()` manager. 호출 기록(visited, updates, refresh)을 남긴다.
struct MockBm {
    categories: Mutex<(Value, Option<String>)>,
    /// `side_effect` 목록. 비면 `fixed_page`를 돌려준다.
    page_queue: Mutex<VecDeque<Page>>,
    fixed_page: Mutex<Option<Page>>,
    update_result: Mutex<(Value, Option<String>)>,
    visited: Mutex<Vec<String>>,
    updates: Mutex<Vec<PathBuf>>,
    /// `refresh` 호출 횟수는 `es.refresh_count()`로 읽는다.
    es: Arc<FakeEsManager>,
}

#[async_trait::async_trait]
impl BookManagerApi for MockBm {
    async fn get_categories(&self) -> (Value, Option<String>) {
        self.categories.lock().unwrap().clone()
    }

    async fn get_books_in_category_paged(&self, category: &str, _size: Option<i64>, _cursor: Option<&str>) -> Page {
        self.visited.lock().unwrap().push(category.to_string());
        if let Some(page) = self.page_queue.lock().unwrap().pop_front() {
            return page;
        }
        self.fixed_page.lock().unwrap().clone().unwrap_or_default()
    }

    async fn update_book(&self, _book_id: i64, _new_category: &str, _new_title: &str, _new_author: &str, new_path: &Path, _new_type: &str, _force: bool) -> (Value, Option<String>) {
        self.updates.lock().unwrap().push(new_path.to_path_buf());
        self.update_result.lock().unwrap().clone()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        self.es.clone()
    }
}

impl MockBm {
    fn target_names(&self) -> Vec<String> {
        self.updates.lock().unwrap().iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }
}

fn admin_payload() -> Value {
    json!({"email": "admin@test.com", "role": "admin", "name": "Admin", "picture": "", "exp": now_secs() + 3600})
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

fn client(bm: &Arc<MockBm>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm.clone())))
}

fn client_with_fs(bm: &Arc<MockBm>, fs: Arc<FaultyFs>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(admin_payload())).with_book_manager(bm.clone()).with_fs(fs)))
}

fn mock_bm() -> Arc<MockBm> {
    Arc::new(MockBm {
        categories: Mutex::default(),
        page_queue: Mutex::default(),
        fixed_page: Mutex::default(),
        update_result: Mutex::default(),
        visited: Mutex::default(),
        updates: Mutex::default(),
        es: FakeEsManager::new(),
    })
}

fn book(path: &Path, book_id: i64) -> Book {
    Book { book_id, category: "cat".into(), author: "author".into(), file_type: "txt".into(), file_path: path.to_path_buf(), ..Default::default() }
}

/// paths를 한 카테고리의 책으로 돌려주는 manager. update_book은 파일을 옮기지 않는다.
fn setup(bm: &MockBm, paths: &[PathBuf]) {
    *bm.categories.lock().unwrap() = (json!(["cat"]), None);
    let books: Vec<Book> = paths.iter().enumerate().map(|(i, p)| book(p, i as i64 + 1)).collect();
    *bm.fixed_page.lock().unwrap() = Some((books, paths.len() as i64, None, None));
    *bm.update_result.lock().unwrap() = (json!({"ok": true}), None);
}

fn touch(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let p = dir.join(name);
            fs::write(&p, "x").unwrap();
            p
        })
        .collect()
}

#[tokio::test]
async fn test_requires_directory_category() {
    for category in ["", "_root"] {
        let bm = mock_bm();
        let resp = client(&bm).post_json(URL, &json!({"category": category, "pattern": "a", "replacement": "b"})).await;
        assert_eq!(resp.status, 400, "category={category:?}");
    }
}

#[tokio::test]
async fn test_rejects_pattern_length() {
    for pattern in ["".to_string(), "a".repeat(257)] {
        let bm = mock_bm();
        let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": pattern, "replacement": "b"})).await;
        assert_eq!(resp.json()["status"], "failure", "pattern 길이={}", pattern.len());
        assert_eq!(resp.json()["changed_count"], 0, "pattern 길이={}", pattern.len());
    }
}

#[tokio::test]
async fn test_category_lookup_error() {
    let bm = mock_bm();
    *bm.categories.lock().unwrap() = (json!([]), Some("es down".into()));
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "a", "replacement": "b"})).await;
    assert_eq!(resp.json(), json!({"status": "failure", "error": "es down", "changed_count": 0}));
}

#[tokio::test]
async fn test_invalid_regex() {
    let bm = mock_bm();
    *bm.categories.lock().unwrap() = (json!(["cat"]), None);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "(", "replacement": "b"})).await;
    let body = resp.json();
    assert!(body["status"] == "failure" && body["error"].as_str().unwrap().starts_with("정규표현식 오류"), "{body}");
}

#[tokio::test]
async fn test_paging_follows_cursor_and_includes_subcategories() {
    let tmp = TempDir::new().unwrap();
    let files = touch(tmp.path(), &["old1.txt", "old2.txt"]);
    let (first, second) = (files[0].clone(), files[1].clone());
    let bm = mock_bm();
    *bm.categories.lock().unwrap() = (json!(["cat", "cat/sub", "other"]), None);
    *bm.page_queue.lock().unwrap() = VecDeque::from([(vec![book(&first, 1)], 2, Some("next".to_string()), None), (vec![book(&second, 2)], 2, None, None), (vec![], 0, None, None)]);
    *bm.update_result.lock().unwrap() = (json!({}), None);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "old", "replacement": "new"})).await;
    assert_eq!(resp.json()["result"]["changed_count"], 2);
    let visited = bm.visited.lock().unwrap().clone();
    assert_eq!(visited, ["cat", "cat", "cat/sub"]);
    assert_eq!(bm.es.refresh_count(), 1, "es_manager.refresh는 정확히 한 번 불려야 한다");
}

#[tokio::test]
async fn test_paging_error() {
    let _tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    *bm.categories.lock().unwrap() = (json!(["cat"]), None);
    *bm.fixed_page.lock().unwrap() = Some((vec![], 0, None, Some("page failed".into())));
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "a", "replacement": "b"})).await;
    assert_eq!(resp.json(), json!({"status": "failure", "error": "page failed", "changed_count": 0}));
}

#[tokio::test]
async fn test_rejects_unsafe_new_name() {
    for replacement in ["", "x/y", "x\\y"] {
        let tmp = TempDir::new().unwrap();
        let bm = mock_bm();
        let paths = touch(tmp.path(), &["a"]);
        setup(&bm, &paths);
        // 이름 전체를 바꾸는 패턴이라 replacement 가 곧 새 이름이다. ".", ".." 도 막는다.
        let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "^a$", "replacement": replacement})).await;
        let body = resp.json();
        assert!(body["status"] == "failure" && body["changed_count"] == 0, "replacement={replacement:?}: {body}");
        assert!(bm.updates.lock().unwrap().is_empty(), "replacement={replacement:?}: update_book이 불리면 안 된다");
    }
}

#[tokio::test]
async fn test_rejects_dot_names() {
    for replacement in [".", ".."] {
        let tmp = TempDir::new().unwrap();
        let bm = mock_bm();
        let paths = touch(tmp.path(), &["a"]);
        setup(&bm, &paths);
        let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "^a$", "replacement": replacement})).await;
        assert_eq!(resp.json()["status"], "failure", "replacement={replacement:?}");
    }
}

#[tokio::test]
async fn test_unchanged_names_are_skipped() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["keep.txt"]);
    setup(&bm, &paths);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "zzz", "replacement": "b"})).await;
    assert_eq!(resp.json()["result"], json!({"changed_count": 0, "failed_count": 0, "failures": []}));
    assert!(bm.updates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_update_failure_is_reported() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["old.txt"]);
    setup(&bm, &paths);
    *bm.update_result.lock().unwrap() = (Value::Null, Some("db error".into()));
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "old", "replacement": "new"})).await;
    assert_eq!(resp.json()["result"], json!({"changed_count": 0, "failed_count": 1, "failures": [{"file": "old.txt", "error": "db error"}]}));
}

#[tokio::test]
async fn test_existing_target_gets_next_serial() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["a1.txt", "ax.txt", "ax (1).txt", "ax (4).txt", "unrelated.txt"]);
    setup(&bm, &paths[..1]);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": "a1", "replacement": "ax"})).await;
    assert_eq!(resp.json()["result"]["changed_count"], 1);
    // call_args.args[4].name: 마지막 호출의 new_path 파일명
    assert_eq!(bm.target_names().last().unwrap(), "ax (5).txt");
}

#[tokio::test]
async fn test_serial_skips_names_reserved_by_earlier_files() {
    // update_book 이 파일을 옮기지 않으므로 iterdir 은 앞서 정한 이름을 못 본다. reserved 가 막아야 한다.
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["a1.txt", "a2.txt", "a3.txt", "ax.txt"]);
    setup(&bm, &paths[..3]);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": r"a\d", "replacement": "ax"})).await;
    assert_eq!(resp.json()["result"]["changed_count"], 3);
    assert_eq!(bm.target_names(), ["ax (1).txt", "ax (2).txt", "ax (3).txt"]);
}

#[tokio::test]
async fn test_same_new_name_in_one_batch_gets_serials() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["a1.txt", "a2.txt", "a3.txt"]);
    setup(&bm, &paths);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": r"a\d", "replacement": "ax"})).await;
    assert_eq!(resp.json()["result"]["changed_count"], 3);
    assert_eq!(bm.target_names(), ["ax.txt", "ax (1).txt", "ax (2).txt"]);
}

#[tokio::test]
async fn test_reserved_serial_skips_file_already_on_disk() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["a1.txt", "a2.txt", "ax (1).txt"]);
    setup(&bm, &paths[..2]);
    let resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": r"a\d", "replacement": "ax"})).await;
    assert_eq!(resp.json()["result"]["changed_count"], 2);
    assert_eq!(bm.target_names(), ["ax.txt", "ax (2).txt"]);
}

#[tokio::test]
async fn test_name_without_extension_gets_serial() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["a1", "a2"]);
    setup(&bm, &paths);
    let _resp = client(&bm).post_json(URL, &json!({"category": "cat", "pattern": r"a\d", "replacement": "ax"})).await;
    assert_eq!(bm.target_names(), ["ax", "ax (1)"]);
}

#[tokio::test]
async fn test_directory_scan_failure_is_reported() {
    let tmp = TempDir::new().unwrap();
    let bm = mock_bm();
    let paths = touch(tmp.path(), &["a1.txt", "ax.txt"]);
    setup(&bm, &paths[..1]);
    // 디렉토리 열거 실패 주입. 첫 `read_dir` 호출만 실패한다(Python `mock.patch`는 한 번만 던진다).
    let fs = FaultyFs::new();
    fs.set_read_dir(fail_on_nth_path(1, "denied"));
    let resp = client_with_fs(&bm, fs).post_json(URL, &json!({"category": "cat", "pattern": "a1", "replacement": "ax"})).await;
    assert_eq!(resp.json()["result"], json!({"changed_count": 0, "failed_count": 1, "failures": [{"file": "a1.txt", "error": "denied"}]}));
    assert!(bm.updates.lock().unwrap().is_empty());
}
