//! 분류기 껍데기 테스트: `BookClassifierApi` 기본값과 `ModelOnlyClassifier` 어댑터.

use std::path::Path;
use std::sync::Arc;

use classifier::{BookClassifierApi, Classify, ModelOnlyClassifier, Prediction};
use serde_json::{Value, json};

struct Fixed {
    available: bool,
    result: Result<Prediction, String>,
}

impl Classify for Fixed {
    fn is_available(&self) -> bool {
        self.available
    }
    fn classify_path(&self, _path: &Path, _min_confidence: Option<f64>) -> Result<Prediction, String> {
        self.result.clone()
    }
    fn classify_document(&self, _doc: &Value, _min_confidence: Option<f64>) -> Result<Prediction, String> {
        self.result.clone()
    }
}

fn adapter(available: bool, result: Result<Prediction, String>) -> ModelOnlyClassifier {
    ModelOnlyClassifier { model: Arc::new(Fixed { available, result }) }
}

struct Nothing;
impl BookClassifierApi for Nothing {}

#[test]
fn book_classifier_api_defaults_mean_no_evidence() {
    assert_eq!(Nothing.classify_file(Path::new("a.txt")), (None, "not_found".to_string(), String::new(), json!({})));
    assert_eq!(Nothing.override_below(), 0.0);
    assert!(!Nothing.prefers_bookstore(0.9));
    assert_eq!(Nothing.min_confidence(), None);
    assert_eq!(Nothing.expected_accuracy(Some(0.5)), None);
    assert!(Nothing.as_any().is::<()>(), "기본 as_any는 빈 값을 돌려준다");
}

#[test]
fn classify_defaults_have_no_threshold_and_no_calibration() {
    let model = Fixed { available: true, result: Err("미사용".to_string()) };
    assert_eq!(model.min_confidence(), None);
    assert_eq!(model.expected_accuracy(0.9), None);
}

#[test]
fn model_only_uses_the_model_category_and_candidates() {
    let ranked = vec![("3_판타지".to_string(), 0.8), ("1_소설".to_string(), 0.1)];
    let clf = adapter(true, Ok(Prediction::new(Some("3_판타지"), 0.8, ranked.clone(), "확신")));
    let (target, method, reason, entry) = clf.classify_file(Path::new("a.txt"));
    assert_eq!(target.as_deref(), Some("3_판타지"));
    assert_eq!(method, "model");
    assert_eq!(reason, "확신");
    assert_eq!(entry, json!({"model_category": "3_판타지", "confidence": 0.8, "model_candidates": ranked}));
}

#[test]
fn model_only_without_a_decision_reports_the_top_candidate() {
    let clf = adapter(true, Ok(Prediction::new(None, 0.05, vec![("1_소설".to_string(), 0.05)], "낮음")));
    let (target, method, _, entry) = clf.classify_file(Path::new("a.txt"));
    assert_eq!(target, None);
    assert_eq!(method, "not_found");
    assert_eq!(entry["model_category"], "1_소설");
}

#[test]
fn model_only_without_a_model_or_on_error_finds_nothing() {
    let unavailable = adapter(false, Ok(Prediction::new(Some("x"), 1.0, vec![], "")));
    assert_eq!(unavailable.classify_file(Path::new("a.txt")), (None, "not_found".to_string(), String::new(), json!({})));
    let failing = adapter(true, Err("읽기 실패".to_string()));
    assert_eq!(failing.classify_file(Path::new("a.txt")), (None, "not_found".to_string(), "읽기 실패".to_string(), json!({})));
}
