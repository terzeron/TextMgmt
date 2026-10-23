//! 원본: tests/test_cover_status.py
//!
//! 가정한 API (아직 구현 전이다. Python 이름 그대로이고 비공개 `_x`는 `pub fn x`로 본다. 설계: docs/rust-test-seams.md 3.11, 3.12):
//! - `backend::cover_status::STANDARD_COVER_SIZES: &[(u32, u32)]`
//! - `backend::cover_status::pdf_first_image_size(path: &Path, lock: &Mutex<()>) -> Option<(u32, u32)>`
//! - `backend::cover_status::has_official_cover(path: &Path, file_type: &str, lock: &Mutex<()>) -> bool`
//! - `backend::cover_status::CoverStatusCache::new(path: &Path)`, `get_or_compute(&self, path: &Path, compute: &dyn Fn() -> bool) -> bool`
//! - `backend::book_manager::{BookManager, BookManagerDeps, CoverBook, CoverPage, CoverBooksPager}`
//!   - `BookManager::with_deps(BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>)...)` (구 `BookManager::without_es(path)`. ES는 `common::es_fake::FakeEsManager`)
//!   - `struct CoverBook { pub file_path: PathBuf, pub file_type: String, pub name: String }` (Clone)
//!   - `type CoverPage = (Vec<(CoverBook, String)>, usize, Option<String>, Option<String>)` (목록, 전체 수, 다음 커서, 오류)
//!   - `type CoverBooksPager = Arc<dyn Fn(Option<String>) -> BoxFuture<'static, CoverPage> + Send + Sync>`
//!   - `BookManagerDeps::with_cover_books_pager(self, pager: CoverBooksPager) -> Self` (구 `set_cover_books_pager`. Python `manager.get_cover_books_paged = AsyncMock(...)`)
//!   - `BookManagerDeps::with_uncovered_scan_budget_seconds(self, seconds: f64) -> Self`, `with_uncovered_max_scan_pages(self, pages: usize) -> Self` (구 `set_*`. 값 설정자), `BookManager::UNCOVERED_MAX_CONCURRENT_SCANS: usize`
//!   - `async fn get_uncovered_books_paged(&self, size: Option<usize>, cursor: Option<&str>) -> (Vec<(CoverBook, String)>, Option<usize>, Option<String>, Option<String>)` (목록, 전체 수, 다음 커서, 오류)
//!   - `fn uncovered_flags(&self, books: &[CoverBook]) -> Vec<bool>`
//! - `backend::cover_status::{pdf_first_image_size_with, has_official_cover_with}`: 마지막 인자로 `open: &dyn Fn(&Path) -> Box<dyn PdfDoc>`를 더 받는다.
//!   `backend::pdf::PdfDoc`(구 `PdfDocument`): `fn page_count(&self) -> usize; fn close(&self);`. 나머지 메서드는 기본 구현이다. (Python `monkeypatch.setattr(pypdfium2, "PdfDocument", ...)`)

mod common;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::book_manager::{BookManager, BookManagerDeps, CoverBook, CoverBooksPager, CoverPage};
use backend::cover_status::{CoverStatusCache, STANDARD_COVER_SIZES, has_official_cover, has_official_cover_with, pdf_first_image_size, pdf_first_image_size_with};
use backend::pdf::PdfDoc;
use common::{
    epub::{EpubSpec, make_epub},
    es_fake::FakeEsManager,
    image::solid,
    pdf::{write_blank_pdf, write_image_pdf},
};

static LOCK: Mutex<()> = Mutex::new(());

fn make_pdf(path: &Path, size: Option<(u32, u32)>) -> PathBuf {
    match size {
        None => write_blank_pdf(path, 100, 100),
        Some((w, h)) => write_image_pdf(path, &[solid(w, h, [30, 30, 200])]),
    }
    path.to_path_buf()
}

#[test]
fn test_pdf_with_standard_cover_size_is_covered() {
    let mut sizes = STANDARD_COVER_SIZES.to_vec();
    sizes.sort();
    for size in sizes {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = make_pdf(&tmp.path().join("a.pdf"), Some(size));

        assert_eq!(pdf_first_image_size(&pdf, &LOCK), Some(size), "size={size:?}");
        assert!(has_official_cover(&pdf, "pdf", &LOCK), "size={size:?}");
    }
}

