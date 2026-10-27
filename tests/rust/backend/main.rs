//! 원본: tests/test_main.py
//!
//! backend/main.py 라우트·쿠키·인증·카테고리 매핑 테스트.
//! 앞쪽(TestBackend ~ TestAuthMeLogout, 마지막 DB 통합 테스트)은 Python과 같은 MySQL·Nori Elasticsearch testcontainers를 쓴다.
//! Docker 통합은 fixture가 컨테이너를 띄워 기본 실행에서 돈다(Docker가 없으면 panic).
//! 나머지는 가짜 manager/category_mapping을 `AppDeps`로 주입해서 ES/MySQL 없이 돈다.
//!
//! 원본의 `def test_`는 95개로 세어지지만 그중 `test_book`(68행)은 fixture이고 실제 테스트는 94개다.
//! Rust에서는 같은 이름의 헬퍼 함수(`test_book`)가 되고 `#[test]`가 아니다.
//!
//! 가정한 API (아직 구현 전이다. Python과 이름·시그니처가 같다고 본다):
//! - `backend::main`
//!   - `app_with(deps: AppDeps) -> axum::Router`, `AppDeps::new(auth: AuthMode)`와 소비형 빌더 (설계 3.2):
//!     `.with_book_manager(..)`, `.with_comics_manager(..)`, `.with_category_mapping(..)` (각각 `Arc<dyn Trait>`),
//!     `.with_fs(Arc<dyn sysdeps::FsOps>)`, `.with_google_verifier(Arc<dyn GoogleVerifier>)`, `.with_role_resolver(Arc<dyn RoleResolver>)`,
//!     `.with_refresh_token_store(Arc<dyn RefreshTokenStoreApi>)`, `.with_bookstore_factory(Arc<dyn Fn() -> Box<dyn BookstoreSearch> + Send + Sync>)`,
//!     `.with_reload_progress_flush_interval(Duration)`
//!   - `AuthMode::{Cookies, Fixed(Value), Split { auth: Value, admin: Value }}`: 실제 쿠키 인증 / `require_auth`·`require_admin` override
//!   - `GoogleVerifier { fn client_id(&self) -> Option<String>; fn verify(&self, credential: &str, audience: &str) -> Result<Value, String> }`, `RoleResolver { fn determine_role(&self, email: &str) -> Option<String> }`
//!   - `BookstoreSearch`: 문서 3.2는 이름만 정했다. 아래 `StubStore`가 `search`/`build_search_url` 두 메서드를 가정한다(Python `DummyStore`와 같은 모양).
//!   - `LazyProxy<T: Debug>`: `new(Box<dyn Fn() -> T + Send + Sync>, name: &str)`, `lock() -> MutexGuard<T>`
//!   - `custom_jsonable_encoder(&Value) -> Value`, `CustomJsonResponse::new(Value)` (`.body: Vec<u8>`)
//!   - `get_cookie_settings() -> (bool, String)`, `set_auth_cookies(&mut Response, access: &str, refresh: Option<&str>)`, `clear_auth_cookies(&mut Response)`
//!   - `issue_auth_tokens(email: &str, role: &str) -> (String, String)`
//!   - `stale_running_status(status: &Value, stale_seconds: f64, now: Option<f64>) -> Option<Value>`
//!   - `create_bookstore() -> Yes24Bookstore`
//!   - 상수: `ERR_MISSING_INPUT`, `GENERIC_MISMATCH_ERROR`, `GENERIC_MAPPING_ERROR_DETAIL`, `GENERIC_HIDDEN_CATEGORY_ERROR_DETAIL`, `GENERIC_LATEST_EXCLUDED_CATEGORY_ERROR_DETAIL`
//!   - 구 `backend::main::testing::*` 전역 가드는 없다. `RELOAD_PROGRESS_FLUSH_INTERVAL_SECONDS` 패치는 `AppDeps::with_reload_progress_flush_interval`, `os.listdir` 패치는 `AppDeps::with_fs`가 대신한다.
//!   - 백그라운드 작업(reload-*)은 Starlette `BackgroundTasks`처럼 응답을 돌려주기 전에 끝난다.
//! - `backend::auth`: `create_jwt_token(email: &str, role: &str, name: Option<&str>, picture: Option<&str>) -> String`, `create_refresh_token`(같은 인자),
//!   `tm_admin_email() -> String`, `jwt_secret() -> String`, `JWT_ALGORITHM: jsonwebtoken::Algorithm`
//! - `backend::book::Book` (`Default + Clone + Serialize`): `book_id, category, title, author, file_path: PathBuf, file_type, file_size, line_count, page_count, isbn, created_time: Option<String>, updated_time, score: f64`, `Book::path_prefix() -> PathBuf`
//! - `backend::bookstore::Yes24Bookstore { verbose: bool }`
//! - `backend::book_manager::{BookManager, BookManagerApi, BookContent, BookResponse}`: `BookManager::from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<BookManager, _>`, `BookManager::media_type(suffix_with_dot: &str) -> &'static str`
//!   - `bm.path_prefix() -> PathBuf`, `bm.es_manager() -> Arc<dyn EsManagerApi>` (`insert(&Docs) -> Result<Vec<i64>, EsError>`, `refresh() -> Result<(), EsError>`, `delete(i64) -> Result<bool, EsError>`, `count_by_category(&str, bool) -> Result<i64, EsError>`)
//!   - `async add_book(BTreeMap<i64, Value>) -> (Option<i64>, Option<String>)`, `async get_book(i64) -> (Option<Book>, Option<String>)`
//!   - `BookManagerApi`(모든 메서드에 기본 구현이 있다): 아래 `DummyManager` 구현부의 시그니처를 따른다. `fn es_manager(&self) -> Arc<dyn EsManagerApi>`.
//!     통일 시그니처(설계 3.2): `get_book_content -> BookContent`, `get_book_preview(id, chapters, resource_base_url) -> BookResponse`, `get_pdf_pages(id, start, end) -> BookResponse`, `get_category_mismatches(force_refresh) -> Result<Value, String>`
//! - `backend::es_manager::{EsManagerApi, EsError, ESManager}`: `delete(&self, i64) -> Result<bool, EsError>`, `search_categories_pdf_stats(&self, &[String]) -> Result<Value, EsError>`. 대역은 `common::es_fake::FakeEsManager`다.
//!   (실제 ES 통합 테스트만 `ESManager::connect(env, &RealEsConnector, &SystemClock)`를 가정한다. 커넥터 이름은 문서에 없다.)
//! - `backend::comics_manager::ComicsManager::from_env(env, es_factory)` (`BookManagerApi` 구현)
//! - `backend::category_mapping::{CategoryMapping, CategoryMappingApi, MappingError}`
//!   - `MappingError::{Http(u16, String), Other(String)}`, `CategoryMappingApi`는 동기 메서드이고 기본 구현은 `Err(Other("unimplemented"))`다.
//!   - `CategoryMapping::new() -> Result<Self, MappingError>`(env + 실제 MySQL), `get_connection() -> Result<Box<dyn DbConnection>, DbError>`, `DbConnection::{execute(&mut self, sql, &[DbValue]) -> Result<u64, DbError>, commit(&mut self) -> Result<(), DbError>}` (설계 3.4)
//! - `loader::{config::Config, record::read_file, record::now_iso, flow::direct_files}`, `doc_reader::pdf::InProcess`, `stat::Stat` (utils/loader/src에서 확인한 실제 API)
//! - 개발 의존성: `jsonwebtoken`, `async-trait`, `tempfile`, `tokio`, `serde_json`
//!
//! ES 인덱스(`TM_ES_BOOK_INDEX`)는 conftest의 `es_index`와 같은 매핑으로 미리 만들어져 있다고 본다.
//! 모듈 범위 fixture(`backend_test_setup`)는 테스트마다 새로 만든다.

use crate::{common, elasticsearch_fixture, mysql_fixture};

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::Method;
use axum::response::IntoResponse;
use backend::auth::{JWT_ALGORITHM, create_jwt_token, create_refresh_token, jwt_secret, tm_admin_email};
use backend::book::Book;
use backend::book_manager::{BookContent, BookManager, BookManagerApi, BookResponse};
use backend::category_mapping::CategoryMapping;
use backend::comics_manager::ComicsManager;
use backend::es_manager::{ESManager, EsManagerApi, RealEsConnector};
use backend::main::{AppDeps, AuthMode, BookstoreFactory, BookstoreSearch, CategoryMappingApi, CustomJSONResponse, GoogleVerifier, MappingError, RoleResolver, Slot, app_with};
use common::{TestClient, env::EnvGuard, es_fake::FakeEsManager};
use serde_json::{Value, json};
use sysdeps::{FsOps, SystemClock};
use tempfile::TempDir;

#[test]
fn backend_binary_dispatches_the_isbn_pdf_worker_command() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("not-a-pdf.pdf");
    fs::write(&path, b"not a PDF").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_backend")).args(["__isbn-pdf-stage", "fallback"]).arg(path).output().unwrap();

    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "0\n\n");
}

const CATEGORY: &str = "_epub";

// ── 공용 헬퍼 ──────────────────────────────────────────────────────────────

/// 쿼리 값을 퍼센트 인코딩한다.
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// 경로 조각을 인코딩한다. `/`는 그대로 둔다.
fn enc_path(s: &str) -> String {
    s.split('/').map(enc).collect::<Vec<_>>().join("/")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// `finally` 블록에 대응한다. 패닉이 나도 정리가 돈다.
struct Defer<F: FnMut()>(F);

impl<F: FnMut()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

fn set_cookie_headers(resp: &common::Response) -> Vec<&str> {
    resp.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.as_str()).collect()
}

/// `response.cookies.get(name)`에 대응한다.
fn cookie_value(resp: &common::Response, name: &str) -> Option<String> {
    set_cookie_headers(resp).into_iter().find_map(|h| h.strip_prefix(&format!("{name}="))).map(|rest| rest.split(';').next().unwrap_or("").to_string())
}

async fn post_empty(client: &TestClient, path: &str) -> common::Response {
    client.request(Method::POST, path, None, Vec::new()).await
}

fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

// ── 통합 테스트 setup (ES + MySQL) ─────────────────────────────────────────

struct Setup {
    bm: Arc<BookManager>,
    cm: Arc<ComicsManager>,
    client: TestClient,
    _db_guard: std::sync::MutexGuard<'static, ()>,
}

/// `Loader.read_file`에 대응한다. 읽지 못하면 빈 map이다.
fn read_file(path: &Path) -> BTreeMap<i64, Value> {
    let root = common::project_root();
    let cfg = loader::config::Config::new(root.join("tests/books"), root.join("tests/comics"));
    let stat = stat::Stat::new();
    let out = loader::record::read_file(&cfg, path, None, false, None, &doc_reader::pdf::InProcess, &stat);
    out.item.into_iter().collect()
}

/// `Loader.read_files(path, num_files=n)`에 대응한다.
fn read_files(dir: &Path, num_files: usize) -> BTreeMap<i64, Value> {
    loader::flow::direct_files(dir).into_iter().take(num_files).flat_map(|p| read_file(&p)).collect()
}

fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// 실제 ES에 연결하는 `es_factory`. 실제 ES가 필요한 통합 테스트만 쓴다.
/// TODO(common): 실제 커넥터 이름(`RealEsConnector`)은 문서 3.3에 없어 가정했다. 구현이 생기면 이름만 맞춘다.
fn real_es(_url: &str) -> Arc<dyn EsManagerApi> {
    Arc::new(ESManager::connect(&process_env, &RealEsConnector, &SystemClock).expect("실제 ES 연결"))
}

/// `backend_test_setup` fixture. Python처럼 테스트용 ES와 MySQL 컨테이너를 쓴다.
async fn backend_test_setup() -> Setup {
    common::setup_env();
    tokio::task::spawn_blocking(|| {
        mysql_fixture::mysql_container();
        elasticsearch_fixture::elasticsearch_container();
    })
    .await
    .expect("testcontainers fixture thread panicked");
    let db_guard = mysql_fixture::db_lock();
    let bm = Arc::new(BookManager::from_env(&process_env, &real_es).expect("BookManager 생성"));
    let cm = Arc::new(ComicsManager::from_env(&process_env, &real_es).expect("ComicsManager 생성"));

    let epub_path = bm.path_prefix().join(CATEGORY);
    if epub_path.exists() {
        let data = read_files(&epub_path, 5);
        if !data.is_empty() {
            let _ = bm.es_manager().insert(&data);
        }
    }
    let _ = bm.es_manager().refresh();

    let token = create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
    let app = app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(bm.clone()).with_comics_manager(cm.clone()));
    let client = TestClient::with_app(app).cookie("tm_access_token", &token);
    Setup { bm, cm, client, _db_guard: db_guard }
}

fn epub_files() -> Vec<PathBuf> {
    let dir = Book::path_prefix().join(CATEGORY);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "epub")).collect()).unwrap_or_default();
    files.sort();
    files
}

/// `assert book_id and not error`
fn expect_id(book_id: Option<i64>, error: Option<String>) -> i64 {
    assert!(book_id.unwrap_or(0) != 0 && error.is_none(), "book_id={book_id:?}, error={error:?}");
    book_id.unwrap()
}

/// 임시로 복사한 epub을 등록한 책. 버려질 때 ES 문서와 임시 파일을 지운다 (class fixture의 정리 부분).
struct TestBook {
    book: Book,
    temp: PathBuf,
    bm: Arc<BookManager>,
}

impl Drop for TestBook {
    fn drop(&mut self) {
        let _ = self.bm.es_manager().delete(self.book.book_id);
        let _ = fs::remove_file(&self.temp);
    }
}

/// `test_book` fixture. epub이 없으면 panic한다.
async fn test_book(s: &Setup) -> TestBook {
    let files = epub_files();
    let src = files.first().expect("epub 샘플 파일 필요: No epub files available for testing");
    let temp = Book::path_prefix().join(src.parent().unwrap().file_name().unwrap()).join(format!("temp_{}", src.file_name().unwrap().to_string_lossy()));
    fs::copy(src, &temp).unwrap();
    let data = read_file(&temp);

    let (book_id, error) = s.bm.add_book(&data).await;
    let book_id = expect_id(book_id, error);
    let (book, error) = s.bm.get_book(book_id).await;
    assert!(book.is_some() && error.is_none());
    TestBook { book: book.unwrap(), temp, bm: s.bm.clone() }
}

