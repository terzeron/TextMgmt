//! 책 표지 도메인. backend의 `api_router`와 `book_manager`가 쓴다.
//!
//! - `cover_management`: 서점 표지 후보 검색·캐시·저장과 그 라우터 계약.
//! - `cover_sources`: 서점별 표지 후보 추출.
//! - `epub_cover`: EPUB 표지 교체, 본문 이미지 편집, 표지 멤버 찾기.
//! - `cover_status`: PDF·EPUB 표지 유무 판정과 그 결과 저장(MySQL).
//! - `pdf_cover`: PDF 표지 렌더링과 거의 빈 표지 판정. EPUB 표지 썸네일은 `epub_cover`에 있다.

pub mod cover_management;
pub mod cover_sources;
pub mod cover_status;
pub mod epub_cover;
pub mod pdf_cover;
