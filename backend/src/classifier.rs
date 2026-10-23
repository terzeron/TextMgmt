//! 분류기 표면. `BookManager`가 쓰는 최소 trait와 타입만 있다.
//!
//! TODO(classifier): 분류기 구현 에이전트가 확장한다. 모델 포맷(`CategoryModelApi`, `BookCategoryClassifier`,
//! `BookClassifierApi`, `BookClassifierService`)은 아직 정해지지 않아 여기서 구현하지 않는다.
//! 설계: docs/rust-test-seams.md 3.12 "분류기 3계층".

use std::any::Any;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

pub mod model;

use model::Prediction;

/// 경로나 문서 하나를 분류한다. 구 `Classifier`, `Classify`.
pub trait Classify: Send + Sync {
    /// 모델을 읽어 판정할 수 있는가. 구 `is_loaded`.
    fn is_available(&self) -> bool;
    fn classify_path(&self, path: &Path, min_confidence: Option<f64>) -> Result<Prediction, String>;
    fn classify_document(&self, doc: &Value, min_confidence: Option<f64>) -> Result<Prediction, String>;
}

/// 분류기 서비스 facade. `BookManager`가 제안을 만들 때 쓰는 능력이다(구 `ClassifierService`, `FileClassifier`).
/// 설계: docs/rust-test-seams.md 3.12. 모든 메서드에 기본 구현이 있고 기본값은 "근거 없음"이다.
///
/// TODO(classifier): 서점 조회, 정책 경계(`override_below`), 보정 곡선을 가진 실제 서비스(`BookClassifierService`)는
/// 분류기 구현 에이전트가 만든다. 여기는 trait와, `Classify` 모델 하나만 감싼 `ModelOnlyClassifier`뿐이다.
pub trait BookClassifierApi: Send + Sync {
    /// 파일 하나를 분류한다. 반환: (목적지, 방법, 사유, 항목). 항목은 `model_category`, `confidence`,
    /// `bookstore_candidates`, `model_candidates`를 담을 수 있다. 방법은 `model`, `bookstore_majority`, `bookstore_single`, `conflict`, `not_found`다.
    fn classify_file(&self, _path: &Path) -> (Option<String>, String, String, Value) {
        (None, "not_found".to_string(), String::new(), json!({}))
    }

    /// 서점 정책의 경계. 0 이하면 정책이 없는 것이다.
    fn override_below(&self) -> f64 {
        0.0
    }

    fn prefers_bookstore(&self, _confidence: f64) -> bool {
        false
    }

    /// 모델이 "이 점수 미만이면 판정하지 않는다"고 정한 값. 모르면 `None`.
    fn min_confidence(&self) -> Option<f64> {
        None
    }

    /// 확신도를 예상 정답률로 옮긴다. 바깥 `None`은 보정 함수가 없다는 뜻이고, 안쪽 `None`은 값을 못 얻었다는 뜻이다.
    fn expected_accuracy(&self, _confidence: Option<f64>) -> Option<Option<f64>> {
        None
    }

    /// 판정 대체 훅이 분류기 본체를 보게 한다(`BookManagerDeps::with_is_high_confidence_override`).
    fn as_any(&self) -> &dyn Any {
        &()
    }
}

/// `Classify` 모델 하나를 서비스처럼 보이게 감싼다. 서점 득표와 정책 경계는 없다.
pub struct ModelOnlyClassifier {
    pub model: Arc<dyn Classify>,
}

impl BookClassifierApi for ModelOnlyClassifier {
    fn classify_file(&self, path: &Path) -> (Option<String>, String, String, Value) {
        if !self.model.is_available() {
            return (None, "not_found".to_string(), String::new(), json!({}));
        }
        match self.model.classify_path(path, None) {
            Ok(prediction) => {
                let method = if prediction.category.is_some() { "model" } else { "not_found" };
                let model_category = prediction.category.clone().or_else(|| prediction.ranked.first().map(|(c, _)| c.clone()));
                let entry = json!({"model_category": model_category, "confidence": prediction.confidence, "model_candidates": prediction.ranked});
                (prediction.category, method.to_string(), prediction.reason, entry)
            }
            Err(reason) => (None, "not_found".to_string(), reason, json!({})),
        }
    }

    fn as_any(&self) -> &dyn Any {
        &self.model
    }
}
