//! `BookManagerApi`(라우트가 쓰는 facade) 구현. inherent 메서드에 이어 줄 뿐 로직은 두지 않는다.
//!
//! 이 trait에 메서드가 늘면(`main/traits.rs`) 여기에 구현을 한 줄씩 덧붙인다. 아직 이어 주지 않은 trait 메서드는 trait의 기본 구현
//! (미구현 오류)이 쓰인다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{BookContent, BookManager, BookManagerApi, BookResponse, CoverPageOpts};
use crate::book::Book;
use crate::es_manager::EsManagerApi;

/// 라우트가 쪽 크기를 정하지 않을 때 쓰는 카테고리 목록 쪽 크기. Python `size: int = 500`.
const DEFAULT_CATEGORY_PAGE_SIZE: i64 = 500;

#[async_trait]
impl BookManagerApi for BookManager {
    async fn delete_book(&self, id: i64) -> (String, Option<String>) {
        BookManager::delete_book(self, id).await
    }
    async fn rename_category(&self, old: &str, new: &str) -> (Value, Option<String>) {
        BookManager::rename_category(self, old, new).await
    }
    async fn delete_category(&self, category: &str, delete_files: bool) -> (Value, Option<String>) {
        BookManager::delete_category(self, category, delete_files).await
    }
    async fn preview_delete_category(&self, category: &str) -> (Value, Option<String>) {
        BookManager::preview_delete_category(self, category).await
    }
    async fn preview_insert_image(&self, url: &str) -> (Option<Vec<u8>>, Option<String>) {
        BookManager::preview_insert_image(self, url).await
    }

    async fn preview_insert_upload(&self, data: &[u8]) -> (Option<Vec<u8>>, Option<String>) {
        BookManager::preview_insert_upload(self, data).await
    }

    async fn delete_pdf_pages(&self, id: i64, pages: &[i64], crops: Option<&[Value]>, splits: Option<&[Value]>, inserts: Option<&[Value]>, order: Option<&[i64]>) -> (Option<Value>, Option<String>) {
        BookManager::delete_pdf_pages(self, id, pages, crops, splits, inserts, order).await
    }

    fn path_prefix(&self) -> PathBuf {
        BookManager::path_prefix(self)
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        self.es.clone()
    }

    async fn get_book(&self, id: i64) -> (Option<Book>, Option<String>) {
        BookManager::get_book(self, id).await
    }

    async fn get_categories(&self) -> (Value, Option<String>) {
        let (categories, error) = BookManager::get_categories(self).await;
        (json!(categories), error)
    }

    async fn get_books_in_category_paged(&self, category: &str, size: Option<i64>, cursor: Option<&str>) -> (Vec<Book>, i64, Option<String>, Option<String>) {
        BookManager::get_books_in_category_paged(self, category, size.unwrap_or(DEFAULT_CATEGORY_PAGE_SIZE), cursor.unwrap_or("")).await
    }

    async fn get_book_content(&self, id: i64) -> BookContent {
        BookManager::get_book_content(self, id).await
    }

    async fn get_book_preview(&self, id: i64, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        // 음수와 0은 전체 챕터다(Python `chapters <= 0`).
        let chapters = chapters.map(|n| n.clamp(0, i64::from(u32::MAX)) as u32);
        BookManager::get_book_preview_with_base(self, id, None, chapters, resource_base_url.unwrap_or("")).await
    }

    async fn get_book_preview_with_pages(&self, id: i64, pages: Option<i64>, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        let pages = pages.map(|n| n.clamp(0, i64::from(u32::MAX)) as u32);
        let chapters = chapters.map(|n| n.clamp(0, i64::from(u32::MAX)) as u32);
        BookManager::get_book_preview_with_base(self, id, pages, chapters, resource_base_url.unwrap_or("")).await
    }

    async fn get_pdf_pages(&self, id: i64, start: i64, end: i64) -> BookResponse {
        BookManager::get_pdf_pages(self, id, start, end).await
    }

    async fn get_html_resource(&self, book_id: i64, resource_path: &str) -> BookResponse {
        BookManager::get_html_resource(self, book_id, resource_path).await
    }

    async fn get_cover(&self, book_id: i64, width: Option<i64>) -> BookResponse {
        BookManager::get_cover(self, book_id, width).await
    }

    async fn validate_epub(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        BookManager::validate_epub(self, book_id).await
    }

    async fn validate_pdf(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        BookManager::validate_pdf(self, book_id).await
    }

    async fn get_books_in_category(&self, category: &str) -> (Vec<Book>, Option<String>) {
        BookManager::get_books_in_category(self, category).await
    }

    async fn get_latest_books(&self, size: i64, exclude_categories: Option<&[String]>) -> (Vec<Book>, i64, Option<String>) {
        BookManager::get_latest_books(self, size, exclude_categories.map(<[String]>::to_vec)).await
    }

    async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, excluded: Option<&[String]>, category: Option<&str>) -> (Vec<Book>, i64, Option<String>) {
        BookManager::search_by_keyword_paged(self, keyword, size, offset, excluded, category).await
    }

    async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> (Vec<Book>, i64, Option<String>) {
        BookManager::search_similar_books_paged(self, book_id, size, offset).await
    }

    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> (Vec<(Book, String)>, i64, Option<String>, Option<String>) {
        let opts = CoverPageOpts {
            category: Some(category.to_string()),
            size: size.max(0) as usize,
            cursor: Some(cursor.to_string()),
            exclude_categories: exclude_categories.map(<[String]>::to_vec).unwrap_or_default(),
        };
        let (items, total, next_cursor, error) = BookManager::get_cover_books_paged(self, opts).await;
        (items, total as i64, next_cursor, error)
    }

    fn get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String> {
        self.try_get_category_mismatches(force_refresh)
    }

    async fn get_uncovered_books_paged(&self, category: &str, size: i64, cursor: &str, exclude_categories: Option<&[String]>) -> (Vec<(Book, String)>, Option<i64>, Option<String>, Option<String>) {
        let opts = CoverPageOpts { category: Some(category.into()), size: size.max(0) as usize, cursor: Some(cursor.into()), exclude_categories: exclude_categories.unwrap_or_default().to_vec() };
        let (items, total, next, error) = BookManager::get_uncovered_books_paged(self, opts).await;
        (items, total.map(|n| n as i64), next, error)
    }

    async fn set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> (Option<Value>, Option<String>) {
        BookManager::set_epub_cover(self, book_id, url, image_data).await
    }

    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> (Option<Value>, Option<String>) {
        BookManager::edit_epub_images(self, book_id, crops, splits, deletes, inserts).await
    }

    async fn remove_epub_cover(&self, book_id: i64) -> (Option<Value>, Option<String>) {
        BookManager::remove_epub_cover(self, book_id).await
    }

    async fn insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> (Option<Value>, Option<String>) {
        BookManager::insert_epub_illustration(self, book_id, before_idref, url, image_data).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, file_type: &str, force: bool) -> (Value, Option<String>) {
        let (status, error) = BookManager::update_book_with(self, id, category, title, author, path, file_type, force).await;
        (json!(status), error)
    }
}
