#[path = "../common/book_fixtures.rs"]
mod fx;

#[path = "../common/mod.rs"]
mod common;
use backend::book_manager::{BookManager, BookManagerDeps};
use common::es_fake::FakeEsManager;
use common::faulty_fs::FaultyFs;
use common::pdf::make_pdf;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use fx::{image_env, Opts};

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

mod upload_limits {
    use super::{common, fx};
    use backend::book_manager::{BookManager, BookManagerDeps};
    use backend::epub_cover::{EpubCoverHooks, Limits};
    use common::image::png_solid;
    use fx::{Env, image_env, Opts};

    /// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
    fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
        match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    }

    /// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
    // TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
    fn with_deps(mut e: Env, loader: bool, configure: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Env {
        let mut deps = BookManagerDeps::new(e.root.clone(), e.es.clone());
        if loader {
            deps = deps.with_file_reader(e.reader.clone());
        }
        e.bm = BookManager::with_deps(configure(deps));
        e
    }

    fn error_text(error: &Option<String>) -> &str {
        error.as_deref().unwrap_or("")
    }

    // ── 올린 이미지 ──

    #[tokio::test]
    async fn test_upload_preview_rejects_too_many_pixels() {
        let e = with_deps(image_env(Opts::plain()), false, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));

        let (jpeg, error) = split(e.bm.preview_insert_upload(&png_solid(20, 20, [10, 120, 200])).await);

        assert!(jpeg.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
    }
}

fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

#[tokio::test]
async fn test_upload_preview_rejects_oversized_data() {
    let e = image_env(Opts::plain());

    let (jpeg, error) = split(e.bm.preview_insert_upload(&vec![0u8; backend::epub_cover::MAX_IMAGE_BYTES + 1]).await);

    assert!(jpeg.is_none() && error_text(&error).contains("큽니다"), "{error:?}");
}

mod backup_reporting {
    use backend::es_manager::{Docs};
    use super::fx;
    use fx::Opts;

    fn error_text(error: &Option<String>) -> &str {
        error.as_deref().unwrap_or("")
    }

    // ── 재색인 실패 ──

    #[tokio::test]
    async fn test_unreadable_saved_file_reports_backup_location() {
        let e = fx::env(Opts::loader());
        // patch("utils.loader.Loader.read_file", return_value={})
        *e.reader.docs.lock().unwrap() = Docs::new();

        let (result, error) = e.run_pages(&[3]).await;

        assert!(result.is_none());
        assert!(error_text(&error).contains("재색인할 문서를 읽지 못했습니다") && error_text(&error).contains("0_trash/book.pdf"), "{error:?}");
        assert!(e.es.inserted().is_empty());
        assert!(e.root.join("0_trash").join("book.pdf").is_file());
    }
}
