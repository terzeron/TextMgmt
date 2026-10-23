//! 분류기 표면. `BookManager`가 쓰는 최소 trait와 타입만 있다.
//!
//! TODO(classifier): 분류기 구현 에이전트가 확장한다. 모델 포맷(`CategoryModelApi`, `BookCategoryClassifier`,
//! `BookClassifierApi`, `BookClassifierService`)은 아직 정해지지 않아 여기서 구현하지 않는다.
//! 설계: docs/rust-test-seams.md 3.12 "분류기 3계층".

use std::path::Path;

use serde_json::Value;

pub mod model;

use model::Prediction;

/// 경로나 문서 하나를 분류한다. 구 `Classifier`, `Classify`.
pub trait Classify: Send + Sync {
    /// 모델을 읽어 판정할 수 있는가. 구 `is_loaded`.
    fn is_available(&self) -> bool;
    fn classify_path(&self, path: &Path, min_confidence: Option<f64>) -> Result<Prediction, String>;
    fn classify_document(&self, doc: &Value, min_confidence: Option<f64>) -> Result<Prediction, String>;
}
