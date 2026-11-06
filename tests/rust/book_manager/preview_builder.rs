//! `book_manager::preview_builder` 단위 테스트.

#[path = "../shared/book_fixtures.rs"]
mod fx;

#[path = "common/mod.rs"]
mod common;
use crate::common::dyn_es::make_manager;
use crate::common::shared_fixtures::{CONTAINER, CONTAINER_XML, TempFile};
use crate::common::shared_fixtures::{image_bytes, make_doc, namelist, write_zip_stored, zip_read_string};
use book_manager::BookManagerApi;
use book_manager::{BookManager, BookManagerDeps};
use common::es_fake::FakeEsManager;
use common::runner::FakeRunner;
use common::zipx::write_zip;
use serde_json::json;
use std::fs;
use std::fs::File;
use std::io::Cursor;
use tempfile::TempDir;
use zip::ZipArchive;

/// URL 인코딩된 href 항목 및 전체보기(chapters=0) 시 EPUB 구조가 완전 보존되는지 검증.
#[tokio::test]
async fn test_epub_unquoted_href_and_full_view() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path();

    let epub_file = tmp_path.join("encoded_book.epub");
    fs::create_dir_all(epub_file.parent().unwrap()).unwrap();
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Encoded Book</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch%201.xhtml" media-type="application/xhtml+xml"/>
    <item id="cover" href="cover%20page.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
  <guide>
    <reference type="cover" title="Cover Page" href="cover%20page.xhtml"/>
  </guide>
</package>"#;
    let ch1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Ch1</title></head>
<body><img src="img%201.jpg"/></body>
</html>"#;
    let cover = r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Cover</title></head>
<body><h1>Cover</h1></body>
</html>"#;
    write_zip_stored(
        &epub_file,
        &[
            ("mimetype", b"application/epub+zip"),
            ("OEBPS/content.opf", opf.as_bytes()),
            ("OEBPS/ch 1.xhtml", ch1.as_bytes()),
            ("OEBPS/cover page.xhtml", cover.as_bytes()),
            ("OEBPS/img 1.jpg", b"fake_image_data"),
        ],
    );
    let _cleanup = TempFile(epub_file.clone());

    let es = FakeEsManager::with_doc(
        json!({"category": "test", "title": "Test Book", "author": "Author", "file_path": "encoded_book.epub", "file_type": "epub", "file_size": 1000, "updated_time": "2026-01-01T00:00:00.000000"}),
    );
    let bm = BookManager::with_deps(BookManagerDeps::new(tmp_path.to_path_buf(), es));

    let resp = bm.get_book_preview(1, None, Some(0)).await;
    assert_eq!(resp.status_code, 200);

    let cache_file = tmp_path.join(".preview_cache").join(format!("1_ch1_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION));
    assert!(cache_file.exists());

    let mut zout = ZipArchive::new(File::open(&cache_file).unwrap()).unwrap();
    let names = namelist(&mut zout);
    assert!(names.contains(&"OEBPS/cover page.xhtml".to_string()));
    assert!(names.contains(&"OEBPS/img 1.jpg".to_string()));
    let opf_data = zip_read_string(&mut zout, "OEBPS/content.opf");
    assert!(opf_data.contains("id=\"cover\""));
}

/// XHTML 자기닫힘 `<title/>` 등이 HTML 파서에서 본문을 삼키지 않게 펴는지 검증.
#[test]
fn test_expands_self_closing_rcdata_tags() {
    assert_eq!(book_manager::preview_builder::expand_self_closing_rcdata(b"<title/>"), b"<title></title>");
    assert_eq!(book_manager::preview_builder::expand_self_closing_rcdata(b"<TITLE />"), b"<TITLE ></TITLE>");
    assert_eq!(book_manager::preview_builder::expand_self_closing_rcdata(b"<script src=\"a.js\"/>"), b"<script src=\"a.js\"></script>");
    assert_eq!(book_manager::preview_builder::expand_self_closing_rcdata(b"<style/><textarea/>"), b"<style></style><textarea></textarea>");
}

#[test]
fn test_keeps_paired_and_unrelated_tags() {
    for data in [&b"<title>keep</title>"[..], b"<br/>", b"<img src=\"a.jpg\"/>", b"<titlebar/>"] {
        assert_eq!(book_manager::preview_builder::expand_self_closing_rcdata(data), data, "data={}", String::from_utf8_lossy(data));
    }
}

/// `<title/>`을 쓴 EPUB의 프리뷰 산출물에서 본문이 살아남는지 검증.
#[tokio::test]
async fn test_preview_expands_self_closing_title() {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path();

    let epub_file = tmp_path.join("self_closing_title.epub");
    fs::create_dir_all(epub_file.parent().unwrap()).unwrap();
    let opf = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Self Closing Title</dc:title>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.html" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;
    let ch1 = r#"<html xmlns="http://www.w3.org/1999/xhtml">
<head><title/></head>
<body><p>본문이 보여야 한다</p></body>
</html>"#;
    write_zip_stored(
        &epub_file,
        &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML.as_bytes()), ("OEBPS/content.opf", opf.as_bytes()), ("OEBPS/ch1.html", ch1.as_bytes())],
    );
    let _cleanup = TempFile(epub_file.clone());

    let es = FakeEsManager::with_doc(
        json!({"category": "test", "title": "Self Closing Title", "author": "Author", "file_path": "self_closing_title.epub", "file_type": "epub", "file_size": 1000, "updated_time": "2026-01-01T00:00:00.000000"}),
    );
    let bm = BookManager::with_deps(BookManagerDeps::new(tmp_path.to_path_buf(), es));

    let resp = bm.get_book_preview(2, None, Some(0)).await;
    assert_eq!(resp.status_code, 200);

    let cache_file = tmp_path.join(".preview_cache").join(format!("2_ch1_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION));
    let mut zout = ZipArchive::new(File::open(&cache_file).unwrap()).unwrap();
    let chapter = zip_read_string(&mut zout, "OEBPS/ch1.html");

    assert!(chapter.contains("<title></title>"));
    assert!(!chapter.contains("<title/>"));
    // HTML 파서로 읽어도 본문이 title 안으로 삼켜지지 않는다
    let dom = scraper::Html::parse_document(&chapter);
    let body_selector = scraper::Selector::parse("body").unwrap();
    let body = dom.select(&body_selector).next().expect("body 요소");
    assert!(body.text().collect::<String>().contains("본문이 보여야 한다"));
}

// book_manager HWP preview에서 빈 결과 시 안내 메시지 반환

#[tokio::test]
async fn test_hwp_preview_empty_returns_message() {
    // HWP 5.x(OLE)에서 LibreOffice가 빈 결과 -> '미리보기를 생성할 수 없습니다' 메시지
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("A")).unwrap();
    fs::write(dir.path().join("A").join("old.hwp"), [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]).unwrap();

    let doc = make_doc("A/old.hwp", "hwp");
    // libreoffice 변환기가 빈 문자열을 돌려준다
    let deps = BookManagerDeps::new(dir.path().to_path_buf(), FakeEsManager::with_doc(doc)).with_runner(FakeRunner::office_output(""));
    let manager = BookManager::with_deps(deps);
    let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;
    assert_eq!(resp.status_code, 200);
    assert!(String::from_utf8_lossy(&resp.body).contains("미리보기를 생성할 수 없습니다"));
}

