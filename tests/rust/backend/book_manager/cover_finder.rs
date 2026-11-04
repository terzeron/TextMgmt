//! `backend::book_manager::cover_finder` 단위 테스트.

#[path = "../common/book_fixtures.rs"]
mod fx;

#[path = "../common/mod.rs"]
mod common;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use backend::book_manager::{BookManager, BookManagerDeps, Thumbnailer};
use common::epub::{BODY, EpubSpec, make_epub};
use common::es_fake::FakeEsManager;
use common::fsx::ino_mtime;
use common::image::{b64, red_jpeg};
use common::pdf::write_blank_pdf;
use serde_json::{json, Value};
use backend::cover_management::CoverManagement;
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover;
use backend::epub_cover::{CoverError, MAX_IMAGE_BYTES};
use common::es_dummy;
use common::zipx::{read_zip_entry, rewrite_epub_members};
use fx::{Edit, Opts};

fn create_test_pdf(output_path: &Path) {
    write_blank_pdf(output_path, 612, 792);
}

fn make_doc(relative_path: &str, file_type: &str) -> Value {
    json!({"category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type, "file_size": 1, "line_count": 0, "page_count": 1, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"})
}

/// 클로저를 `Thumbnailer`로 감싼다.
// TODO(common): `common`에 `Thumbnailer` 클로저 어댑터가 없다(docs/rust-test-seams.md 3.12는 trait 이름만 정한다). 생기면 지운다.
struct FnThumbnailer<F>(F);

impl<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync> Thumbnailer for FnThumbnailer<F> {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        (self.0)(path, file_type, width)
    }
}

fn thumbnailer<F: Fn(&Path, &str, Option<u32>) -> Option<Vec<u8>> + Send + Sync + 'static>(f: F) -> Arc<dyn Thumbnailer> {
    Arc::new(FnThumbnailer(f))
}

fn use_doc(es: &FakeEsManager, relative_path: &str, file_type: &str) {
    es.set_doc(Some(make_doc(relative_path, file_type)));
}

#[tokio::test]
async fn test_cover_ignores_legacy_id_only_cache() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let es = FakeEsManager::new();
    let relative = "test_category/legacy-cache.pdf";
    create_test_pdf(&tmp.path().join(relative));
    use_doc(&es, relative, "pdf");
    let legacy = tmp.path().join(".cover_cache/124.v6.jpg");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"wrong book cover").unwrap();

    let extractor = thumbnailer(|_path, _type, _width| Some(b"correct cover".to_vec()));
    let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_thumbnailer(extractor));
    let response = manager.get_cover(124, None).await;

    assert_eq!(response.body, b"correct cover");
}

/// `common::fx::fetcher`가 생기면 지운다. 클로저를 `ImageFetcher`로 감싼다.
// TODO(common): `common::fx::fetcher(|url| ...)` (docs/rust-test-seams.md 3.7). 아직 없어 로컬로 둔다.
struct FnFetcher<F>(F);

impl<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync> ImageFetcher for FnFetcher<F> {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0)(url)
    }
}

fn fetcher<F: Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static>(f: F) -> Arc<dyn ImageFetcher> {
    Arc::new(FnFetcher(f))
}

// TODO(common): `es_dummy::truthy(Option<&Value>)`를 `&Value`로 받는 얇은 래퍼. cover_fixtures::truthy를 대신한다.
fn truthy(value: &Value) -> bool {
    es_dummy::truthy(Some(value))
}

fn broken_cover(tmp: &Path) -> PathBuf {
    let path = make_epub(&tmp.join("book.epub"), &EpubSpec::cover("3.0", true));
    rewrite_epub_members(&path, |members| members.set("OPS/cover.jpg", &b"not a JPEG"[..]));
    path
}

fn first_item(service: &CoverManagement, covered: bool) -> Value {
    service.missing("", "", 60, covered).unwrap()["items"][0].clone()
}

