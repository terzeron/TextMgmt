//! 테스트가 꽂는 보조 seam과, 매니저가 소유하는 PDF 캐시·백필 스레드 접근자.
//! 모두 소비형 `BookManagerDeps::with_*`로 꽂고 생성 뒤에는 바꾸지 않는다(스레드 슬롯만 예외다. 값 설정자다).

use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{BookManager, BookManagerDeps, CREATED_TIME_BACKFILL_ENV};
use crate::main::{TaskHandle, TaskRunner, ThreadRunner};
use crate::pdf::{PageCountCache, PdfCacheLimits, PdfError, PdfReaderCache, PdfReaderHandle};
use std::os::unix::fs::MetadataExt;

/// `BookManager`가 내는 오류. Python `RuntimeError`에 대응한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookManagerError {
    Runtime(String),
}

impl fmt::Display for BookManagerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BookManagerError::Runtime(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BookManagerError {}

/// 문서 색인(`add_book`)을 통째로 바꾸는 대역. 이동 롤백처럼 ES 색인이 실패하는 경로를 시험할 때 쓴다(구 `override_add_book`).
pub trait BookAdder: Send + Sync {
    fn add_book(&self) -> Result<(Option<i64>, Option<String>), BookManagerError>;
}

/// created_time 백필 스레드. 구 `threading.Thread` 대역.
pub trait BackfillThread: Send + Sync {
    fn is_alive(&self) -> bool;
    fn start(&self) -> io::Result<()>;
    /// 끝나기를 `timeout`까지 기다린다. 기본 구현은 기다리지 않는다.
    fn join(&self, _timeout: Duration) {}
}

pub type PreviewEpubValidator = Arc<dyn Fn(&Path) -> (bool, String) + Send + Sync>;
pub type PathIsRelativeTo = Arc<dyn Fn(&Path, &Path) -> io::Result<bool> + Send + Sync>;
/// 카테고리 이름 검사를 바꾼다. 둘째 인자는 원래 검사다. `Err`는 검사 중 파일시스템 오류다.
pub type CategoryNameCheck = Arc<dyn Fn(&str, &dyn Fn(&str) -> bool) -> io::Result<bool> + Send + Sync>;

/// 이미지 축소 함수 대체(관찰용). 원본 바이트를 받아 줄인 바이트를 돌려준다.
pub type DownscaleHook = Arc<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>;
/// 파일별 분류 제안 대체. `Err`는 그 파일의 제안 실패다.
pub type ProposeHook = Arc<dyn Fn(&Path) -> Result<serde_json::Value, String> + Send + Sync>;
/// 파일 이동 대체. 인자는 (원본 경로, 목적지 카테고리). 반환은 `_move_classified_file`과 같다.
pub type MoveHook = Arc<dyn Fn(&Path, &str) -> (Option<serde_json::Value>, Option<String>) + Send + Sync>;
/// 여러 파일 색인 대체. 인자는 (상대 경로 목록, clean_existing). 반환은 (경로 -> 새 book_id, 실패 목록).
pub type BulkIndexHook = Arc<dyn Fn(&[String], bool) -> (std::collections::BTreeMap<String, i64>, Vec<serde_json::Value>) + Send + Sync>;

#[derive(Clone, Default)]
pub(crate) struct Seams {
    pub epub_image_min_bytes: Option<usize>,
    pub downscale_hook: Option<DownscaleHook>,
    pub propose_hook: Option<ProposeHook>,
    pub move_hook: Option<MoveHook>,
    pub bulk_index_hook: Option<BulkIndexHook>,
    pub book_adder: Option<Arc<dyn BookAdder>>,
    pub preview_epub_validator: Option<PreviewEpubValidator>,
    pub path_is_relative_to: Option<PathIsRelativeTo>,
    pub category_name_check: Option<CategoryNameCheck>,
    pub task_runner: Option<Arc<dyn TaskRunner>>,
    pub pdf_cache_limits: Option<PdfCacheLimits>,
    pub page_count_cache_max: Option<usize>,
}

impl BookManagerDeps {
    pub(super) fn seams(&self) -> Seams {
        self.ext.get::<Seams>().map(|s| (*s).clone()).unwrap_or_default()
    }

    /// `add_book`을 이 대역으로 바꾼다.
    pub fn with_book_adder(self, adder: Arc<dyn BookAdder>) -> Self {
        let seams = Seams { book_adder: Some(adder), ..self.seams() };
        self.with_ext(seams)
    }

    /// 만든 미리보기 EPUB을 검증하는 함수를 바꾼다. 반환은 (유효 여부, 사유).
    pub fn with_preview_epub_validator(self, validator: PreviewEpubValidator) -> Self {
        let seams = Seams { preview_epub_validator: Some(validator), ..self.seams() };
        self.with_ext(seams)
    }

    /// 경로 포함 관계 검사(`is_relative_to`)를 바꾼다. HTML 리소스 경로 검증이 쓴다.
    pub fn with_path_is_relative_to(self, check: PathIsRelativeTo) -> Self {
        let seams = Seams { path_is_relative_to: Some(check), ..self.seams() };
        self.with_ext(seams)
    }

    pub fn with_category_name_check(self, check: CategoryNameCheck) -> Self {
        let seams = Seams { category_name_check: Some(check), ..self.seams() };
        self.with_ext(seams)
    }

    /// 백그라운드 작업(created_time 백필)을 띄우는 실행기. 기본은 스레드 하나다.
    pub fn with_task_runner(self, runner: Arc<dyn TaskRunner>) -> Self {
        let seams = Seams { task_runner: Some(runner), ..self.seams() };
        self.with_ext(seams)
    }

    pub fn with_pdf_cache_limits(self, limits: PdfCacheLimits) -> Self {
        let seams = Seams { pdf_cache_limits: Some(limits), ..self.seams() };
        self.with_ext(seams)
    }

    /// 미리보기 EPUB에서 이미지 축소를 시도하는 최소 크기(바이트, 기본 400KiB). 값 설정이다.
    pub fn with_epub_image_min_bytes(self, bytes: usize) -> Self {
        let seams = Seams { epub_image_min_bytes: Some(bytes), ..self.seams() };
        self.with_ext(seams)
    }

    pub fn with_downscale_epub_image_hook(self, hook: DownscaleHook) -> Self {
        let seams = Seams { downscale_hook: Some(hook), ..self.seams() };
        self.with_ext(seams)
    }

    pub fn with_propose_category_for_file_hook(self, hook: ProposeHook) -> Self {
        let seams = Seams { propose_hook: Some(hook), ..self.seams() };
        self.with_ext(seams)
    }

    pub fn with_move_classified_file_hook(self, hook: MoveHook) -> Self {
        let seams = Seams { move_hook: Some(hook), ..self.seams() };
        self.with_ext(seams)
    }

    pub fn with_bulk_index_files_hook(self, hook: BulkIndexHook) -> Self {
        let seams = Seams { bulk_index_hook: Some(hook), ..self.seams() };
        self.with_ext(seams)
    }

    /// 쪽수 캐시 항목 수 상한(기본 4096).
    pub fn with_page_count_cache_max(self, max: usize) -> Self {
        let seams = Seams { page_count_cache_max: Some(max), ..self.seams() };
        self.with_ext(seams)
    }
}

/// `TaskRunner`로 띄우는 백필 작업.
struct RunnerBackfill {
    runner: Arc<dyn TaskRunner>,
    name: String,
    job: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    handle: Mutex<Option<Box<dyn TaskHandle>>>,
}

impl BackfillThread for RunnerBackfill {
    fn is_alive(&self) -> bool {
        self.handle.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|h| !h.is_finished())
    }

    fn start(&self) -> io::Result<()> {
        let Some(job) = self.job.lock().unwrap_or_else(|e| e.into_inner()).take() else { return Ok(()) };
        // 실행기가 시작에 실패해 panic해도 매니저 생성이나 호출자를 막지 않는다.
        match catch_unwind(AssertUnwindSafe(|| self.runner.spawn(&self.name, job))) {
            Ok(handle) => {
                *self.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
                Ok(())
            }
            Err(_) => Err(io::Error::other("백그라운드 작업을 시작하지 못했다")),
        }
    }

    fn join(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.is_alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if !self.is_alive() && let Some(handle) = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take() {
            handle.join();
        }
    }
}

