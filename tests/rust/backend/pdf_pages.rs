//! 원본: tests/test_pdf_pages.py
//!
//! get_pdf_pages 메서드 및 /pdf-pages/ 엔드포인트 단위 테스트. Docker/ES 없이 mock 기반이다.
//!
//! 실제 PDF fixture로 페이지 범위·캐시 예산·디스크 캐시 재사용을 검증한다.
//! HTTP 테스트는 같은 BookManager를 AppDeps에 주입한다.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::{AppDeps, AuthMode, app_with};
use backend::pdf::{LopdfPageOps, PageCountCache, PdfCacheLimits, PdfError, PdfPageOps, PdfReaderCache, PdfReaderHandle};
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::faulty_fs::FaultyFs;
use common::pdf::{make_blank_pdf, page_count_bytes};
use serde_json::{Value, json};
use tempfile::TempDir;

trait ResponseExt {
    fn header(&self, name: &str) -> Option<&str>;
}

impl ResponseExt for backend::main::BookResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

/// `create_test_pdf`
fn create_test_pdf(num_pages: usize, output_path: &Path) {
    make_blank_pdf(output_path, num_pages);
}

/// ES 문서 dict 생성 헬퍼.
fn make_doc(relative_path: &str, file_size: u64, page_count: i64, file_type: &str) -> Value {
    json!({
        "category": "test_category", "title": "Test Book", "author": "Author", "file_path": relative_path, "file_type": file_type,
        "file_size": file_size, "line_count": 0, "page_count": page_count, "isbn": "", "summary": "test", "updated_time": "2024-01-01T00:00:00.000000"
    })
}

struct Env {
    bm: Arc<BookManager>,
    es: Arc<FakeEsManager>,
    temp_dir: PathBuf,
    _tmp: TempDir,
}

/// `configure`는 임시 디렉터리와 기본 Deps를 받는다. 캐시 한도처럼 만들어 둔 파일 크기가 필요한 설정에 쓴다.
fn build(configure: impl FnOnce(&Path, BookManagerDeps) -> BookManagerDeps) -> Env {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let temp_dir = tmp.path().to_path_buf();
    let pdf = temp_dir.join("test_category").join("test_book.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    create_test_pdf(5, &pdf);
    let default_doc = make_doc("test_category/test_book.pdf", fs::metadata(&pdf).unwrap().len(), 5, "pdf");
    let es = FakeEsManager::with_doc(default_doc.clone());
    let deps = configure(&temp_dir, BookManagerDeps::new(temp_dir.clone(), es.clone()));
    Env { bm: Arc::new(BookManager::with_deps(deps)), es, temp_dir, _tmp: tmp }
}

fn env() -> Env {
    build(|_, d| d)
}

fn body_contains(body: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(body).contains(needle)
}

#[test]
fn pdf_caches_expose_consistent_empty_clear_and_lookup_state() {
    let reader_cache = PdfReaderCache::new(PdfCacheLimits::default());
    assert!(reader_cache.is_empty());
    assert_eq!(reader_cache.len(), 0);
    assert!(!reader_cache.contains("book.pdf"));
    reader_cache.insert_raw("book.pdf", 1, 5, 12);
    assert!(!reader_cache.is_empty());
    assert!(reader_cache.contains("book.pdf"));
    reader_cache.clear();
    assert!(reader_cache.is_empty());

    let page_cache = PageCountCache::new(2);
    let first = ("one.pdf".to_string(), 10);
    let second = ("two.pdf".to_string(), 20);
    assert!(page_cache.is_empty());
    assert!(!page_cache.contains(&first));
    page_cache.insert(first.clone(), 3);
    page_cache.insert(second.clone(), 4);
    assert_eq!(page_cache.get(&first), Some(3));
    assert!(page_cache.contains(&second));
    page_cache.evict();
    assert_eq!(page_cache.len(), 2);
    page_cache.clear();
    assert!(page_cache.is_empty());
}

#[test]
fn lopdf_page_ops_falls_back_to_path_for_a_reader_without_a_document() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reader-fallback.pdf");
    create_test_pdf(2, &path);
    let reader = PdfReaderHandle::new(path, 2);

    let bytes = LopdfPageOps.extract_pages_from_reader(&reader, 0, 1).unwrap();

    assert_eq!(page_count_bytes(&bytes), 1);
}

struct CountingPdfPages {
    opens: Arc<AtomicUsize>,
    cached_extracts: Arc<AtomicUsize>,
}

impl PdfPageOps for CountingPdfPages {
    fn page_count(&self, _path: &Path) -> Result<usize, PdfError> {
        Ok(5)
    }

