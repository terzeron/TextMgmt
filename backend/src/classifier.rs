//! 분류기 표면. `BookManager`가 쓰는 최소 trait와 타입만 있다.
//!
//! 모델 추론 코어는 `model`(내보낸 파일을 읽는다), 설정·코퍼스·교정·리더는 각 모듈에 있다. `BookCategoryClassifier` 가 모델과 ES 를
//! 묶어 파일 하나를 판정한다. 서점 정책을 가진 `BookClassifierService` 는 다음 단계다.
//! 설계: docs/rust-test-seams.md 3.12 "분류기 3계층".

use std::any::Any;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::es_manager::EsTransport;

pub mod calibration;
pub mod config;
pub mod corpus;
pub mod export_format;
pub mod features;
pub mod model;
pub mod pyjson;
pub mod pyunicode_table;
pub mod reader;
pub mod softmax;
pub mod tokenize;
pub mod training;
pub mod vectorize;

use model::{CategoryModelApi, Prediction};

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

/// Python `ValueError` 대응. 입력이 쓸 수 없는 값일 때 돌려준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueError(pub String);

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValueError {}

/// 본문을 파일에서 직접 읽을 때 몇 글자까지 볼 것인가. ES summary 상한과 맞춘다.
pub const FALLBACK_TEXT_CHARS: usize = 4096;
const NO_MODEL_REASON: &str = "모델 파일이 없어 판정하지 않음";

/// `BookCategoryClassifier::new` 옵션.
#[derive(Clone, Default)]
pub struct ClassifierOptions {
    pub es: Option<Arc<dyn EsTransport>>,
    pub library_root: Option<PathBuf>,
    pub min_confidence: Option<f64>,
}

/// 모델 하나를 들고 파일이나 레코드를 판정한다. ES 를 먼저 보고 없을 때만 파일을 읽는다.
pub struct BookCategoryClassifier {
    model: Option<Arc<dyn CategoryModelApi>>,
    es: Option<Arc<dyn EsTransport>>,
    min_confidence: Option<f64>,
    /// ES 의 `file_path` 는 이 디렉토리 기준 상대 경로다. 절대 경로로 질의하면 안 맞는다.
    library_root: PathBuf,
}