#[test]
fn test_broken_cover_search_and_replacement_preserve_body_and_backup() {
    for method in ["image", "url"] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = broken_cover(root);
        let original = std::fs::read(&path).unwrap();
        let stat = ino_mtime(&path);
        let mut service = CoverManagement::new(root);
        let item = first_item(&service, false);
        service.set_hooks(service.hooks().clone().with_search(|_store, _metadata| Ok(json!({"status": "not_found", "candidates": []}))));
        assert_eq!(service.candidates("yes24", "book.epub", true, false).unwrap()["status"], "not_found", "method={method}");
        // 기존 표지 존재 여부와 덮어쓰기 동의는 손상 여부와 별개다.
        let revision = item["revision"].as_str().unwrap();
        let err = epub_cover::add_cover(&path, &red_jpeg(), revision, &root.join(".cover_backups"), false).unwrap_err();
        assert!(err.to_string().contains("이미 표지"), "method={method}: {err}");
        let bad: Result<Value, CoverError> = service.apply_image("book.epub", &b64(b"bad replacement"), revision);
        assert!(bad.is_err(), "method={method}");
        assert_eq!(std::fs::read(&path).unwrap(), original, "method={method}");
        assert!(!root.join(".cover_backups").exists(), "method={method}");
        let result = if method == "image" {
            service.apply_image("book.epub", &b64(&red_jpeg()), revision).unwrap()
        } else {
            service.set_hooks(service.hooks().clone().with_image_fetcher(fetcher(|_url| Ok(red_jpeg()))));
            service.apply_url("book.epub", "https://example.org/cover.jpg", revision, true).unwrap()
        };
        assert!(!truthy(&epub_cover::inspect_epub(&path).unwrap()["cover_error"]), "method={method}");
        assert_eq!(std::fs::read(root.join(result["backup_path"].as_str().unwrap())).unwrap(), original, "method={method}");
        assert_eq!(ino_mtime(&path), stat, "method={method}");
        assert_eq!(read_zip_entry(&path, "OPS/chapter.xhtml"), BODY.as_bytes(), "method={method}");
    }
}

mod upload_limits {
    use super::{common, fx};
    use std::fs;
    use backend::book_manager::{BookManager, BookManagerDeps};
    use backend::epub_cover::{EpubCoverHooks, Limits};
    use common::edits;

    use common::image::png_solid;
    use fx::{Edit, Env, image_env, Opts};

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

    #[tokio::test]
    async fn test_upload_insert_with_too_many_pixels_changes_nothing() {
        let e = with_deps(image_env(Opts::loader()), true, |d| d.with_epub_hooks(EpubCoverHooks::default().with_limits(Limits { max_image_pixels: 100, ..Limits::default() })));
        let original = fs::read(&e.pdf).unwrap();

        let (result, error) = e.run(&Edit { inserts: Some(vec![edits::upload(2, &png_solid(20, 20, [10, 120, 200]))]), ..Edit::default() }).await;

        assert!(result.is_none() && error_text(&error).contains("해상도가 너무 큽니다"), "{error:?}");
        e.assert_nothing_changed(&original);
    }
}

fn error_text(error: &Option<String>) -> &str {
    error.as_deref().unwrap_or("")
}

#[tokio::test]
async fn test_upload_rejects_encoded_data_over_byte_limit_before_decoding() {
    let e = fx::env(Opts::plain());
    let encoded_limit = MAX_IMAGE_BYTES.div_ceil(3) * 4;
    let insert = serde_json::json!({"page": 1, "image_data": "A".repeat(encoded_limit + 1)});

    let (result, error) = e.run(&Edit { inserts: Some(vec![insert]), ..Edit::default() }).await;

    assert!(result.is_none() && error_text(&error).contains("이미지가 비어 있거나 너무 큽니다"), "{error:?}");
}

mod download_failures {
    use super::common;
    use std::fs;

    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use backend::book_metadata::BookLookupError;
    use backend::book_manager::{BookLookup, BookManager, BookManagerDeps, BookRef};
    use backend::direct_cover_url::{FetchError, ImageFetcher};


    use common::epub::{EpubSpec, make_epub};
    use common::es_fake::FakeEsManager;


    use common::image::red_jpeg;

    use serde_json::Value;

    const URL: &str = "https://img.example.com/cover.jpg";

    // ── 대역 (env fixture) ─────────────────────────────────────

    type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync>;

    /// `monkeypatch.setattr("backend.cover_management.fetch_url_image", ...)`의 대응. 테스트 중간에 바꿀 수 있다.
    /// `BookManagerDeps::with_image_fetcher`로 주입한다. 오류 타입은 `FetchError`(구 `EpubCoverError`).
    struct FetchSlot(Mutex<Fetch>);

    impl FetchSlot {
        fn set(&self, f: impl Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static) {
            *self.0.lock().unwrap() = Box::new(f);
        }
    }

