//! 앱이 주입받는 능력(trait)과 실제·기본 구현. 설계: docs/rust-test-seams.md 3.0, 3.2.
//!
//! 규칙: 모든 trait는 `Send + Sync`이고 모든 메서드에 기본 구현이 있다. 기본값은 `Err(..unimplemented)`/빈 값이다.
//! 테스트 대역은 쓰는 메서드만 구현한다. facade 이름은 `XxxApi`, 좁은 능력은 역할 명사다.
//!
//! 이 파일에는 라우트가 쓰는 viewer 접근 확인(`BookAccess`)만 있다. 도메인 facade는 각 crate에 있다:
//! `BookManagerApi`는 `book_manager`, `CategoryMappingApi`·`ViewHistoryApi`는 `db`, `GoogleVerifier`는 `auth_manager`.

use async_trait::async_trait;

use auth_manager::auth_service::{Claims, HttpStatusError};

pub(crate) use book_manager::UNIMPLEMENTED;

// ---------------------------------------------------------------------------
// viewer 접근 확인
// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyBookAccess;

    impl BookAccess for EmptyBookAccess {}

    #[tokio::test]
    async fn unavailable_book_access_fails_closed() {
        assert!(EmptyBookAccess.get_book_and_ensure_viewer_access(1, &Claims::default(), "book").await.is_err());
        assert!(UnavailableBookAccess.get_book_and_ensure_viewer_access(1, &Claims::default(), "book").await.is_err());
    }
}
