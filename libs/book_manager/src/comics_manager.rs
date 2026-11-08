//! 만화 관리자.
//!
//! 만화 루트와 ES 색인을 따로 보관하고 도서 처리 기능은 `BookManager`에 위임한다.

use crate::ApplyCategoryChanges;
use crate::ApplyResult;
use crate::BookContent;
use crate::BookDeletion;
use crate::BookManager;
use crate::BookManagerApi;
use crate::BookManagerDeps;
use crate::BookResponse;
use crate::CategoryDeletePreview;
use crate::CategoryDeletion;
use crate::CategoryRename;
use crate::CoverPage;
use crate::EpubValidation;
use crate::MismatchDetails;
use crate::MismatchReload;
use crate::MismatchSummary;
use crate::PdfEditResult;
use crate::PdfValidation;
use crate::ProposeCategoryChanges;
use crate::ProposeResult;
use crate::ReloadProgress;
use crate::ReloadedFiles;
use book_model::book_metadata::Book;
use book_model::book_metadata::BookLookupError;
use book_model::book_metadata::BookPage;
use cover_manager::cover_management::CoverSave;
use search_manager::EsManagerApi;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use sysdeps::EnvFn;

pub use crate::BookManagerError;

const CREATED_TIME_BACKFILL_ENV: &str = "TM_BACKFILL_CREATED_TIME_ON_STARTUP";

