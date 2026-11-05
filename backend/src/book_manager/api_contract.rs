//! 라우트가 쓰는 책 관리 facade(`BookManagerApi`)와 그 요청·응답 타입. 구현은 `api.rs`에 있다.
//! HTTP 계층(`main`)이 아니라 이 모듈에 두어 `book_manager`가 `main`에 기대지 않게 한다.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::book_manager::{BookDeletion, CoverPage};
use crate::book_manager::{CategoryDeletePreview, CategoryDeletion, CategoryRename, MismatchDetails, MismatchReload, MismatchSummary, ReloadedFiles};
use crate::book_metadata::{Book, BookLookupError, BookPage};
use crate::cover_management::CoverSave;
use crate::es_manager::EsManagerApi;

/// 사용 가능한 구현이 없을 때 쓰는 사유 문구. 각 trait의 기본 구현이 돌려준다.
pub const UNIMPLEMENTED: &str = "unimplemented";

// ---------------------------------------------------------------------------
// 책 관리자
// ---------------------------------------------------------------------------

/// `get_book_content`의 결과: 디스크 파일, 본문 텍스트 또는 변환 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum BookContent {
    File(PathBuf),
    Text(String),
    Error(String),
}

/// 미리보기·PDF 쪽·표지 같은 응답 하나. 구 `PreviewResponse`, `PdfPagesResponse`, `CoverResponse`를 합친 것이다.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BookResponse {
    pub status_code: u16,
    pub media_type: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

struct NoEs;

impl EsManagerApi for NoEs {}

pub struct ProposeCategoryChanges<'a> {
    pub category: &'a str,
    pub mappings: &'a Value,
    pub content_type: &'a str,
    pub use_bookstore: bool,
    pub use_content_meta: bool,
    pub delay: f64,
    pub on_progress: Option<&'a (dyn Fn(Value) + Send + Sync)>,
}

pub struct ApplyCategoryChanges<'a> {
    pub items: &'a [Value],
    pub allowed_file_paths: &'a std::collections::HashSet<String>,
    pub content_type: &'a str,
    pub clean_existing: bool,
    pub on_progress: Option<&'a (dyn Fn(Value) + Send + Sync)>,
    pub on_item_done: Option<&'a (dyn Fn(Value) + Send + Sync)>,
    pub should_continue: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
}

pub type ReloadProgress = Arc<dyn Fn(crate::book_manager::ReloadCounts) + Send + Sync>;

