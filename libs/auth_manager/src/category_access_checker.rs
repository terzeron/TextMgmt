//! viewer의 카테고리 접근 정책: 숨긴 카테고리와 그 하위의 책은 viewer가 볼 수 없다. admin은 제한을 받지 않는다.
//!
//! 모든 viewer 접근 검사(책 하나 열기, 목록·카테고리 걸러내기)가 이 모듈의 판정을 쓴다.
//! 책을 찾는 일은 [`BookLookup`]으로 받는다. 이 crate는 책 관리자를 알지 못한다.

use std::sync::Arc;

use async_trait::async_trait;
use common::UNIMPLEMENTED;
use db::category_mapping::CategoryMappingApi;

use crate::auth_service::{Claims, HttpStatusError};

/// 숨긴 카테고리의 책을 viewer가 열려고 할 때의 403 문구.
pub const FORBIDDEN_CATEGORY: &str = "접근 권한이 없는 카테고리입니다.";

/// 카테고리가 숨긴 카테고리이거나 그 하위면 참이다.
pub fn is_hidden_category(category: &str, hidden: &[String]) -> bool {
    !category.is_empty() && hidden.iter().any(|h| category == h || category.strip_prefix(h.as_str()).is_some_and(|rest| rest.starts_with('/')))
}

/// 숨긴 카테고리면 403 오류다.
pub fn ensure_category_visible(category: &str, hidden: &[String]) -> Result<(), HttpStatusError> {
    if is_hidden_category(category, hidden) { Err(HttpStatusError::new(403, FORBIDDEN_CATEGORY)) } else { Ok(()) }
}

/// `BookAccess`가 돌려주는 책 요약.
#[derive(Debug, Clone, PartialEq)]
pub struct BookStub {
    pub title: String,
    pub category: Option<String>,
}

/// 책을 찾고, 없으면 404, viewer가 볼 수 없는 카테고리면 403 오류를 돌려준다.
#[async_trait]
pub trait BookAccess: Send + Sync {
    async fn get_book_and_ensure_viewer_access(&self, _book_id: i64, _payload: &Claims, _content_type: &str) -> Result<BookStub, HttpStatusError> {
        Err(HttpStatusError::new(500, UNIMPLEMENTED))
    }
}

/// 설정이 없을 때 쓰는 접근 확인. 기본 구현(미구현 오류)이다.
pub struct UnavailableBookAccess;

impl BookAccess for UnavailableBookAccess {}

/// 책 하나의 제목·카테고리를 찾는다. 찾지 못한 사유는 HTTP 상태로 돌려준다(관리자를 못 만들면 503, 책이 없으면 404).
#[async_trait]
pub trait BookLookup: Send + Sync {
    async fn find_book(&self, book_id: i64, content_type: &str) -> Result<BookStub, HttpStatusError>;
}

/// 카테고리 매핑 저장소를 쓸 때 얻는다. 아직 만들 수 없으면 사유 문자열이다.
pub type CategoryMappingSource = Arc<dyn Fn() -> Result<Arc<dyn CategoryMappingApi>, String> + Send + Sync>;

/// 운영용 접근 확인. 책을 찾은 뒤 viewer면 숨긴 카테고리를 읽어 정책을 적용한다. admin은 숨긴 카테고리를 읽지 않는다.
pub struct ViewerBookAccess {
    books: Arc<dyn BookLookup>,
    categories: CategoryMappingSource,
}

impl ViewerBookAccess {
    pub fn new(books: Arc<dyn BookLookup>, categories: CategoryMappingSource) -> Self {
        Self { books, categories }
    }
}

#[async_trait]
impl BookAccess for ViewerBookAccess {
    async fn get_book_and_ensure_viewer_access(&self, book_id: i64, payload: &Claims, content_type: &str) -> Result<BookStub, HttpStatusError> {
        let book = self.books.find_book(book_id, content_type).await?;
        if payload.is_viewer() {
            let categories = (self.categories)().map_err(|error| HttpStatusError::new(503, &error))?;
            let hidden = categories.get_hidden_categories(content_type).map_err(|error| HttpStatusError::new(500, &error.to_string()))?;
            ensure_category_visible(book.category.as_deref().unwrap_or(""), &hidden)?;
        }
        Ok(book)
    }
}
