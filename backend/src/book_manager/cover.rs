//! 표지 관련 보조. EPUB 표지 멤버 찾기와 썸네일은 `epub_cover`가 구현하고 여기서는 `BookManager`의 이름으로 이어 준다.
//! Python `_find_epub_cover_member`, `_extract_cover_thumbnail`(EPUB 부분).

use std::io::{Read, Seek};
use std::path::Path;

use zip::ZipArchive;

use super::BookManager;
use crate::epub_cover;

/// 표지 썸네일 추출 능력. 구 `ThumbnailExtractor`.
pub trait Thumbnailer: Send + Sync {
    fn extract_cover_thumbnail(&self, path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>>;
}

impl BookManager {
    /// EPUB 표지 이미지 멤버 경로. 명시한 표지가 없으면 guide·manifest의 표지 XHTML에서 이미지를 찾는다.
    pub fn find_epub_cover_member<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Option<String> {
        epub_cover::find_epub_cover_member(archive)
    }

    /// `file_size`가 값을 돌려주면 ZIP 헤더 크기 대신 쓴다(Python이 `ZipInfo.file_size`를 덮어쓰는 것에 대응).
    pub fn find_epub_cover_member_with<R: Read + Seek>(archive: &mut ZipArchive<R>, file_size: &dyn Fn(&str) -> Option<u64>) -> Option<String> {
        epub_cover::find_epub_cover_member_with(archive, file_size)
    }

    /// EPUB 표지를 JPEG 썸네일로 만든다. PDF는 아직 연결하지 않았다.
    pub fn extract_cover_thumbnail(path: &Path, file_type: &str, width: Option<u32>) -> Option<Vec<u8>> {
        if file_type == "epub" { epub_cover::extract_epub_thumbnail(path, width) } else { None }
    }
}
