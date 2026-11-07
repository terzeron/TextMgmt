//! 도서 분류기. 특징 해시, softmax 모델, 모델 파일 핫 리로드를 갖는다. ES를 모른다.
//!
//! backend는 `BookManagerDeps::with_classifier_factory`로 [`Classify`] 구현을 꽂는다.
//! 꽂지 않거나 모델 파일이 없으면 분류 제안은 ISBN·키워드 근거만으로 점수를 낸다.

pub mod feature_hasher;
pub mod model_reloader;
pub mod softmax_model;

use serde_json::Value;

/// 분류 모델의 판정 한 건.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// 확률 내림차순 `(카테고리, 확률)`. 확률 합은 1이다.
    pub ranked: Vec<(String, f64)>,
}

/// ES 문서(`_source`) 하나를 분류한다.
pub trait Classify: Send + Sync {
    fn classify_document(&self, doc: &Value) -> Result<Prediction, String>;
}