fn env_flag(value: Option<String>) -> bool {
    matches!(value.unwrap_or_default().trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

impl BookManager {
    pub(super) fn seams(&self) -> Seams {
        self.ext::<Seams>().map(|s| (*s).clone()).unwrap_or_default()
    }

    pub fn pdf_cache(&self) -> &PdfReaderCache {
        &self.pdf_cache
    }

    pub fn page_count_cache(&self) -> &PageCountCache {
        &self.page_count_cache
    }

    pub fn created_time_backfill_thread(&self) -> Option<Arc<dyn BackfillThread>> {
        self.backfill_thread.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_created_time_backfill_thread(&self, thread: Option<Arc<dyn BackfillThread>>) {
        *self.backfill_thread.lock().unwrap_or_else(|e| e.into_inner()) = thread;
    }

    /// 환경 변수(`TM_BACKFILL_CREATED_TIME_ON_STARTUP`)가 켜져 있으면 백필을 시작한다. 이미 도는 스레드가 있으면 아무것도 안 한다.
    pub fn _backfill_created_time_if_enabled(&self) {
        self.start_created_time_backfill(env_flag(std::env::var(CREATED_TIME_BACKFILL_ENV).ok()), "");
    }

    pub(super) fn start_created_time_backfill(&self, enabled: bool, index_name: &str) {
        if !enabled {
            return;
        }
        if self.created_time_backfill_thread().is_some_and(|t| t.is_alive()) {
            return;
        }
        let (es, prefix) = (self.es.clone(), self.path_prefix.clone());
        let job: Box<dyn FnOnce() + Send> = Box::new(move || {
            if let Err(e) = es.backfill_created_time(&prefix) {
                log::warn!("created_time backfill failed: {e}");
            }
        });
        let runner = self.seams().task_runner.unwrap_or_else(|| Arc::new(ThreadRunner));
        let name = format!("created-time-backfill-{}", if index_name.is_empty() { "book" } else { index_name });
        let thread: Arc<dyn BackfillThread> = Arc::new(RunnerBackfill { runner, name, job: Mutex::new(Some(job)), handle: Mutex::new(None) });
        self.set_created_time_backfill_thread(Some(thread.clone()));
        if let Err(e) = thread.start() {
            self.set_created_time_backfill_thread(None);
            log::warn!("created_time backfill failed to start: {e}");
        }
    }

    /// 파일 수정 시각(ns). 쪽수 캐시 키에 쓴다.
    fn mtime_key(&self, path: &Path) -> std::io::Result<(i64, u64)> {
        let meta = self.fs.metadata(path)?;
        Ok((meta.mtime() * 1_000_000_000 + meta.mtime_nsec(), meta.len()))
    }

    /// PDF 리더 핸들과 총 쪽수. 파일이 바뀌면(mtime 불일치) 캐시를 무효화한다. 쪽수는 항상 캐시하고, 핸들은 한도 안에서만 보관한다.
    pub(crate) fn try_get_cached_pdf_reader(&self, file_path: &Path) -> Result<(Arc<PdfReaderHandle>, i64), PdfError> {
        let (mtime, size) = self.mtime_key(file_path).map_err(|e| PdfError::new("OSError", e.to_string()))?;
        let key = file_path.to_string_lossy().into_owned();
        if let Some(hit) = self.pdf_cache.get(&key, mtime) {
            return Ok(hit);
        }
        let total = self.pdf_pages.page_count(file_path)?;
        let reader = Arc::new(PdfReaderHandle { path: file_path.to_path_buf(), page_count: total });
        let page_key = (key.clone(), mtime);
        self.page_count_cache.insert(page_key, total as i64);
        self.page_count_cache.evict();
        self.pdf_cache.store(&key, mtime, reader.clone(), total as i64, size);
        Ok((reader, total as i64))
    }

    /// `_get_cached_pdf_reader`. 읽지 못하면 panic한다(Python은 예외가 올라간다). 서비스 코드는 `try_get_cached_pdf_reader`를 쓴다.
    pub fn _get_cached_pdf_reader(&self, file_path: &Path) -> (Arc<PdfReaderHandle>, i64) {
        self.try_get_cached_pdf_reader(file_path).unwrap_or_else(|e| panic!("PDF를 읽지 못했다: {e}"))
    }

    /// (총 쪽수, 방금 만든 리더 또는 `None`). 쪽수 캐시에 있으면 리더를 만들지 않는다.
    pub(crate) fn try_get_cached_page_count(&self, file_path: &Path) -> Result<(i64, Option<Arc<PdfReaderHandle>>), PdfError> {
        let (mtime, _) = self.mtime_key(file_path).map_err(|e| PdfError::new("OSError", e.to_string()))?;
        let key = (file_path.to_string_lossy().into_owned(), mtime);
        if let Some(pages) = self.page_count_cache.get(&key) {
            return Ok((pages, None));
        }
        let (reader, total) = self.try_get_cached_pdf_reader(file_path)?;
        Ok((total, Some(reader)))
    }
}