fn ext_with_dot(path: &Path) -> String {
    path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default()
}

mod test_backend {
    use super::*;

    #[tokio::test]
    async fn test_update_book() {
        let s = backend_test_setup().await;
        let files = epub_files();
        let src = files.first().expect("epub 샘플 파일 필요: No epub files available for testing");
        let temp = Book::path_prefix().join(src.parent().unwrap().file_name().unwrap()).join(format!("update_temp_{}", src.file_name().unwrap().to_string_lossy()));
        fs::copy(src, &temp).unwrap();
        let data = read_file(&temp);

        let (book_id, error) = s.bm.add_book(&data).await;
        let book_id = expect_id(book_id, error);
        let (book, error) = s.bm.get_book(book_id).await;
        assert!(book.is_some() && error.is_none());
        let book = book.unwrap();
        let renamed_temp = Book::path_prefix().join(&book.category).join(format!("renamed_{}.{}", book.title, book.file_type));

        let bm = s.bm.clone();
        let temp_cleanup = temp.clone();
        let renamed_cleanup = renamed_temp.clone();
        let _cleanup = Defer(move || {
            let _ = bm.es_manager().delete(book_id);
            let _ = fs::remove_file(&temp_cleanup);
            let _ = fs::remove_file(&renamed_cleanup);
        });

        let doc = json!({
            "book_id": book.book_id,
            "category": book.category,
            "title": format!("renamed_{}", book.title),
            "author": format!("anonymous_{}", book.author),
            "file_path": format!("{}/renamed_{}.{}", book.category, book.title, book.file_type),
            "file_type": book.file_type,
            "file_size": 100,
            "summary": "summary1",
            "updated_time": "2021-01-01T00:00:00.000000",
        });

        let response = s.client.put_json(&format!("/books/{}", book.book_id), &doc).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);

        assert_eq!(response.json(), json!({"status": "success", "result": "Ok"}));
        drop(_cleanup);
        assert!(!temp.exists(), "원본 임시 EPUB이 남았다");
        assert!(!renamed_temp.exists(), "이동된 임시 EPUB이 남았다");
    }

    #[tokio::test]
    async fn test_delete_book() {
        let s = backend_test_setup().await;
        let files = epub_files();
        let src = files.first().expect("epub 샘플 파일 필요: No epub files available for testing");
        let temp = Book::path_prefix().join(src.parent().unwrap().file_name().unwrap()).join(format!("to_be_deleted_{}", src.file_name().unwrap().to_string_lossy()));
        fs::copy(src, &temp).unwrap();
        let data = read_file(&temp);
        let temp_cleanup = temp.clone();
        let _cleanup = Defer(move || {
            let _ = fs::remove_file(&temp_cleanup);
        });

        let (book_id, error) = s.bm.add_book(&data).await;
        let book_id = expect_id(book_id, error);
        let (book, error) = s.bm.get_book(book_id).await;
        assert!(book.is_some() && error.is_none());
        let book = book.unwrap();

        let response = s.client.delete(&format!("/books/{}", book.book_id)).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "success", "result": "Ok"}));
    }

    #[tokio::test]
    async fn test_get_file_content() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get(&format!("/download/{}", book.book_id)).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        assert!(!response.body.is_empty());
        assert!(response.body.len() > 1024);
        let media_type = match ext_with_dot(&book.file_path).as_str() {
            ".pdf" => "application/pdf",
            ".epub" => "application/epub+zip",
            _ => "application/octet-stream",
        };
        assert_eq!(response.header("content-type").unwrap().split(';').next().unwrap(), media_type);
    }

    #[tokio::test]
    async fn test_get_book() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get(&format!("/books/{}", book.book_id)).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        let body = response.json();
        assert_eq!(body["status"], "success");
        assert_eq!(body["result"]["book_id"], book.book_id);
        assert_eq!(body["result"]["title"], book.title);
        assert!(body["result"]["file_path"].as_str().unwrap().ends_with(book.file_path.file_name().unwrap().to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn test_get_books_in_category() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get(&format!("/categories/{}", enc_path(&book.category))).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        let response_data = response.json();
        assert_eq!(response_data["status"], "success");
        let books = response_data["result"].as_array().unwrap();
        assert!(!books.is_empty());
        assert!(books.iter().any(|b| b["book_id"] == book.book_id), "book_id={}가 결과에 없다", book.book_id);
    }

    /// limit을 주면 커서 페이지 응답(total, next_cursor)을 돌려준다.
    #[tokio::test]
    async fn test_get_books_in_category_paged() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get(&format!("/categories/{}?limit=1", enc_path(&book.category))).await;
        assert_eq!(response.status, 200);
        let response_data = response.json();
        assert_eq!(response_data["status"], "success");
        assert!(response_data["result"].as_array().unwrap().len() <= 1);
        assert!(response_data["total"].as_i64().unwrap() >= 1);
        assert!(response_data.as_object().unwrap().contains_key("next_cursor"));

        // limit 없이 호출하면 기존(비페이지) 응답 형태를 유지한다
        let legacy = s.client.get(&format!("/categories/{}", enc_path(&book.category))).await.json();
        assert_eq!(legacy["status"], "success");
        assert!(!legacy.as_object().unwrap().contains_key("next_cursor"));
    }

    #[tokio::test]
    async fn test_get_books_in_category_paged_invalid_cursor() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get(&format!("/categories/{}?limit=1&cursor=not-a-cursor", enc_path(&book.category))).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "failure", "error": "invalid cursor"}));
    }

    #[tokio::test]
    async fn test_get_categories() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get("/categories").await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["status"], "success");
        assert!(response.json()["result"].as_object().is_some_and(|m| m.contains_key(&book.category)) || response.json()["result"].as_array().is_some_and(|a| a.contains(&json!(book.category))));
    }

    #[tokio::test]
    async fn test_get_similar_book_list() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let response = s.client.get(&format!("/similar/{}", book.book_id)).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        let response_data = response.json();
        assert_eq!(response_data["status"], "success");
        let books = response_data["result"].as_array().unwrap();
        assert!(!books.is_empty());
    }

    #[tokio::test]
    async fn test_get_category_mismatches() {
        let s = backend_test_setup().await;

        let response = s.client.get("/category-mismatches").await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        let response_data = response.json();
        assert_eq!(response_data["status"], "success");

        let result = &response_data["result"];
        for key in ["mismatches", "es_only", "fs_only"] {
            assert!(result.get(key).is_some(), "{key} 없음");
            assert!(result[key].is_array(), "{key}는 list여야 한다");
        }

        // mismatches 항목 구조 검증
        for item in result["mismatches"].as_array().unwrap() {
            for key in ["category", "es_count", "fs_count", "diff"] {
                assert!(item.get(key).is_some(), "{key} 없음");
            }
            assert_ne!(item["es_count"], item["fs_count"]);
        }
    }

    #[tokio::test]
    async fn test_search_by_keyword() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let keyword = &book.title;
        let response = s.client.get(&format!("/search/{}", enc_path(keyword))).await;
        assert!(response.status < 400);
        assert_eq!(response.status, 200);
        let response_data = response.json();
        assert_eq!(response_data["status"], "success");
        let books = response_data["result"].as_array().unwrap();
        assert!(!books.is_empty());
        assert!(books.iter().any(|b| b["book_id"] == book.book_id));
    }

    #[tokio::test]
    async fn test_search_by_keyword_with_exclude_categories() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let keyword = &book.title;
        // exclude_categories에 해당 책의 카테고리를 넣으면 결과에서 제외됨
        let response = s.client.get(&format!("/search/{}?exclude_categories={}", enc_path(keyword), enc(&book.category))).await;
        assert_eq!(response.status, 200);
        let response_data = response.json();
        if response_data["status"] == "success" && response_data.get("result").is_some_and(|r| r.as_array().is_some_and(|a| !a.is_empty())) {
            let excluded_ids: Vec<&Value> = response_data["result"].as_array().unwrap().iter().filter(|b| b["category"] == book.category).map(|b| &b["book_id"]).collect();
            assert_eq!(excluded_ids.len(), 0, "제외된 카테고리 '{}'의 책이 포함됨", book.category);
        }
    }

    #[tokio::test]
    async fn test_search_by_keyword_without_exclude_categories() {
        let s = backend_test_setup().await;
        let tb = test_book(&s).await;
        let book = &tb.book;

        let keyword = &book.title;
        // exclude_categories 없이 검색하면 결과 포함
        let response = s.client.get(&format!("/search/{}", enc_path(keyword))).await;
        assert_eq!(response.status, 200);
        let response_data = response.json();
        assert_eq!(response_data["status"], "success");

        // 빈 exclude_categories도 동일하게 동작
        let response2 = s.client.get(&format!("/search/{}?exclude_categories=", enc_path(keyword))).await;
        assert_eq!(response2.status, 200);
        let response_data2 = response2.json();
        assert_eq!(response_data2["status"], "success");
        assert_eq!(response_data2.get("total").cloned().unwrap_or(json!(0)), response_data.get("total").cloned().unwrap_or(json!(0)));
    }
}

/// 테스트 문서를 ES에 넣는다. TestRenameCategory/TestDeleteCategory의 `_insert_docs`.
async fn insert_docs(bm: &BookManager, category: &str, count: i64, start_id: i64, title_prefix: &str, file_prefix: &str, summary: &str) -> Vec<i64> {
    let mut ids = Vec::new();
    for i in 0..count {
        let doc_id = start_id + i;
        let doc = json!({
            "category": category,
            "title": format!("{title_prefix} {i}"),
            "author": "Test Author",
            "file_path": format!("{category}/{file_prefix}_{i}.txt"),
            "file_type": "txt",
            "file_size": 100,
            "line_count": 10,
            "page_count": 0,
            "isbn": "",
            "summary": summary,
            "updated_time": loader::record::now_iso(),
        });
        let (book_id, error) = bm.add_book(&BTreeMap::from([(doc_id, doc)])).await;
        ids.push(expect_id(book_id, error));
    }
    ids
}

/// 카테고리 일괄 변경 API 테스트.
mod test_rename_category {
    use super::*;

    async fn insert(bm: &BookManager, category: &str, count: i64, start_id: i64) -> Vec<i64> {
        insert_docs(bm, category, count, start_id, "Rename Test", "rename_test", "rename test doc").await
    }

    /// 정상 rename: ES + FS 모두 변경
    #[tokio::test]
    async fn test_rename_category() {
        let s = backend_test_setup().await;
        let (old_cat, new_cat) = ("_rename_test_src", "_rename_test_dst");
        let old_dir = s.bm.path_prefix().join(old_cat);
        let new_dir = s.bm.path_prefix().join(new_cat);

        // 준비: 디렉토리 + ES 문서
        fs::create_dir_all(&old_dir).unwrap();
        fs::write(old_dir.join("rename_test_0.txt"), "test content").unwrap();
        let doc_ids = insert(&s.bm, old_cat, 1, 400).await;

        let bm = s.bm.clone();
        let (new_dir_c, old_dir_c) = (new_dir.clone(), old_dir.clone());
        let _cleanup = Defer(move || {
            // 정리
            for doc_id in &doc_ids {
                let _ = bm.es_manager().delete(*doc_id);
            }
            let _ = bm.es_manager().refresh();
            if new_dir_c.exists() {
                let _ = fs::remove_dir_all(&new_dir_c);
            }
            if old_dir_c.exists() {
                let _ = fs::remove_dir_all(&old_dir_c);
            }
        });

        let response = s.client.put_json("/categories/rename", &json!({"old_category": old_cat, "new_category": new_cat})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"]["old_category"], old_cat);
        assert_eq!(data["result"]["new_category"], new_cat);
        assert_eq!(data["result"]["updated_count"], 1);
        assert_eq!(data["result"]["fs_renamed"], true);
    }

    /// 대상 카테고리에 문서가 이미 존재하면 에러
    #[tokio::test]
    async fn test_rename_category_conflict() {
        let s = backend_test_setup().await;
        let (old_cat, new_cat) = ("_rename_conflict_src", "_rename_conflict_dst");

        // 양쪽 모두에 문서 삽입
        let old_ids = insert(&s.bm, old_cat, 1, 500).await;
        let new_ids = insert(&s.bm, new_cat, 1, 600).await;

        let bm = s.bm.clone();
        let _cleanup = Defer(move || {
            for doc_id in old_ids.iter().chain(new_ids.iter()) {
                let _ = bm.es_manager().delete(*doc_id);
            }
            let _ = bm.es_manager().refresh();
        });

        let response = s.client.put_json("/categories/rename", &json!({"old_category": old_cat, "new_category": new_cat})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains("이미"), "{data}");
    }

    /// 없는 카테고리 rename 시 에러
    #[tokio::test]
    async fn test_rename_category_not_found() {
        let s = backend_test_setup().await;

        let response = s.client.put_json("/categories/rename", &json!({"old_category": "_nonexistent_cat_xyz", "new_category": "_new_cat_xyz"})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains("문서가 없습니다"), "{data}");
    }
}

/// 카테고리 일괄 삭제 API 테스트.
mod test_delete_category {
    use super::*;

    async fn insert(bm: &BookManager, category: &str, count: i64, start_id: i64) -> Vec<i64> {
        insert_docs(bm, category, count, start_id, "Delete Test", "delete_test", "delete test doc").await
    }

    /// 남은 ES 문서와 디렉토리를 정리하는 guard.
    fn cleanup(bm: Arc<BookManager>, doc_ids: Vec<i64>, dirs: Vec<PathBuf>) -> Defer<impl FnMut()> {
        Defer(move || {
            for doc_id in &doc_ids {
                let _ = bm.es_manager().delete(*doc_id);
            }
            let _ = bm.es_manager().refresh();
            for dir in &dirs {
                if dir.exists() {
                    let _ = fs::remove_dir_all(dir);
                }
            }
        })
    }

