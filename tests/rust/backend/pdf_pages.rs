//! 원본: tests/test_pdf_pages.py
//!
//! get_pdf_pages 메서드 및 /pdf-pages/ 엔드포인트 단위 테스트. Docker/ES 없이 mock 기반이다.
//!
//! 가정한 API (docs/rust-test-seams.md 3.2, 3.5, 3.11, 3.12):
//! - `BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>) -> BookManagerDeps` (인자 순서는 (path, es))
//!   - `.with_fs(Arc<dyn FsOps>)`: `Path.is_file` 바꿔치기. 구 `with_is_file`. 테스트는 `common::faulty_fs::FaultyFs::set_is_file`로 오류를 낸다
//!   - `.with_pdf_cache_limits(PdfCacheLimits)`: 구 클래스 상수 `PDF_READER_CACHE_MAX_*`와 `set_pdf_reader_cache_limits` 가드
//!   - `.with_pdf_opener(Arc<dyn PdfOpener>)`: 구 `with_open_pdf_observer`. 열기 횟수를 세는 대역으로 대신한다
//! - `BookManager::with_deps(BookManagerDeps) -> BookManager`, `BookManager::get_pdf_pages(&self, book_id: i64, start: i64, end: i64) -> BookResponse`
//!   - `BookManager`는 `BookManagerApi`를 구현한다(`with_book_manager(Arc<dyn BookManagerApi>)`에 `Arc<BookManager>`를 넘긴다)
//! - `backend::book_manager::BookResponse { status_code: u16, media_type: Option<String>, headers: Vec<(String, String)>, body: Vec<u8> }`와
//!   `fn header(&self, name: &str) -> Option<&str>` (대소문자 무시). 구 `PdfPagesResponse`
//! - 인스턴스 소유 캐시(구 클래스 수준 정적 캐시): `BookManager::pdf_cache(&self) -> &PdfReaderCache`
//!   - 문서가 정한 것: `clear()`, `contains(&Path) -> bool`, `len() -> usize`, `entry_sizes() -> Vec<u64>`
//!   - 문서에 없어 가정한 것: `clear_page_counts()`, `page_count_contains(&Path, SystemTime) -> bool`, `max_bytes() -> u64` (구 `pdf_reader_cache_max_bytes()`)
//!   - `BookManager::get_cached_pdf_reader(&self, &Path) -> std::io::Result<(Arc<PdfReader>, usize)>` (구 정적 함수를 인스턴스 메서드로 가정)
//!   - 인스턴스마다 캐시가 따로라 테스트끼리 간섭하지 않는다. 구 `serial()` 잠금과 `set_pdf_reader_cache_limits` 가드는 없앴다
//! - `backend::pdf::PdfCacheLimits { max_file_bytes: Option<u64>, max_bytes: Option<u64>, max_entries: Option<usize> }` (`Default`)
//! - `backend::main::{app_with, AppDeps, AuthMode}`: `app_with(AppDeps::new(AuthMode::Cookies).with_book_manager(Arc<dyn BookManagerApi>))`. 구 `app_with_book_manager`
//! - `backend::auth::create_jwt_token(email, role, name: Option<&str>, picture: Option<&str>) -> String` (4인자로 고정)
//! - 공용 도구: `common::es_fake::FakeEsManager`, `common::faulty_fs::FaultyFs`, `common::pdf::{make_blank_pdf, page_count_bytes}`
//!
//! Python의 모듈 범위 fixture(공유 임시 디렉터리)는 테스트마다 새 디렉터리로 바꿨다. 각 테스트가 자기 파일만 쓴다.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use backend::book_manager::{BookManager, BookManagerDeps};
use backend::main::{AppDeps, AuthMode, app_with};
use backend::pdf::PdfCacheLimits;
use common::TestClient;
use common::es_fake::FakeEsManager;
use common::faulty_fs::FaultyFs;
use common::pdf::{make_blank_pdf, page_count_bytes};
use serde_json::{Value, json};
use tempfile::TempDir;

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

    fn mtime(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

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

        let (reader, total_pages) = e.bm.get_cached_pdf_reader(&pdf).unwrap();

        assert_eq!(total_pages, 3);
        assert!(Arc::strong_count(&reader) >= 1); // reader 자체는 정상 반환되어야 한다
        assert!(!e.bm.pdf_cache().contains(&pdf)); // 그러나 보관하지 않는다
        // 페이지 수는 int 하나라 비용이 없으므로 항상 캐시한다
        assert!(e.bm.pdf_cache().page_count_contains(&pdf, mtime(&pdf)));
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

        let (reader1, _) = e.bm.get_cached_pdf_reader(&pdf).unwrap();
        assert!(e.bm.pdf_cache().contains(&pdf));
        let (reader2, _) = e.bm.get_cached_pdf_reader(&pdf).unwrap();
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
            e.bm.get_cached_pdf_reader(p).unwrap();
        }

        assert_eq!(e.bm.pdf_cache().len(), 2);
        assert!(!e.bm.pdf_cache().contains(&pdfs[0])); // 가장 오래된 것이 축출
        assert!(e.bm.pdf_cache().contains(&pdfs[2]));
        let total: u64 = e.bm.pdf_cache().entry_sizes().iter().sum();
        assert!(total <= e.bm.pdf_cache().max_bytes());
    }

    #[test]
    fn test_mtime_change_invalidates_reader_cache() {
        let e = build(|_, d| d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(10 * 1024 * 1024), ..PdfCacheLimits::default() }));
        let pdf = e.temp_dir.join("test_category").join("budget_mtime.pdf");
        create_test_pdf(2, &pdf);

        let (reader1, pages1) = e.bm.get_cached_pdf_reader(&pdf).unwrap();
        assert_eq!(pages1, 2);

        create_test_pdf(4, &pdf); // 내용 교체
        let meta = fs::metadata(&pdf).unwrap();
        let times = fs::FileTimes::new().set_accessed(meta.accessed().unwrap() + Duration::from_secs(10)).set_modified(meta.modified().unwrap() + Duration::from_secs(10));
        fs::OpenOptions::new().write(true).open(&pdf).unwrap().set_times(times).unwrap();

        let (reader2, pages2) = e.bm.get_cached_pdf_reader(&pdf).unwrap();
        assert!(!Arc::ptr_eq(&reader2, &reader1));
        assert_eq!(pages2, 4);
        assert_eq!(e.bm.pdf_cache().len(), 1); // 낡은 항목이 남지 않는다
    }

    /// PDF를 열 때마다 횟수를 세고 실제 열기에 맡기는 `PdfOpener`. 구 `with_open_pdf_observer`.
    // TODO(common): `common/fake_pdf.rs`(PdfOpener/PdfDoc 대역)가 아직 없어 로컬로 둔다. 실제 열기는 `backend::pdf::RealPdfOpener`(실제 구현체 이름 규칙 3.0.5)로 가정한다.
    struct CountingOpener {
        inner: Arc<dyn backend::pdf::PdfOpener>,
        opened: Arc<AtomicUsize>,
    }

    impl backend::pdf::PdfOpener for CountingOpener {
        fn open(&self, path: &Path) -> Result<Box<dyn backend::pdf::PdfDoc>, backend::pdf::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.open(path)
        }
    }

    /// 디스크 .preview_cache 히트 시 PdfReader 를 만들지 않아야 한다.
    ///
    /// 예전에는 reader 를 디스크 캐시 확인보다 먼저 만들었기 때문에, 캐시 히트여도
    /// GB급 PDF 를 통째로 파싱했다 (관측된 최장 응답 125초).
    #[tokio::test]
    #[ignore = "RealPdfOpener 이름이 문서에 없다(열기를 세면서 실제 PDF에 위임하는 PdfOpener 필요); 확정 후 활성화"]
    async fn test_disk_cache_hit_does_not_open_pdf() {
        let opened = Arc::new(AtomicUsize::new(0));
        let counter = opened.clone();
        let e = build(move |_, d| d.with_pdf_opener(Arc::new(CountingOpener { inner: Arc::new(backend::pdf::RealPdfOpener), opened: counter })));

        let first = e.bm.get_pdf_pages(1, 1, 2).await;
        assert_eq!(first.status_code, 200);

        // reader 캐시만 비운다. 페이지 수 캐시와 디스크 캐시는 남은 상태.
        e.bm.pdf_cache().clear();
        let before = opened.load(Ordering::SeqCst);

        let second = e.bm.get_pdf_pages(1, 1, 2).await;

        assert_eq!(second.status_code, 200);
        assert_eq!(second.header("X-Total-Pages"), Some("5"));
        // `assert not mock_reader.called`: 두 번째 호출은 PDF를 열지 않는다.
        assert_eq!(opened.load(Ordering::SeqCst), before);
    }
}