fn env_flag_enabled(env: EnvFn, name: &str) -> bool {
    matches!(env(name).unwrap_or_default().trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

pub struct ComicsManager {
    inner: BookManager,
    path_prefix: PathBuf,
    es_index_name: String,
    es_manager: Arc<dyn EsManagerApi>,
    backfill_thread: Mutex<Option<JoinHandle<()>>>,
    mismatch_cache: Option<Value>,
    mismatch_cache_time: f64,
    mismatch_state_lock: Mutex<()>,
    mismatch_scan_lock: Mutex<()>,
}

impl ComicsManager {
    /// 책과 분리된 경로·ES·테스트 대역으로 만화 관리자를 만든다.
    pub fn with_deps(deps: BookManagerDeps) -> Self {
        let inner = BookManager::with_deps(deps);
        Self {
            path_prefix: inner.path_prefix(),
            es_manager: BookManagerApi::es_manager(&inner),
            inner,
            es_index_name: String::new(),
            backfill_thread: Mutex::new(None),
            mismatch_cache: None,
            mismatch_cache_time: 0.0,
            mismatch_state_lock: Mutex::new(()),
            mismatch_scan_lock: Mutex::new(()),
        }
    }

    pub fn from_env(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<ComicsManager, BookManagerError> {
        Self::from_env_with(env, es_factory, |deps| deps)
    }

    /// `from_env`와 같고, 운영용 reader 등 manager 의존성을 구성할 때 꽂는다.
    /// `TM_LOADER_BIN`도 `from_env_with_es`와 같이 읽는다.
    pub fn from_env_with(env: EnvFn, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>, customize: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Result<ComicsManager, BookManagerError> {
        // 디렉터리·색인 설정이 없으면 ES에 연결하기 전에 멈춘다.
        let (_, index_name) = Self::comics_env(env)?;
        Self::from_env_with_es(env, es_factory(&index_name), customize)
    }

    fn comics_env(env: EnvFn) -> Result<(PathBuf, String), BookManagerError> {
        let missing = |name: &str| BookManagerError(format!("The environment variable {name} is not set."));
        let dir = env("TM_COMICS_DIR").ok_or_else(|| missing("TM_COMICS_DIR"))?;
        let index_name = env("TM_ES_COMICS_INDEX").ok_or_else(|| missing("TM_ES_COMICS_INDEX"))?;
        Ok((PathBuf::from(dir), index_name))
    }

    /// 이미 연결한 ES manager로 운영 인스턴스를 만든다.
    pub fn from_env_with_es(env: EnvFn, es_manager: Arc<dyn EsManagerApi>, customize: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Result<ComicsManager, BookManagerError> {
        let (path_prefix, index_name) = Self::comics_env(env)?;
        log::debug!("{}", path_prefix.display());
        es_manager.create_index().map_err(|e| BookManagerError(format!("create_index failed: {e}")))?;
        let mut deps = BookManagerDeps::new(path_prefix, es_manager);
        if let Some(program) = env("TM_LOADER_BIN").filter(|p| !p.is_empty()) {
            deps = deps.with_loader_program(&program);
        }
        let mut manager = Self::with_deps(customize(deps));
        manager.es_index_name = index_name;
        manager.backfill_created_time_if_enabled(env);
        Ok(manager)
    }

    fn backfill_created_time_if_enabled(&self, env: EnvFn) {
        if !env_flag_enabled(env, CREATED_TIME_BACKFILL_ENV) {
            return;
        }
        let mut slot = self.backfill_thread.lock().unwrap();
        if slot.as_ref().is_some_and(|t| !t.is_finished()) {
            return;
        }
        let es = self.es_manager.clone();
        let prefix = self.path_prefix.clone();
        let spawned = std::thread::Builder::new().name(format!("created-time-backfill-{}", self.es_index_name)).spawn(move || {
            if let Err(e) = es.backfill_created_time(&prefix) {
                log::warn!("created_time backfill failed: {e}");
            }
        });
        match spawned {
            Ok(handle) => *slot = Some(handle),
            Err(e) => log::warn!("created_time backfill failed to start: {e}"),
        }
    }

    /// 백필 스레드가 끝나기를 기다린다. 테스트와 종료 처리에 쓴다.
    pub fn join_backfill(&self) {
        let handle = self.backfill_thread.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    pub fn path_prefix(&self) -> &Path {
        &self.path_prefix
    }

    pub fn es_index_name(&self) -> &str {
        &self.es_index_name
    }

    pub fn es_manager(&self) -> &Arc<dyn EsManagerApi> {
        &self.es_manager
    }

    pub fn mismatch_cache(&self) -> Option<&Value> {
        self.mismatch_cache.as_ref()
    }

    pub fn mismatch_cache_time(&self) -> f64 {
        self.mismatch_cache_time
    }

    pub fn set_mismatch_cache(&mut self, cache: Value) {
        self.mismatch_cache = Some(cache);
        // 0.0은 "없음"으로 쓰므로 단조 시각에 1초를 더한다.
        self.mismatch_cache_time = sysdeps::Clock::monotonic(&sysdeps::SystemClock) + 1.0;
    }

    pub fn clear_mismatch_cache(&mut self) {
        self.mismatch_cache = None;
        self.mismatch_cache_time = 0.0;
    }

    pub fn mismatch_state_lock(&self) -> &Mutex<()> {
        &self.mismatch_state_lock
    }

    pub fn mismatch_scan_lock(&self) -> &Mutex<()> {
        &self.mismatch_scan_lock
    }
}

// ---- api_adapter ----
// 만화 API는 동일한 도서 처리 로직을 만화 전용 의존성으로 실행한다.
#[async_trait::async_trait]
impl BookManagerApi for ComicsManager {
    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> Result<ProposeResult, String> {
        BookManagerApi::propose_category_changes(&self.inner, request).await
    }
    async fn index_single_file(&self, file_path: &str) -> Result<i64, String> {
        BookManagerApi::index_single_file(&self.inner, file_path).await
    }
    async fn delete_file(&self, file_path: &str) -> Result<(), String> {
        BookManagerApi::delete_file(&self.inner, file_path).await
    }
    async fn reload_category(&self, category: &str, _content_type: &str) -> Result<ReloadedFiles, String> {
        BookManager::reload_category(&self.inner, category, Some("comic")).await
    }
    async fn reload_category_mismatch_files(&self, category: &str, _content_type: &str, on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        BookManager::reload_category_mismatch_files_for(&self.inner, category, "comic", on_progress.as_deref()).await
    }
    async fn reload_category_mismatches(&self, _content_type: &str, on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        BookManager::reload_category_mismatches_for(&self.inner, "comic", on_progress.as_deref()).await
    }
    async fn apply_category_changes(&self, items: &[Value], allowed_file_paths: &std::collections::HashSet<String>, clean_existing: bool) -> Result<ApplyResult, String> {
        BookManagerApi::apply_category_changes(&self.inner, items, allowed_file_paths, clean_existing).await
    }
    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<ApplyResult, String> {
        BookManagerApi::apply_category_changes_with(&self.inner, request).await
    }
    async fn delete_book(&self, id: i64) -> Result<BookDeletion, String> {
        BookManagerApi::delete_book(&self.inner, id).await
    }
    async fn rename_category(&self, old: &str, new: &str) -> Result<CategoryRename, String> {
        BookManagerApi::rename_category(&self.inner, old, new).await
    }
    async fn delete_category(&self, category: &str, delete_files: bool) -> Result<CategoryDeletion, String> {
        BookManagerApi::delete_category(&self.inner, category, delete_files).await
    }
    async fn preview_delete_category(&self, category: &str) -> Result<CategoryDeletePreview, String> {
        BookManagerApi::preview_delete_category(&self.inner, category).await
    }
    async fn preview_insert_image(&self, url: &str) -> Result<Vec<u8>, String> {
        BookManagerApi::preview_insert_image(&self.inner, url).await
    }

    async fn preview_insert_upload(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        BookManagerApi::preview_insert_upload(&self.inner, data).await
    }

    async fn delete_pdf_pages(&self, id: i64, pages: &[i64], crops: Option<&[Value]>, splits: Option<&[Value]>, inserts: Option<&[Value]>, order: Option<&[i64]>) -> Result<PdfEditResult, String> {
        BookManagerApi::delete_pdf_pages(&self.inner, id, pages, crops, splits, inserts, order).await
    }

    fn path_prefix(&self) -> PathBuf {
        BookManagerApi::path_prefix(&self.inner)
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        BookManagerApi::es_manager(&self.inner)
    }

    async fn get_book(&self, id: i64) -> Result<Book, BookLookupError> {
        BookManagerApi::get_book(&self.inner, id).await
    }

    async fn get_categories(&self) -> Result<Value, String> {
        BookManagerApi::get_categories(&self.inner).await
    }

    async fn get_books_in_category_paged(&self, category: &str, size: Option<i64>, cursor: Option<&str>) -> Result<BookPage, String> {
        BookManagerApi::get_books_in_category_paged(&self.inner, category, size, cursor).await
    }

    async fn get_book_content(&self, id: i64) -> BookContent {
        BookManagerApi::get_book_content(&self.inner, id).await
    }

    async fn get_book_preview(&self, id: i64, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        BookManagerApi::get_book_preview(&self.inner, id, chapters, resource_base_url).await
    }

    async fn get_book_preview_with_pages(&self, id: i64, pages: Option<i64>, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        BookManagerApi::get_book_preview_with_pages(&self.inner, id, pages, chapters, resource_base_url).await
    }

    async fn get_pdf_pages(&self, id: i64, start: i64, end: i64) -> BookResponse {
        BookManagerApi::get_pdf_pages(&self.inner, id, start, end).await
    }

    async fn get_html_resource(&self, book_id: i64, resource_path: &str) -> BookResponse {
        BookManagerApi::get_html_resource(&self.inner, book_id, resource_path).await
    }

    async fn get_cover(&self, book_id: i64, width: Option<i64>) -> BookResponse {
        BookManagerApi::get_cover(&self.inner, book_id, width).await
    }

    async fn validate_epub(&self, book_id: i64) -> Result<EpubValidation, String> {
        BookManagerApi::validate_epub(&self.inner, book_id).await
    }

    async fn validate_pdf(&self, book_id: i64) -> Result<PdfValidation, String> {
        BookManagerApi::validate_pdf(&self.inner, book_id).await
    }

    async fn get_books_in_category(&self, category: &str) -> Result<Vec<Book>, String> {
        BookManagerApi::get_books_in_category(&self.inner, category).await
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> Result<BookPage, String> {
        BookManagerApi::get_latest_books(&self.inner, size, exclude_categories).await
    }

    async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, excluded: Option<&[String]>, category: Option<&str>) -> Result<BookPage, String> {
        BookManagerApi::search_by_keyword_paged(&self.inner, keyword, size, offset, excluded, category).await
    }

    async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> Result<BookPage, String> {
        BookManagerApi::search_similar_books_paged(&self.inner, book_id, size, offset).await
    }

    async fn get_latest_comic_covers(&self, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        BookManagerApi::get_latest_comic_covers(&self.inner, size, cursor, exclude_categories).await
    }

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        BookManagerApi::get_cover_books_paged(&self.inner, category, size, cursor, exclude_categories).await
    }

    fn get_category_mismatches(&self, force_refresh: bool) -> Result<MismatchSummary, String> {
        BookManagerApi::get_category_mismatches(&self.inner, force_refresh)
    }

    fn get_category_mismatch_details(&self, category: &str) -> Result<MismatchDetails, String> {
        BookManagerApi::get_category_mismatch_details(&self.inner, category)
    }

    async fn get_uncovered_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        BookManagerApi::get_uncovered_books_paged(&self.inner, category, size, cursor, exclude_categories).await
    }

    async fn set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> Result<CoverSave, String> {
        BookManagerApi::set_epub_cover(&self.inner, book_id, url, image_data).await
    }

    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> Result<CoverSave, String> {
        BookManagerApi::edit_epub_images(&self.inner, book_id, crops, splits, deletes, inserts).await
    }

    async fn remove_epub_cover(&self, book_id: i64) -> Result<CoverSave, String> {
        BookManagerApi::remove_epub_cover(&self.inner, book_id).await
    }

    async fn insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> Result<CoverSave, String> {
        BookManagerApi::insert_epub_illustration(&self.inner, book_id, before_idref, url, image_data).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, file_type: &str, force: bool) -> Result<(), String> {
        BookManagerApi::update_book(&self.inner, id, category, title, author, path, file_type, force).await
    }
}