    /// 정상 삭제: ES + FS 모두 삭제
    #[tokio::test]
    async fn test_delete_category() {
        let s = backend_test_setup().await;
        let cat = "_delete_test_cat";
        let cat_dir = s.bm.path_prefix().join(cat);

        // 준비: 디렉토리 + 파일 + ES 문서
        fs::create_dir_all(&cat_dir).unwrap();
        fs::write(cat_dir.join("delete_test_0.txt"), "test content").unwrap();
        fs::write(cat_dir.join("delete_test_1.txt"), "test content 2").unwrap();
        let doc_ids = insert(&s.bm, cat, 2, 700).await;
        let _cleanup = cleanup(s.bm.clone(), doc_ids, vec![cat_dir.clone()]);

        let response = s.client.post_json("/categories/delete", &json!({"category": cat})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"]["category"], cat);
        assert_eq!(data["result"]["deleted_count"], 2);

        // ES에서 삭제 확인
        assert_eq!(s.bm.es_manager().count_by_category(cat, false).unwrap(), 0);
    }

    /// 없는 카테고리 삭제 시 에러
    #[tokio::test]
    async fn test_delete_category_not_found() {
        let s = backend_test_setup().await;

        let response = s.client.post_json("/categories/delete", &json!({"category": "_nonexistent_del_xyz"})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains("문서가 없습니다"), "{data}");
    }

    /// 삭제 시 하위 카테고리 문서도 함께 삭제
    #[tokio::test]
    async fn test_delete_category_with_subcategories() {
        let s = backend_test_setup().await;
        let cat = "_delete_sub_test";
        let sub_cat = format!("{cat}/sub");
        let cat_dir = s.bm.path_prefix().join(cat);
        let sub_dir = s.bm.path_prefix().join(&sub_cat);

        // 준비: 디렉토리 + 파일 + ES 문서
        fs::create_dir_all(&cat_dir).unwrap();
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(cat_dir.join("root.txt"), "root content").unwrap();
        fs::write(sub_dir.join("child.txt"), "child content").unwrap();
        let mut ids = insert(&s.bm, cat, 1, 850).await;
        ids.extend(insert(&s.bm, &sub_cat, 1, 860).await);
        let _cleanup = cleanup(s.bm.clone(), ids, vec![cat_dir.clone()]);

        let response = s.client.post_json("/categories/delete", &json!({"category": cat})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"]["deleted_count"], 2);

        // ES에서 하위 카테고리도 삭제 확인
        assert_eq!(s.bm.es_manager().count_by_category(cat, true).unwrap(), 0);
    }

    /// ES만 있는 경우에도 정상 삭제
    #[tokio::test]
    async fn test_delete_category_es_only() {
        let s = backend_test_setup().await;
        let cat = "_delete_no_fs_cat";
        let doc_ids = insert(&s.bm, cat, 1, 800).await;
        let _cleanup = cleanup(s.bm.clone(), doc_ids, vec![]);

        let response = s.client.post_json("/categories/delete", &json!({"category": cat})).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"]["deleted_count"], 1);
    }
}

/// update_book 충돌 감지 및 force 덮어쓰기 테스트.
mod test_update_book_conflict {
    use super::*;

    fn make_test_file(path: &Path, content: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    async fn register_book(bm: &BookManager, file_path: &Path, category: &str) -> i64 {
        let rel = file_path.strip_prefix(bm.path_prefix()).unwrap();
        let meta = fs::metadata(file_path).unwrap();
        let doc = json!({
            "category": category,
            "title": file_path.file_stem().unwrap().to_string_lossy(),
            "author": "Test Author",
            "file_path": rel.to_string_lossy(),
            "file_type": file_path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default(),
            "file_size": meta.len(),
            "line_count": 0,
            "page_count": 0,
            "isbn": "",
            "summary": "conflict test",
            "updated_time": loader::record::now_iso(),
        });
        let (book_id, error) = bm.add_book(&BTreeMap::from([(meta.ino() as i64, doc)])).await;
        assert!(book_id.unwrap_or(0) != 0 && error.is_none(), "Failed to register: {error:?}");
        book_id.unwrap()
    }

    /// 책 문서 본문. 매번 같은 필드 구성이다.
    fn update_doc(book_id: i64, category: &str, title: &str, author: &str, file_path: &str) -> Value {
        json!({"book_id": book_id, "category": category, "title": title, "author": author, "file_path": file_path, "file_type": "epub", "file_size": 100, "updated_time": "2021-01-01T00:00:00.000000"})
    }

    /// finally: 책을 지우고 파일을 정리한다. ES 문서 삭제는 `DELETE /books/{id}`와 같은 효과의 동기 호출이다.
    fn cleanup(bm: Arc<BookManager>, book_id: i64, files: Vec<PathBuf>) -> Defer<impl FnMut()> {
        Defer(move || {
            let _ = bm.es_manager().delete(book_id);
            for f in &files {
                let _ = fs::remove_file(f);
            }
        })
    }

    /// 대상 경로에 다른 파일이 이미 존재하면 CONFLICT: 에러를 반환한다.
    #[tokio::test]
    async fn test_conflict_detected_when_destination_exists() {
        let s = backend_test_setup().await;
        let src_path = s.bm.path_prefix().join(CATEGORY).join("conflict_src.epub");
        let dst_path = s.bm.path_prefix().join(CATEGORY).join("conflict_dst.epub");
        make_test_file(&src_path, b"source");
        make_test_file(&dst_path, b"existing");

        let book_id = register_book(&s.bm, &src_path, CATEGORY).await;
        let _cleanup = cleanup(s.bm.clone(), book_id, vec![src_path.clone(), dst_path.clone()]);

        let doc = update_doc(book_id, CATEGORY, "conflict_dst", "Test Author", &format!("{CATEGORY}/conflict_dst.epub"));

        let response = s.client.put_json(&format!("/books/{book_id}"), &doc).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "failure");
        assert!(data["error"].as_str().unwrap().contains("CONFLICT:"), "{data}");
        assert!(src_path.exists(), "Source file should not have been moved");
        assert_eq!(fs::read(&dst_path).unwrap(), b"existing", "Destination should be unchanged");
    }

    /// CONFLICT 에러 메시지에 상대 경로가 포함된다.
    #[tokio::test]
    async fn test_conflict_error_contains_relative_path() {
        let s = backend_test_setup().await;
        let src_path = s.bm.path_prefix().join(CATEGORY).join("conflict_relpath_src.epub");
        let dst_path = s.bm.path_prefix().join(CATEGORY).join("conflict_relpath_dst.epub");
        make_test_file(&src_path, b"test content");
        make_test_file(&dst_path, b"test content");

        let book_id = register_book(&s.bm, &src_path, CATEGORY).await;
        let _cleanup = cleanup(s.bm.clone(), book_id, vec![src_path.clone(), dst_path.clone()]);

        let doc = update_doc(book_id, CATEGORY, "conflict_relpath_dst", "Test Author", &format!("{CATEGORY}/conflict_relpath_dst.epub"));

        let response = s.client.put_json(&format!("/books/{book_id}"), &doc).await;
        let data = response.json();
        let error_msg = data.get("error").and_then(Value::as_str).unwrap_or("");
        assert!(error_msg.contains(&format!("{CATEGORY}/conflict_relpath_dst.epub")), "CONFLICT error should contain relative path, got: {error_msg}");
    }

    /// force=true이면 대상 경로의 기존 파일을 덮어쓰고 성공한다.
    #[tokio::test]
    async fn test_force_overwrite_succeeds() {
        let s = backend_test_setup().await;
        let src_path = s.bm.path_prefix().join(CATEGORY).join("force_src.epub");
        let dst_path = s.bm.path_prefix().join(CATEGORY).join("force_dst.epub");
        make_test_file(&src_path, b"source content");
        make_test_file(&dst_path, b"existing content");

        let book_id = register_book(&s.bm, &src_path, CATEGORY).await;
        let _cleanup = cleanup(s.bm.clone(), book_id, vec![src_path.clone(), dst_path.clone()]);

        let doc = update_doc(book_id, CATEGORY, "force_dst", "Test Author", &format!("{CATEGORY}/force_dst.epub"));

        let response = s.client.put_json(&format!("/books/{book_id}?force=true"), &doc).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"], "Ok");

        assert!(!src_path.exists(), "Source should have been moved");
        assert!(dst_path.exists(), "Destination should exist");
        assert_eq!(fs::read(&dst_path).unwrap(), b"source content");
    }

    /// 동일 파일 경로에 대한 메타데이터 변경 시 충돌이 발생하지 않는다.
    #[tokio::test]
    async fn test_same_file_no_conflict() {
        let s = backend_test_setup().await;
        let file_path = s.bm.path_prefix().join(CATEGORY).join("samefile_test.epub");
        make_test_file(&file_path, b"test content");

        let book_id = register_book(&s.bm, &file_path, CATEGORY).await;
        let _cleanup = cleanup(s.bm.clone(), book_id, vec![file_path.clone()]);

        let doc = update_doc(book_id, CATEGORY, "samefile_test", "Changed Author Name", &format!("{CATEGORY}/samefile_test.epub"));

        let response = s.client.put_json(&format!("/books/{book_id}"), &doc).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success", "Same file path should not cause conflict, got: {data}");
        assert!(file_path.exists(), "File should still exist");
    }

    /// 다른 디렉토리로 이동 시 정상 동작한다.
    #[tokio::test]
    async fn test_move_to_new_directory_succeeds() {
        let s = backend_test_setup().await;
        let new_dir = "_conflict_test_dir";
        let src_path = s.bm.path_prefix().join(CATEGORY).join("movedir_test.epub");
        let dst_path = s.bm.path_prefix().join(new_dir).join("movedir_test.epub");
        make_test_file(&src_path, b"test content");

        let book_id = register_book(&s.bm, &src_path, CATEGORY).await;
        let new_dir_path = s.bm.path_prefix().join(new_dir);
        let bm = s.bm.clone();
        let (src_c, dst_c, dir_c) = (src_path.clone(), dst_path.clone(), new_dir_path.clone());
        let _cleanup = Defer(move || {
            let _ = bm.es_manager().delete(book_id);
            let _ = fs::remove_file(&src_c);
            let _ = fs::remove_file(&dst_c);
            if dir_c.exists() && fs::read_dir(&dir_c).map(|mut d| d.next().is_none()).unwrap_or(false) {
                let _ = fs::remove_dir(&dir_c);
            }
        });

        let doc = update_doc(book_id, new_dir, "movedir_test", "Test Author", &format!("{new_dir}/movedir_test.epub"));

        let response = s.client.put_json(&format!("/books/{book_id}"), &doc).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");

        assert!(!src_path.exists(), "Source should have been moved");
        assert!(dst_path.exists(), "Destination should exist");
    }

    /// 원본 파일이 없으면 이동 실패 에러를 반환한다.
    #[tokio::test]
    async fn test_source_missing_returns_error() {
        let s = backend_test_setup().await;
        let src_path = s.bm.path_prefix().join(CATEGORY).join("missing_src.epub");
        make_test_file(&src_path, b"test content");

        let book_id = register_book(&s.bm, &src_path, CATEGORY).await;
        // 등록 후 원본 파일 삭제
        fs::remove_file(&src_path).unwrap();
        let _cleanup = cleanup(s.bm.clone(), book_id, vec![]);

        let doc = update_doc(book_id, CATEGORY, "missing_renamed", "Test Author", &format!("{CATEGORY}/missing_renamed.epub"));

        let response = s.client.put_json(&format!("/books/{book_id}"), &doc).await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "failure");
        let error = data.get("error").and_then(Value::as_str).unwrap_or("").to_lowercase();
        assert!(error.contains("can't move") || error.contains("error"), "{data}");
    }
}

/// POST /auth/refresh 엔드포인트 통합 테스트
mod test_auth_refresh_endpoint {
    use super::*;

    /// `_get_unauthenticated_client`: 쿠키 없는 TestClient
    fn unauthenticated_client() -> TestClient {
        TestClient::new()
    }

    #[tokio::test]
    async fn test_refresh_returns_new_access_token() {
        common::setup_env();
        let refresh_token = create_refresh_token(&tm_admin_email(), "admin", None, None);
        let app = app_with(AppDeps::new(AuthMode::Cookies).with_refresh_token_store(Arc::new(AuthRefreshStore)));
        let client = TestClient::with_app(app).cookie("tm_refresh_token", &refresh_token);
        let response = post_empty(&client, "/auth/refresh").await;
        assert_eq!(response.status, 200);
        assert!(response.header("set-cookie").is_some());
    }

    #[tokio::test]
    async fn test_refresh_with_expired_token_returns_401() {
        common::setup_env();
        let now = now_secs() as i64;
        let expired_payload = json!({"type": "refresh", "email": tm_admin_email(), "role": "admin", "exp": now - 100, "iat": now - 200});

        let token = jsonwebtoken::encode(&jsonwebtoken::Header::new(JWT_ALGORITHM), &expired_payload, &jsonwebtoken::EncodingKey::from_secret(jwt_secret().as_bytes())).unwrap();
        let client = unauthenticated_client().cookie("tm_refresh_token", &token);
        let response = post_empty(&client, "/auth/refresh").await;
        assert_eq!(response.status, 401);
    }

    #[tokio::test]
    async fn test_refresh_with_access_token_returns_401() {
        common::setup_env();
        let access_token = create_jwt_token(&tm_admin_email(), "admin", None, None);
        let client = unauthenticated_client().cookie("tm_refresh_token", &access_token);
        let response = post_empty(&client, "/auth/refresh").await;
        assert_eq!(response.status, 401);
    }

    #[tokio::test]
    async fn test_refresh_without_token_returns_400() {
        common::setup_env();
        let response = post_empty(&unauthenticated_client(), "/auth/refresh").await;
        assert_eq!(response.status, 400);
    }

    #[tokio::test]
    async fn test_refresh_with_unauthorized_email_returns_403() {
        common::setup_env();
        let refresh_token = create_refresh_token("hacker@evil.com", "admin", None, None);
        let client = unauthenticated_client().cookie("tm_refresh_token", &refresh_token);
        let response = post_empty(&client, "/auth/refresh").await;
        assert_eq!(response.status, 403);
    }

