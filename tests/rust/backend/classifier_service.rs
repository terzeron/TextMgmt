//! 원본: tests/test_classifier_service.py
//!
//! BookCategoryClassifier 의 대비책 경로 테스트. ES 가 문서 대부분을 갖고 있어 평소엔 안 밟히는 갈래
//! (ES 미적재, stat 실패, 본문 빈 문서, 파일 읽기 실패, 모델 없음)를 직접 확인한다.
//!
//! 가정한 API (docs/rust-test-seams.md 3.3, 3.12, 3.13):
//! - `backend::es_manager::EsTransport` 트레이트: `index_name(&self) -> &str`, `call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError>`
//!   (`get`, `search`는 `call("get")`, `call("search")`로 가는 기본 구현이다. Python `es_manager.es.get(**kwargs)` / `.search(**kwargs)`의 kwargs를 JSON 객체로 받는다).
//!   구 `EsLike`를 대체한다. 대역은 `common::es_dummy::DummyEs`다.
//! - `backend::classifier::model::CategoryModelApi` 트레이트: `min_confidence(&self) -> f64`,
//!   `predict_one(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction`,
//!   `predict(&self, docs: &[Value], min_confidence: Option<f64>) -> Vec<Prediction>` (`expected_accuracy`는 기본 구현). 구 `Model`, `ClassifierModel`을 대체한다.
//! - `backend::classifier::model::Prediction::new(category: Option<&str>, confidence: f64, top: Vec<(String, f64)>, reason: &str)`
//!   (필드 `category: Option<String>`, `reason: String`)
//! - `backend::classifier::BookCategoryClassifier::new(model: Option<Arc<dyn CategoryModelApi>>, ClassifierOptions { es: Option<Arc<dyn EsTransport>>, library_root: Option<PathBuf>, min_confidence: Option<f64> }) -> Self`
//!   (`ClassifierOptions`는 `Default` 구현. 구 `new(model, es_manager, root).with_min_confidence(x)`는 `ClassifierOptions`의 필드로 옮겼다)
//!   - `min_confidence(&self) -> f64`, `is_available(&self) -> bool` (Python `bool(clf)`)
//!   - `build_document(&self, path: &Path) -> Value`, `indexed_path(&self, path: &Path) -> String`
//!   - `classify_path(&self, &Path) -> Prediction`, `classify_document(&self, &Value, Option<f64>) -> Prediction`,
//!     `classify_documents(&self, &[Value], Option<f64>) -> Vec<Prediction>` (고유 메서드; 트레이트 `Classify`와 달리 Result 없이 Prediction을 돌려준다)
//!   - 연관 함수 `read_text(path: &Path) -> String`, `read_publisher(path: &Path) -> String`

mod common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use backend::classifier::model::{CategoryModelApi, Prediction};
use backend::classifier::{BookCategoryClassifier, ClassifierOptions};
use backend::es_manager::EsTransport;
use common::es_dummy::DummyEs;
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Default)]
struct FakeState {
    predict_one_calls: Vec<Value>,
    predict_calls: Vec<Vec<Value>>,
    last_floor: Option<f64>,
}

/// 호출을 기록하는 모델 대역. 상태는 `Arc`로 공유해서 분류기에 넘긴 뒤에도 읽을 수 있다.
#[derive(Clone, Default)]
struct FakeModel {
    state: Arc<Mutex<FakeState>>,
}

fn verdict() -> Prediction {
    Prediction::new(Some("2_소설한국"), 0.9, vec![], "판정함")
}

impl CategoryModelApi for FakeModel {
    fn min_confidence(&self) -> f64 {
        0.3
    }

    fn predict_one(&self, doc: &Value, min_confidence: Option<f64>) -> Prediction {
        let mut s = self.state.lock().unwrap();
        s.predict_one_calls.push(doc.clone());
        s.last_floor = min_confidence;
        verdict()
    }

    fn predict(&self, docs: &[Value], min_confidence: Option<f64>) -> Vec<Prediction> {
        let mut s = self.state.lock().unwrap();
        s.predict_calls.push(docs.to_vec());
        s.last_floor = min_confidence;
        docs.iter().map(|_| verdict()).collect()
    }
}