/// 책/만화 관리자 facade. 책과 만화가 같은 trait를 쓴다. 지금은 문서 3.2의 통일 시그니처 일부만 있다.
#[async_trait]
pub trait BookManagerApi: Send + Sync {
    async fn propose_category_changes(&self, _request: ProposeCategoryChanges<'_>) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn apply_category_changes(&self, _items: &[Value], _allowed_file_paths: &std::collections::HashSet<String>, _clean_existing: bool) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn apply_category_changes_with(&self, request: ApplyCategoryChanges<'_>) -> Result<Value, String> {
        self.apply_category_changes(request.items, request.allowed_file_paths, request.clean_existing).await
    }
    async fn delete_book(&self, _id: i64) -> Result<BookDeletion, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn index_single_file(&self, _file_path: &str) -> Result<i64, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn delete_file(&self, _file_path: &str) -> Result<(), String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn reload_category(&self, _category: &str, _content_type: &str) -> Result<ReloadedFiles, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn reload_category_mismatch_files(&self, _category: &str, _content_type: &str, _on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn reload_category_mismatches(&self, _content_type: &str, _on_progress: Option<ReloadProgress>) -> Result<MismatchReload, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn rename_category(&self, _old: &str, _new: &str) -> Result<CategoryRename, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn delete_category(&self, _category: &str, _delete_files: bool) -> Result<CategoryDeletion, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    async fn preview_delete_category(&self, _category: &str) -> Result<CategoryDeletePreview, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    fn path_prefix(&self) -> PathBuf {
        PathBuf::new()
    }

    fn es_manager(&self) -> Arc<dyn EsManagerApi> {
        Arc::new(NoEs)
    }

    async fn get_book(&self, _id: i64) -> Result<Book, BookLookupError> {
        Err(BookLookupError::Failed(UNIMPLEMENTED.to_string()))
    }

    async fn get_categories(&self) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_books_in_category_paged(&self, _category: &str, _size: Option<i64>, _cursor: Option<&str>) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_book_content(&self, _id: i64) -> BookContent {
        BookContent::Text(String::new())
    }

    async fn get_book_preview(&self, _id: i64, _chapters: Option<i64>, _resource_base_url: Option<&str>) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    /// PDF 페이지 수를 받는 확장. 기존 facade 대역은 챕터 기반 메서드를 계속 쓸 수 있다.
    async fn get_book_preview_with_pages(&self, id: i64, _pages: Option<i64>, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse {
        self.get_book_preview(id, chapters, resource_base_url).await
    }

    async fn get_pdf_pages(&self, _id: i64, _start: i64, _end: i64) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn delete_pdf_pages(&self, _id: i64, _pages: &[i64], _crops: Option<&[Value]>, _splits: Option<&[Value]>, _inserts: Option<&[Value]>, _order: Option<&[i64]>) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn preview_insert_image(&self, _url: &str) -> Result<Vec<u8>, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn preview_insert_upload(&self, _data: &[u8]) -> Result<Vec<u8>, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_html_resource(&self, _book_id: i64, _resource_path: &str) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn get_cover(&self, _book_id: i64, _width: Option<i64>) -> BookResponse {
        BookResponse { status_code: 501, ..BookResponse::default() }
    }

    async fn validate_epub(&self, _book_id: i64) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn validate_pdf(&self, _book_id: i64) -> Result<Value, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_books_in_category(&self, _category: &str) -> Result<Vec<Book>, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_latest_books(&self, _size: i64, _exclude_categories: Option<&[String]>) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn search_by_keyword_paged(&self, _keyword: &str, _size: i64, _offset: i64, _excluded: Option<&[String]>, _category: Option<&str>) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn search_similar_books_paged(&self, _book_id: i64, _size: i64, _offset: i64) -> Result<BookPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_cover_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    async fn get_uncovered_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _exclude_categories: Option<&[String]>) -> Result<CoverPage, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    fn get_category_mismatches(&self, _force_refresh: bool) -> Result<MismatchSummary, String> {
        Err(UNIMPLEMENTED.to_string())
    }
    fn get_category_mismatch_details(&self, _category: &str) -> Result<MismatchDetails, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 표지를 URL 또는 올린 이미지(base64)로 넣거나 바꾼다.
    async fn set_epub_cover(&self, _book_id: i64, _url: Option<&str>, _image_data: Option<&str>) -> Result<CoverSave, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 쪽 이미지의 크롭·분할·삭제·삽입을 한 번에 저장한다. 항목은 라우트가 검증한 JSON 객체다.
    async fn edit_epub_images(&self, _book_id: i64, _crops: &[Value], _splits: &[Value], _deletes: &[Value], _inserts: &[Value]) -> Result<CoverSave, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// EPUB 표지를 뺀다.
    async fn remove_epub_cover(&self, _book_id: i64) -> Result<CoverSave, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    /// `before_idref` 페이지 앞에 삽화를 넣는다.
    async fn insert_epub_illustration(&self, _book_id: i64, _before_idref: &str, _url: Option<&str>, _image_data: Option<&str>) -> Result<CoverSave, String> {
        Err(UNIMPLEMENTED.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_book(&self, _id: i64, _category: &str, _title: &str, _author: &str, _path: &std::path::Path, _file_type: &str, _force: bool) -> Result<(), String> {
        Err(UNIMPLEMENTED.to_string())
    }
}

/// 설정이 없을 때 쓰는 관리자. 모든 메서드가 기본 구현(미구현 오류)이다.
pub struct UnavailableBookManager;

impl BookManagerApi for UnavailableBookManager {}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn unavailable_book_manager_returns_documented_defaults() {
        let manager = UnavailableBookManager;
        let allowed = HashSet::new();
        let items = [];
        let request = ApplyCategoryChanges { items: &items, allowed_file_paths: &allowed, content_type: "book", clean_existing: false, on_progress: None, on_item_done: None, should_continue: None };

        assert_eq!(
            manager
                .propose_category_changes(ProposeCategoryChanges {
                    category: "fiction",
                    mappings: &json!({}),
                    content_type: "book",
                    use_bookstore: false,
                    use_content_meta: false,
                    delay: 0.0,
                    on_progress: None,
                })
                .await
                .unwrap_err(),
            UNIMPLEMENTED
        );
        assert_eq!(manager.apply_category_changes(&items, &allowed, false).await.unwrap_err(), UNIMPLEMENTED);
        assert_eq!(manager.apply_category_changes_with(request).await.unwrap_err(), UNIMPLEMENTED);
        assert!(manager.delete_book(1).await.is_err());
        assert!(manager.index_single_file("a.epub").await.is_err());
        assert!(manager.delete_file("a.epub").await.is_err());
        assert!(manager.reload_category("fiction", "book").await.is_err());
        assert!(manager.reload_category_mismatch_files("fiction", "book", None).await.is_err());
        assert!(manager.reload_category_mismatches("book", None).await.is_err());
        assert!(manager.rename_category("old", "new").await.is_err());
        assert!(manager.delete_category("fiction", false).await.is_err());
        assert!(manager.preview_delete_category("fiction").await.is_err());
        assert!(manager.path_prefix().as_os_str().is_empty());
        assert!(matches!(manager.es_manager().create_index(), Err(crate::es_manager::EsError::Unsupported)));
        assert!(manager.get_book(1).await.is_err());
        assert!(manager.get_categories().await.is_err());
        assert!(manager.get_books_in_category_paged("fiction", None, None).await.is_err());
        assert_eq!(manager.get_book_content(1).await, BookContent::Text(String::new()));
        assert_eq!(manager.get_book_preview(1, None, None).await.status_code, 501);
        assert_eq!(manager.get_book_preview_with_pages(1, Some(2), None, None).await.status_code, 501);
        assert_eq!(manager.get_pdf_pages(1, 0, 1).await.status_code, 501);
        assert!(manager.delete_pdf_pages(1, &[], None, None, None, None).await.is_err());
        assert!(manager.preview_insert_image("https://example.test/a.png").await.is_err());
        assert!(manager.preview_insert_upload(&[]).await.is_err());
        assert_eq!(manager.get_html_resource(1, "a.html").await.status_code, 501);
        assert_eq!(manager.get_cover(1, None).await.status_code, 501);
        assert!(manager.validate_epub(1).await.is_err());
        assert!(manager.validate_pdf(1).await.is_err());
        assert!(manager.get_books_in_category("fiction").await.is_err());
        assert!(manager.get_latest_books(10, None).await.is_err());
        assert!(manager.search_by_keyword_paged("query", 10, 0, None, None).await.is_err());
        assert!(manager.search_similar_books_paged(1, 10, 0).await.is_err());
        assert!(manager.get_cover_books_paged("fiction", 10, "", None).await.is_err());
        assert!(manager.get_uncovered_books_paged("fiction", 10, "", None).await.is_err());
        assert!(manager.get_category_mismatches(false).is_err());
        assert!(manager.get_category_mismatch_details("fiction").is_err());
        assert!(manager.set_epub_cover(1, None, None).await.is_err());
        assert!(manager.edit_epub_images(1, &[], &[], &[], &[]).await.is_err());
        assert!(manager.remove_epub_cover(1).await.is_err());
        assert!(manager.insert_epub_illustration(1, "page-1", None, None).await.is_err());
        assert!(manager.update_book(1, "fiction", "title", "author", std::path::Path::new("a.epub"), "epub", false).await.is_err());
    }
}