    fn extract_pages(&self, _path: &Path, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
        panic!("cached reader path should be used")
    }

    fn open_reader(&self, path: &Path) -> Result<PdfReaderHandle, PdfError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(PdfReaderHandle::new(path.to_path_buf(), 5))
    }

    fn extract_pages_from_reader(&self, _reader: &PdfReaderHandle, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
        self.cached_extracts.fetch_add(1, Ordering::SeqCst);
        Ok(b"cached PDF pages".to_vec())
    }
}

mod test_get_pdf_pages {
    use super::*;

    #[tokio::test]
    async fn test_returns_single_page() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 1, 1).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));
        assert_eq!(response.media_type.as_deref(), Some("application/pdf"));
        assert_eq!(response.header("Content-Encoding"), Some("identity"));

        assert_eq!(page_count_bytes(&response.body), 1);
    }

    #[tokio::test]
    async fn get_pdf_pages_reuses_the_cached_reader_for_different_ranges() {
        let opens = Arc::new(AtomicUsize::new(0));
        let cached_extracts = Arc::new(AtomicUsize::new(0));
        let pages = Arc::new(CountingPdfPages { opens: opens.clone(), cached_extracts: cached_extracts.clone() });
        let e = build(|_, deps| deps.with_pdf_pages(pages));

        assert_eq!(e.bm.get_pdf_pages(1, 1, 1).await.status_code, 200);
        assert_eq!(e.bm.get_pdf_pages(1, 2, 2).await.status_code, 200);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        assert_eq!(cached_extracts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_returns_page_range() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 2, 4).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));

        assert_eq!(page_count_bytes(&response.body), 3); // 2, 3, 4
    }

    #[tokio::test]
    async fn test_returns_all_pages() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 1, 5).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(page_count_bytes(&response.body), 5);
    }

    #[tokio::test]
    async fn test_end_clamped_to_total_pages() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 3, 100).await;

        assert_eq!(response.status_code, 200);
        assert_eq!(page_count_bytes(&response.body), 3); // 3, 4, 5
    }

    #[tokio::test]
    async fn test_start_exceeds_total_pages() {
        let e = env();
        let response = e.bm.get_pdf_pages(1, 100, 200).await;

        assert_eq!(response.status_code, 400);
    }

    #[tokio::test]
    async fn test_end_before_start_is_rejected() {
        let e = env();
        assert_eq!(e.bm.get_pdf_pages(1, 3, 2).await.status_code, 400);
        assert!(!e.temp_dir.join(".preview_cache").join("1_p3-2.pdf").exists());
    }

    #[tokio::test]
    async fn test_range_over_the_page_limit_is_rejected() {
        // 범위마다 캐시 파일이 생기므로 한 요청의 쪽 수를 제한한다. 뷰어는 11쪽씩 요청한다.
        let e = env();
        create_test_pdf(60, &e.temp_dir.join("test_category").join("test_book.pdf"));
        assert_eq!(e.bm.get_pdf_pages(1, 1, 50).await.status_code, 200);
        assert_eq!(e.bm.get_pdf_pages(1, 1, 51).await.status_code, 400);
    }

    #[tokio::test]
    async fn test_preview_cache_is_keyed_by_page_count() {
        // 큰 pages 요청이 캐시를 덮어 다른 사용자의 기본 미리보기가 전체 책이 되면 안 된다.
        let e = env();
        let cache_dir = e.temp_dir.join(".preview_cache");

        let two = e.bm.get_book_preview(1, Some(2), None).await;
        assert_eq!(page_count_bytes(&two.body), 2);
        let four = e.bm.get_book_preview(1, Some(4), None).await;
        assert_eq!(page_count_bytes(&four.body), 4);
        let two_again = e.bm.get_book_preview(1, Some(2), None).await;
        assert_eq!(page_count_bytes(&two_again.body), 2);

        assert!(cache_dir.join("1_n2.pdf").exists());
        assert!(!cache_dir.join("1_n4.pdf").exists(), "같은 책의 다른 쪽 수 캐시는 하나만 남긴다");
        assert!(!cache_dir.join("1.pdf").exists());
    }

    #[tokio::test]
    async fn test_cache_writes_leave_no_temp_files() {
        let e = env();
        assert_eq!(e.bm.get_book_preview(1, Some(2), None).await.status_code, 200);
        assert_eq!(e.bm.get_pdf_pages(1, 1, 2).await.status_code, 200);
        let names: Vec<String> = fs::read_dir(e.temp_dir.join(".preview_cache")).unwrap().map(|x| x.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[tokio::test]
    async fn test_book_not_found() {
        let e = env();
        e.es.set_doc(None);

        let response = e.bm.get_pdf_pages(999, 1, 1).await;
        assert_eq!(response.status_code, 404);
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let e = env();

        // 첫 번째 호출: 캐시 생성
        let response1 = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(response1.status_code, 200);

        // 캐시 파일 존재 확인
        let cache_file = e.temp_dir.join(".preview_cache").join("1_p1-2.pdf");
        assert!(cache_file.exists());

        // 두 번째 호출: 캐시 히트 (같은 결과)
        let response2 = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(response2.status_code, 200);
        assert_eq!(response2.header("X-Total-Pages"), Some("5"));
    }

    #[tokio::test]
    async fn test_not_a_pdf_file() {
        let e = env();

        let txt_path = e.temp_dir.join("test_category").join("test.txt");
        fs::write(&txt_path, "hello").unwrap();

        e.es.set_doc(Some(make_doc("test_category/test.txt", fs::metadata(&txt_path).unwrap().len(), 5, "txt")));

        let response = e.bm.get_pdf_pages(1, 1, 1).await;
        assert_eq!(response.status_code, 400);
    }

    #[tokio::test]
    async fn test_missing_file_returns_404() {
        let e = env();
        e.es.set_doc(Some(make_doc("test_category/gone.pdf", 1234, 5, "pdf")));

        let response = e.bm.get_pdf_pages(1, 1, 1).await;
        assert_eq!(response.status_code, 404);
        assert!(body_contains(&response.body, "File not found"));
    }

    /// 스토리지 마운트가 끊겨 stat()이 EIO를 내면 사유를 담은 503을 반환해야 한다.
    ///
    /// 예전에는 이 OSError가 그대로 escape해 CORSMiddleware를 우회한 500이 나갔고,
    /// 브라우저에는 "Failed to fetch"로만 보여 원인을 알 수 없었다.
    #[tokio::test]
    async fn test_storage_io_error_returns_503_with_reason() {
        // errno.EIO(5)와 os.strerror(EIO)
        let faulty = FaultyFs::new();
        faulty.set_is_file(|_| Some(std::io::Error::from_raw_os_error(5)));
        let e = build(|_, d| d.with_fs(faulty));

        let response = e.bm.get_pdf_pages(1, 1, 1).await;

        assert_eq!(response.status_code, 503);
        assert!(body_contains(&response.body, "Storage access error"));
        assert!(body_contains(&response.body, "Input/output error"));
    }
}

mod test_pdf_pages_endpoint {
    use super::*;
    use backend::main::{CategoryMappingApi, MappingError};

    struct HiddenCategories;

    impl CategoryMappingApi for HiddenCategories {
        fn get_hidden_categories(&self, content_type: &str) -> Result<Vec<String>, MappingError> {
            Ok(if content_type == "book" { vec!["test_category".into()] } else { vec![] })
        }
    }

    #[tokio::test]
    async fn test_authentication_precedes_input_validation() {
        let e = env();
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(e.bm.clone())));
        for path in ["/pdf-pages/1", "/pdf-pages/invalid?start=bad", "/comics/pdf-pages/1", "/preview/1", "/download/1", "/validate/1", "/html-resource/1?path=x", "/comics/preview/1"] {
            assert_eq!(client.get(path).await.status, 401);
        }
    }

    #[tokio::test]
    async fn test_invalid_integer_returns_422() {
        let (client, _e) = setup_client();
        for path in ["/pdf-pages/invalid", "/pdf-pages/1?start=no", "/pdf-pages/1?end=no"] {
            assert_eq!(client.get(path).await.status, 422);
        }
    }

    #[tokio::test]
    async fn test_viewer_hidden_category_and_descendants_are_forbidden() {
        let e = env();
        let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_book_manager(e.bm.clone()).with_category_mapping(Arc::new(HiddenCategories));
        let client = TestClient::with_app(app_with(deps));
        for category in ["test_category", "test_category/child"] {
            e.es.with(|state| state.default_doc.as_mut().unwrap()["category"] = json!(category));
            for path in ["/pdf-pages/1", "/preview/1", "/download/1", "/validate/1", "/cover/1", "/html-resource/1?path=x"] {
                let response = client.get(path).await;
                assert_eq!(response.status, 403, "{path}");
                assert_eq!(response.json()["detail"], "접근 권한이 없는 카테고리입니다.");
            }
        }
        e.es.with(|state| state.default_doc.as_mut().unwrap()["category"] = json!("test_category_other"));
        assert_eq!(client.get("/pdf-pages/1").await.status, 200);
    }

    #[tokio::test]
    async fn test_comics_uses_comic_manager_and_category_policy() {
        let e = env();
        let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_comics_manager(e.bm.clone()).with_category_mapping(Arc::new(HiddenCategories));
        let client = TestClient::with_app(app_with(deps));
        let response = client.get("/comics/pdf-pages/1?start=2&end=3").await;
        assert_eq!(response.status, 200);
        assert_eq!(page_count_bytes(&response.body), 2);
    }

    #[tokio::test]
    async fn test_preview_page_limit_and_comics_route() {
        let e = env();
        let comics = backend::comics_manager::ComicsManager::with_deps(BookManagerDeps::new(e.temp_dir.clone(), e.es.clone()));
        let deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(e.bm.clone()).with_comics_manager(Arc::new(comics));
        let client = TestClient::with_app(app_with(deps));
        for prefix in ["", "/comics"] {
            let response = client.get(&format!("{prefix}/preview/1?pages=2")).await;
            assert_eq!(response.status, 200);
            assert_eq!(page_count_bytes(&response.body), 2);
            assert_eq!(client.get(&format!("{prefix}/preview/1?pages=bad")).await.status, 422);
            let validation = client.get(&format!("{prefix}/validate/1")).await;
            assert_eq!(validation.json()["status"], "success");
            assert_eq!(validation.json()["result"]["publication"]["page_count"], 5);
        }
    }

    /// 라우터 클로저가 캡처한 manager의 내부 속성을 패치하는 fixture를 `AppDeps::with_book_manager`로 대신한다.
    fn setup_client() -> (TestClient, Env) {
        let e = env();
        let token = backend::auth::create_jwt_token("admin@test.com", "admin", Some("Test Admin"), None);
        let client = TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(e.bm.clone()))).cookie("tm_access_token", &token);
        (client, e)
    }

    #[tokio::test]
    async fn test_get_pdf_pages_default_params() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1").await;
        assert_eq!(response.status, 200);
        assert!(response.header("X-Total-Pages").is_some());
    }

    #[tokio::test]
    async fn test_get_pdf_pages_with_range() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1?start=2&end=3").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));
        assert_eq!(response.header("content-type"), Some("application/pdf"));
    }

    #[tokio::test]
    async fn test_get_pdf_pages_x_total_pages_exposed() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1?start=1&end=1").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.header("X-Total-Pages"), Some("5"));
    }

    #[tokio::test]
    async fn test_get_pdf_pages_start_exceeds_total() {
        let (client, _e) = setup_client();
        let response = client.get("/pdf-pages/1?start=100&end=200").await;
        assert_eq!(response.status, 400);
    }
}