/// 함수 두 개로 만드는 ES 대역. `None`이면 호출 즉시 실패시킨다. `common::es_dummy::DummyEs`(색인 `books`)의 `get`/`search`를 바꿔 만든다.
fn fake_es(get: Option<Box<dyn Fn(&Value) -> Value + Send + Sync>>, search: Option<Box<dyn Fn(&Value) -> Value + Send + Sync>>) -> Arc<DummyEs> {
    let es = DummyEs::with_index("books");
    es.on("get", move |params| Ok((get.as_ref().expect("get 을 부르면 안 된다"))(params)));
    es.on("search", move |params| Ok((search.as_ref().expect("search 를 부르면 안 된다"))(params)));
    es
}

fn options(root: &Path) -> ClassifierOptions {
    ClassifierOptions { library_root: Some(root.to_path_buf()), ..Default::default() }
}

fn classifier(root: &Path, es: Option<Arc<DummyEs>>, model: Option<FakeModel>) -> BookCategoryClassifier {
    let model = model.unwrap_or_default();
    BookCategoryClassifier::new(Some(Arc::new(model)), ClassifierOptions { es: es.map(|e| e as Arc<dyn EsTransport>), ..options(root) })
}

#[test]
fn test_build_document_falls_back_to_file_when_no_es() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("2_소설한국").join("어떤책.txt");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, "그는 아무 말도 하지 않았다 ".repeat(10)).unwrap();

    let doc = classifier(tmp.path(), None, None).build_document(&f);

    assert_eq!(doc["name"], "어떤책.txt");
    assert_eq!(doc["type"], "txt");
    assert!(doc["text"].as_str().unwrap().contains("아무 말도"));
    // EPUB 이 아니면 OPF 를 풀지 않는다.
    assert_eq!(doc["publisher"], "");
}

#[test]
fn test_build_document_fills_name_when_es_record_has_none() {
    // file_path 가 비면 name 도 빈다. 파일명으로 메워야 특징이 살아난다.
    let es = fake_es(Some(Box::new(|_| json!({"_id": "1", "_source": {"category": "2_소설한국", "summary": "본문".repeat(20), "file_path": "", "file_type": "txt", "publisher": "민음사"}}))), None);
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("이름없는레코드.txt");
    fs::write(&f, "내용").unwrap();

    let doc = classifier(tmp.path(), Some(es), None).build_document(&f);

    assert_eq!(doc["name"], "이름없는레코드.txt");
    assert_eq!(doc["publisher"], "민음사");
}

/// 파일이 사라져 inode 를 못 읽어도 경로로 다시 찾는다.
#[test]
fn test_from_elasticsearch_survives_stat_failure() {
    let searched: Arc<Mutex<Value>> = Arc::new(Mutex::new(json!({})));
    let sink = searched.clone();
    let es = fake_es(
        // inode 가 없으면 get 을 부르면 안 된다
        Some(Box::new(|_| panic!("inode 가 없으면 get 을 부르면 안 된다"))),
        Some(Box::new(move |params| {
            *sink.lock().unwrap() = params.clone();
            json!({"hits": {"hits": [{"_id": "9", "_source": {"category": "2_소설한국", "summary": "본문".repeat(20), "file_path": "2_소설한국/a.txt", "file_type": "txt"}}]}})
        })),
    );

    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("2_소설한국").join("없는파일.txt");
    let doc = classifier(tmp.path(), Some(es), None).build_document(&missing);

    assert_eq!(doc["id"], "9");
    assert_eq!(searched.lock().unwrap()["query"]["term"]["file_path"], "2_소설한국/없는파일.txt");
}

/// 인덱싱은 됐지만 본문이 비었으면 ES 가 도움이 안 된다 — 파일에서 읽는다.
#[test]
fn test_from_elasticsearch_reads_file_when_indexed_text_is_empty() {
    let es = fake_es(Some(Box::new(|_| json!({"_id": "1", "_source": {"category": "2_소설한국", "summary": "", "file_path": "2_소설한국/a.txt", "file_type": "txt"}}))), None);

    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("2_소설한국").join("a.txt");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, "그는 아무 말도 하지 않았다 ".repeat(10)).unwrap();

    let doc = classifier(tmp.path(), Some(es), None).build_document(&f);

    assert!(doc["text"].as_str().unwrap().contains("아무 말도"));
}

