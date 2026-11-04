#[path = "../common/mod.rs"]
mod common;

use backend::book_manager::{BookManager, BookManagerDeps};
use common::{
    es_fake::FakeEsManager,
    faulty_fs::FaultyFs,
    pdf::{make_pdf, widths},
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
struct Env {
    bm: BookManager,
    es: Arc<FakeEsManager>,
    fs: Arc<FaultyFs>,
    pdf: PathBuf,
    root: tempfile::TempDir,
}

fn env() -> Env {
    common::setup_env();
    let root = tempfile::tempdir().unwrap();
    let pdf = root.path().join("cat/book.pdf");
    make_pdf(&pdf, 5);
    let es = FakeEsManager::with_doc(
        json!({"category":"cat", "title":"Title", "author":"Author", "file_path":"cat/book.pdf", "file_type":"pdf", "file_size": 0, "page_count":5, "updated_time":"2024-01-01T00:00:00.000000"}),
    );
    let fs = FaultyFs::new();
    let bm = BookManager::with_deps(BookManagerDeps::new(root.path().to_path_buf(), es.clone()).with_fs(fs.clone()));
    Env { bm, es, fs, pdf, root }
}

// 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
#[allow(dead_code)]
impl Env {
    async fn edit(&self, pages: &[i64], order: Option<&[i64]>) -> (Option<Value>, Option<String>) {
        split(self.bm.delete_pdf_pages(1, pages, None, None, None, order).await)
    }
    fn backup(&self) -> PathBuf {
        self.root.path().join("0_trash/book.pdf")
    }
    fn assert_no_parts(&self) {
        for dir in [self.pdf.parent().unwrap().to_path_buf(), self.root.path().join("0_trash")] {
            if let Ok(entries) = fs::read_dir(dir) {
                assert!(entries.flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
            }
        }
    }
}

#[tokio::test]
async fn http_edits_books_and_comics_with_python_response_contract() {
    use backend::main::{AppDeps, AuthMode, app_with};
    for prefix in ["", "/comics"] {
        let e = env();
        let mut deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"})));
        if prefix.is_empty() {
            deps = deps.with_book_manager(Arc::new(e.bm));
        } else {
            deps = deps.with_comics_manager(Arc::new(backend::comics_manager::ComicsManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()))));
        }
        let client = common::TestClient::with_app(app_with(deps));
        let response = client.post_json(&format!("{prefix}/pdf-pages/1/delete"), &json!({"pages":[2,4]})).await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.json()["status"], "success");
        assert_eq!(response.json()["result"]["remaining_pages"], 3);
        assert_eq!(widths(&e.pdf), [100, 102, 104]);
        let response = client.post_json(&format!("{prefix}/pdf-pages/1/delete"), &json!({"pages":[99]})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json()["status"], "failure");
    }
}

#[tokio::test]
async fn http_rejects_non_admin_before_editing_or_body_validation() {
    use backend::main::{AppDeps, AuthMode, app_with};
    for prefix in ["", "/comics"] {
        for viewer in [false, true] {
            let e = env();
            let original = fs::read(&e.pdf).unwrap();
            let bm = Arc::new(e.bm);
            let mut client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(bm.clone()).with_comics_manager(bm)));
            if viewer {
                let jwt = backend::auth_service::create_jwt_token("viewer@test.com", "viewer", None, None);
                client = client.cookie("tm_access_token", &jwt);
            }
            let response = client.post_json(&format!("{prefix}/pdf-pages/1/delete"), &json!({"pages":"invalid"})).await;
            assert_eq!(response.status, if viewer { 403 } else { 401 }, "{}", response.text());
            assert_eq!(fs::read(&e.pdf).unwrap(), original);
        }
    }
}

#[tokio::test]
async fn custom_trash_directory_is_used() {
    let e = env();
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_trash_dirname("backup"));
    let result = bm.delete_pdf_pages(1, &[1], None, None, None, None).await.unwrap();
    assert_eq!(result["backup"], "backup/book.pdf");
    assert!(e.root.path().join("backup/book.pdf").is_file());
}

struct FetchImage(Result<Vec<u8>, backend::direct_cover_url::FetchError>);
impl backend::direct_cover_url::ImageFetcher for FetchImage {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, backend::direct_cover_url::FetchError> {
        assert_eq!(url, "https://example.com/image.png");
        self.0.clone()
    }
}

#[tokio::test]
async fn url_preview_is_the_same_jpeg_that_gets_inserted() {
    let e = env();
    let fetcher = Arc::new(FetchImage(Ok(common::image::png_solid(80, 160, [10, 20, 30]))));
    let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(fetcher));
    let preview = bm.preview_insert_image("  https://example.com/image.png  ").await.unwrap();
    let inserts = [json!({"page":1,"url":"https://example.com/image.png"})];
    bm.delete_pdf_pages(1, &[], None, None, Some(&inserts), None).await.unwrap();
    assert_eq!(common::pdf::first_xobject(&common::pdf::load(&e.pdf), 1).content, preview);
}

#[tokio::test]
async fn url_preview_reports_safe_fetch_errors() {
    use backend::direct_cover_url::FetchError;
    for (effect, expected) in [
        (FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".into()), "내부 주소의 이미지는 사용할 수 없습니다"),
        (FetchError::Os("dns error".into()), "이미지 서버 주소를 확인하지 못했습니다"),
        (FetchError::Other("private details".into()), "이미지를 가져오지 못했습니다"),
    ] {
        let e = env();
        let bm = BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(Arc::new(FetchImage(Err(effect)))));
        assert_eq!(bm.preview_insert_image("https://example.com/image.png").await, Err(expected.into()));
    }
    let e = env();
    for url in ["", "  ", "http://127.0.0.1/a", "ftp://example.com/a"] {
        let (image, error) = split(e.bm.preview_insert_image(url).await);
        assert!(image.is_none() && error.is_some());
    }
}

#[tokio::test]
async fn http_url_preview_returns_uncached_jpeg() {
    use backend::main::{AppDeps, AuthMode, app_with};
    let e = env();
    let bm =
        Arc::new(BookManager::with_deps(BookManagerDeps::new(e.root.path().to_path_buf(), e.es.clone()).with_image_fetcher(Arc::new(FetchImage(Ok(common::image::png_solid(80, 160, [10, 20, 30])))))));
    let client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(bm.clone()).with_comics_manager(bm)));
    for prefix in ["", "/comics"] {
        let path = format!("{prefix}/pdf-pages/1/insert-preview");
        let response = client.post_json(&path, &json!({"url":"https://example.com/image.png"})).await;
        assert_eq!(response.status, 200, "{}", response.text());
        assert_eq!(response.header("content-type"), Some("image/jpeg"));
        assert_eq!(response.header("cache-control"), Some("private, no-store"));
        assert_eq!(common::image::image_size(&response.body), (80, 160));
        assert_eq!(client.post_json(&path, &json!({"url":""})).await.status, 422);
        assert_eq!(client.post_json(&path, &json!({"url":"  "})).await.status, 400);
    }
}