#[test]
fn test_pdf_with_other_first_image_is_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("a.pdf"), Some((690, 1600)));

    assert!(!has_official_cover(&pdf, "pdf", &LOCK));
}

#[test]
fn test_pdf_without_image_is_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("a.pdf"), None);

    assert_eq!(pdf_first_image_size(&pdf, &LOCK), None);
    assert!(!has_official_cover(&pdf, "pdf", &LOCK));
}

#[test]
fn test_unsupported_type_is_not_reported_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(has_official_cover(&tmp.path().join("a.txt"), "txt", &LOCK));
}

#[test]
fn test_cache_reuses_result_until_file_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("a.pdf"), Some((690, 1600)));
    let cache = CoverStatusCache::new(&tmp.path().join(".cover_cache").join("status.sqlite3"));
    let calls = AtomicUsize::new(0);

    let compute = || {
        calls.fetch_add(1, Ordering::SeqCst);
        has_official_cover(&pdf, "pdf", &LOCK)
    };

    assert!(!cache.get_or_compute(&pdf, &compute));
    assert!(!cache.get_or_compute(&pdf, &compute));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    make_pdf(&pdf, Some((480, 623)));
    assert!(cache.get_or_compute(&pdf, &compute));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// 구 `BookManager::without_es(path)`. ES는 쓰지 않으므로 빈 `FakeEsManager`를 넘긴다.
fn deps(tmp: &Path) -> BookManagerDeps {
    BookManagerDeps::new(tmp.to_path_buf(), FakeEsManager::new())
}

fn make_manager(tmp: &Path) -> BookManager {
    BookManager::with_deps(deps(tmp))
}

fn book(name: &str, tmp: &Path, size: (u32, u32)) -> CoverBook {
    CoverBook { file_path: make_pdf(&tmp.join(name), Some(size)), file_type: "pdf".to_string(), name: name.to_string() }
}

/// 스크립트한 페이지를 차례로 돌려주고, 받은 커서를 기록하는 pager. Python `AsyncMock(side_effect=[...])`.
fn scripted(pages: Vec<CoverPage>) -> (CoverBooksPager, Arc<Mutex<Vec<Option<String>>>>) {
    let queue = Arc::new(Mutex::new(VecDeque::from(pages)));
    let cursors = Arc::new(Mutex::new(Vec::new()));
    let (q, c) = (queue.clone(), cursors.clone());
    let pager: CoverBooksPager = Arc::new(move |cursor| {
        c.lock().unwrap().push(cursor);
        let page = q.lock().unwrap().pop_front().expect("스크립트한 페이지를 모두 썼다");
        Box::pin(async move { page })
    });
    (pager, cursors)
}

fn names(items: &[(CoverBook, String)]) -> Vec<&str> {
    items.iter().map(|entry| entry.0.name.as_str()).collect()
}

#[tokio::test]
async fn test_uncovered_page_collects_across_scan_pages_and_keeps_scan_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let plain1 = book("plain1.pdf", tmp.path(), (690, 1600));
    let plain2 = book("plain2.pdf", tmp.path(), (690, 1600));
    let (pager, cursors) = scripted(vec![(vec![(covered, String::new()), (plain1, String::new())], 3, Some("c1".into()), None), (vec![(plain2, String::new())], 3, Some("c2".into()), None)]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(pager));

    let (items, total, next_cursor, error) = manager.get_uncovered_books_paged(Some(2), None).await;

    assert_eq!(error, None);
    assert_eq!(names(&items), ["plain1.pdf", "plain2.pdf"]);
    assert_eq!(total, None);
    assert_eq!(next_cursor.as_deref(), Some("c2"));
    assert_eq!(cursors.lock().unwrap()[1].as_deref(), Some("c1"));
}

#[tokio::test]
async fn test_uncovered_page_ends_when_list_ends() {
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let (pager, _cursors) = scripted(vec![(vec![(covered, String::new())], 1, None, None)]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(pager));

    let (items, total, next_cursor, error) = manager.get_uncovered_books_paged(Some(5), None).await;

    assert!(items.is_empty());
    assert_eq!((total, next_cursor, error), (None, None, None));
}

