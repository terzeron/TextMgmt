//! 모델 판정 결과와, 내보낸 파일에서 읽는 추론 모델. Python `backend/classifier/model.py` 의 `CategoryModel.predict` 대응.
//!
//! 학습은 Python 이 하고 이 모델은 추론만 한다. 문서 본문은 로그에 남기지 않는다.

use std::path::Path;

use serde_json::Value;

use super::export_format::{self, ModelError};
use super::softmax::{fixed, rank_desc, softmax_confidence};
use super::vectorize::{FieldVectorizer, field_text};

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

/// 기본 상위 후보 수. Python `predict(top_k=3)`.
pub const DEFAULT_TOP_K: usize = 3;

/// 내보낸 파일에서 읽은 추론 전용 모델.
#[derive(Debug)]
pub struct CategoryModel {
    classes: Vec<String>,
    fields: Vec<FieldVectorizer>,
    /// `[n_features, n_classes]` 행 우선.
    coef: Vec<f32>,
    intercept: Vec<f64>,
    config: Value,
    meta: Value,
}

impl CategoryModel {
    /// `backend.classifier.export` 가 쓴 디렉토리를 읽는다. sha256, 길이, 열 수 합계, 어휘 중복을 검증한다.
    pub fn load_dir(path: &Path) -> Result<Self, ModelError> {
        let loaded = export_format::load_dir(path)?;
        let m = loaded.manifest;
        Ok(Self { classes: m.classes, fields: loaded.fields, coef: loaded.coef, intercept: m.intercept, config: m.config, meta: m.meta })
    }

    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    pub fn n_features(&self) -> usize {
        self.fields.iter().map(FieldVectorizer::vocab_size).sum()
    }

    pub fn config(&self) -> &Value {
        &self.config
    }

    pub fn meta(&self) -> &Value {
        &self.meta
    }

    /// `config["decision"]["temperature"]`, 없으면 1.0.
    pub fn temperature(&self) -> f64 {
        self.config.pointer("/decision/temperature").and_then(Value::as_f64).unwrap_or(1.0)
    }

    /// 판정 임계값. 설정에 숫자가 있으면 그것, 없으면 `meta.calibrated_threshold`, 그것도 없으면 1.0.
    pub fn min_confidence(&self) -> f64 {
        self.config
            .pointer("/decision/min_confidence")
            .and_then(Value::as_f64)
            .or_else(|| self.meta.get("calibrated_threshold").and_then(Value::as_f64))
            .unwrap_or(1.0)
    }

    /// 문서 하나의 희소 특징 벡터 `(전체 열, 값)`. 필드 순서대로 열 offset 이 더해져 오름차순이다.
    pub fn features(&self, doc: &Value) -> Vec<(u32, f32)> {
        let mut x = Vec::new();
        for field in &self.fields {
            field.transform_into(&field_text(doc, &field.source, field.max_chars), &mut x);
        }
        x
    }

    /// 결정값 `X · coef + intercept`. 클래스마다 하나.
    pub fn scores(&self, doc: &Value) -> Vec<f64> {
        let n = self.classes.len();
        let mut d = self.intercept.clone();
        for (col, value) in self.features(doc) {
            let row = &self.coef[col as usize * n..(col as usize + 1) * n];
            for (acc, &w) in d.iter_mut().zip(row) {
                *acc += f64::from(value) * f64::from(w);
            }
        }
        d
    }

    /// 문서마다 판정한다. `min_confidence` 가 `None` 이면 모델의 임계값을 쓴다. `top_k` 는 1 이상으로 본다.
    pub fn predict(&self, docs: &[Value], min_confidence: Option<f64>, top_k: usize) -> Vec<Prediction> {
        let floor = min_confidence.unwrap_or_else(|| self.min_confidence());
        let temperature = self.temperature();
        docs.iter().map(|doc| self.predict_scores(&self.scores(doc), floor, temperature, top_k.max(1))).collect()
    }

    pub fn predict_one(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction {
        self.predict(std::slice::from_ref(doc), min_confidence, DEFAULT_TOP_K).remove(0)
    }

    fn predict_scores(&self, scores: &[f64], floor: f64, temperature: f64, top_k: usize) -> Prediction {
        let prob = softmax_confidence(scores, temperature);
        let ranked: Vec<(String, f64)> = rank_desc(scores).into_iter().take(top_k).map(|j| (self.classes[j].clone(), prob[j])).collect();
        let (best_cat, best_p) = ranked[0].clone();
        if best_p < floor {
            let reason = format!("확신도 {} < 임계값 {} 이라 판정하지 않음 (1위 후보 {best_cat})", fixed(best_p, 3), fixed(floor, 2));
            return Prediction { category: None, confidence: best_p, ranked, reason };
        }
        let reason = match ranked.get(1) {
            Some((runner, p2)) => format!("모델 확신도 {} -> {best_cat} (2위 {runner} {})", fixed(best_p, 3), fixed(*p2, 3)),
            None => format!("모델 확신도 {} -> {best_cat}", fixed(best_p, 3)),
        };
        Prediction { category: Some(best_cat), confidence: best_p, ranked, reason }
    }
}
