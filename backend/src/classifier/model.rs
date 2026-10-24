//! 모델 판정 결과와, 내보낸 파일에서 읽는 추론 모델. Python `backend/classifier/model.py` 대응.
//!
//! 학습은 Python 이 하고 이 모델은 추론만 한다. 문서 본문은 로그에 남기지 않는다.
//!
//! 모델 파일은 pickle(joblib)이 아니라 `backend.classifier.export` 가 쓴 디렉토리다. 환경변수 `TM_CLASSIFIER_MODEL` 은
//! Rust 에서 그 디렉토리를 가리킨다.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use super::calibration;
use super::export_format::{self, ModelError};
use super::softmax::{fixed, rank_desc};
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

/// 모델 경로를 담는 환경변수. `books-pv` 가 호스트의 /mnt/data/text 를 pod 의 /books 로 마운트하므로 같은 파일을 본다.
pub const MODEL_ENV: &str = "TM_CLASSIFIER_MODEL";
/// 환경변수가 없을 때의 기본 경로. Python 의 `backend/classifier/model.joblib` 자리다.
pub const DEFAULT_MODEL_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/classifier/model.joblib");

/// 설정된 모델 경로.
pub fn model_path() -> PathBuf {
    match std::env::var(MODEL_ENV) {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(DEFAULT_MODEL_PATH),
    }
}

/// 결정값을 확률로 바꾼다(문서마다 한 행). 온도가 크면 평탄해져 확신도가 내려간다.
pub fn softmax_confidence(scores: &[Vec<f64>], temperature: f64) -> Vec<Vec<f64>> {
    scores.iter().map(|row| super::softmax::softmax_confidence(row, temperature)).collect()
}

/// 분류기가 모델에 요구하는 능력. 시험에서는 대역으로 바꾼다.
pub trait CategoryModelApi: Send + Sync {
    fn min_confidence(&self) -> f64;
    fn predict_one(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction;
    fn predict(&self, docs: &[Value], min_confidence: Option<f64>) -> Vec<Prediction>;
    /// 확신도를 예상 정답률로 옮긴다. 보정이 없으면 `None`.
    fn expected_accuracy(&self, _confidence: f64) -> Option<f64> {
        None
    }
}

/// 크기가 큰 부분. 모델을 복제해도 공유한다.
#[derive(Debug)]
struct Weights {
    fields: Vec<FieldVectorizer>,
    /// `[n_features, n_classes]` 행 우선.
    coef: Vec<f32>,
    intercept: Vec<f64>,
}

/// 내보낸 파일에서 읽은 추론 전용 모델. 복제는 가중치를 공유한다.
#[derive(Debug, Clone)]
pub struct CategoryModel {
    pub classes: Vec<String>,
    pub config: Value,
    pub meta: Value,
    weights: Arc<Weights>,
}

impl CategoryModel {
    /// `backend.classifier.export` 가 쓴 디렉토리를 읽는다. sha256, 길이, 열 수 합계, 어휘 중복을 검증한다.
    pub fn load_dir(path: &Path) -> Result<Self, ModelError> {
        let loaded = export_format::load_dir(path)?;
        let m = loaded.manifest;
        Ok(Self { classes: m.classes, config: m.config, meta: m.meta, weights: Arc::new(Weights { fields: loaded.fields, coef: loaded.coef, intercept: m.intercept }) })
    }

    /// 경로(기본은 `model_path()`)의 모델을 읽는다. 없거나 못 읽으면 `None` 이다(오류 내용은 로그에만 남긴다).
    pub fn load(path: Option<&Path>) -> Option<Self> {
        let p = path.map(Path::to_path_buf).unwrap_or_else(model_path);
        if !p.exists() {
            log::info!("모델 파일이 없다: {}", p.display());
            return None;
        }
        match Self::load_dir(&p) {
            Ok(model) => Some(model),
            Err(e) => {
                log::warn!("모델을 읽지 못했다 ({}): {e}", p.display());
                None
            }
        }
    }

    /// 내보내기 형식으로 쓴다. 경로를 안 주면 `model_path()` 다. 쓴 경로를 돌려준다.
    pub fn save(&self, path: Option<&Path>) -> std::io::Result<PathBuf> {
        let p = path.map(Path::to_path_buf).unwrap_or_else(model_path);
        let w = &self.weights;
        export_format::write_dir(&p, &self.classes, &w.fields, &w.coef, &w.intercept, &self.config, &self.meta).map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(p)
    }

