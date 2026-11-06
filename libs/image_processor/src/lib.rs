//! 이미지 공용 처리. 표지 관리(`cover_manager`)와 PDF 쪽 편집(`book_manager::pdf_edit`)이 함께 쓴다.
//!
//! - `image_cropper`: 비율 좌표와 기울기로 회전 크롭하는 기하.
//! - `image_fetcher`: 공개 이미지 URL을 DNS로 검증한 IP에만 연결해 내려받는다.
//! - 이미지 한 장의 크기·픽셀 상한(`MAX_IMAGE_BYTES`, `MAX_IMAGE_PIXELS`).

pub mod image_cropper;
pub mod image_fetcher;

/// 이미지 한 장의 최대 바이트. 업로드·URL 받기·EPUB 멤버 읽기가 같은 값을 쓴다.
pub const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024;
/// 디코딩할 이미지의 최대 픽셀 수(압축 폭탄 방지).
pub const MAX_IMAGE_PIXELS: usize = 80_000_000;