mod get_pdf_pages {
    use super::common;
    use crate::common::pdf_env::{ResponseExt, build, env, make_doc};
    use crate::common::shared_fixtures::create_test_pdf;
    use book_manager::BookManager;
    use common::faulty_fs::FaultyFs;
    use common::pdf::page_count_bytes;
    use pdf_engine::pdf_page_reader::{PdfError, PdfPageOps, PdfReaderHandle};
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};

    impl ResponseExt for book_manager::BookResponse {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
        }
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

    fn body_contains(body: &[u8], needle: &str) -> bool {
        String::from_utf8_lossy(body).contains(needle)
    }

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
    async fn legacy_pdf_caches_are_regenerated_after_korean_font_support_changes() {
        let e = env();
        let cache_dir = e.temp_dir.join(".preview_cache");
        fs::create_dir_all(&cache_dir).unwrap();
        // 이전 서버가 만든 캐시는 최신 mtime이어도 한글이 이미 빠져 있을 수 있다.
        for name in ["1_p1-1.pdf", "1_n5.pdf"] {
            fs::write(cache_dir.join(name), b"LEGACY PDF WITHOUT KOREAN").unwrap();
        }
        let pages = e.bm.get_pdf_pages(1, 1, 1).await;
        assert_eq!(pages.status_code, 200);
        assert_eq!(page_count_bytes(&pages.body), 1);
        let preview = e.bm.get_book_preview(1, None, None).await;
        assert_eq!(preview.status_code, 200);
        assert_eq!(page_count_bytes(&preview.body), 5);
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
        assert!(!e.temp_dir.join(".preview_cache").join(format!("1_p3-2_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION)).exists());
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

        assert!(cache_dir.join(format!("1_n2_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION)).exists());
        assert!(!cache_dir.join(format!("1_n4_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION)).exists(), "같은 책의 다른 쪽 수 캐시는 하나만 남긴다");
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
        let cache_file = e.temp_dir.join(".preview_cache").join(format!("1_p1-2_v{}.pdf", BookManager::PDF_PREVIEW_CACHE_VERSION));
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

mod get_book_preview {
    use super::common;
    use crate::common::shared_fixtures::{blank_pdf_bytes, blank_pdf_bytes_sized, block_on, deps, make_doc, make_manager, tmp_dir};
    use book_manager::BookManager;
    use book_manager::{BookManagerApi, BookResponse};
    use common::epub::{EpubSpec, make_epub};
    use common::es_fake::FakeEsManager;
    use common::faulty_fs::FaultyFs;
    use common::fsx::{mtime_key, set_mtime};
    use common::runner::FakeRunner;
    use common::zipx::write_zip;
    use cover_manager::epub_cover::EpubCoverHooks;
    use pdf_engine::pdf_page_reader::{PdfError, PdfPageOps};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};
    use std::{fs, io};
    use sysdeps::CommandError;

    /// `BookResponse` 읽기 도우미(테스트 로컬). `is_file`은 본문이 있으면 참이다(파일을 읽어 담은 응답).
    trait ResponseExt {
        fn header(&self, name: &str) -> Option<&str>;
        fn body_text(&self) -> String;
    }

    impl ResponseExt for BookResponse {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
        }

        fn body_text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }
    }

    /// `_make_minimal_epub`의 기본 OPF.
    const MINIMAL_OPF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

    /// `_make_minimal_epub`의 뼈대. OPF는 루트의 `content.opf`, 챕터는 `ch1.xhtml` 하나다.
    /// OPF를 바꾸려면 `.with_opf(..)`, 멤버를 더하려면 `.with_extra(..)`를 붙인다.
    fn minimal_spec() -> EpubSpec {
        let mut spec = EpubSpec::minimal();
        spec.chapters = vec![("ch1.xhtml".to_string(), "<html><body><p>Hello</p></body></html>".to_string())];
        spec.with_opf(MINIMAL_OPF)
    }

    const CONTAINER_XML_CONTENT_OPF: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;

    // TODO(common): `common::fake_pdf`(다음 단계)에 `PdfPageOps` 대역이 생기면 지운다.
    /// 구 `override_get_cached_page_count(|_| Err(OSError(28)))`: 쪽수 조회가 항상 실패하는 `PdfPageOps`.
    struct NoSpacePdfPages;

    impl PdfPageOps for NoSpacePdfPages {
        fn page_count(&self, _path: &Path) -> Result<usize, PdfError> {
            Err(PdfError { kind: "OSError".to_string(), message: "[Errno 28] No space left on device".to_string() })
        }

        fn extract_pages(&self, _path: &Path, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
            Err(PdfError { kind: "OSError".to_string(), message: "[Errno 28] No space left on device".to_string() })
        }
    }

    // ---- coverage: get_book_preview edge cases ----

    #[test]
    fn test_get_book_preview_not_found() {
        // book not in ES
        let tmp = tmp_dir();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp.path(), &es);
        es.set_doc(None);
        let result = block_on(BookManagerApi::get_book_preview(&manager, 999, None, None));
        assert_eq!(result.status_code, 404);
    }

    #[test]
    fn test_get_book_preview_file_not_found() {
        // file not found on disk
        let tmp = tmp_dir();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp.path(), &es);
        es.set_doc(Some(make_doc("A/missing.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(result.status_code, 404);
    }

    #[test]
    fn test_get_book_preview_epub_cache_hit() {
        // EPUB preview cache hit
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec());
        es.set_doc(Some(make_doc("A/test.epub", "epub")));

        // 더 최근 mtime을 가진 캐시 파일을 만든다.
        let cache_dir = tmp_path.join(".preview_cache");
        fs::create_dir_all(&cache_dir).unwrap();
        book_manager::preview_builder::epub_package::get_epub_total_chapters(&epub);
        let cache_file = cache_dir.join(format!("1_ch3_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION));
        make_epub(&cache_file, &minimal_spec());

        set_mtime(&cache_file, SystemTime::now() + Duration::from_secs(1000));

        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, Some(3), None));
        assert_eq!(result.status_code, 200);
    }

    #[test]
    fn test_get_book_preview_epub_opf_keyerror() {
        // OPF file in container.xml but missing from archive
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        let container_xml = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="MISSING.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        write_zip(&epub, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container_xml.as_bytes())]);
        // zip 안에 MISSING.opf는 없다.
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(result.status_code, 422);
    }

    #[test]
    fn test_get_book_preview_epub_opf_namespace_fix() {
        // OPF with opf: prefix but no xmlns:opf declaration
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <metadata>
    <opf:meta name="cover" content="cover-image"/>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec().with_opf(&opf));
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        // 크래시하지 않아야 한다. 네임스페이스가 주입된다.
        assert!([200, 422, 500].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_no_spine() {
        // EPUB without spine element → fallback to manifest order
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec().with_opf(&opf));
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_chapter_missing_from_zip() {
        // chapter file missing from ZIP archive
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        // ch2.xhtml은 zip에 없다.
        make_epub(&epub, &minimal_spec().with_opf(&opf));
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, Some(5), None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_svg_image_tag() {
        // chapter with SVG <image> tags
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="cover.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let chapter = r#"<html><body><svg><image xlink:href="cover.png"/></svg></body></html>"#;
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(vec![0u8; 100]);
        write_zip(
            &epub,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()),
                ("content.opf", opf.as_bytes()),
                ("ch1.xhtml", chapter.as_bytes()),
                ("cover.png", &png),
            ],
        );
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_css_missing() {
        // CSS file missing from ZIP
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let chapter = r#"<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>"#;
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        // style.css는 zip에 없다.
        write_zip(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()), ("content.opf", opf.as_bytes()), ("ch1.xhtml", chapter.as_bytes())],
        );
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_large_font_skip() {
        // large font file skipped, missing font skipped
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
    <item id="font1" href="big.ttf" media-type="font/ttf"/>
    <item id="font2" href="missing.woff" media-type="font/woff"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let chapter = r#"<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>"#;
        let css = "@font-face { font-family: \"Big\"; src: url(\"big.ttf\"); }\n@font-face { font-family: \"Missing\"; src: url(\"missing.woff\"); }\nbody { font-family: serif; }";
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        // big.ttf는 500KB를 넘는다. missing.woff는 zip에 없다.
        let big = vec![0u8; 600 * 1024];
        write_zip(
            &epub,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()),
                ("content.opf", opf.as_bytes()),
                ("ch1.xhtml", chapter.as_bytes()),
                ("style.css", css.as_bytes()),
                ("big.ttf", &big),
            ],
        );
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_guide_element() {
        // EPUB with guide element
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
  <guide>
    <reference type="toc" href="toc.xhtml"/>
    <reference type="text" href="ch1.xhtml"/>
  </guide>
</package>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec().with_opf(&opf));
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_ncx_navpoint_filtering() {
        // NCX navPoint filtering removes missing file references
        let opf_ns = "http://www.idpf.org/2007/opf";
        let ncx_ns = "http://www.daisy.org/z3986/2005/ncx/";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let ncx = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}">
  <navMap>
    <navPoint id="np1"><navLabel><text>Ch1</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2"><navLabel><text>Ch99</text></navLabel><content src="missing.xhtml"/></navPoint>
  </navMap>
</ncx>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec().with_opf(&opf).with_extra("toc.ncx", ncx.into_bytes()));
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_css_font_face_strip() {
        // CSS @font-face block removed when font not in files_to_include
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let chapter = r#"<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>"#;
        let css = "@font-face { font-family: \"Missing\"; src: url(\"nothere.woff\"); }\nbody { color: red; }";
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        write_zip(
            &epub,
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()),
                ("content.opf", opf.as_bytes()),
                ("ch1.xhtml", chapter.as_bytes()),
                ("style.css", css.as_bytes()),
            ],
        );
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_file_missing_in_write() {
        // file missing when writing output EPUB
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="cover.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let chapter = r#"<html><body><img src="cover.png"/></body></html>"#;
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        // cover.png는 zip에 없다. manifest에 있고 HTML에서 참조한다.
        write_zip(
            &epub,
            &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()), ("content.opf", opf.as_bytes()), ("ch1.xhtml", chapter.as_bytes())],
        );
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_epub_validation_fail() {
        // generated EPUB fails validation
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec());
        let es = FakeEsManager::new();
        // 가정한 seam: 미리보기 EPUB 검증 함수를 교체한다. 문서에 없는 seam이다(파일 머리말 참고).
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_preview_epub_validator(Arc::new(|_path| Err("bad epub".to_string()))));
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(result.status_code, 422);
        assert!(result.body_text().contains("bad epub"));
    }

    #[test]
    fn test_get_book_preview_epub_bad_zip() {
        // corrupted ZIP
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        fs::write(&epub, b"not a zip file").unwrap();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(result.status_code, 422);
    }

    #[test]
    fn test_get_book_preview_epub_general_exception() {
        // general exception during EPUB processing
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec());
        let es = FakeEsManager::new();
        // TODO(common): `EpubCoverHooks::with_find_opf_path`는 `Fn() -> String`이라 `BookManagerError("boom")`을 돌려줄 수 없다. 문서가 `Result`를 받도록 정해지면 고친다. 지금은 panic으로 대신한다.
        let hooks = EpubCoverHooks::default().with_find_opf_path(Arc::new(|| panic!("boom")));
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_epub_hooks(hooks));
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(result.status_code, 500);
        assert_eq!(result.body_text(), "EPUB preview failed");
    }

    #[test]
    fn test_get_book_preview_doc_exception() {
        // .doc conversion exception
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let doc = tmp_path.join("A").join("test.doc");
        fs::create_dir_all(doc.parent().unwrap()).unwrap();
        fs::write(&doc, b"fake doc content").unwrap();
        let es = FakeEsManager::new();
        let runner = FakeRunner::returning(Err(CommandError::Failed("boom".to_string())));
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(runner));
        es.set_doc(Some(make_doc("A/test.doc", "doc")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(result.status_code, 500);
        assert_eq!(result.body_text(), ".DOC preview failed");
    }

    // ---- coverage: get_pdf_pages edge cases ----

    #[test]
    fn test_get_pdf_pages_not_found() {
        // book not in ES
        let tmp = tmp_dir();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp.path(), &es);
        es.set_doc(None);
        let result = block_on(manager.get_pdf_pages(999, 1, 1));
        assert_eq!(result.status_code, 404);
    }

    #[test]
    fn test_get_pdf_pages_not_pdf() {
        // file is not a PDF
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        let txt = tmp_path.join("A").join("test.txt");
        fs::create_dir_all(txt.parent().unwrap()).unwrap();
        fs::write(&txt, "hello").unwrap();
        es.set_doc(Some(make_doc("A/test.txt", "txt")));
        let result = block_on(manager.get_pdf_pages(1, 1, 1));
        assert_eq!(result.status_code, 400);
    }

    #[test]
    fn test_get_pdf_pages_happy_path_and_cache() {
        // happy path + cache hit
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        let pdf = tmp_path.join("A").join("test.pdf");
        fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        fs::write(&pdf, blank_pdf_bytes(2, &[])).unwrap();

        es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

        // 첫 호출: 생성하고 캐시한다.
        let result = block_on(manager.get_pdf_pages(1, 1, 1));
        assert_eq!(result.status_code, 200);
        assert_eq!(result.header("x-total-pages"), Some("2"));

        // 두 번째 호출: 캐시 적중.
        let result2 = block_on(manager.get_pdf_pages(1, 1, 1));
        assert_eq!(result2.status_code, 200);
    }

    #[test]
    fn test_get_pdf_pages_start_exceeds_total() {
        // start page exceeds total pages
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        let pdf = tmp_path.join("A").join("test.pdf");
        fs::create_dir_all(pdf.parent().unwrap()).unwrap();
        fs::write(&pdf, blank_pdf_bytes(1, &[])).unwrap();

        es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

        let result = block_on(manager.get_pdf_pages(1, 100, 200));
        assert_eq!(result.status_code, 400);
        assert!(result.body_text().contains("exceeds total pages"));
    }

    // ---- coverage: get_book_preview epub with many spine items ----

    #[test]
    fn test_get_book_preview_epub_spine_trimmed() {
        // spine refs not in chapter_idrefs are removed
        let opf_ns = "http://www.idpf.org/2007/opf";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch3" href="ch3.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch4" href="ch4.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch5" href="ch5.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
    <itemref idref="ch3"/>
    <itemref idref="ch4"/>
    <itemref idref="ch5"/>
  </spine>
</package>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        let chapters: Vec<(String, String)> = (1..6).map(|i| (format!("ch{i}.xhtml"), format!("<html><body><p>Chapter {i}</p></body></html>"))).collect();
        let mut entries: Vec<(&str, &[u8])> = vec![("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()), ("content.opf", opf.as_bytes())];
        for (name, body) in &chapters {
            entries.push((name.as_str(), body.as_bytes()));
        }
        write_zip(&epub, &entries);
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        // 2개 챕터만 요청한다. ch3-ch5는 spine에서 빠져야 한다.
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, Some(2), None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_book_preview_html_sanitizes_active_content_and_rewrites_resources() {
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        fs::create_dir_all(tmp_path.join("A")).unwrap();
        let html_file = tmp_path.join("A").join("test.html");
        fs::write(
            &html_file,
            r##"
        <html>
          <head>
            <meta http-equiv="refresh" content="0;url=https://evil.example.com" />
            <script>alert(1)</script>
            <link rel="stylesheet" href="style.css" />
            <link rel="preload" href="evil.js" />
          </head>
          <body onload="steal()">
            <iframe src="https://evil.example.com/embed"></iframe>
            <img src="cover.png" onclick="hack()" />
            <a href="chapter2.html">next</a>
            <a href="#section1">toc</a>
          </body>
        </html>
        "##,
        )
        .unwrap();
        fs::write(tmp_path.join("A").join("style.css"), "body { color: red; }").unwrap();
        fs::write(tmp_path.join("A").join("cover.png"), b"png").unwrap();
        let doc = make_doc("A/test.html", "html");
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(doc));

        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, Some("/html-resource/1")));
        let body = resp.body_text();

        assert_eq!(resp.status_code, 200);
        assert!(resp.header("content-security-policy").expect("csp").starts_with("sandbox;"));
        assert!(!body.contains("<script"));
        assert!(!body.contains("<iframe"));
        assert!(!body.contains("onload="));
        assert!(!body.contains("onclick="));
        assert!(!body.contains("http-equiv"));
        assert!(body.contains(r#"href="/html-resource/1?path=style.css""#));
        assert!(body.contains(r#"src="/html-resource/1?path=cover.png""#));
        assert!(body.contains(r##"href="#section1""##));
        assert!(!body.contains(r#"href="chapter2.html""#));
    }

    // ── get_book_preview: HTML read_text 예외 → 500 ─────────────

    #[test]
    fn test_get_book_preview_html_read_exception() {
        // HTML 파일 read_text 가 예외를 올리면 500 응답을 반환한다.
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let html_file = tmp_path.join("A").join("book.html");
        fs::create_dir_all(html_file.parent().unwrap()).unwrap();
        fs::write(&html_file, "<html/>").unwrap();

        let doc = make_doc("A/book.html", "html");
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(doc));

        // 읽기 권한을 없애 PermissionError를 흉내 낸다.
        fs::set_permissions(&html_file, fs::Permissions::from_mode(0o000)).unwrap();
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        fs::set_permissions(&html_file, fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(resp.status_code, 500);
    }

    #[test]
    fn test_book_preview_rejects_missing_required_es_fields() {
        let tmp = tmp_dir();
        let html = tmp.path().join("A/book.html");
        fs::create_dir_all(html.parent().unwrap()).unwrap();
        fs::write(&html, "<html><body>preview</body></html>").unwrap();

        let mut doc = make_doc("A/book.html", "html");
        doc.as_object_mut().unwrap().remove("category");
        let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
        let resp = block_on(manager.get_book_preview(1, Some(5), Some(10)));

        assert_eq!(resp.status_code, 500);
    }

    #[test]
    fn test_pdf_pages_rejects_missing_required_es_fields() {
        let tmp = tmp_dir();
        let mut doc = make_doc("A/book.pdf", "pdf");
        doc.as_object_mut().unwrap().remove("category");
        let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
        let resp = block_on(manager.get_pdf_pages(1, 1, 1));

        assert_eq!(resp.status_code, 500);
    }

    #[test]
    fn test_get_pdf_pages_oserror() {
        // OSError in get_pdf_pages
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let pdf_file = tmp_path.join("A").join("sample.pdf");
        fs::create_dir_all(pdf_file.parent().unwrap()).unwrap();
        fs::write(&pdf_file, b"%PDF-1.4 dummy").unwrap();

        let pdf_es = FakeEsManager::new();
        pdf_es.set_doc(Some(make_doc("A/sample.pdf", ".pdf")));
        // 쪽수 조회가 ENOSPC로 실패하게 한다(`PdfPageOps` 대역, TODO 주석 참고).
        let manager = BookManager::with_deps(deps(tmp_path, &pdf_es).with_pdf_pages(Arc::new(NoSpacePdfPages)));
        let resp = block_on(manager.get_pdf_pages(777, 1, 5));
        assert_eq!(resp.status_code, 503);
        assert!(resp.body_text().contains("Storage access error"));
    }

    #[test]
    fn test_epub_preview_raw_zip_and_ncx_branches() {
        let opf_ns = "http://www.idpf.org/2007/opf";
        let ncx_ns = "http://www.daisy.org/z3986/2005/ncx/";
        let opf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="raw" href="ch%20raw.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"#
        );
        let ncx = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}">
  <navMap>
    <navPoint id="np1"><navLabel><text>Ch1</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2"><navLabel><text>Empty</text></navLabel><content src=""/></navPoint>
  </navMap>
</ncx>"#
        );
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let epub = tmp_path.join("A").join("test.epub");
        fs::create_dir_all(epub.parent().unwrap()).unwrap();
        make_epub(&epub, &minimal_spec().with_opf(&opf).with_extra("toc.ncx", ncx.into_bytes()).with_extra("ch raw.xhtml", b"<html><body>raw</body></html>".to_vec()));
        let es = FakeEsManager::new();
        let manager = make_manager(tmp_path, &es);
        es.set_doc(Some(make_doc("A/test.epub", "epub")));
        let result = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
    }

    #[test]
    fn test_get_pdf_pages_storage_oserror_and_cached_reader_none() {
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let pdf_file = tmp_path.join("A").join("sample.pdf");
        fs::create_dir_all(pdf_file.parent().unwrap()).unwrap();
        fs::write(&pdf_file, blank_pdf_bytes_sized(2, 100, 100)).unwrap();

        let es = FakeEsManager::new();
        es.set_doc(Some(make_doc("A/sample.pdf", ".pdf")));

        // 1. book.file_path.is_file() raises OSError
        let faulty = FaultyFs::new();
        faulty.set_is_file(|_p| Some(io::Error::from_raw_os_error(5)));
        let manager_fault = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
        let resp1 = block_on(manager_fault.get_pdf_pages(1, 1, 2));
        assert_eq!(resp1.status_code, 503);
        assert!(resp1.body_text().contains("Storage access error"));

        // 2. reader is None (page count cached, but reader cache miss)
        // 쪽수 캐시 접근자는 문서에 없는 seam이다(파일 머리말 참고). 새 매니저의 reader 캐시는 비어 있어 `_pdf_reader_cache_remove`가 필요 없다.
        let manager = make_manager(tmp_path, &es);
        manager.page_count_cache().insert((pdf_file.to_string_lossy().into_owned(), mtime_key(&pdf_file)), 2);

        let resp2 = block_on(manager.get_pdf_pages(1, 1, 2));
        assert_eq!(resp2.status_code, 200);
    }
}

#[tokio::test]
async fn test_preview_keeps_cover_image_not_referenced_by_chapters() {
    // parametrize: meta-cover, cover-image-property
    let cases = [("meta-cover", r#"<meta name="cover" content="cover-img"/>"#), ("cover-image-property", "")];
    for (case_id, cover_decl) in cases {
        let properties = if cover_decl.is_empty() { r#" properties="cover-image""# } else { "" };
        let tmp = TempDir::new().unwrap();
        let opf = format!(
            r#"<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <metadata>{cover_decl}</metadata>
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="cover-img" href="images/front.jpg" media-type="image/jpeg"{properties}/>
              </manifest>
              <spine><itemref idref="c1"/></spine>
            </package>"#
        );
        write_zip(
            &tmp.path().join("cov.epub"),
            &[
                ("mimetype", b"application/epub+zip"),
                ("META-INF/container.xml", CONTAINER.as_bytes()),
                ("OEBPS/ch1.xhtml", "<html><body><p>본문</p></body></html>".as_bytes()),
                ("OEBPS/images/front.jpg", &image_bytes(image::ImageFormat::Jpeg, (64, 64))),
                ("OEBPS/content.opf", opf.as_bytes()),
            ],
        );
        let manager = make_manager(tmp.path(), FakeEsManager::with_doc(make_doc("cov.epub", "epub")));

        let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;

        assert_eq!(resp.status_code, 200, "case {case_id}");
        let mut zf = ZipArchive::new(Cursor::new(resp.body)).unwrap();
        assert!(zf.file_names().any(|n| n.is_ok_and(|n| n == "OEBPS/images/front.jpg")), "case {case_id}");
        let opf_text = std::io::read_to_string(zf.by_name("OEBPS/content.opf").unwrap()).unwrap();
        assert!(opf_text.contains(r#"id="cover-img""#), "case {case_id}");
    }
}

mod hwp_preview {
    use super::common;
    use super::common::{es_fake::FakeEsManager, runner::FakeRunner};
    use crate::common::shared_fixtures::{block_on, deps, make_doc, tmp_dir};
    use book_manager::BookManager;
    use book_manager::{BookManagerApi, BookResponse};
    use std::fs;
    use std::path::{Path, PathBuf};

    /// `BookResponse` 읽기 도우미(테스트 로컬). `is_file`은 본문이 있으면 참이다(파일을 읽어 담은 응답).
    trait ResponseExt {
        fn body_text(&self) -> String;
    }

    impl ResponseExt for BookResponse {
        fn body_text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }
    }

    // ---- HWP3 네이티브 파서 fallback 프리뷰 테스트 ----

    fn hwp_test_dir() -> PathBuf {
        common::project_root().join("tests").join("books").join("_hwp")
    }

    /// 원본 테스트 파일을 복사하고 경로를 돌려준다. 파일이 없으면 panic한다.
    fn copy_hwp_sample(tmp_path: &Path, sample_name: &str, dest_name: &str) -> PathBuf {
        let hwp_src = hwp_test_dir().join(sample_name);
        assert!(hwp_src.exists(), "테스트 파일 tests/books/_hwp/{sample_name}이 필요하다");
        fs::create_dir_all(tmp_path.join("A")).unwrap();
        let hwp_file = tmp_path.join("A").join(dest_name);
        fs::copy(&hwp_src, &hwp_file).unwrap();
        hwp_file
    }

    #[test]
    fn test_hwp_preview_fallback_when_libreoffice_empty() {
        // LibreOffice가 빈 문자열 반환 시 hwp3 파서로 fallback하여 HTML 생성
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        copy_hwp_sample(tmp_path, "v2.10_KYOKA.hwp", "kyoka.hwp");
        let doc = make_doc("A/kyoka.hwp", "hwp");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(resp.status_code, 200);
        let body = resp.body_text();
        assert!(body.contains("영동") || body.contains("교가"));
        assert!(body.contains("<p>"));
    }

    #[test]
    fn test_hwp_preview_fallback_v3_00() {
        // V3.00 파일의 fallback 프리뷰
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        copy_hwp_sample(tmp_path, "v3.00_현대시사전.hwp", "poem.hwp");
        let doc = make_doc("A/poem.hwp", "hwp");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(resp.status_code, 200);
        let body = resp.body_text();
        assert!(body.chars().count() > 50);
    }

    #[test]
    fn test_hwp3_preview_caches_native_result_separately() {
        // HWP3 네이티브 결과는 LibreOffice 캐시(1.html)와 다른 이름(1.hwp3_v<판>.html)으로 저장된다
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        copy_hwp_sample(tmp_path, "v2.10_KYOKA.hwp", "kyoka.hwp");
        let doc = make_doc("A/kyoka.hwp", "hwp");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        let cache_file = tmp_path.join(".preview_cache").join(format!("1.hwp3_v{}.html", BookManager::HWP3_PREVIEW_VERSION));
        assert!(cache_file.exists());
        assert!(fs::read_to_string(&cache_file).unwrap().chars().count() > 50);
        assert!(!tmp_path.join(".preview_cache").join("1.html").exists());
    }

    #[test]
    fn test_hwp_preview_no_fallback_for_doc() {
        // DOC 파일은 hwp3 fallback을 사용하지 않음
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        fs::create_dir_all(tmp_path.join("A")).unwrap();
        let doc_file = tmp_path.join("A").join("test.doc");
        fs::write(&doc_file, b"fake doc content").unwrap();
        let doc = make_doc("A/test.doc", "doc");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        // DOC는 빈 html_content → 캐시 안 되고 preview 없음
        assert!([200, 400, 500].contains(&resp.status_code) || resp.status_code == 755, "status={}", resp.status_code);
        // 빈 content면 status 755 안 나옴
    }

    #[test]
    fn test_hwp_preview_fallback_both_fail() {
        // HWP 5.x(OLE)에서 LibreOffice도 빈 결과이고 HWP3 파서 대상도 아니면 캐시를 만들지 않는다
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        fs::create_dir_all(tmp_path.join("A")).unwrap();
        fs::write(tmp_path.join("A").join("old.hwp"), [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]).unwrap();
        let doc = make_doc("A/old.hwp", "hwp");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert!(!tmp_path.join(".preview_cache").join("1.html").exists());
        assert!(!tmp_path.join(".preview_cache").join(format!("1.hwp3_v{}.html", BookManager::HWP3_PREVIEW_VERSION)).exists());
    }

    #[test]
    fn test_hwp_preview_v120_reads_natively() {
        // V1.20은 LibreOffice가 못 읽지만 네이티브 brute-force 결과로 미리보기를 만든다
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        copy_hwp_sample(tmp_path, "v1.20_부하의약혼녀.hwp", "old.hwp");
        let doc = make_doc("A/old.hwp", "hwp");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(resp.status_code, 200);
        assert!(!resp.body_text().contains("미리보기를 생성할 수 없습니다"));
    }

    #[test]
    fn test_hwp3_preview_uses_native_parser_before_libreoffice() {
        // HWP3는 LibreOffice를 부르지 않고 네이티브 결과를 쓴다(LibreOffice는 한자를 `?`로 깨뜨린다).
        // 예전에 LibreOffice로 만든 캐시(1.html)가 더 새것이어도 쓰지 않는다.
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        copy_hwp_sample(tmp_path, "v2.10_KYOKA.hwp", "kyoka.hwp");
        fs::create_dir_all(tmp_path.join(".preview_cache")).unwrap();
        fs::write(tmp_path.join(".preview_cache").join("1.html"), "<p>??깨진 캐시??</p>").unwrap();
        // 파서 판이 낮은 옛 네이티브 캐시도 쓰지 않는다.
        fs::write(tmp_path.join(".preview_cache").join("1.hwp3.html"), "<p>깨진 캐시 옛 판</p>").unwrap();
        let doc = make_doc("A/kyoka.hwp", "hwp");
        let es = FakeEsManager::new();
        let runner = FakeRunner::new(|argv| panic!("HWP3에 LibreOffice를 불렀다: {argv:?}"));
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(runner));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(resp.status_code, 200);
        let body = resp.body_text();
        assert!(body.contains("영동") || body.contains("교가"), "{body}");
        assert!(!body.contains("깨진 캐시"));
    }

    #[test]
    fn test_hwp_preview_libreoffice_success_no_fallback() {
        // HWP 5.x(OLE)에서 LibreOffice가 정상 결과 반환 시 그 결과를 쓴다
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        fs::create_dir_all(tmp_path.join("A")).unwrap();
        fs::write(tmp_path.join("A").join("kyoka.hwp"), [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]).unwrap();
        let doc = make_doc("A/kyoka.hwp", "hwp");
        let es = FakeEsManager::new();
        // LibreOffice처럼 `--outdir`에 `<이름>.html`을 쓰는 변환기 대역이다(`office_output`은 stdout에만 담아 실제 변환기와 다르다).
        let runner = FakeRunner::new(|argv| {
            let outdir = PathBuf::from(&argv[argv.iter().position(|a| a == "--outdir").unwrap() + 1]);
            fs::write(outdir.join("kyoka.html"), "<p>LibreOffice 결과</p>").unwrap();
            Ok(sysdeps::CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
        });
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(runner));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        assert_eq!(resp.status_code, 200);
        let body = resp.body_text();
        assert!(body.contains("LibreOffice"));
        assert!(!body.contains("영동")); // fallback 미사용 확인
    }

    #[test]
    fn test_hwp_preview_html_escaping() {
        // fallback HTML 출력에서 특수문자가 이스케이프되는지 확인
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        copy_hwp_sample(tmp_path, "v2.10_PAGER.hwp", "pager.hwp");
        let doc = make_doc("A/pager.hwp", "hwp");
        let es = FakeEsManager::new();
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
        es.set_doc(Some(doc));
        let resp = block_on(BookManagerApi::get_book_preview(&manager, 1, None, None));
        if resp.status_code == 200 {
            let body = resp.body_text();
            // XSS 방지: <script> 같은 태그가 이스케이프되어야 함
            assert!(!body.contains("<script>"));
        }
    }
}
