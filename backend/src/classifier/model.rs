//! 모델 판정 결과. TODO(classifier): 분류기 구현 에이전트가 확장한다.

/// 분류 모델의 판정 한 건. 설계: docs/rust-test-seams.md 3.12 `Prediction::new(category, confidence, top, reason)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Prediction {
    /// 판정한 카테고리. 확신도가 모자라 판정하지 않았으면 `None`.
    pub category: Option<String>,
    pub confidence: f64,
    /// 점수 내림차순 후보 `(카테고리, 점수)`.
    pub ranked: Vec<(String, f64)>,
    pub reason: String,
}

impl Prediction {
    pub fn new(category: Option<&str>, confidence: f64, top: Vec<(String, f64)>, reason: &str) -> Self {
        Self { category: category.map(str::to_string), confidence, ranked: top, reason: reason.to_string() }
    }
}