    /// 메타만 가진 빈 모델. 보정 눈금처럼 메타만 보는 코드를 시험할 때 쓴다(Python `__new__` 후 meta 대입).
    pub fn with_meta(meta: Value) -> Self {
        Self { classes: Vec::new(), config: serde_json::json!({"decision": {}}), meta, weights: Arc::new(Weights { fields: Vec::new(), coef: Vec::new(), intercept: Vec::new() }) }
    }

    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    pub fn n_features(&self) -> usize {
        self.weights.fields.iter().map(FieldVectorizer::vocab_size).sum()
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
            .unwrap_or_else(|| {
                // 홀드아웃 기록이 없는 모델. 아무것도 판정하지 않는 편이 오분류보다 낫다.
                log::warn!("임계값을 못 정했다. 설정에 min_confidence 를 넣거나 다시 학습할 것");
                1.0
            })
    }

    /// 표시용 눈금. 학습이 넣어 둔 것을 먼저 쓰고, 없으면 홀드아웃 곡선에서 만든다.
    pub fn confidence_calibration(&self) -> Option<Value> {
        if let Some(cached) = self.meta.get("confidence_calibration").filter(|c| c.as_object().is_some_and(|o| !o.is_empty())) {
            return Some(cached.clone());
        }
        calibration::from_coverage_report(self.meta.get("holdout"))
    }

    /// 확신도를 "이 점수대의 실측 정답률" 로 옮긴다. 보정이 없는 모델이면 `None`.
    pub fn expected_accuracy(&self, confidence: f64) -> Option<f64> {
        calibration::expected_accuracy(self.confidence_calibration().as_ref(), Some(confidence))
    }

    /// 문서 하나의 희소 특징 벡터 `(전체 열, 값)`. 필드 순서대로 열 offset 이 더해져 오름차순이다.
    pub fn features(&self, doc: &Value) -> Vec<(u32, f32)> {
        let mut x = Vec::new();
        for field in &self.weights.fields {
            field.transform_into(&field_text(doc, &field.source, field.max_chars), &mut x);
        }
        x
    }

    /// 결정값 `X · coef + intercept`. 클래스마다 하나.
    pub fn scores(&self, doc: &Value) -> Vec<f64> {
        let n = self.classes.len();
        let w = &self.weights;
        let mut d = w.intercept.clone();
        for (col, value) in self.features(doc) {
            let row = &w.coef[col as usize * n..(col as usize + 1) * n];
            for (acc, &c) in d.iter_mut().zip(row) {
                *acc += f64::from(value) * f64::from(c);
            }
        }
        d
    }

    /// 문서마다 판정한다. `min_confidence` 가 `None` 이면 모델의 임계값을 쓴다. `top_k` 는 1 이상으로 본다.
    pub fn predict_top_k(&self, docs: &[Value], min_confidence: Option<f64>, top_k: usize) -> Vec<Prediction> {
        if docs.is_empty() {
            return Vec::new();
        }
        let floor = min_confidence.unwrap_or_else(|| self.min_confidence());
        let temperature = self.temperature();
        docs.iter().map(|doc| self.predict_scores(&self.scores(doc), floor, temperature, top_k.max(1))).collect()
    }

    /// 모델의 임계값으로 문서마다 판정한다(상위 3개 후보).
    pub fn predict(&self, docs: &[Value]) -> Vec<Prediction> {
        self.predict_top_k(docs, None, DEFAULT_TOP_K)
    }

    /// 문서 하나를 `min_confidence` 기준으로 판정한다.
    pub fn predict_one(&self, doc: &Value, min_confidence: f64) -> Prediction {
        self.predict_top_k(std::slice::from_ref(doc), Some(min_confidence), DEFAULT_TOP_K).remove(0)
    }

    fn predict_scores(&self, scores: &[f64], floor: f64, temperature: f64, top_k: usize) -> Prediction {
        let prob = super::softmax::softmax_confidence(scores, temperature);
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

impl CategoryModelApi for CategoryModel {
    fn min_confidence(&self) -> f64 {
        CategoryModel::min_confidence(self)
    }

    fn predict_one(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction {
        self.predict_top_k(std::slice::from_ref(doc), min_confidence, DEFAULT_TOP_K).remove(0)
    }

    fn predict(&self, docs: &[Value], min_confidence: Option<f64>) -> Vec<Prediction> {
        self.predict_top_k(docs, min_confidence, DEFAULT_TOP_K)
    }

    fn expected_accuracy(&self, confidence: f64) -> Option<f64> {
        CategoryModel::expected_accuracy(self, confidence)
    }
}