#[test]
fn test_indexed_path_keeps_absolute_path_outside_library() {
    let tmp = TempDir::new().unwrap();
    let outside = tmp.path().parent().unwrap().join("밖에있는파일.txt");
    let clf = classifier(tmp.path(), None, None);
    assert_eq!(clf.indexed_path(&outside), outside.to_str().unwrap());
}

/// 가정한 seam: `BookCategoryClassifier::read_text_with(path, reader)`가 reader 함수를 주입받는다.
#[test]
#[ignore = "mock.patch: backend.classifier.reader.read_document_text 가 예외를 던지도록 바꿈; DI 설계 후 활성화"]
fn test_read_text_swallows_reader_errors() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("a.txt");
    fs::write(&f, "내용").unwrap();
    let boom = |_: &Path| -> Result<String, String> { Err("파서가 죽음".to_string()) };
    // 한 권이 읽히지 않아도 판정 자체는 계속해야 한다.
    assert_eq!(BookCategoryClassifier::read_text_with(&f, &boom), "");
}

/// 가정한 seam: `BookCategoryClassifier::read_publisher_with(path, reader)`가 reader 함수를 주입받는다.
#[test]
#[ignore = "mock.patch: backend.classifier.reader.read_epub_publisher 가 예외를 던지도록 바꿈; DI 설계 후 활성화"]
fn test_read_publisher_skips_non_epub_and_swallows_errors() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(BookCategoryClassifier::read_publisher(&tmp.path().join("a.pdf")), "");

    let boom = |_: &Path| -> Result<String, String> { Err("OPF 가 깨짐".to_string()) };
    assert_eq!(BookCategoryClassifier::read_publisher_with(&tmp.path().join("a.epub"), &boom), "");
}

#[test]
fn test_classify_without_model_returns_no_verdict() {
    let tmp = TempDir::new().unwrap();
    let clf = BookCategoryClassifier::new(None, options(tmp.path()));
    assert!(!clf.is_available());

    let f = tmp.path().join("a.txt");
    fs::write(&f, "내용").unwrap();

    for pred in [clf.classify_path(&f), clf.classify_document(&json!({"name": "a.txt"}), None)] {
        assert_eq!(pred.category, None);
        assert_eq!(pred.reason, "모델 파일이 없어 판정하지 않음");
    }

    let preds = clf.classify_documents(&[json!({"name": "a.txt"}), json!({"name": "b.txt"})], None);
    assert_eq!(preds.len(), 2);
    assert!(preds.iter().all(|p| p.category.is_none()));
}

#[test]
fn test_classify_documents_passes_confidence_floor() {
    let tmp = TempDir::new().unwrap();
    let model = FakeModel::default();
    let clf = classifier(tmp.path(), None, Some(model.clone()));
    let floor = || model.state.lock().unwrap().last_floor.unwrap();
    let approx = |a: f64, b: f64| (a - b).abs() < 1e-9;

    clf.classify_documents(&[json!({"name": "a.txt"})], None);
    // 호출자가 안 주면 모델 기본값을 쓴다.
    assert!(approx(floor(), 0.3), "floor={}", floor());

    clf.classify_documents(&[json!({"name": "a.txt"})], Some(0.75));
    assert!(approx(floor(), 0.75), "floor={}", floor());

    clf.classify_document(&json!({"name": "a.txt"}), Some(0.5));
    assert!(approx(floor(), 0.5), "floor={}", floor());
}

#[test]
fn test_min_confidence_falls_back_to_config_without_model() {
    let tmp = TempDir::new().unwrap();
    let clf = BookCategoryClassifier::new(None, options(tmp.path()));
    // 모델이 없으면 판정 자체를 안 하므로 어떤 값이든 실제로는 안 쓰인다.
    let default_floor: f64 = clf.min_confidence();
    assert!(default_floor.is_finite());

    let explicit = BookCategoryClassifier::new(None, ClassifierOptions { min_confidence: Some(0.42), ..options(tmp.path()) });
    assert!((explicit.min_confidence() - 0.42).abs() < 1e-9);
}
