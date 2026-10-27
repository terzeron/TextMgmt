//! 코퍼스 디렉토리 레이아웃 상수.
//!
//! 색인(loader)과 불일치 검사가 같은 목록을 써야 해서 아무것도 의존하지 않는 이 crate에 둔다.
//! 원본: `utils/corpus_layout.py`.

/// 책이 아니라 OCR·EPUB 빌드 파이프라인의 중간 산출물이 쌓이는 디렉토리.
/// 한쪽에서만 빼면 이미 색인된 문서가 전부 "파일 없는 고아"로 잡히고, 재적재가 디스크에
/// 멀쩡히 있는 파일의 문서를 지운다.
pub const IGNORED_DIR_NAMES: [&str; 5] = ["page_images", "tesseract_text", "source_chapters", "OEBPS", "META-INF"];

/// 경로 한 구간이 색인 대상에서 빠지는지 판정한다. 숨김(`.` 시작)이거나 파이프라인 디렉토리면 참.
pub fn is_ignored_segment(segment: &str) -> bool {
    segment.starts_with('.') || IGNORED_DIR_NAMES.contains(&segment)
}
