//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

mod common;

use std::fs;

use classifier::export_format::{ModelError, write_dir};
use classifier::vectorizer::FieldVectorizer;
use serde_json::{Value, json};
use crate::common::shared_fixtures::{spec};

#[test]
fn exporter_rejects_models_without_vectorizers_like_python() {
    let dir = tempfile::tempdir().unwrap();
    let result = write_dir(&dir.path().join("export"), &[], &[], &[], &[], &json!({}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("벡터라이저")));
}

#[test]
fn exporter_rejects_coefficient_shape_and_unsafe_field_names() {
    let classes = vec!["A".to_string()];
    let dir = tempfile::tempdir().unwrap();
    let field = FieldVectorizer::new("title", "title", 1.0, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let result = write_dir(&dir.path().join("bad-shape"), &classes, &[field], &[], &[0.0], &json!({}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("계수 개수")));

    let field = FieldVectorizer::new("title-name", "title", 1.0, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let result = write_dir(&dir.path().join("bad-name"), &classes, &[field], &[1.0], &[0.0], &json!({}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("파일 이름")));
}

#[test]
fn model_errors_render_all_variants_and_expose_only_io_sources() {
    let errors = [
        ModelError::Json { file: "model.json".into(), message: "bad json".into() },
        ModelError::UnsupportedVersion(2),
        ModelError::UnsafePath("../secret".into()),
        ModelError::LengthMismatch { file: "coef.f32".into(), expected: 8, actual: 4 },
        ModelError::Sha256Mismatch { file: "coef.f32".into() },
        ModelError::NotFinite { file: "coef.f32".into() },
        ModelError::DuplicateTerm { field: "title".into() },
    ];
    for error in errors {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }

    let io_error = ModelError::Io { file: "model.json".into(), source: std::io::Error::other("disk error") };
    assert!(io_error.to_string().contains("disk error"));
    assert!(std::error::Error::source(&io_error).is_some());
    assert!(ModelError::Invalid("bad shape".into()).to_string().contains("bad shape"));
}

#[test]
fn exporter_defaults_a_missing_field_weight_and_rejects_non_numeric_weight() {
    let classes = vec!["A".to_string()];
    let field = FieldVectorizer::new("title", "title", 0.25, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("export-default-weight");
    write_dir(&output, &classes, &[field], &[1.0], &[0.0], &json!({"fields":{"title":{}}}), &json!({})).unwrap();
    let manifest: Value = serde_json::from_slice(&fs::read(output.join("model.json")).unwrap()).unwrap();
    assert_eq!(manifest["fields"][0]["weight"], json!(1.0));

    let output = dir.path().join("export-invalid-weight");
    let field = FieldVectorizer::new("title", "title", 0.25, 0, spec("word", 1, 1), true, 0, vec!["a".into()], vec![1.0]).unwrap();
    let result = write_dir(&output, &classes, &[field], &[1.0], &[0.0], &json!({"fields":{"title":{"weight":"heavy"}}}), &json!({}));
    assert!(matches!(result, Err(ModelError::Invalid(message)) if message.contains("weight")));
}
