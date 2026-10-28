//! 만화 API는 동일한 도서 처리 로직을 만화 전용 의존성으로 실행한다.
use super::ComicsManager;
use crate::book::{Book, BookLookupError, BookPage};
use crate::book_manager::{BookDeletion, BookManager, CoverPage};
use crate::es_manager::EsManagerApi;
use crate::main::{ApplyCategoryChanges, BookContent, BookManagerApi, BookResponse, ProposeCategoryChanges, ReloadProgress};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[async_trait::async_trait]
impl BookManagerApi for ComicsManager {
    async fn propose_category_changes(&self, request: ProposeCategoryChanges<'_>) -> (Value, Option<String>) {
        BookManagerApi::propose_category_changes(&self.inner, request).await
    }
    async fn index_single_file(&self, file_path: &str) -> Result<i64, String> {
        BookManagerApi::index_single_file(&self.inner, file_path).await
    }
    async fn delete_file(&self, file_path: &str) -> Result<(), String> {
        BookManagerApi::delete_file(&self.inner, file_path).await
    }
    async fn reload_category(&self, category: &str, _content_type: &str) -> (Value, Option<String>) {
        BookManager::reload_category(&self.inner, category, Some("comic")).await
    }
    async fn reload_category_mismatch_files(&self, category: &str, _content_type: &str, on_progress: Option<ReloadProgress>) -> (Value, Option<String>) {
        BookManager::reload_category_mismatch_files_for(&self.inner, category, "comic", on_progress.as_deref()).await
    }
    async fn reload_category_mismatches(&self, _content_type: &str, on_progress: Option<ReloadProgress>) -> (Value, Option<String>) {
        BookManager::reload_category_mismatches_for(&self.inner, "comic", on_progress.as_deref()).await
    }
    async fn apply_category_changes(&self, items: &[Value], allowed_file_paths: &std::collections::HashSet<String>, clean_existing: bool) -> (Value, Option<String>) {
        BookManagerApi::apply_category_changes(&self.inner, items, allowed_file_paths, clean_existing).await
    }
    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> (Value, Option<String>) {
        BookManagerApi::apply_category_changes_with(&self.inner, request).await
    }
    async fn delete_book(&self, id: i64) -> Result<BookDeletion, String> {
        BookManagerApi::delete_book(&self.inner, id).await
    }
    async fn rename_category(&self, old: &str, new: &str) -> (Value, Option<String>) {
        BookManagerApi::rename_category(&self.inner, old, new).await
    }
    async fn delete_category(&self, category: &str, delete_files: bool) -> (Value, Option<String>) {
        BookManagerApi::delete_category(&self.inner, category, delete_files).await
    }
    async fn preview_delete_category(&self, category: &str) -> (Value, Option<String>) {
        BookManagerApi::preview_delete_category(&self.inner, category).await
    }
    async fn preview_insert_image(&self, url: &str) -> Result<Vec<u8>, String> {
        BookManagerApi::preview_insert_image(&self.inner, url).await
    }

    async fn preview_insert_upload(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        BookManagerApi::preview_insert_upload(&self.inner, data).await
    }

    async fn delete_pdf_pages(&self, id: i64, pages: &[i64], crops: Option<&[Value]>, splits: Option<&[Value]>, inserts: Option<&[Value]>, order: Option<&[i64]>) -> Result<Value, String> {
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

    async fn validate_epub(&self, book_id: i64) -> Result<Value, String> {
        BookManagerApi::validate_epub(&self.inner, book_id).await
    }

    async fn validate_pdf(&self, book_id: i64) -> Result<Value, String> {
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

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        BookManagerApi::get_cover_books_paged(&self.inner, category, size, cursor, exclude_categories).await
    }

    fn get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String> {
        BookManagerApi::get_category_mismatches(&self.inner, force_refresh)
    }

    fn get_category_mismatch_details(&self, category: &str) -> Result<Value, String> {
        BookManagerApi::get_category_mismatch_details(&self.inner, category)
    }

    async fn get_uncovered_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        BookManagerApi::get_uncovered_books_paged(&self.inner, category, size, cursor, exclude_categories).await
    }

    async fn set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> Result<Value, String> {
        BookManagerApi::set_epub_cover(&self.inner, book_id, url, image_data).await
    }

    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> Result<Value, String> {
        BookManagerApi::edit_epub_images(&self.inner, book_id, crops, splits, deletes, inserts).await
    }

    async fn remove_epub_cover(&self, book_id: i64) -> Result<Value, String> {
        BookManagerApi::remove_epub_cover(&self.inner, book_id).await
    }

    async fn insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> Result<Value, String> {
        BookManagerApi::insert_epub_illustration(&self.inner, book_id, before_idref, url, image_data).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, file_type: &str, force: bool) -> Result<(), String> {
        BookManagerApi::update_book(&self.inner, id, category, title, author, path, file_type, force).await
    }
}