    #[tokio::test]
    async fn test_refreshed_token_works_for_api_calls() {
        common::setup_env();
        let temp = TempDir::new().unwrap();
        let manager = DummyManager::new(temp.path());
        let app = app_with(
            AppDeps::new(AuthMode::Cookies)
                .with_book_manager(manager.clone())
                .with_comics_manager(manager)
                .with_category_mapping(DummyCategoryMapping::new())
                .with_refresh_token_store(Arc::new(AuthRefreshStore)),
        );
        let refresh_token = create_refresh_token(&tm_admin_email(), "admin", None, None);
        let client = TestClient::with_app(app.clone()).cookie("tm_refresh_token", &refresh_token);
        let response = post_empty(&client, "/auth/refresh").await;
        let new_access_token = cookie_value(&response, "tm_access_token").expect("새 access token 쿠키");
        // 새 토큰으로 인증 필요한 API 호출
        let auth_client = TestClient::with_app(app).cookie("tm_access_token", &new_access_token);
        let cat_response = auth_client.get("/categories").await;
        assert_eq!(cat_response.status, 200);
    }
}

mod test_auth_me_logout {
    use super::*;

    #[tokio::test]
    async fn test_auth_me_returns_user_info() {
        common::setup_env();
        let token = create_jwt_token("admin@test.com", "admin", Some("Test"), Some("pic"));
        let client = TestClient::new().cookie("tm_access_token", &token);
        let response = client.get("/auth/me").await;
        assert_eq!(response.status, 200);
        let data = response.json();
        assert_eq!(data["status"], "success");
        assert_eq!(data["result"]["role"], "admin");
        assert_eq!(data["result"]["email"], "admin@test.com");
        assert_eq!(data["result"]["name"], "Test");
        assert_eq!(data["result"]["picture"], "pic");
    }

    #[tokio::test]
    async fn test_logout_clears_cookies() {
        common::setup_env();
        let token = create_jwt_token("admin@test.com", "admin", None, None);
        let client = TestClient::new().header("cookie", &format!("tm_access_token={token}; tm_refresh_token={token}"));
        let response = post_empty(&client, "/auth/logout").await;
        assert_eq!(response.status, 200);
        assert!(response.header("set-cookie").is_some());
    }
}

// ---- merged from test_main_api_extra.py ----

fn user_payload() -> Value {
    json!({"email": "user@example.com", "role": "user"})
}

fn admin_example_payload() -> Value {
    json!({"email": "admin@example.com", "role": "admin"})
}

/// `client` fixture: require_auth/require_admin을 override한 TestClient.
/// 가짜 manager를 주입하지 않으면 기본 인스턴스를 쓴다.
fn client() -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Split { auth: user_payload(), admin: admin_example_payload() })))
}

#[derive(Debug)]
struct Ns {
    value: i32,
}

#[test]
fn test_lazy_proxy_initializes_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let factory = move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Ns { value: 1 }
    };

    let slot = Slot::lazy(move || Ok(Arc::new(factory())));
    let first = slot.get().unwrap();
    assert_eq!(first.value, 1);
    let second = slot.get().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn test_custom_jsonable_encoder() {
    let data = json!({"korean": "테스트", "list": ["값"], "nested": {"a": "나"}});
    let encoded = backend::main::custom_jsonable_encoder(&data);
    assert_eq!(encoded, data);
}

#[test]
fn test_custom_json_response_render_preserves_unicode() {
    let response = CustomJSONResponse::new(json!({"message": "테스트"}));
    assert!(!contains_bytes(&response.body, b"\\u"));
    assert!(contains_bytes(&response.body, "테스트".as_bytes()));
}

/// `os.listdir` 대역. 열거 결과(또는 오류)를 테스트가 정한다.
/// TODO(common): `FaultyFs`에는 읽기 결과를 바꾸는 훅이 없어 로컬로 정의했다. 나머지 `FsOps` 메서드는 trait 기본 구현(설계 3.0 규칙 4)에 맡긴다.
struct ListDirFs(Result<Vec<PathBuf>, String>);

impl ListDirFs {
    fn ok(names: &[&str]) -> Arc<Self> {
        Arc::new(Self(Ok(names.iter().map(PathBuf::from).collect())))
    }

    fn err(message: &str) -> Arc<Self> {
        Arc::new(Self(Err(message.to_string())))
    }
}

impl FsOps for ListDirFs {
    fn read_dir(&self, _p: &Path) -> std::io::Result<Vec<PathBuf>> {
        self.0.clone().map_err(std::io::Error::other)
    }
}

/// `client` fixture에 저장소 열거 대역만 바꾼 TestClient.
fn client_with_fs(auth: AuthMode, fs: Arc<dyn FsOps>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(auth).with_fs(fs)))
}

#[tokio::test]
async fn test_wake_storage_success_and_failure() {
    let mut env = EnvGuard::new();
    env.set("TM_FRONTEND_URL", "http://testserver");
    // 열거 가능한 저장소 흉내.
    let client = client_with_fs(AuthMode::Split { auth: user_payload(), admin: admin_example_payload() }, ListDirFs::ok(&["a", "b"])).header("host", "testserver");
    let resp = client.get("/wake").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json(), json!({"status": "success"}));

    let client = client_with_fs(AuthMode::Split { auth: user_payload(), admin: admin_example_payload() }, ListDirFs::err("boom")).header("host", "testserver");
    let resp = client.get("/wake").await;
    assert_eq!(resp.status, 503);
    assert_eq!(resp.json(), json!({"status": "failure"}));
}

#[tokio::test]
async fn test_wake_storage_rejects_non_frontend_host() {
    let mut env = EnvGuard::new();
    env.set("TM_FRONTEND_URL", "https://tm.terzeron.com");
    let resp = client().get("/wake").await;
    assert_eq!(resp.status, 404);
}

#[tokio::test]
async fn test_search_bookstore_api_errors() {
    let client = client();
    let resp = client.get("/search/bookstore/unknown").await;
    assert_eq!(resp.status, 404);

    let resp = client.get("/search/bookstore/yes24").await;
    assert_eq!(resp.status, 400);
    assert_eq!(resp.json()["detail"], "제목 또는 저자를 입력해주세요");
}

type BookRow = (String, String, String, String, String, String);

/// Python `DummyStore`. `search`가 정한 행을 돌려주고, `build_search_url`은 `https://example.com?q={keyword}`다.
/// TODO(common): `BookstoreSearch`의 메서드 모양은 문서 3.2에 없다. 두 메서드를 가정했다. 서점이 13곳이어도 factory는 하나라 모든 이름이 같은 대역을 쓴다.
struct StubStore {
    rows: Vec<BookRow>,
    keyword: String,
    kind: String,
}

impl BookstoreSearch for StubStore {
    fn search(&self, _isbn: &str, _title: &str, _author: &str) -> (Vec<BookRow>, String, String) {
        (self.rows.clone(), self.keyword.clone(), self.kind.clone())
    }

    fn api_search_url(&self, keyword: &str) -> String {
        format!("https://example.com?q={keyword}")
    }
}

/// `client` fixture에 서점 대역만 바꾼 TestClient.
fn client_with_store(store: Arc<StubStore>) -> TestClient {
    common::setup_env();
    let factory: BookstoreFactory = Arc::new(move |name| {
        let known = ["yes24", "aladin", "kyobo", "ridi", "naver", "naverseries", "munpia", "joara", "naverwebtoon", "kakaowebtoon", "bomtoon", "lezhin", "toptoon"];
        known
            .contains(&name.to_ascii_lowercase().as_str())
            .then(|| Box::new(StubStore { rows: store.rows.clone(), keyword: store.keyword.clone(), kind: store.kind.clone() }) as Box<dyn BookstoreSearch>)
    });
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Split { auth: user_payload(), admin: admin_example_payload() }).with_bookstore_factory(factory)))
}

#[tokio::test]
async fn test_search_bookstore_api_success() {
    // DummyStore: search -> ([("T","A","C","U","S","ISBN")], "kw", "title"), build_search_url -> "https://example.com?q={keyword}"
    let store = Arc::new(StubStore { rows: vec![("T".into(), "A".into(), "C".into(), "U".into(), "S".into(), "ISBN".into())], keyword: "kw".into(), kind: "title".into() });
    let resp = client_with_store(store).get("/search/bookstore/yes24?title=Hello").await;
    assert_eq!(resp.status, 200);
    let body = resp.json();
    assert_eq!(body["status"], "success");
    assert_eq!(body["search_keyword"], "kw");
    assert_eq!(body["result"][0]["isbn"], "ISBN");
}

#[tokio::test]
async fn test_search_bookstore_api_limits_results_to_four() {
    let rows: Vec<BookRow> = (0..6).map(|i| (format!("T{i}"), "A".into(), "C".into(), format!("U{i}"), "S".into(), String::new())).collect();
    let store = Arc::new(StubStore { rows, keyword: "kw".into(), kind: "title".into() });
    let resp = client_with_store(store).get("/search/bookstore/yes24?title=Hello").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["result"].as_array().unwrap().len(), 4);
}

/// 엔드포인트 응답 확인용 카테고리 매핑.
struct EndpointMapping;

impl CategoryMappingApi for EndpointMapping {
    fn get_all_mappings(&self, _ct: &str) -> Result<Value, MappingError> {
        Ok(json!({"A": ["a"]}))
    }

    fn get_keywords(&self, _category: &str, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["k1".into(), "k2".into()])
    }

    fn set_keywords(&self, _category: &str, _keywords: &[String], _ct: &str) -> Result<bool, MappingError> {
        Ok(true)
    }

    fn add_keyword(&self, _category: &str, keyword: &str, _ct: &str) -> Result<bool, MappingError> {
        Ok(keyword != "dup")
    }

    fn remove_keyword(&self, _category: &str, keyword: &str, _ct: &str) -> Result<bool, MappingError> {
        Ok(keyword == "ok")
    }

    fn delete_category(&self, category: &str, _ct: &str, _prefix: bool) -> Result<bool, MappingError> {
        Ok(category == "ok")
    }

    fn update_all_mappings(&self, _mappings: &Value, _ct: &str) -> Result<bool, MappingError> {
        Ok(true)
    }

    fn get_hidden_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["A".into()])
    }

    fn set_hidden(&self, _category: &str, _hidden: bool, _ct: &str) -> Result<bool, MappingError> {
        Ok(true)
    }

    fn get_latest_excluded_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["B".into()])
    }

    fn set_latest_excluded(&self, _category: &str, _excluded: bool, _ct: &str) -> Result<bool, MappingError> {
        Ok(true)
    }
}

/// 기본 override(user/admin)로 `category_mapping`만 바꾼 TestClient.
fn client_with_mapping(mapping: Arc<dyn CategoryMappingApi>) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(AppDeps::new(AuthMode::Split { auth: user_payload(), admin: admin_example_payload() }).with_category_mapping(mapping)))
}

