//! `book_manager::pdf_reader_cache` 단위 테스트.

mod common;
#[path = "../shared/book_fixtures.rs"]
mod fx;
use crate::common::pdf_env::{ResponseExt, build};
use crate::common::shared_fixtures::create_test_pdf;
use book_manager::pdf_reader_cache::{PageCountCache, PdfCacheLimits, PdfReaderCache};
use std::fs;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering};

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

/// `_pdf_reader_cache`가 개수가 아니라 바이트로 제한되는지 검증한다.
///
/// cached PdfReader 1개는 PDF 파일 크기와 거의 1:1로 RSS를 차지한다
/// (pod 실측: 134MiB 파일 -> RSS +134.2MiB). 개수만 제한하면 /comics 의 1.5GiB 급
/// PDF 8개가 한 worker 에서 12GiB 를 점유해 노드 메모리를 고갈시킨다.
mod pdf_reader_cache_budget {
    use super::*;

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
            e.bm.get_cached_pdf_reader(p);
        }

        assert_eq!(e.bm.pdf_cache().len(), 2);
        assert!(!e.bm.pdf_cache().contains(&pdfs[0].to_string_lossy())); // 가장 오래된 것이 축출
        assert!(e.bm.pdf_cache().contains(&pdfs[2].to_string_lossy()));
        let total: u64 = pdfs.iter().filter(|p| e.bm.pdf_cache().contains(&p.to_string_lossy())).map(|p| fs::metadata(p).unwrap().len()).sum();
        assert!(total <= e.bm.pdf_cache().limits().max_bytes.unwrap());
    }

    /// 실제 페이지 수 조회와 추출에서 PDF를 읽는 횟수를 센다.
    struct CountingOpener {
        inner: doc_reader::pdf_page_reader::LopdfPageOps,
        opened: Arc<AtomicUsize>,
    }

    impl doc_reader::pdf_page_reader::PdfPageOps for CountingOpener {
        fn page_count(&self, path: &Path) -> Result<usize, doc_reader::pdf_page_reader::PdfError> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            self.inner.page_count(path)
        }

        fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, doc_reader::pdf_page_reader::PdfError> {
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
        let e = build(move |_, d| d.with_pdf_pages(Arc::new(CountingOpener { inner: doc_reader::pdf_page_reader::LopdfPageOps, opened: counter })));

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

impl ResponseExt for book_manager::BookResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}