/// `_pdf_reader_cache`가 개수가 아니라 바이트로 제한되는지 검증한다.
///
/// cached PdfReader 1개는 PDF 파일 크기와 거의 1:1로 RSS를 차지한다
/// (pod 실측: 134MiB 파일 -> RSS +134.2MiB). 개수만 제한하면 /comics 의 1.5GiB 급
/// PDF 8개가 한 worker 에서 12GiB 를 점유해 노드 메모리를 고갈시킨다.
mod test_pdf_reader_cache_budget {
    use super::*;

    /// 파일당 상한을 넘는 PDF 는 reader 를 캐시하지 않는다 (페이지 수만 캐시).
    #[test]
    fn test_large_pdf_reader_not_cached() {
        let e = build(|dir, d| {
            let pdf = dir.join("test_category").join("budget_large.pdf");
            create_test_pdf(3, &pdf);
            let size = fs::metadata(&pdf).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(size - 1), ..PdfCacheLimits::default() })
        });
        let pdf = e.temp_dir.join("test_category").join("budget_large.pdf");

        let (reader, total_pages) = e.bm._get_cached_pdf_reader(&pdf);

        assert_eq!(total_pages, 3);
        assert!(Arc::strong_count(&reader) >= 1); // reader 자체는 정상 반환되어야 한다
        assert!(!e.bm.pdf_cache().contains(&pdf.to_string_lossy())); // 그러나 보관하지 않는다
        // 페이지 수는 int 하나라 비용이 없으므로 항상 캐시한다
        assert!(e.bm.page_count_cache().contains(&(pdf.to_string_lossy().into_owned(), common::fsx::mtime_key(&pdf))));
    }

    #[test]
    fn test_small_pdf_reader_is_cached_and_reused() {
        let e = build(|dir, d| {
            let pdf = dir.join("test_category").join("budget_small.pdf");
            create_test_pdf(2, &pdf);
            let size = fs::metadata(&pdf).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(size + 1), ..PdfCacheLimits::default() })
        });
        let pdf = e.temp_dir.join("test_category").join("budget_small.pdf");

        let (reader1, _) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(e.bm.pdf_cache().contains(&pdf.to_string_lossy()));
        let (reader2, _) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(Arc::ptr_eq(&reader1, &reader2)); // 같은 객체를 재사용 (재파싱 없음)
    }

    /// 전체 예산을 넘으면 개수와 무관하게 LRU 로 축출한다.
    #[test]
    fn test_byte_budget_evicts_lru() {
        let e = build(|dir, d| {
            let first = dir.join("test_category").join("budget_0.pdf");
            create_test_pdf(2, &first);
            let one_size = fs::metadata(&first).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits {
                max_file_bytes: Some(one_size + 1),
                max_bytes: Some(one_size * 2 + 1),
                max_entries: Some(100), // 개수는 넉넉히
            })
        });
        let mut pdfs = Vec::new();
        for i in 0..3 {
            let p = e.temp_dir.join("test_category").join(format!("budget_{i}.pdf"));
            create_test_pdf(2, &p);
            pdfs.push(p);
        }

        for p in &pdfs {
            e.bm._get_cached_pdf_reader(p);
        }

        assert_eq!(e.bm.pdf_cache().len(), 2);
        assert!(!e.bm.pdf_cache().contains(&pdfs[0].to_string_lossy())); // 가장 오래된 것이 축출
        assert!(e.bm.pdf_cache().contains(&pdfs[2].to_string_lossy()));
        let total: u64 = pdfs.iter().filter(|p| e.bm.pdf_cache().contains(&p.to_string_lossy())).map(|p| fs::metadata(p).unwrap().len()).sum();
        assert!(total <= e.bm.pdf_cache().limits().max_bytes.unwrap());
    }

    #[test]
    fn test_mtime_change_invalidates_reader_cache() {
        let e = build(|_, d| d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(10 * 1024 * 1024), ..PdfCacheLimits::default() }));
        let pdf = e.temp_dir.join("test_category").join("budget_mtime.pdf");
        create_test_pdf(2, &pdf);

        let (reader1, pages1) = e.bm._get_cached_pdf_reader(&pdf);
        assert_eq!(pages1, 2);

        create_test_pdf(4, &pdf); // 내용 교체
        let meta = fs::metadata(&pdf).unwrap();
        let times = fs::FileTimes::new().set_accessed(meta.accessed().unwrap() + Duration::from_secs(10)).set_modified(meta.modified().unwrap() + Duration::from_secs(10));
        fs::OpenOptions::new().write(true).open(&pdf).unwrap().set_times(times).unwrap();

        let (reader2, pages2) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(!Arc::ptr_eq(&reader2, &reader1));
        assert_eq!(pages2, 4);
        assert_eq!(e.bm.pdf_cache().len(), 1); // 낡은 항목이 남지 않는다
    }

    /// 실제 페이지 수 조회와 추출에서 PDF를 읽는 횟수를 센다.
    struct CountingOpener {
        inner: backend::pdf::LopdfPageOps,
        opened: Arc<AtomicUsize>,
    }

    impl backend::pdf::PdfPageOps for CountingOpener {
        fn page_count(&self, path: &Path) -> Result<usize, backend::pdf::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.page_count(path)
        }

        fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, backend::pdf::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.extract_pages(path, start, end)
        }
    }

    /// 디스크 .preview_cache 히트 시 PdfReader 를 만들지 않아야 한다.
    ///
    /// 예전에는 reader 를 디스크 캐시 확인보다 먼저 만들었기 때문에, 캐시 히트여도
    /// GB급 PDF 를 통째로 파싱했다 (관측된 최장 응답 125초).
    #[tokio::test]
    async fn test_disk_cache_hit_does_not_open_pdf() {
        let opened = Arc::new(AtomicUsize::new(0));
        let counter = opened.clone();
        let e = build(move |_, d| d.with_pdf_pages(Arc::new(CountingOpener { inner: backend::pdf::LopdfPageOps, opened: counter })));

        let first = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(first.status_code, 200);

        // reader 캐시만 비운다. 페이지 수 캐시와 디스크 캐시는 남은 상태.
        e.bm.pdf_cache().clear();
        let before = opened.load(Ordering::SeqCst);
        assert!(before > 0);

        let second = e.bm.get_pdf_pages(1, 1, 2).await;

        assert_eq!(second.status_code, 200);
        assert_eq!(second.header("X-Total-Pages"), Some("5"));
        // `assert not mock_reader.called`: 두 번째 호출은 PDF를 열지 않는다.
        assert_eq!(opened.load(Ordering::SeqCst), before);
    }
}
