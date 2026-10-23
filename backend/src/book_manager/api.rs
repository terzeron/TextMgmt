//! `BookManagerApi`(라우트가 쓰는 facade) 구현. inherent 메서드에 이어 줄 뿐 로직은 두지 않는다.
//!
//! 이 trait에 메서드가 늘면(`main/traits.rs`) 여기에 구현을 한 줄씩 덧붙인다. 아직 이어 주지 않은 trait 메서드는 trait의 기본 구현
//! (미구현 오류)이 쓰인다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{BookContent, BookManager, BookManagerApi, BookResponse};
use crate::book::Book;
use crate::es_manager::EsManagerApi;

/// 라우트가 쪽 크기를 정하지 않을 때 쓰는 카테고리 목록 쪽 크기. Python `size: int = 500`.
const DEFAULT_CATEGORY_PAGE_SIZE: i64 = 500;

#[async_trait]
impl BookManagerApi for BookManager {
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

    async fn get_pdf_pages(&self, id: i64, start: i64, end: i64) -> BookResponse {
        BookManager::get_pdf_pages(self, id, start, end).await
    }

    fn get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String> {
        self.try_get_category_mismatches(force_refresh)
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, id: i64, category: &str, title: &str, author: &str, path: &Path, file_type: &str, force: bool) -> (Value, Option<String>) {
        let (status, error) = BookManager::update_book_with(self, id, category, title, author, path, file_type, force).await;
        (json!(status), error)
    }
}
