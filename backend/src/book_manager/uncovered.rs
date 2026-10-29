//! 표지가 없는 책을 제한된 시간·페이지 수 안에서 조회한다.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{BookManager, BookManagerDeps, CoverPageOpts, MAX_CATEGORY_PAGE_SIZE};
use crate::book::Book;
use crate::cover_status::{CoverStatusCache, has_official_cover};

/// 표지 목록 한 쪽. `items`는 (대표 책, 작품 디렉터리 이름 또는 "")이다.
/// `total`은 표지 목록에서만 채운다. 미표지 목록은 전체를 훑지 않으므로 모른다.
#[derive(Debug, Clone, Default)]
pub struct CoverPage {
    pub items: Vec<(Book, String)>,
    pub total: Option<usize>,
    pub next_cursor: Option<String>,
}
pub type CoverBooksPager = Arc<dyn Fn(CoverPageOpts) -> Pin<Box<dyn Future<Output = Result<CoverPage, String>> + Send>> + Send + Sync>;

#[derive(Clone)]
struct Options {
    pager: Option<CoverBooksPager>,
    budget_seconds: f64,
    max_pages: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self { pager: None, budget_seconds: 10.0, max_pages: 20 }
    }
}

struct ScanSlots(tokio::sync::Semaphore);
impl Default for ScanSlots {
    fn default() -> Self {
        Self(tokio::sync::Semaphore::new(BookManager::UNCOVERED_MAX_CONCURRENT_SCANS))
    }
}

impl BookManagerDeps {
    fn uncovered_options(&self) -> Options {
        self.ext.get::<Options>().map(|v| (*v).clone()).unwrap_or_default()
    }

    pub fn with_cover_books_pager(self, pager: CoverBooksPager) -> Self {
        let options = Options { pager: Some(pager), ..self.uncovered_options() };
        self.with_ext(options)
    }
    pub fn with_uncovered_scan_budget_seconds(self, seconds: f64) -> Self {
        let options = Options { budget_seconds: seconds, ..self.uncovered_options() };
        self.with_ext(options)
    }
    pub fn with_uncovered_max_scan_pages(self, pages: usize) -> Self {
        let options = Options { max_pages: pages.max(1), ..self.uncovered_options() };
        self.with_ext(options)
    }
}

fn flags(books: &[Book]) -> Vec<bool> {
    static LOCK: Mutex<()> = Mutex::new(());
    let cache = CoverStatusCache::new();
    books
        .iter()
        .map(|book| match cache.get_or_compute(&book.file_path, &|| has_official_cover(&book.file_path, &book.file_type, &LOCK)) {
            Ok(covered) => !covered,
            Err(error) => {
                log::warn!("Cover status failed for {}: {error}", book.file_path.display());
                false
            }
        })
        .collect()
}

impl BookManager {
    pub const UNCOVERED_MAX_CONCURRENT_SCANS: usize = 2;

    pub fn uncovered_flags(&self, books: &[Book]) -> Vec<bool> {
        flags(books)
    }

    pub async fn get_uncovered_books_paged(&self, mut opts: CoverPageOpts) -> Result<CoverPage, String> {
        opts.size = opts.size.clamp(1, MAX_CATEGORY_PAGE_SIZE as usize);
        let options = self.ext::<Options>().map(|v| (*v).clone()).unwrap_or_default();
        let start = Instant::now();
        let slots = self.state::<ScanSlots>();
        let _permit = slots.0.acquire().await.expect("private semaphore stays open");
        let mut items = Vec::new();
        let mut next = None;
        for _ in 0..options.max_pages {
            let page = match &options.pager {
                Some(pager) => pager(opts.clone()).await,
                None => self.get_cover_books_paged(opts.clone()).await,
            }?;
            let books: Vec<Book> = page.items.iter().map(|(book, _)| book.clone()).collect();
            let uncovered = tokio::task::spawn_blocking(move || flags(&books)).await.map_err(|error| format!("표지 판정 실패: {error}"))?;
            items.extend(page.items.into_iter().zip(uncovered).filter_map(|(entry, uncovered)| uncovered.then_some(entry)));
            next = page.next_cursor;
            if next.as_deref().is_none_or(str::is_empty) || items.len() >= opts.size || start.elapsed().as_secs_f64() > options.budget_seconds {
                break;
            }
            opts.cursor = next.clone();
        }
        Ok(CoverPage { items, total: None, next_cursor: next })
    }
}
