//! 쪽 이미지 편집(크롭·분할·삭제·삽입). 아직 구현 전이다.

use serde_json::Value;

use super::CoverError;

#[derive(Default, Debug, Clone)]
pub struct EditImages {
    pub crops: Vec<Value>,
    pub splits: Vec<Value>,
    pub deletes: Vec<Value>,
    pub inserts: Vec<(String, Vec<u8>)>,
    pub backup_name: Option<String>,
}

pub fn edit_images(_path: &std::path::Path, _revision: &str, _backup_dir: &std::path::Path, _args: EditImages) -> Result<Value, CoverError> {
    Err(CoverError::new("미구현"))
}

pub fn edit_images_with(_path: &std::path::Path, _revision: &str, _backup_dir: &std::path::Path, _args: EditImages, _hooks: &super::EpubCoverHooks) -> Result<Value, CoverError> {
    Err(CoverError::new("미구현"))
}

pub fn edit_page_images(_html: &[u8], _image_count: usize, _edits: &[(usize, &str, Option<Value>)]) -> Result<Vec<u8>, CoverError> {
    Err(CoverError::new("미구현"))
}
