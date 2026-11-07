//! `auth_manager::category_access_checker` 단위 테스트.

use std::sync::Arc;

use async_trait::async_trait;
use auth_manager::auth_service::{Claims, HttpStatusError};
use auth_manager::category_access_checker::{BookAccess, BookLookup, BookStub, FORBIDDEN_CATEGORY, UnavailableBookAccess, ViewerBookAccess, ensure_category_visible, is_hidden_category};
use db::category_mapping::{CategoryMappingApi, MappingError};

#[test]
fn is_hidden_matches_the_category_and_its_descendants_only() {
    let hidden = vec!["A".to_string()];
    assert!(is_hidden_category("A", &hidden));
    assert!(is_hidden_category("A/b/c", &hidden));
    assert!(!is_hidden_category("AB", &hidden), "이름이 같은 접두어인 형제 카테고리는 숨기지 않는다");
    assert!(!is_hidden_category("", &hidden));
    assert!(!is_hidden_category("A", &[]));
}

#[test]
fn hidden_category_is_forbidden_with_403() {
    let hidden = vec!["A".to_string()];
    let error = ensure_category_visible("A/b", &hidden).unwrap_err();
    assert_eq!((error.status_code, error.detail.as_str()), (403, FORBIDDEN_CATEGORY));
    assert!(ensure_category_visible("B", &hidden).is_ok());
}

struct EmptyBookAccess;

impl BookAccess for EmptyBookAccess {}

#[tokio::test]
async fn unavailable_book_access_fails_closed() {
    assert!(EmptyBookAccess.get_book_and_ensure_viewer_access(1, &Claims::default(), "book").await.is_err());
    assert!(UnavailableBookAccess.get_book_and_ensure_viewer_access(1, &Claims::default(), "book").await.is_err());
}

struct FixedLookup(BookStub);

#[async_trait]
impl BookLookup for FixedLookup {
    async fn find_book(&self, _book_id: i64, _content_type: &str) -> Result<BookStub, HttpStatusError> {
        Ok(self.0.clone())
    }
}

struct FixedHidden(Vec<String>);

impl CategoryMappingApi for FixedHidden {
    fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        Ok(self.0.clone())
    }
}

fn access(category: &str, hidden: &[&str], mapping_available: bool) -> ViewerBookAccess {
    let book = BookStub { title: "title".into(), category: Some(category.into()) };
    let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FixedHidden(hidden.iter().map(|h| h.to_string()).collect()));
    ViewerBookAccess::new(Arc::new(FixedLookup(book)), Arc::new(move || if mapping_available { Ok(mapping.clone()) } else { Err("mapping offline".into()) }))
}

#[tokio::test]
async fn viewer_access_denies_hidden_category_and_descendants() {
    let result = access("소설/판타지/무협", &["소설/판타지"], true).get_book_and_ensure_viewer_access(1, &Claims::with_role("viewer"), "book").await;
    assert_eq!(result.unwrap_err().status_code, 403);
}

#[tokio::test]
async fn admin_access_does_not_load_hidden_categories() {
    let book = access("소설/판타지", &["소설"], false).get_book_and_ensure_viewer_access(1, &Claims::with_role("admin"), "book").await.unwrap();
    assert_eq!(book, BookStub { title: "title".into(), category: Some("소설/판타지".into()) });
}

#[tokio::test]
async fn viewer_access_reports_unavailable_mapping_as_503() {
    let result = access("소설", &[], false).get_book_and_ensure_viewer_access(1, &Claims::with_role("viewer"), "book").await;
    assert_eq!(result.unwrap_err().status_code, 503);
}

struct FailingHidden;

impl CategoryMappingApi for FailingHidden {
    fn get_hidden_categories(&self, _content_type: &str) -> Result<Vec<String>, MappingError> {
        Err(MappingError::Other("query failed".into()))
    }
}

#[tokio::test]
async fn viewer_access_reports_a_hidden_category_query_failure_as_500() {
    let book = BookStub { title: "title".into(), category: Some("소설".into()) };
    let mapping: Arc<dyn CategoryMappingApi> = Arc::new(FailingHidden);
    let access = ViewerBookAccess::new(Arc::new(FixedLookup(book)), Arc::new(move || Ok(mapping.clone())));
    let error = access.get_book_and_ensure_viewer_access(1, &Claims::with_role("viewer"), "book").await.unwrap_err();
    assert_eq!((error.status_code, error.detail.as_str()), (500, "query failed"));
}
