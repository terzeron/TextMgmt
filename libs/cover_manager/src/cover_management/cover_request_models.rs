//! 표지 API 요청 본문 모델과 검증.

use super::*;

/// 요청 본문 검증 오류. `Display`는 오류 종류(`string_too_long` 등)를 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub field: &'static str,
    pub kind: &'static str,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.field, self.message, self.kind)
    }
}

impl std::error::Error for ValidationError {}

fn check_length(field: &'static str, value: &str, min: usize, max: usize) -> Result<(), ValidationError> {
    let count = value.chars().count();
    if count < min {
        return Err(ValidationError { field, kind: "string_too_short", message: format!("String should have at least {min} characters") });
    }
    if count > max {
        return Err(ValidationError { field, kind: "string_too_long", message: format!("String should have at most {max} characters") });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawBookCover")]
pub struct BookCoverModel {
    pub file_path: String,
    pub revision: String,
}

#[derive(serde::Deserialize)]
struct RawBookCover {
    file_path: String,
    revision: String,
}

impl BookCoverModel {
    pub fn new(file_path: &str, revision: &str) -> Result<Self, ValidationError> {
        check_length("file_path", file_path, 1, 4096)?;
        check_length("revision", revision, 1, 200)?;
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string() })
    }
}

impl TryFrom<RawBookCover> for BookCoverModel {
    type Error = ValidationError;

    fn try_from(raw: RawBookCover) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawApplyCover")]
pub struct ApplyCoverModel {
    pub file_path: String,
    pub revision: String,
    pub candidate_id: String,
}

#[derive(serde::Deserialize)]
struct RawApplyCover {
    file_path: String,
    revision: String,
    candidate_id: String,
}

impl ApplyCoverModel {
    pub fn new(file_path: &str, revision: &str, candidate_id: &str) -> Result<Self, ValidationError> {
        BookCoverModel::new(file_path, revision)?;
        let valid = candidate_id.len() == 64 && candidate_id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(ValidationError { field: "candidate_id", kind: "string_pattern_mismatch", message: "String should match pattern '^[a-f0-9]{64}$'".to_string() });
        }
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string(), candidate_id: candidate_id.to_string() })
    }
}

impl TryFrom<RawApplyCover> for ApplyCoverModel {
    type Error = ValidationError;

    fn try_from(raw: RawApplyCover) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision, &raw.candidate_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawApplyUrl")]
pub struct ApplyUrlModel {
    pub file_path: String,
    pub revision: String,
    pub url: String,
    pub replace_existing: bool,
}

#[derive(serde::Deserialize)]
struct RawApplyUrl {
    file_path: String,
    revision: String,
    url: String,
    #[serde(default)]
    replace_existing: bool,
}

impl ApplyUrlModel {
    pub fn new(file_path: &str, revision: &str, url: &str, replace_existing: bool) -> Result<Self, ValidationError> {
        BookCoverModel::new(file_path, revision)?;
        check_length("url", url, 1, 4096)?;
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string(), url: url.to_string(), replace_existing })
    }
}

impl TryFrom<RawApplyUrl> for ApplyUrlModel {
    type Error = ValidationError;

    fn try_from(raw: RawApplyUrl) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision, &raw.url, raw.replace_existing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "RawApplyImage")]
pub struct ApplyImageModel {
    pub file_path: String,
    pub revision: String,
    pub image_data: String,
}

#[derive(serde::Deserialize)]
struct RawApplyImage {
    file_path: String,
    revision: String,
    image_data: String,
}

impl ApplyImageModel {
    /// 디코딩하기 전에 base64 문자열 길이를 확인한다(15MB 이미지의 base64 길이가 상한이다).
    pub fn new(file_path: &str, revision: &str, image_data: &str) -> Result<Self, ValidationError> {
        BookCoverModel::new(file_path, revision)?;
        check_length("image_data", image_data, 1, MAX_IMAGE_BYTES.div_ceil(3) * 4)?;
        Ok(Self { file_path: file_path.to_string(), revision: revision.to_string(), image_data: image_data.to_string() })
    }
}

impl TryFrom<RawApplyImage> for ApplyImageModel {
    type Error = ValidationError;

    fn try_from(raw: RawApplyImage) -> Result<Self, Self::Error> {
        Self::new(&raw.file_path, &raw.revision, &raw.image_data)
    }
}