#[tokio::test]
async fn test_uncovered_page_stops_at_budget_even_without_items() {
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let (pager, cursors) = scripted(vec![(vec![(covered.clone(), String::new())], 9, Some("c1".into()), None), (vec![(covered, String::new())], 9, Some("c2".into()), None)]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_uncovered_scan_budget_seconds(-1.0).with_cover_books_pager(pager));

    let (items, _total, next_cursor, error) = manager.get_uncovered_books_paged(Some(5), None).await;

    assert!(items.is_empty());
    assert_eq!((next_cursor.as_deref(), error), (Some("c1"), None));
    assert_eq!(cursors.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_uncovered_page_stops_at_page_cap_and_returns_resume_cursor() {
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let (pager, cursors) = scripted(vec![
        (vec![(covered.clone(), String::new())], 9, Some("c1".into()), None),
        (vec![(covered.clone(), String::new())], 9, Some("c2".into()), None),
        (vec![(covered, String::new())], 9, Some("c3".into()), None),
    ]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_uncovered_max_scan_pages(2).with_cover_books_pager(pager));

    let (items, _total, next_cursor, _error) = manager.get_uncovered_books_paged(Some(5), None).await;

    assert!(items.is_empty());
    assert_eq!(next_cursor.as_deref(), Some("c2"));
    assert_eq!(cursors.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_uncovered_scans_run_at_most_two_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let (r, p) = (running.clone(), peak.clone());
    let slow_page: CoverBooksPager = Arc::new(move |_cursor| {
        let (r, p) = (r.clone(), p.clone());
        Box::pin(async move {
            let now = r.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            r.fetch_sub(1, Ordering::SeqCst);
            (Vec::new(), 0, None, None)
        })
    });
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(slow_page));

    futures::future::join_all((0..6).map(|_| manager.get_uncovered_books_paged(None, None))).await;

    assert_eq!(peak.load(Ordering::SeqCst), BookManager::UNCOVERED_MAX_CONCURRENT_SCANS);
}

#[tokio::test]
async fn test_uncovered_page_passes_through_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let (pager, _cursors) = scripted(vec![(Vec::new(), 0, None, Some("invalid cursor".into()))]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(pager));

    let (items, total, next_cursor, error) = manager.get_uncovered_books_paged(None, Some("x")).await;

    assert!(items.is_empty());
    assert_eq!((total, next_cursor, error.as_deref()), (None, None, Some("invalid cursor")));
}

#[test]
fn test_unreadable_file_is_not_reported_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    let broken = CoverBook { file_path: tmp.path().join("missing.pdf"), file_type: "pdf".to_string(), name: "missing.pdf".to_string() };
    let manager = make_manager(tmp.path());

    assert_eq!(manager.uncovered_flags(&[broken]), [false]);
}

/// Python `EmptyDocument`: 쪽이 없고 `close` 호출을 기록한다.
struct EmptyDocument {
    closed: Arc<Mutex<Vec<bool>>>,
}

impl PdfDoc for EmptyDocument {
    fn page_count(&self) -> usize {
        0
    }

    fn close(&self) {
        self.closed.lock().unwrap().push(true);
    }
}

#[test]
fn test_pdf_without_pages_has_no_first_image() {
    let tmp = tempfile::tempdir().unwrap();
    let closed = Arc::new(Mutex::new(Vec::new()));
    let sink = closed.clone();
    let open = move |_path: &Path| -> Box<dyn PdfDoc> { Box::new(EmptyDocument { closed: sink.clone() }) };
    let pdf = tmp.path().join("empty.pdf");
    std::fs::write(&pdf, b"%PDF-1.4").unwrap();

    assert_eq!(pdf_first_image_size_with(&pdf, &LOCK, &open), None);
    assert!(!has_official_cover_with(&pdf, "pdf", &LOCK, &open));
    assert_eq!(*closed.lock().unwrap(), [true, true]);
}

#[test]
fn test_epub_is_judged_by_cover_metadata() {
    for cover in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", cover));

        assert_eq!(has_official_cover(&path, "epub", &LOCK), cover, "cover={cover}");
    }
}