    impl ImageFetcher for FetchSlot {
        fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
            (self.0.lock().unwrap())(url)
        }
    }

    /// `AsyncMock(return_value=(book, None))` get_book.
    struct FakeBooks(Mutex<(Option<BookRef>, Option<String>)>);

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    impl FakeBooks {
        fn set_file_type(&self, file_type: &str) {
            self.0.lock().unwrap().0.as_mut().unwrap().file_type = file_type.to_string();
        }

        fn set_missing(&self, message: &str) {
            *self.0.lock().unwrap() = (None, Some(message.to_string()));
        }
    }

    #[async_trait::async_trait]
    impl BookLookup for FakeBooks {
        async fn get_book(&self, book_id: i64) -> Result<BookRef, BookLookupError> {
            match self.0.lock().unwrap().clone() {
                (Some(book), _) => Ok(book),
                (None, Some(error)) => Err(BookLookupError::Failed(error)),
                (None, None) => Err(BookLookupError::NotFound(book_id)),
            }
        }
    }

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    /// (manager 대역, EPUB 경로, 책). get_book은 이 EPUB를 가리키는 책을 돌려준다.
    struct Env {
        tmp: tempfile::TempDir,
        path: PathBuf,
        manager: BookManager,
        es: Arc<FakeEsManager>,
        books: Arc<FakeBooks>,
        fetcher: Arc<FetchSlot>,
    }

    fn env() -> Env {
        common::setup_env();
        let tmp = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(FetchSlot(Mutex::new(Box::new(|_| Ok(red_jpeg())))));
        let path = make_epub(&tmp.path().join("분류").join("book.epub"), &EpubSpec::cover("3.0", false));
        let es = FakeEsManager::new();
        let books = Arc::new(FakeBooks(Mutex::new((Some(BookRef { book_id: 1, file_path: path.clone(), file_type: "epub".into() }), None))));
        let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_trash_dirname("0_trash").with_book_lookup(books.clone()).with_image_fetcher(fetcher.clone()));
        Env { tmp, path, manager, es, books, fetcher }
    }

    // 테스트를 나눈 뒤 이 파일의 테스트는 이 정의의 일부만 쓴다.
    #[allow(dead_code)]
    impl Env {
        fn dir(&self) -> &Path {
            self.tmp.path()
        }

        async fn call(&self, url: &str) -> (Option<Value>, Option<String>) {
            split(self.manager.set_epub_cover(1, Some(url), None).await)
        }

        async fn upload(&self, image_data: &str) -> (Option<Value>, Option<String>) {
            split(self.manager.set_epub_cover(1, None, Some(image_data)).await)
        }

        async fn remove(&self) -> (Option<Value>, Option<String>) {
            split(self.manager.remove_epub_cover(1).await)
        }

        async fn illustrate(&self, before: &str) -> (Option<Value>, Option<String>) {
            split(self.manager.insert_epub_illustration(1, before, Some(URL), None).await)
        }

        fn size(&self) -> u64 {
            fs::metadata(&self.path).unwrap().len()
        }
    }

    /// `"x" in error`: error가 None이면 Python은 TypeError로 실패한다.
    /// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
    fn split(result: Result<Value, String>) -> (Option<Value>, Option<String>) {
        match result {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    }

    fn err_has(error: &Option<String>, needle: &str) -> bool {
        error.as_deref().unwrap_or_else(|| panic!("error가 None이다 (기대: {needle:?} 포함)")).contains(needle)
    }

    #[tokio::test]
    async fn test_download_failure_returns_the_reason_and_keeps_the_file() {
        let env = env();
        env.fetcher.set(|_| Err(FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".to_string())));
        let before = fs::read(&env.path).unwrap();

        let (result, error) = env.call(URL).await;

        assert!(result.is_none() && err_has(&error, "내부 주소"));
        assert_eq!(fs::read(&env.path).unwrap(), before);
        assert!(env.es.updates().is_empty(), "update는 불리면 안 된다");
    }

    #[tokio::test]
    async fn test_unexpected_io_error_hides_internal_details() {
        let env = env();
        env.fetcher.set(|_| Err(FetchError::Os("/secret/disk/path failed".to_string())));

        let (result, error) = env.call(URL).await;

        assert!(result.is_none());
        assert!(!err_has(&error, "secret"));
    }
}