#[tokio::test]
async fn test_category_mapping_endpoints() {
    let client = client_with_mapping(Arc::new(EndpointMapping));

    let resp = client.get("/category-mappings").await;
    assert_eq!(resp.json()["result"], json!({"A": ["a"]}));

    let resp = client.get("/category-mappings/A").await;
    assert_eq!(resp.json()["result"], json!(["k1", "k2"]));

    let resp = client.put_json("/category-mappings/A", &json!({"keywords": ["x"]})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = client.post_json("/category-mappings/A/keywords", &json!({"keyword": ""})).await;
    assert_eq!(resp.status, 400);

    // 이미 있는 키워드는 실패가 아니다 — "등록해 달라"는 요청이 이미 충족돼 있다.
    // 대신 아무 일도 안 일어났다는 사실을 경고로 알린다.
    let resp = client.post_json("/category-mappings/A/keywords", &json!({"keyword": "dup"})).await;
    assert_eq!(resp.json()["status"], "success");
    let warning = &resp.json()["warning"];
    assert!(!warning.is_null() && warning != &json!(""), "warning이 비어 있다: {warning}");

    let resp = client.post_json("/category-mappings/A/keywords", &json!({"keyword": "new"})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = client.delete("/category-mappings/A/keywords/ok").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = client.delete("/category-mappings/A/keywords/miss").await;
    assert_eq!(resp.status, 404);

    let resp = client.delete("/category-mappings/ok").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = client.delete("/category-mappings/miss").await;
    assert_eq!(resp.status, 404);

    let resp = client.put_json("/category-mappings", &json!({"mappings": {"A": ["a"]}})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = client.get("/hidden-categories").await;
    assert_eq!(resp.json()["result"], json!(["A"]));

    let resp = client.post_json("/hidden-categories/A", &json!({"hidden": true})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = client.get("/latest-excluded-categories").await;
    assert_eq!(resp.json()["result"], json!(["B"]));

    let resp = client.post_json("/latest-excluded-categories/B", &json!({"excluded": true})).await;
    assert_eq!(resp.json()["status"], "success");
}

// ---- merged from test_main_routes_extra.py ----

const NOW_ISO: &str = "2024-01-01T00:00:00.000000";

/// DummyBook. 필요한 값만 받고 나머지는 원본의 고정값이다.
fn dummy_book(book_id: i64, category: &str, title: &str, file_path: &str, file_type: &str, score: f64, created_time: Option<&str>) -> Book {
    Book {
        book_id,
        category: category.into(),
        title: title.into(),
        author: "U".into(),
        // Python `Book.__init__` always anchors the serialized relative path under `TM_BOOK_DIR`.
        file_path: Book::path_prefix().join(file_path),
        file_type: file_type.into(),
        file_size: 1,
        line_count: 0,
        page_count: 0,
        isbn: String::new(),
        created_time: chrono::NaiveDateTime::parse_from_str(created_time.unwrap_or(NOW_ISO), "%Y-%m-%dT%H:%M:%S%.f").unwrap(),
        updated_time: chrono::NaiveDateTime::parse_from_str(NOW_ISO, "%Y-%m-%dT%H:%M:%S%.f").unwrap(),
        score,
        summary: String::new(),
    }
}

type MismatchRet = (Value, Option<String>);
type CategoryFn = Arc<dyn Fn(&str) -> (Option<Value>, Option<String>) + Send + Sync>;
type RenameFn = Arc<dyn Fn(&str, &str) -> (Option<Value>, Option<String>) + Send + Sync>;

/// DummyManager의 메서드를 테스트가 `monkeypatch.setattr`로 바꾸던 부분을 값으로 모은 것이다.
#[derive(Clone, Default)]
struct Behavior {
    /// test_main_error_branches: 13개 메서드를 오류 반환으로 바꾼다.
    errors: bool,
    /// reload_category_mismatch_files가 RuntimeError("mismatch boom")을 던진다.
    mismatch_files_raise: bool,
    /// search_similar_books_paged를 search_similar_books_paged_ok로 바꾼다.
    similar_ok: bool,
    /// get_book이 돌려주는 책의 file_type을 바꾼다 (None이면 txt).
    book_type: Option<&'static str>,
    /// get_book이 (None, "not found")를 돌려준다.
    book_missing: bool,
    delete_book_ok: bool,
    update_fail: bool,
    validate_epub_fail: bool,
    rename: Option<RenameFn>,
    delete_category: Option<CategoryFn>,
    preview: Option<CategoryFn>,
    categories: Option<(Value, Option<String>)>,
    /// SilentScanManager: 콜백 없는 구간을 흉내 내는 지연(ms).
    scan_delay_ms: u64,
}

/// 본문만 있는 200 응답.
fn book_response(media_type: &str, body: &str) -> BookResponse {
    BookResponse { status_code: 200, media_type: Some(media_type.into()), headers: Vec::new(), body: body.as_bytes().to_vec() }
}

struct DummyManager {
    path_prefix: PathBuf,
    behavior: Mutex<Behavior>,
    /// 고수준 ES 대역. `search_categories_pdf_stats`는 `set_categories_pdf_stats`로 값을 주기 전에는 "not configured" 오류다.
    es: Arc<FakeEsManager>,
    latest_size: Mutex<Option<i64>>,
    latest_exclude: Mutex<Option<Vec<String>>>,
}

impl DummyManager {
    fn new(tmp_path: &Path) -> Arc<Self> {
        let es = FakeEsManager::new();
        es.fail("search_categories_pdf_stats", "not configured");
        Arc::new(Self { path_prefix: tmp_path.to_path_buf(), behavior: Mutex::new(Behavior::default()), es, latest_size: Mutex::new(None), latest_exclude: Mutex::new(None) })
    }

    fn b(&self) -> Behavior {
        self.behavior.lock().unwrap().clone()
    }

    fn set(&self, f: impl FnOnce(&mut Behavior)) {
        f(&mut self.behavior.lock().unwrap());
    }

    fn default_book(&self, book_id: i64) -> Book {
        dummy_book(book_id, "A", "T", "a.txt", "txt", 0.0, None)
    }
}

#[async_trait::async_trait]
impl BookManagerApi for DummyManager {
    fn path_prefix(&self) -> PathBuf {
        self.path_prefix.clone()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        self.es.clone()
    }

    async fn update_book(&self, _book_id: i64, _new_category: &str, _new_title: &str, _new_author: &str, _new_path: &Path, _new_type: &str, _force: bool) -> (Value, Option<String>) {
        if self.b().update_fail {
            return (Value::Null, Some("update failed".into()));
        }
        (json!("Ok"), None)
    }

    async fn delete_book(&self, _book_id: i64) -> (String, Option<String>) {
        let b = self.b();
        if b.errors {
            return ("Error".into(), Some("boom".into()));
        }
        if b.delete_book_ok {
            return ("Ok".into(), None);
        }
        ("Warning".into(), Some("warn".into()))
    }

    async fn get_book_content(&self, _book_id: i64) -> BookContent {
        BookContent::Text("content".into())
    }

    async fn get_book_preview(&self, _book_id: i64, _chapters: Option<i64>, _resource_base_url: Option<&str>) -> BookResponse {
        book_response("text/plain", "preview")
    }

    async fn get_pdf_pages(&self, _book_id: i64, _start: i64, _end: i64) -> BookResponse {
        book_response("application/pdf", "pdf")
    }

    async fn get_book(&self, book_id: i64) -> (Option<Book>, Option<String>) {
        let b = self.b();
        if b.errors || b.book_missing {
            return (None, Some("not found".into()));
        }
        let mut book = self.default_book(book_id);
        if let Some(t) = b.book_type {
            book.file_type = t.into();
            book.file_path = format!("a.{t}").into();
        }
        (Some(book), None)
    }

    async fn validate_epub(&self, _book_id: i64) -> (Option<Value>, Option<String>) {
        if self.b().validate_epub_fail {
            return (None, Some("validation failed".into()));
        }
        (Some(json!({"valid": true})), None)
    }

    async fn validate_pdf(&self, _book_id: i64) -> (Option<Value>, Option<String>) {
        (Some(json!({"valid": true})), None)
    }

    async fn get_books_in_category(&self, category: &str) -> (Vec<Book>, Option<String>) {
        if self.b().errors {
            return (vec![], Some("nope".into()));
        }
        (vec![dummy_book(1, category, "T", "a.txt", "txt", 0.0, None)], None)
    }

    async fn get_categories(&self) -> (Value, Option<String>) {
        let b = self.b();
        if b.errors {
            return (json!({}), Some("fail".into()));
        }
        b.categories.unwrap_or((json!({"A": 1}), None))
    }

    async fn search_similar_books_paged(&self, _book_id: i64, _size: i64, _offset: i64) -> (Vec<Book>, i64, Option<String>) {
        let b = self.b();
        if b.errors {
            return (vec![], 0, Some("none".into()));
        }
        if b.similar_ok {
            return (vec![dummy_book(2, "A", "T2", "b.txt", "txt", 10.0, None)], 1, None);
        }
        (vec![], 0, Some("No similar books found".into()))
    }

    async fn search_by_keyword_paged(&self, _keyword: &str, _size: i64, _offset: i64, _exclude_categories: Option<&[String]>, _category: Option<&str>) -> (Vec<Book>, i64, Option<String>) {
        if self.b().errors {
            return (vec![], 0, Some("kw".into()));
        }
        (vec![dummy_book(3, "A", "T3", "c.txt", "txt", 9.0, None)], 1, None)
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> (Vec<Book>, i64, Option<String>) {
        *self.latest_size.lock().unwrap() = Some(size);
        *self.latest_exclude.lock().unwrap() = exclude_categories.map(|e| e.to_vec());
        (vec![dummy_book(4, "A", "T4", "d.txt", "txt", 0.0, Some("2023-12-31T00:00:00.000000"))], 1, None)
    }

    fn get_category_mismatches(&self, _force_refresh: bool) -> Result<Value, String> {
        if self.b().errors {
            return Err("boom".into());
        }
        Ok(json!({"mismatches": []}))
    }

    fn get_category_mismatch_details(&self, category: &str) -> Result<Value, String> {
        if self.b().errors {
            return Err("boom2".into());
        }
        Ok(json!({"category": category}))
    }

    async fn index_single_file(&self, _file_path: &str) -> (Option<i64>, Option<String>) {
        if self.b().errors {
            return (None, Some("err".into()));
        }
        (Some(1), None)
    }

    async fn delete_file(&self, _file_path: &str) -> (String, Option<String>) {
        if self.b().errors {
            return ("Error".into(), Some("bad".into()));
        }
        ("Ok".into(), None)
    }

    async fn reload_category(&self, category: &str, _content_type: &str) -> MismatchRet {
        if self.b().errors {
            return (Value::Null, Some("fail".into()));
        }
        (json!({"category": category, "processed_count": 1}), None)
    }

    async fn reload_category_mismatches(&self, content_type: &str, on_progress: Option<backend::main::ReloadProgress>) -> MismatchRet {
        let b = self.b();
        if b.errors {
            return (Value::Null, Some("bulk fail".into()));
        }
        if b.scan_delay_ms > 0 {
            // 콜백 없는 전체 스캔 구간
            tokio::time::sleep(std::time::Duration::from_millis(b.scan_delay_ms)).await;
            if let Some(cb) = on_progress.as_deref() {
                cb(json!({"before_count": 10}));
            }
            // 콜백 없는 refresh/재스캔 구간
            tokio::time::sleep(std::time::Duration::from_millis(b.scan_delay_ms)).await;
            return (json!({"content_type": content_type, "category_count": 1, "indexed_count": 2, "deleted_count": 3, "failed_count": 0, "before_count": 10, "after_count": 0}), None);
        }
        if let Some(cb) = on_progress.as_deref() {
            cb(json!({"indexed_count": 2, "deleted_count": 3}));
        }
        (json!({"content_type": content_type, "category_count": 1, "indexed_count": 2, "deleted_count": 3, "failed_count": 0, "before_count": 5, "after_count": 0}), None)
    }

    async fn reload_category_mismatch_files(&self, category: &str, content_type: &str, on_progress: Option<backend::main::ReloadProgress>) -> MismatchRet {
        let b = self.b();
        if b.mismatch_files_raise {
            panic!("mismatch boom");
        }
        if b.errors {
            return (Value::Null, Some("mismatch fail".into()));
        }
        if let Some(cb) = on_progress.as_deref() {
            cb(json!({"indexed_count": 1, "deleted_count": 1}));
        }
        (json!({"content_type": content_type, "category": category, "indexed_count": 1, "deleted_count": 1, "failed_count": 0, "before_count": 2, "after_count": 0}), None)
    }

    async fn rename_category(&self, old_category: &str, new_category: &str) -> (Value, Option<String>) {
        match self.b().rename {
            Some(f) => {
                let (value, error) = f(old_category, new_category);
                (value.unwrap_or(Value::Null), error)
            }
            None => (Value::Null, Some("rename_category not implemented".into())),
        }
    }

    async fn delete_category(&self, category: &str, _delete_files: bool) -> (Value, Option<String>) {
        match self.b().delete_category {
            Some(f) => {
                let (value, error) = f(category);
                (value.unwrap_or(Value::Null), error)
            }
            None => (Value::Null, Some("delete_category not implemented".into())),
        }
    }

    async fn preview_delete_category(&self, category: &str) -> (Value, Option<String>) {
        match self.b().preview {
            Some(f) => {
                let (value, error) = f(category);
                (value.unwrap_or(Value::Null), error)
            }
            None => (Value::Null, Some("preview_delete_category not implemented".into())),
        }
    }
}

/// DummyCategoryMapping: 재적재 락을 메모리로 흉내 낸다. heartbeat는 호출 기록도 남긴다 (RecordingCategoryMapping).
struct DummyCategoryMapping {
    locks: Mutex<HashMap<String, Value>>,
    heartbeats: Mutex<Vec<Value>>,
}

const BULK_LOCK_KEY: &str = "__all__";

impl DummyCategoryMapping {
    fn new() -> Arc<Self> {
        Arc::new(Self { locks: Mutex::new(HashMap::new()), heartbeats: Mutex::new(Vec::new()) })
    }
}

impl CategoryMappingApi for DummyCategoryMapping {
    fn get_hidden_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec![])
    }

    fn set_hidden(&self, _category: &str, _hidden: bool, _ct: &str) -> Result<bool, MappingError> {
        Ok(true)
    }

    fn get_latest_excluded_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec![])
    }

    fn set_latest_excluded(&self, _category: &str, _excluded: bool, _ct: &str) -> Result<bool, MappingError> {
        Ok(true)
    }

    fn acquire_reload_lock(&self, _ct: &str, category: Option<&str>, reload_source: Option<&str>) -> Result<(bool, Option<String>, Option<Value>), backend::main::MappingError> {
        let lock_key = category.unwrap_or(BULK_LOCK_KEY).to_string();
        let check_keys: Vec<String> = if lock_key == BULK_LOCK_KEY { vec![lock_key.clone()] } else { vec![lock_key.clone(), BULK_LOCK_KEY.to_string()] };
        let mut locks = self.locks.lock().unwrap();
        for key in check_keys {
            if let Some(status) = locks.get(&key).filter(|s| s["status"] == "running") {
                let message = if key == lock_key {
                    "이미 재적재 작업이 진행 중입니다. 완료 후 다시 시도하세요."
                } else {
                    "다른 재적재 작업이 진행 중이라 지금은 실행할 수 없습니다. 완료 후 다시 시도하세요."
                };
                return Ok((false, Some(message.to_string()), Some(status.clone())));
            }
        }
        locks.insert(
            lock_key,
            json!({"category": category, "reload_source": reload_source, "status": "running", "indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0, "error": null}),
        );
        Ok((true, None, None))
    }

    fn heartbeat_reload_lock(&self, _ct: &str, category: Option<&str>, counts: &Value) -> Result<(), String> {
        self.heartbeats.lock().unwrap().push(counts.clone());
        let lock_key = category.unwrap_or(BULK_LOCK_KEY);
        if let Some(entry) = self.locks.lock().unwrap().get_mut(lock_key) {
            for (k, v) in counts.as_object().into_iter().flatten() {
                entry[k] = v.clone();
            }
        }
        Ok(())
    }

    fn complete_reload_lock(&self, _ct: &str, status: &str, error: Option<&str>, category: Option<&str>, counts: &Value) -> Result<(), String> {
        let lock_key = category.unwrap_or(BULK_LOCK_KEY).to_string();
        let mut locks = self.locks.lock().unwrap();
        let mut entry = locks.get(&lock_key).cloned().unwrap_or(json!({}));
        entry["status"] = json!(status);
        entry["error"] = json!(error);
        for (k, v) in counts.as_object().into_iter().flatten() {
            entry[k] = v.clone();
        }
        locks.insert(lock_key, entry);
        Ok(())
    }

    fn get_reload_status(&self, _ct: &str, category: Option<&str>) -> Result<Option<Value>, backend::main::MappingError> {
        Ok(self.locks.lock().unwrap().get(category.unwrap_or(BULK_LOCK_KEY)).cloned())
    }
}

/// `dummy_client` fixture: DummyManager를 책·만화 manager로, DummyCategoryMapping을 매핑으로 주입한다.
fn dummy_deps(dummy: &Arc<DummyManager>, mapping: Arc<dyn CategoryMappingApi>, auth: AuthMode) -> AppDeps {
    AppDeps::new(auth).with_book_manager(dummy.clone()).with_comics_manager(dummy.clone()).with_category_mapping(mapping)
}

fn dummy_client_with(dummy: &Arc<DummyManager>, mapping: Arc<dyn CategoryMappingApi>, auth: AuthMode) -> TestClient {
    common::setup_env();
    TestClient::with_app(app_with(dummy_deps(dummy, mapping, auth)))
}

fn dummy_client(dummy: &Arc<DummyManager>) -> TestClient {
    dummy_client_with(dummy, DummyCategoryMapping::new(), AuthMode::Split { auth: user_payload(), admin: admin_example_payload() })
}

fn dummy_client_mapping(dummy: &Arc<DummyManager>, mapping: Arc<dyn CategoryMappingApi>) -> TestClient {
    dummy_client_with(dummy, mapping, AuthMode::Split { auth: user_payload(), admin: admin_example_payload() })
}

async fn wait_for_reload_status(client: &TestClient, path: &str, expected: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let response = client.get(path).await.json();
            if response["result"]["status"] == expected {
                return response;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("재적재 상태가 제한 시간 안에 바뀌어야 한다")
}

fn book_payload() -> Value {
    json!({"book_id": 1, "category": "A", "title": "T", "author": "U", "file_path": "a.txt", "file_type": "txt", "file_size": 1, "line_count": 0, "page_count": 0, "isbn": "", "updated_time": NOW_ISO, "score": 0.0})
}

#[tokio::test]
async fn test_main_routes_basic() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client(&dummy);
    let payload = book_payload();
    let resp = dummy_client.put_json("/books/1", &payload).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.delete("/books/1").await;
    assert_eq!(resp.json()["status"], "success");
    let warning = &resp.json()["warning"];
    assert!(!warning.is_null() && warning != &json!(""), "warning이 비어 있다");

    let resp = dummy_client.get("/download/1").await;
    assert_eq!(resp.status, 200);

    let resp = dummy_client.get("/preview/1").await;
    assert_eq!(resp.status, 200);

    let resp = dummy_client.get("/pdf-pages/1?start=1&end=1").await;
    assert_eq!(resp.status, 200);

    let resp = dummy_client.get("/books/1").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.get("/categories/A").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.get("/categories").await;
    assert_eq!(resp.json()["status"], "success");
}

/// viewer가 숨김 카테고리 접근 시 거부된다.
struct HiddenA;

impl CategoryMappingApi for HiddenA {
    fn get_hidden_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(vec!["A".into()])
    }
}

#[tokio::test]
async fn test_viewer_hidden_category_access_control_routes() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let viewer = json!({"email": "viewer@example.com", "role": "viewer"});
    let dummy_client = dummy_client_with(&dummy, Arc::new(HiddenA), AuthMode::Split { auth: viewer, admin: admin_example_payload() });

    let resp = dummy_client.get("/books/1").await;
    assert_eq!(resp.status, 403);
    assert_eq!(resp.json()["detail"], "접근 권한이 없는 카테고리입니다.");

    let resp = dummy_client.get("/categories").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json(), json!({"status": "success", "result": {}}));

    let resp = dummy_client.get("/hidden-categories").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json(), json!({"status": "success", "result": []}));
}

