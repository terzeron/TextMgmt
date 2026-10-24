//! 만화 API는 동일한 도서 처리 로직을 만화 전용 의존성으로 실행한다.
use super::ComicsManager;
use crate::book::Book;
use crate::es_manager::EsManagerApi;
use crate::main::{BookContent, BookManagerApi, BookResponse};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[async_trait::async_trait]
impl BookManagerApi for ComicsManager {
    fn path_prefix(&self) -> PathBuf {
        BookManagerApi::path_prefix(&self.inner)
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        BookManagerApi::es_manager(&self.inner)
    }

    async fn get_book(&self, id: i64) -> (Option<Book>, Option<String>) {
        BookManagerApi::get_book(&self.inner, id).await
    }

    async fn get_categories(&self) -> (Value, Option<String>) {
        BookManagerApi::get_categories(&self.inner).await
    }

    async fn get_books_in_category_paged(&self, category: &str, size: Option<i64>, cursor: Option<&str>) -> (Vec<Book>, i64, Option<String>, Option<String>) {
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

    async fn validate_epub(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        BookManagerApi::validate_epub(&self.inner, book_id).await
    }

    async fn validate_pdf(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        BookManagerApi::validate_pdf(&self.inner, book_id).await
    }

    async fn get_books_in_category(&self, category: &str) -> (Vec<Book>, Option<String>) {
        BookManagerApi::get_books_in_category(&self.inner, category).await
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> (Vec<Book>, i64, Option<String>) {
        BookManagerApi::get_latest_books(&self.inner, size, exclude_categories).await
    }

    async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> (Vec<Book>, i64, Option<String>) {
        BookManagerApi::search_similar_books_paged(&self.inner, book_id, size, offset).await
    }

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> (Vec<(Book, String)>, i64, Option<String>, Option<String>) {
        BookManagerApi::get_cover_books_paged(&self.inner, category, size, cursor, exclude_categories).await
    }

    fn get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String> {
        BookManagerApi::get_category_mismatches(&self.inner, force_refresh)
    }

    async fn set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> (Option<Value>, Option<String>) {
        BookManagerApi::set_epub_cover(&self.inner, book_id, url, image_data).await
    }

    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> (Option<Value>, Option<String>) {
        BookManagerApi::edit_epub_images(&self.inner, book_id, crops, splits, deletes, inserts).await
    }

    async fn remove_epub_cover(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        BookManagerApi::remove_epub_cover(&self.inner, book_id).await
    }

    async fn insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> (Option<Value>, Option<String>) {
        BookManagerApi::insert_epub_illustration(&self.inner, book_id, before_idref, url, image_data).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, file_type: &str, force: bool) -> (Value, Option<String>) {
        BookManagerApi::update_book(&self.inner, id, category, title, author, path, file_type, force).await
    }
}