/// 존재하지 않는 경로도 Python `Path.resolve()` 처럼 풀어 준다. 가장 깊은 기존 조상을 정규화하고 나머지를 붙인다.
fn resolve_lenient(path: &Path) -> PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(real) = cur.canonicalize() {
            return tail.iter().rev().fold(real, |acc, c| if c == ".." { acc.parent().map(Path::to_path_buf).unwrap_or(acc) } else { acc.join(c) });
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

impl BookCategoryClassifier {
    pub fn new(model: Option<Arc<dyn CategoryModelApi>>, opts: ClassifierOptions) -> Self {
        let library_root = opts.library_root.unwrap_or_else(|| PathBuf::from(config::load_config_from_env()["corpus"]["library_root"].as_str().unwrap_or("/mnt/data/text")));
        Self { model, es: opts.es, min_confidence: opts.min_confidence, library_root }
    }

    /// 모델이 있는가. Python `bool(clf)`.
    pub fn is_enabled(&self) -> bool {
        self.model.is_some()
    }

    pub fn is_available(&self) -> bool {
        self.is_enabled()
    }

    pub fn min_confidence(&self) -> f64 {
        if let Some(v) = self.min_confidence {
            return v;
        }
        if let Some(model) = &self.model {
            return model.min_confidence();
        }
        // 모델이 없으면 판정 자체를 안 한다. 설정값이 있어도 쓸 곳이 없다.
        config::load_config_from_env().pointer("/decision/min_confidence").and_then(Value::as_f64).unwrap_or(1.0)
    }

    /// 판정에 쓸 레코드를 만든다. ES 를 먼저 보고, 없으면 파일에서 읽는다.
    pub fn build_document(&self, path: &Path) -> Value {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut doc = match self.lookup_in_es(path) {
            Some(doc) => doc,
            None => {
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                json!({"id": "", "cat": "", "text": Self::read_text(path), "title": "", "author": "", "type": ext, "name": name, "path": path.to_string_lossy()})
            }
        };
        // publisher 는 ES 가 갖고 있으면 그대로 쓴다. 없을 때만 EPUB 의 OPF 를 푼다.
        if doc.get("publisher").and_then(Value::as_str).is_none_or(str::is_empty) {
            doc["publisher"] = Value::String(Self::read_publisher(path));
        }
        if doc.get("name").and_then(Value::as_str).is_none_or(str::is_empty) {
            doc["name"] = Value::String(name);
        }
        doc
    }

    fn lookup_in_es(&self, path: &Path) -> Option<Value> {
        use std::os::unix::fs::MetadataExt;
        let es = self.es.as_deref()?;
        let inode = std::fs::metadata(path).ok().map(|m| m.ino());
        let mut doc = inode.and_then(|i| corpus::fetch_by_inode(es, i, None));
        if doc.is_none() {
            doc = corpus::fetch_by_path(es, &self.indexed_path(path), None);
        }
        let mut doc = doc?;
        if doc.get("text").and_then(Value::as_str).is_none_or(str::is_empty) {
            // 인덱싱은 됐지만 본문이 비었으면 ES 가 도움이 안 된다.
            doc["text"] = Value::String(Self::read_text(path));
        }
        Some(doc)
    }

    /// ES 에 들어 있는 모양의 경로. loader 가 라이브러리 루트 기준 상대 경로로 넣는다. 루트 밖이면 있는 그대로다.
    pub fn indexed_path(&self, path: &Path) -> String {
        match resolve_lenient(path).strip_prefix(resolve_lenient(&self.library_root)) {
            Ok(rel) => rel.to_string_lossy().into_owned(),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    /// 파일에서 본문 앞부분을 읽는다. ES summary 는 앞부분만 담으므로 뒷부분은 붙이지 않는다.
    pub fn read_text(path: &Path) -> String {
        Self::read_text_with(path, &|p| Ok(reader::read_document_text(p, FALLBACK_TEXT_CHARS, 0)))
    }

    /// `read_text` 의 리더 주입 판. 리더가 실패해도 빈 문자열로 삼킨다.
    pub fn read_text_with(path: &Path, reader: &dyn Fn(&Path) -> Result<String, String>) -> String {
        reader(path).unwrap_or_else(|e| {
            log::debug!("본문을 읽지 못했다: {e}");
            String::new()
        })
    }

    /// EPUB 의 publisher. EPUB 이 아니면 빈 문자열이다.
    pub fn read_publisher(path: &Path) -> String {
        Self::read_publisher_with(path, &|p| Ok(reader::read_epub_publisher(p)))
    }

    pub fn read_publisher_with(path: &Path, reader: &dyn Fn(&Path) -> Result<String, String>) -> String {
        if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("epub")) {
            return String::new();
        }
        reader(path).unwrap_or_else(|e| {
            log::debug!("publisher 를 읽지 못했다: {e}");
            String::new()
        })
    }

    fn no_model() -> Prediction {
        Prediction::new(None, 0.0, Vec::new(), NO_MODEL_REASON)
    }

    pub fn classify_path(&self, path: &Path) -> Prediction {
        match &self.model {
            None => Self::no_model(),
            Some(_) => self.classify_document(&self.build_document(path), None),
        }
    }

    pub fn classify_document(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction {
        match &self.model {
            None => Self::no_model(),
            Some(model) => model.predict_one(doc, Some(min_confidence.unwrap_or_else(|| self.min_confidence()))),
        }
    }

    pub fn classify_documents(&self, docs: &[Value], min_confidence: Option<f64>) -> Vec<Prediction> {
        match &self.model {
            None => docs.iter().map(|_| Self::no_model()).collect(),
            Some(model) => model.predict(docs, Some(min_confidence.unwrap_or_else(|| self.min_confidence()))),
        }
    }

    /// 확신도를 예상 정답률(0~1)로 옮긴다. 보정이 없는 모델이면 `None`.
    pub fn expected_accuracy(&self, confidence: Option<f64>) -> Option<f64> {
        self.model.as_ref()?.expected_accuracy(confidence?)
    }
}

impl Classify for BookCategoryClassifier {
    fn is_available(&self) -> bool {
        self.is_enabled()
    }

    fn classify_path(&self, path: &Path, min_confidence: Option<f64>) -> Result<Prediction, String> {
        match &self.model {
            None => Ok(Self::no_model()),
            Some(_) => Ok(self.classify_document(&self.build_document(path), min_confidence)),
        }
    }

    fn classify_document(&self, doc: &Value, min_confidence: Option<f64>) -> Result<Prediction, String> {
        Ok(BookCategoryClassifier::classify_document(self, doc, min_confidence))
    }
}