#[tokio::test]
async fn test_main_search_validate_and_mismatch() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client(&dummy);

    let resp = dummy_client.get("/similar/1").await;
    assert_eq!(resp.json()["status"], "success");
    assert_eq!(resp.json()["total"], 1);

    dummy.set(|b| b.similar_ok = true);
    let resp = dummy_client.get("/similar/1").await;
    assert_eq!(resp.json()["total"], 1);

    let resp = dummy_client.get("/search/kw?exclude_categories=A,B").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.get("/latest").await;
    assert_eq!(resp.json()["status"], "success");
    assert_eq!(resp.json()["total"], 1);
    assert_eq!(resp.json()["result"][0]["created_time"], "2023-12-31T00:00:00.000000");

    assert_eq!(*dummy.latest_exclude.lock().unwrap(), Some(Vec::<String>::new()));

    let resp = dummy_client.get("/latest?limit=2001").await;
    assert_eq!(resp.status, 422);

    dummy.set(|b| b.book_type = Some("epub"));
    let resp = dummy_client.get("/validate/1").await;
    assert_eq!(resp.json()["status"], "success");

    dummy.set(|b| b.book_type = Some("pdf"));
    let resp = dummy_client.get("/validate/1").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.get("/category-mismatches").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.post_json("/category-mismatches/index-file", &json!({"file_path": "a.txt"})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.post_json("/category-mismatches/delete-file", &json!({"file_path": "a.txt"})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.delete("/category-mismatches/es-doc/1").await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.post_json("/category-mismatches/reload", &json!({"category": "A"})).await;
    assert_eq!(resp.json()["status"], "success");

    let resp = dummy_client.post_json("/category-mismatches/reload-mismatches", &json!({"category": "A"})).await;
    assert_eq!(resp.json()["status"], "success");
    assert_eq!(resp.json()["result"]["started"], true);
    assert_eq!(resp.json()["result"]["category"], "A");
    assert_eq!(resp.json()["result"]["reload_source"], "mismatch");

    // 카테고리별 락은 그 카테고리로 조회해야 한다 — 일괄(전체) 락과는 독립적인 행이다.
    let status = wait_for_reload_status(&dummy_client, "/category-mismatches/reload-status?category=A", "done").await;
    assert_eq!(status["status"], "success");
    assert_eq!(status["result"]["indexed_count"], 1);

    let resp = post_empty(&dummy_client, "/category-mismatches/reload-all").await;
    assert_eq!(resp.json()["status"], "success");
    assert_eq!(resp.json()["result"]["started"], true);
    assert_eq!(resp.json()["result"]["reload_source"], "bulk");

    let status = wait_for_reload_status(&dummy_client, "/category-mismatches/reload-status", "done").await;
    assert_eq!(status["result"]["indexed_count"], 2);
    assert_eq!(status["result"]["reload_source"], "bulk");

    let resp = dummy_client.get("/category-mismatches/A").await;
    assert_eq!(resp.json()["status"], "success");
}

#[tokio::test]
async fn test_similar_names_returns_directory_scores_out_of_100() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.categories = Some((json!({"_root": 1, "Source": 1, "Novel": 1, "Novelty": 1}), None)));
    dummy.es.clear_fault("search_categories_pdf_stats");
    dummy.es.set_categories_pdf_stats(json!({
        "Source": {"file_count": 10, "page_count": 1000, "total_file_size": 100_000_000},
        "Novel": {"file_count": 10, "page_count": 1000, "total_file_size": 100_000_000},
        "Novelty": {"file_count": 100, "page_count": 10000, "total_file_size": 1_000_000_000},
    }));
    let dummy_client = dummy_client(&dummy);

    let response = dummy_client.get("/similar-names?name=Novel&exclude_category=Source").await;

    assert_eq!(response.status, 200);
    let body = response.json();
    let results = body["result"].as_array().unwrap();
    assert_eq!(results.iter().map(|item| item["kind"].clone()).collect::<Vec<_>>(), [json!("directory"), json!("directory")]);
    assert_eq!(results[0]["category"], "Novel");
    assert_eq!(results[0]["score"].as_f64(), Some(100.0));
    // pytest.approx(70 * 5 / 6 + 3)
    let expected = 70.0 * 5.0 / 6.0 + 3.0;
    let actual = results[1]["score"].as_f64().unwrap();
    assert!((actual - expected).abs() <= expected * 1e-6, "score={actual}, expected={expected}");
}

/// 진행 콜백이 없는 구간(전체 스캔, ES refresh 등)에서도 잔여 건수가 주기적으로 갱신돼야 한다.
///
/// book_manager는 파일 단위로 on_progress를 호출하지만, 시작 시 전체 불일치 스캔이나 마지막
/// refresh 구간에는 콜백이 아예 없다. 그 구간에서도 화면의 잔여 건수와 heartbeat가 멈추지
/// 않도록, 재적재 job은 고정 주기로 최신 카운트를 DB에 flush해야 한다.
#[tokio::test]
async fn test_reload_job_flushes_progress_on_a_fixed_interval() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.scan_delay_ms = 150);
    let mapping = DummyCategoryMapping::new();

    // 상수 대신 값 설정자다. 0.02초 주기.
    let deps = dummy_deps(&dummy, mapping.clone(), AuthMode::Split { auth: json!({"email": "u@e.com", "role": "user"}), admin: json!({"email": "a@e.com", "role": "admin"}) })
        .with_reload_progress_flush_interval(Duration::from_millis(20));
    common::setup_env();
    let client = TestClient::with_app(app_with(deps));
    let resp = post_empty(&client, "/category-mismatches/reload-all").await;
    assert_eq!(resp.json()["status"], "success");

    let _ = wait_for_reload_status(&client, "/category-mismatches/reload-status", "done").await;
    let heartbeats = mapping.heartbeats.lock().unwrap().clone();
    // 콜백이 1번뿐인 0.3초짜리 작업에서 0.02초 주기 flush면 여러 번 찍혀야 한다.
    assert!(heartbeats.len() >= 5, "주기적 flush가 일어나지 않았다: {heartbeats:?}");
    // 마지막 flush에는 콜백으로 들어온 최신 카운트가 실려 있어야 한다.
    assert_eq!(heartbeats.last().unwrap().get("before_count"), Some(&json!(10)));
}

/// `get_hidden_categories`/`get_latest_excluded_categories`만 값을 돌려주는 매핑.
struct HiddenAndExcluded {
    hidden: Vec<String>,
    excluded: Vec<String>,
}

impl CategoryMappingApi for HiddenAndExcluded {
    fn get_hidden_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(self.hidden.clone())
    }

    fn get_latest_excluded_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(self.excluded.clone())
    }
}

#[tokio::test]
async fn test_latest_books_uses_latest_excluded_categories() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let mapping = Arc::new(HiddenAndExcluded { hidden: vec![], excluded: vec!["blocked".into()] });
    let dummy_client = dummy_client_mapping(&dummy, mapping);

    let resp = dummy_client.get("/latest?limit=5").await;

    assert_eq!(resp.status, 200);
    assert_eq!(*dummy.latest_size.lock().unwrap(), Some(5));
    assert_eq!(*dummy.latest_exclude.lock().unwrap(), Some(vec!["blocked".to_string()]));
}

#[tokio::test]
async fn test_latest_books_merges_viewer_hidden_and_latest_excluded_categories() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let viewer = json!({"email": "viewer@example.com", "role": "viewer"});
    let mapping = Arc::new(HiddenAndExcluded { hidden: vec!["hidden".into(), "shared".into()], excluded: vec!["latest".into(), "shared".into()] });
    let dummy_client = dummy_client_with(&dummy, mapping, AuthMode::Split { auth: viewer, admin: admin_example_payload() });

    let resp = dummy_client.get("/latest").await;

    assert_eq!(resp.status, 200);
    assert_eq!(*dummy.latest_exclude.lock().unwrap(), Some(vec!["hidden".to_string(), "shared".to_string(), "latest".to_string()]));
}

#[test]
fn test_custom_jsonable_encoder_fallback() {
    assert_eq!(backend::main::custom_jsonable_encoder(&json!(1)), json!(1));
}

#[test]
fn test_lazy_proxy_repr() {
    let slot = Slot::lazy(|| Ok(Arc::new(json!({"ok": true}))));
    assert_eq!(slot.get().unwrap().as_ref(), &json!({"ok": true}));
}

#[tokio::test]
async fn test_main_error_branches() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client(&dummy);
    // 13개 메서드를 오류 반환으로 바꾼다 (delete_book, get_book, get_books_in_category, get_categories, search_similar_books_paged,
    // search_by_keyword_paged, index_single_file, delete_file, reload_category, reload_category_mismatches,
    // reload_category_mismatch_files, get_category_mismatches, get_category_mismatch_details).
    dummy.set(|b| b.errors = true);

    let resp = dummy_client.delete("/books/1").await;
    assert_eq!(resp.json()["error"], "boom");

    let resp = dummy_client.get("/books/1").await;
    assert_eq!(resp.json()["error"], "not found");

    let resp = dummy_client.get("/categories/A").await;
    assert_eq!(resp.json()["error"], "nope");

    let resp = dummy_client.get("/categories").await;
    assert_eq!(resp.json()["error"], "fail");

    let resp = dummy_client.get("/similar/1").await;
    assert_eq!(resp.json()["error"], "none");

    let resp = dummy_client.get("/search/kw").await;
    assert_eq!(resp.json()["error"], "kw");

    let resp = dummy_client.get("/category-mismatches").await;
    assert_eq!(resp.json()["error"], "카테고리 불일치 조회 중 오류가 발생했습니다");

    let resp = dummy_client.post_json("/category-mismatches/index-file", &json!({})).await;
    assert_eq!(resp.status, 400);

    let resp = dummy_client.post_json("/category-mismatches/index-file", &json!({"file_path": "a.txt"})).await;
    assert_eq!(resp.json()["error"], "err");

    let resp = dummy_client.post_json("/category-mismatches/delete-file", &json!({})).await;
    assert_eq!(resp.status, 400);

    let resp = dummy_client.post_json("/category-mismatches/delete-file", &json!({"file_path": "a.txt"})).await;
    assert_eq!(resp.json()["error"], "bad");

    dummy.es.set_delete_result(false);
    let resp = dummy_client.delete("/category-mismatches/es-doc/1").await;
    assert!(resp.json()["error"].as_str().unwrap().contains("ES 문서 삭제 실패"), "{}", resp.json());

    let resp = dummy_client.post_json("/category-mismatches/reload", &json!({"category": "A"})).await;
    assert_eq!(resp.json()["error"], "fail");

    let resp = dummy_client.post_json("/category-mismatches/reload-mismatches", &json!({"category": "A"})).await;
    assert_eq!(resp.json()["result"]["started"], true);
    let status = wait_for_reload_status(&dummy_client, "/category-mismatches/reload-status?category=A", "failed").await;
    assert_eq!(status["result"]["error"], "mismatch fail");

    dummy.set(|b| b.mismatch_files_raise = true);
    let resp = dummy_client.post_json("/category-mismatches/reload-mismatches", &json!({"category": "A"})).await;
    assert_eq!(resp.json()["result"]["started"], true);
    let status = wait_for_reload_status(&dummy_client, "/category-mismatches/reload-status?category=A", "failed").await;
    assert_eq!(status["result"]["error"], "카테고리 불일치 조회 중 오류가 발생했습니다");

    let resp = post_empty(&dummy_client, "/category-mismatches/reload-all").await;
    assert_eq!(resp.json()["result"]["started"], true);
    assert_eq!(resp.json()["result"]["reload_source"], "bulk");
    let status = wait_for_reload_status(&dummy_client, "/category-mismatches/reload-status", "failed").await;
    assert_eq!(status["result"]["error"], "bulk fail");
    assert_eq!(status["result"]["reload_source"], "bulk");

    let resp = dummy_client.get("/category-mismatches/A").await;
    assert_eq!(resp.json()["error"], "카테고리 불일치 조회 중 오류가 발생했습니다");
}

#[tokio::test]
async fn test_reload_all_mismatches_preserves_requested_reload_source() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.post_json("/category-mismatches/reload-all", &json!({"reload_source": "mismatch"})).await;
    assert_eq!(resp.json()["status"], "success");
    assert_eq!(resp.json()["result"]["started"], true);
    assert_eq!(resp.json()["result"]["reload_source"], "mismatch");

    let status = wait_for_reload_status(&dummy_client, "/category-mismatches/reload-status", "done").await;
    assert_eq!(status["status"], "success");
    assert_eq!(status["result"]["reload_source"], "mismatch");
}

#[tokio::test]
async fn test_reload_all_mismatches_rejects_unknown_reload_source() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.post_json("/category-mismatches/reload-all", &json!({"reload_source": "unknown"})).await;
    assert_eq!(resp.status, 422);
}

#[tokio::test]
async fn test_wake_storage_error() {
    let mut env = EnvGuard::new();
    env.set("TM_FRONTEND_URL", "http://testserver");
    let client = client_with_fs(AuthMode::Cookies, ListDirFs::err("fail")).header("host", "testserver");
    let resp = client.get("/wake").await;
    assert_eq!(resp.status, 503);
    assert_eq!(resp.json(), json!({"status": "failure"}));
}

#[tokio::test]
async fn test_search_bookstore_api_branches() {
    // fake_search -> ([], "", "unknown"). factory 하나가 13개 서점 이름 모두에 쓰인다.
    let dummy_client = client_with_store(Arc::new(StubStore { rows: Vec::new(), keyword: String::new(), kind: "unknown".into() }));

    let names = ["yes24", "aladin", "kyobo", "ridi", "naver", "naverseries", "munpia", "joara", "naverwebtoon", "kakaowebtoon", "bomtoon", "lezhin", "toptoon"];

    for name in names {
        assert_eq!(dummy_client.get(&format!("/search/bookstore/{name}?title=t")).await.json()["status"], "not_found", "bookstore={name}");
    }

    let resp = dummy_client.get("/search/bookstore/unknown?title=t").await;
    assert_eq!(resp.status, 404);
}

/// 모든 메서드가 RuntimeError를 던지는 매핑. `set_keywords`만 False를 돌려준다.
struct FailingMapping;

impl CategoryMappingApi for FailingMapping {
    fn get_all_mappings(&self, _ct: &str) -> Result<Value, MappingError> {
        Err(MappingError::Other("boom".into()))
    }

    fn get_keywords(&self, _category: &str, _ct: &str) -> Result<Vec<String>, MappingError> {
        Err(MappingError::Other("boom2".into()))
    }

    fn set_keywords(&self, _category: &str, _keywords: &[String], _ct: &str) -> Result<bool, MappingError> {
        Ok(false)
    }

    fn add_keyword(&self, _category: &str, _keyword: &str, _ct: &str) -> Result<bool, MappingError> {
        Err(MappingError::Other("boom3".into()))
    }

    fn remove_keyword(&self, _category: &str, _keyword: &str, _ct: &str) -> Result<bool, MappingError> {
        Err(MappingError::Other("boom4".into()))
    }

    fn delete_category(&self, _category: &str, _ct: &str, _prefix: bool) -> Result<bool, MappingError> {
        Err(MappingError::Other("boom5".into()))
    }

    fn update_all_mappings(&self, _mappings: &Value, _ct: &str) -> Result<bool, MappingError> {
        Err(MappingError::Other("boom6".into()))
    }

    fn get_hidden_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Err(MappingError::Other("boom7".into()))
    }

    fn set_hidden(&self, _category: &str, _hidden: bool, _ct: &str) -> Result<bool, MappingError> {
        Err(MappingError::Other("boom8".into()))
    }

    fn get_latest_excluded_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Err(MappingError::Other("boom9".into()))
    }

    fn set_latest_excluded(&self, _category: &str, _excluded: bool, _ct: &str) -> Result<bool, MappingError> {
        Err(MappingError::Other("boom10".into()))
    }
}

#[tokio::test]
async fn test_category_mapping_error_branches() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(FailingMapping));

    let resp = dummy_client.get("/category-mappings").await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");

    let resp = dummy_client.get("/category-mappings/A").await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");

    let resp = dummy_client.put_json("/category-mappings/A", &json!({"keywords": ["x"]})).await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "Failed to set keywords");

    let resp = dummy_client.post_json("/category-mappings/A/keywords", &json!({"keyword": "x"})).await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");

    let resp = dummy_client.delete("/category-mappings/A/keywords/x").await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");

    let resp = dummy_client.delete("/category-mappings/A").await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");

    let resp = dummy_client.put_json("/category-mappings", &json!({"mappings": {"A": ["x"]}})).await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");

    let resp = dummy_client.get("/hidden-categories").await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "비노출 카테고리 처리 중 오류가 발생했습니다");

    let resp = dummy_client.post_json("/hidden-categories/A", &json!({"hidden": true})).await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "비노출 카테고리 처리 중 오류가 발생했습니다");

    let resp = dummy_client.get("/latest-excluded-categories").await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "최신 자료 검색 제외 카테고리 처리 중 오류가 발생했습니다");

    let resp = dummy_client.post_json("/latest-excluded-categories/A", &json!({"excluded": true})).await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "최신 자료 검색 제외 카테고리 처리 중 오류가 발생했습니다");
}

#[test]
fn test_cookie_settings_variants() {
    let mut env = EnvGuard::new();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.set("TM_COOKIE_SECURE", "false");
    env.set("TM_COOKIE_SAMESITE", "none");
    let (secure, samesite) = backend::main::_get_cookie_settings();
    assert!(!secure);
    assert_eq!(samesite, "lax");

    env.set("TM_COOKIE_SECURE", "true");
    env.set("TM_COOKIE_SAMESITE", "strict");
    let (secure, samesite) = backend::main::_get_cookie_settings();
    assert!(secure);
    assert_eq!(samesite, "strict");
}

#[test]
fn test_set_and_clear_auth_cookies() {
    let mut env = EnvGuard::new();
    env.set("TM_COOKIE_SECURE", "true");
    env.set("TM_COOKIE_SAMESITE", "lax");

    let mut resp = axum::Json(json!({"ok": true})).into_response();
    backend::main::_set_auth_cookies(&mut resp, "access", Some("refresh"));
    let headers = resp.headers().get("set-cookie").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    assert!(headers.contains("tm_access_token"), "{headers}");

    let mut resp2 = axum::Json(json!({"ok": true})).into_response();
    backend::main::_clear_auth_cookies(&mut resp2);
    let headers2 = resp2.headers().get("set-cookie").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
    assert!(headers2.contains("Max-Age=0"), "{headers2}");
}

#[test]
fn test_cookie_settings_defaults_to_lax_for_unknown_value() {
    let mut env = EnvGuard::new();
    env.set("TM_FRONTEND_URL", "http://localhost:3000");
    env.set("TM_COOKIE_SECURE", "false");
    env.set("TM_COOKIE_SAMESITE", "unexpected");
    let (secure, samesite) = backend::main::_get_cookie_settings();
    assert!(!secure);
    assert_eq!(samesite, "lax");
}

#[test]
fn test_set_auth_cookies_without_refresh_token() {
    let mut env = EnvGuard::new();
    env.set("TM_COOKIE_SECURE", "false");
    env.set("TM_COOKIE_SAMESITE", "lax");

    let mut resp = axum::Json(json!({"ok": true})).into_response();
    backend::main::_set_auth_cookies(&mut resp, "access-only", None);
    let headers: Vec<String> = resp.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect();
    assert_eq!(headers.len(), 1);
    assert!(headers[0].contains("tm_access_token=access-only"), "{}", headers[0]);
}

/// `google_id_token.verify_oauth2_token`과 `TM_GOOGLE_CLIENT_ID` 대역. 요청 사이에 테스트가 결과를 바꾼다.
struct ScriptedVerifier {
    client_id: Mutex<Option<String>>,
    result: Mutex<Result<Value, String>>,
}

impl GoogleVerifier for ScriptedVerifier {
    fn client_id(&self) -> Option<String> {
        self.client_id.lock().unwrap().clone()
    }

    fn verify(&self, _credential: &str, _audience: &str) -> Result<Value, String> {
        self.result.lock().unwrap().clone()
    }
}

/// `determine_role` 대역.
struct ScriptedRole(Mutex<Option<String>>);

impl RoleResolver for ScriptedRole {
    fn determine_role(&self, _email: &str) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
}

/// 인증 route 테스트에서 발급·회전만 받는 refresh store 대역.
struct AuthRefreshStore;

impl backend::refresh_token_store::RefreshTokenStoreApi for AuthRefreshStore {
    fn store_issued(&self, _: &str, _: &str, _: &str, _: i64, _: i64, _: Option<&str>, _: Option<&str>) -> Result<(), backend::refresh_token_store::StoreError> {
        Ok(())
    }
    fn rotate(&self, _: &str, _: &str, _: &str, _: &str, _: i64, _: i64, _: Option<&str>, _: Option<&str>) -> Result<String, backend::refresh_token_store::StoreError> {
        Ok("ok".into())
    }
    fn revoke_family(&self, _: &str, _: &str) -> Result<(), backend::refresh_token_store::StoreError> {
        Err(backend::refresh_token_store::StoreError::Unsupported("test store".into()))
    }
    fn get_token_observation(&self, _: &str) -> Result<Option<Value>, backend::refresh_token_store::StoreError> {
        Err(backend::refresh_token_store::StoreError::Unsupported("test store".into()))
    }
    fn _session_rows(&self, _: Option<&str>, _: Option<&str>) -> Result<Vec<backend::db::Row>, backend::refresh_token_store::StoreError> {
        Err(backend::refresh_token_store::StoreError::Unsupported("test store".into()))
    }
}

#[tokio::test]
async fn test_auth_google_branches() {
    let mut env = EnvGuard::new();
    let verifier = Arc::new(ScriptedVerifier { client_id: Mutex::new(None), result: Mutex::new(Err("unset".into())) });
    let role = Arc::new(ScriptedRole(Mutex::new(None)));
    common::setup_env();
    let app = app_with(AppDeps::new(AuthMode::Cookies).with_google_verifier(verifier.clone()).with_role_resolver(role.clone()).with_refresh_token_store(Arc::new(AuthRefreshStore)));
    let client = TestClient::with_app(app);

    let resp = client.post_json("/auth/google", &json!({})).await;
    assert_eq!(resp.status, 400);

    env.set("TM_GOOGLE_CLIENT_ID", "cid");
    *verifier.client_id.lock().unwrap() = Some("cid".into());
    *verifier.result.lock().unwrap() = Err("bad token".to_string());
    let resp = client.post_json("/auth/google", &json!({"credential": "x"})).await;
    assert_eq!(resp.status, 401);

    *verifier.result.lock().unwrap() = Ok(json!({"aud": "cid", "iss": "https://evil.example.com", "email": "a", "email_verified": true}));
    let resp = client.post_json("/auth/google", &json!({"credential": "x"})).await;
    assert_eq!(resp.status, 401);

    *verifier.result.lock().unwrap() = Ok(json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "a", "email_verified": false, "name": "n", "picture": "p"}));
    let resp = client.post_json("/auth/google", &json!({"credential": "x"})).await;
    assert_eq!(resp.status, 401);

    *verifier.result.lock().unwrap() = Ok(json!({"aud": "cid", "iss": "https://accounts.google.com", "email": "a", "email_verified": true, "name": "n", "picture": "p"}));
    *role.0.lock().unwrap() = None;
    let resp = client.post_json("/auth/google", &json!({"credential": "x"})).await;
    assert_eq!(resp.status, 403);

    *role.0.lock().unwrap() = Some("admin".to_string());
    let resp = client.post_json("/auth/google", &json!({"credential": "x"})).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["status"], "success");
}

#[tokio::test]
async fn test_auth_refresh_missing_cookie() {
    let client = TestClient::new();
    let resp = post_empty(&client, "/auth/refresh").await;
    assert_eq!(resp.status, 400);
}

#[tokio::test]
async fn test_auth_refresh_access_denied() {
    // 실제 refresh token을 검증하고, 서버 역할 판정에서 접근을 거부한다.
    common::setup_env();
    let token = create_refresh_token("x", "viewer", None, None);
    let app = app_with(AppDeps::new(AuthMode::Cookies).with_role_resolver(Arc::new(ScriptedRole(Mutex::new(None)))).with_refresh_token_store(Arc::new(AuthRefreshStore)));
    let client = TestClient::with_app(app).cookie("tm_refresh_token", &token);
    let resp = post_empty(&client, "/auth/refresh").await;
    assert_eq!(resp.status, 403);
}

#[tokio::test]
async fn test_auth_me() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.get("/auth/me").await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json()["status"], "success");
}

// ---- coverage: uncovered lines ----

/// 서점 이름 해석은 현재 registry factory를 거친다.
#[test]
fn test_create_bookstore_factory() {
    assert!(backend::bookstore::create_bookstore_search("yes24").is_some());
    assert!(backend::bookstore::create_bookstore_search("unknown").is_none());
}

#[tokio::test]
async fn test_update_book_error_path() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.update_fail = true);
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.put_json("/books/1", &book_payload()).await;
    assert_eq!(resp.json(), json!({"status": "failure", "error": "update failed"}));
}

#[tokio::test]
async fn test_rename_category_error_path() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.rename = Some(Arc::new(|old, new| (None, Some(format!("cannot rename {old} -> {new}"))))));
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.put_json("/categories/rename", &json!({"old_category": "A", "new_category": "B"})).await;
    assert_eq!(resp.json(), json!({"status": "failure", "error": "cannot rename A -> B"}));
}

#[tokio::test]
async fn test_delete_category_error_path() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.delete_category = Some(Arc::new(|category| (None, Some(format!("cannot delete {category}"))))));
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.post_json("/categories/delete", &json!({"category": "A"})).await;
    assert_eq!(resp.json(), json!({"status": "failure", "error": "cannot delete A"}));
}

/// 책·만화 관리 탭이 같은 미리보기 엔드포인트를 쓴다.
#[tokio::test]
async fn test_category_delete_preview() {
    for prefix in ["", "/comics"] {
        let tmp = TempDir::new().unwrap();
        let dummy = DummyManager::new(tmp.path());
        dummy.set(|b| b.preview = Some(Arc::new(|category| (Some(json!({"category": category, "directory_count": 2, "file_count": 5, "es_count": 4})), None))));
        let dummy_client = dummy_client(&dummy);
        let resp = dummy_client.get(&format!("{prefix}/category-delete-preview?category={}", enc("A/B"))).await;
        assert_eq!(resp.json(), json!({"status": "success", "result": {"category": "A/B", "directory_count": 2, "file_count": 5, "es_count": 4}}), "prefix={prefix:?}");
    }
}

#[tokio::test]
async fn test_category_delete_preview_error_path() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.preview = Some(Arc::new(|category| (Some(json!({})), Some(format!("bad {category}"))))));
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.get("/category-delete-preview?category=A").await;
    assert_eq!(resp.json(), json!({"status": "failure", "error": "bad A"}));
}

/// `rename_category`만 구현한 매핑.
struct RenameMapping {
    result: bool,
}

impl CategoryMappingApi for RenameMapping {
    fn rename_category(&self, _old: &str, _new: &str, _ct: &str) -> Result<bool, MappingError> {
        Ok(self.result)
    }
}

/// Line 309: rename_category MySQL mapping update failure
#[tokio::test]
async fn test_rename_category_mysql_mapping_failure() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.rename = Some(Arc::new(|old, new| (Some(json!({"old_category": old, "new_category": new, "updated_count": 1, "fs_renamed": false})), None))));
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(RenameMapping { result: false }));

    let resp = dummy_client.put_json("/categories/rename", &json!({"old_category": "A", "new_category": "B"})).await;
    let data = resp.json();
    assert_eq!(data["status"], "success");
    assert_eq!(data["result"]["mapping_updated"], false);
}

/// delete_category가 숨김·최신 제외 하위 카테고리를 정리하는지 기록한다.
struct DeleteTrackingMapping {
    delete_result: bool,
    hidden: Vec<String>,
    latest_excluded: Vec<String>,
    hidden_removed: Arc<Mutex<Vec<String>>>,
    latest_excluded_removed: Arc<Mutex<Vec<String>>>,
}

impl CategoryMappingApi for DeleteTrackingMapping {
    fn delete_category(&self, _cat: &str, _ct: &str, _prefix: bool) -> Result<bool, MappingError> {
        Ok(self.delete_result)
    }

    fn set_hidden(&self, cat: &str, hidden: bool, _ct: &str) -> Result<bool, MappingError> {
        if !hidden {
            self.hidden_removed.lock().unwrap().push(cat.to_string());
        }
        Ok(true)
    }

    fn get_hidden_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(self.hidden.clone())
    }

    fn set_latest_excluded(&self, cat: &str, excluded: bool, _ct: &str) -> Result<bool, MappingError> {
        if !excluded {
            self.latest_excluded_removed.lock().unwrap().push(cat.to_string());
        }
        Ok(true)
    }

    fn get_latest_excluded_categories(&self, _ct: &str) -> Result<Vec<String>, MappingError> {
        Ok(self.latest_excluded.clone())
    }
}

/// delete_category cleans hidden and latest-excluded subcategories.
#[tokio::test]
async fn test_delete_category_hidden_subcategories() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.delete_category = Some(Arc::new(|category| (Some(json!({"category": category, "deleted_count": 1})), None))));

    let hidden_removed = Arc::new(Mutex::new(Vec::new()));
    let latest_excluded_removed = Arc::new(Mutex::new(Vec::new()));
    let mapping = Arc::new(DeleteTrackingMapping {
        delete_result: true,
        hidden: vec!["A/sub1".into(), "A/sub2".into(), "B/other".into()],
        latest_excluded: vec!["A/sub3".into(), "A/sub4".into(), "B/other".into()],
        hidden_removed: hidden_removed.clone(),
        latest_excluded_removed: latest_excluded_removed.clone(),
    });
    let dummy_client = dummy_client_mapping(&dummy, mapping);

    let resp = dummy_client.post_json("/categories/delete", &json!({"category": "A"})).await;
    let data = resp.json();
    assert_eq!(data["status"], "success");
    let hidden_removed = hidden_removed.lock().unwrap().clone();
    let latest_excluded_removed = latest_excluded_removed.lock().unwrap().clone();
    assert!(hidden_removed.contains(&"A/sub1".to_string()));
    assert!(hidden_removed.contains(&"A/sub2".to_string()));
    assert!(!hidden_removed.contains(&"B/other".to_string()));
    assert!(latest_excluded_removed.contains(&"A/sub3".to_string()));
    assert!(latest_excluded_removed.contains(&"A/sub4".to_string()));
    assert!(!latest_excluded_removed.contains(&"B/other".to_string()));
}

/// `set_keywords`가 일반 예외를 던지는 매핑.
struct SetKeywordsRaises;

impl CategoryMappingApi for SetKeywordsRaises {
    fn set_keywords(&self, _category: &str, _keywords: &[String], _ct: &str) -> Result<bool, MappingError> {
        Err(MappingError::Other("unexpected".into()))
    }
}

/// Lines 673-675: set_category_keywords non-HTTPException error
#[tokio::test]
async fn test_set_category_keywords_unexpected_exception() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(SetKeywordsRaises));

    let resp = dummy_client.put_json("/category-mappings/A", &json!({"keywords": ["x"]})).await;
    assert_eq!(resp.status, 500);
    assert_eq!(resp.json()["detail"], "카테고리 매핑 처리 중 오류가 발생했습니다");
}

/// `update_all_mappings`의 결과를 정하는 매핑.
struct UpdateAllMapping(Result<bool, MappingError>);

impl CategoryMappingApi for UpdateAllMapping {
    fn update_all_mappings(&self, _mappings: &Value, _ct: &str) -> Result<bool, MappingError> {
        match &self.0 {
            Ok(v) => Ok(*v),
            Err(MappingError::Http(status, detail)) => Err(MappingError::Http(*status, detail.clone())),
            Err(MappingError::Other(msg)) => Err(MappingError::Other(msg.clone())),
        }
    }
}

/// Line 739: update_all_category_mappings returns False
#[tokio::test]
async fn test_update_all_category_mappings_failure() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(UpdateAllMapping(Ok(false))));

    let resp = dummy_client.put_json("/category-mappings", &json!({"mappings": {"A": ["x"]}})).await;
    assert_eq!(resp.status, 500);
    assert!(resp.json()["detail"].as_str().unwrap().contains("Failed to update mappings"), "{}", resp.json());
}

/// Line 741: update_all_category_mappings HTTPException re-raise
#[tokio::test]
async fn test_update_all_category_mappings_http_exception_reraise() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(UpdateAllMapping(Err(MappingError::Http(422, "validation error".into())))));

    let resp = dummy_client.put_json("/category-mappings", &json!({"mappings": {"A": ["x"]}})).await;
    assert_eq!(resp.status, 422);
}

/// `set_hidden`의 결과를 정하는 매핑.
struct SetHiddenMapping(Result<bool, MappingError>);

impl CategoryMappingApi for SetHiddenMapping {
    fn set_hidden(&self, _category: &str, _hidden: bool, _ct: &str) -> Result<bool, MappingError> {
        match &self.0 {
            Ok(v) => Ok(*v),
            Err(MappingError::Http(status, detail)) => Err(MappingError::Http(*status, detail.clone())),
            Err(MappingError::Other(msg)) => Err(MappingError::Other(msg.clone())),
        }
    }
}

/// Line 775: set_hidden_category returns False
#[tokio::test]
async fn test_set_hidden_category_failure() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(SetHiddenMapping(Ok(false))));

    let resp = dummy_client.post_json("/hidden-categories/A", &json!({"hidden": true})).await;
    assert_eq!(resp.status, 500);
    assert!(resp.json()["detail"].as_str().unwrap().contains("Failed to update hidden category"), "{}", resp.json());
}

/// Line 777: set_hidden_category HTTPException re-raise
#[tokio::test]
async fn test_set_hidden_category_http_exception_reraise() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    let dummy_client = dummy_client_mapping(&dummy, Arc::new(SetHiddenMapping(Err(MappingError::Http(422, "test error".into())))));

    let resp = dummy_client.post_json("/hidden-categories/A", &json!({"hidden": true})).await;
    assert_eq!(resp.status, 422);
}

/// Lines 238-239: delete_book 'Ok' 경로
#[tokio::test]
async fn test_delete_book_ok_path() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.delete_book_ok = true);
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.delete("/books/1").await;
    let data = resp.json();
    assert_eq!(data["status"], "success");
    assert_eq!(data["result"], "Ok");
}

/// Lines 270-271: validate_book 책 없음 경로
#[tokio::test]
async fn test_validate_book_not_found() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.book_missing = true);
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.get("/validate/999").await;
    let data = resp.json();
    assert_eq!(data["status"], "failure");
    assert!(data["error"].as_str().unwrap().contains("Book not found"), "{data}");
}

/// Lines 278-279: validate_book 지원하지 않는 파일 타입
#[tokio::test]
async fn test_validate_book_unsupported_type() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    // get_book_txt: file_type이 txt인 책 (기본값)
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.get("/validate/1").await;
    let data = resp.json();
    assert_eq!(data["status"], "failure");
    assert!(data["error"].as_str().unwrap().contains("not supported"), "{data}");
}

/// Line 285: validate_book 에러 경로
#[tokio::test]
async fn test_validate_book_error_result() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| {
        b.book_type = Some("epub");
        b.validate_epub_fail = true;
    });
    let dummy_client = dummy_client(&dummy);
    let resp = dummy_client.get("/validate/1").await;
    let data = resp.json();
    assert_eq!(data["status"], "failure");
    assert_eq!(data["error"], "validation failed");
}

/// Line 333: delete_category mapping_deleted=False 경로
#[tokio::test]
async fn test_delete_category_mapping_not_deleted() {
    let tmp = TempDir::new().unwrap();
    let dummy = DummyManager::new(tmp.path());
    dummy.set(|b| b.delete_category = Some(Arc::new(|category| (Some(json!({"category": category, "deleted_count": 1})), None))));
    let mapping = Arc::new(DeleteTrackingMapping {
        delete_result: false,
        hidden: vec![],
        latest_excluded: vec![],
        hidden_removed: Arc::new(Mutex::new(Vec::new())),
        latest_excluded_removed: Arc::new(Mutex::new(Vec::new())),
    });
    let dummy_client = dummy_client_mapping(&dummy, mapping);

    let resp = dummy_client.post_json("/categories/delete", &json!({"category": "A"})).await;
    let data = resp.json();
    assert_eq!(data["status"], "success");
    assert_eq!(data["result"]["mapping_deleted"], false);
}

// ---- merged from test_main_env_guard.py ----

/// 재배포로 죽은 백그라운드 작업이 화면과 재실행을 막지 않아야 한다.
///
/// 실제로 겪은 일이다. 재배포로 백그라운드 태스크가 사라졌는데 상태 파일은 running
/// 으로 남아, POST 가 already_running 으로 막고 화면은 6분째 회전했다.
/// 멈춘 작업이 버튼을 영영 막지 않아야 한다.
///
/// 순수 함수만 검증하면 배선이 빠져도 통과한다. DB 행부터 응답까지, 그리고 실제로
/// 새 요청이 받아들여지는지까지 확인한다.
#[tokio::test]
async fn test_stale_running_status_lets_the_button_work_again() {
    let s = backend_test_setup().await;
    let mapping = CategoryMapping::new().expect("MySQL 연결");
    mapping.set_classify_proposal_status(&json!({"status": "running", "source_category": CATEGORY}), "book").unwrap();
    // 하트비트를 DB 서버 시계로 과거로 민다. 파이썬 지역 시간을 쓰면 앱과 DB 의
    // 시간대 차이가 섞여 의도한 나이가 만들어지지 않는다.
    {
        let mut conn = mapping.get_connection().unwrap();
        conn.execute("UPDATE classify_proposal_status SET updated_at = DATE_SUB(NOW(3), INTERVAL 3600 SECOND) WHERE content_type = 'book'", &[]).unwrap();
        conn.commit().unwrap();
    }
    let _cleanup = Defer(|| {
        CategoryMapping::new().expect("MySQL 연결").set_classify_proposal_status(&json!({"status": "idle"}), "book").unwrap();
    });

    let result = s.client.get("/categories/classify-proposal").await.json();
    assert_eq!(result["result"]["status"], "failed");
    // 여기서 멈추면 "조회만 되고 버튼은 여전히 막힌" 상태를 놓친다. 없는 카테고리로
    // 요청해 백그라운드 작업이 곧바로 끝나게 하고, 선점이 풀렸는지만 본다.
    let started = s.client.post_json("/categories/classify-proposal", &json!({"category": "__no_such_category__"})).await.json();
    assert!(started["result"].get("already_running").is_none_or(Value::is_null));
    assert_eq!(started["result"]["started"], true);
    let _ = &s.cm;
}
