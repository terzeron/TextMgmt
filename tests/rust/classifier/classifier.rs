//! `classifier` 최상위 단위 테스트.

mod common;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use classifier::feature_extractor::field_text;
use classifier::inference_model::{CategoryModel, MODEL_DIR_ENV, Prediction};
use classifier::{BookCategoryClassifier, ClassifierOptions};
use es_client::transport::EsTransport;
use common::env::EnvGuard;
use common::epub::{EpubSpec, make_epub};
use common::es_dummy::{DummyEs, not_found_error};
use serde_json::{json, Value};
use tempfile::TempDir;
use crate::common::shared_fixtures::{load_trained, trained, vocab};

// ---------------------------------------------------------------------------
// features
// ---------------------------------------------------------------------------

#[test]
fn test_field_text_strips_digit_runs_from_filenames_and_honors_max_chars() {
    let spec = json!({"source": "name", "max_chars": 0});
    // 두 자리 이상 숫자만 공백으로 바꾼다. 공백을 다시 접지는 않는다.
    assert_eq!(field_text(&json!({"name": "달빛조각사 01권 2024.epub"}), &spec), "달빛조각사  권  .epub");

    assert_eq!(field_text(&json!({"text": "가나다라마"}), &json!({"source": "text", "max_chars": 3})), "가나다");
    assert_eq!(field_text(&json!({"text": null}), &json!({"source": "text", "max_chars": 0})), "");
    assert_eq!(field_text(&json!({"subject": ["무협", "판타지"]}), &json!({"source": "subject", "max_chars": 0})), "무협 판타지");
}

#[test]
fn test_model_save_and_load_round_trip() {
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let path = model.save(Some(&tmp.path().join("model.joblib"))).unwrap();
    assert!(path.exists());

    let loaded = CategoryModel::load(Some(&path)).expect("로드되어야 한다");
    assert_eq!(loaded.classes.iter().collect::<HashSet<_>>(), model.classes.iter().collect::<HashSet<_>>());
    assert_eq!(loaded.meta["config_hash"], model.meta["config_hash"]);
}

#[test]
fn test_model_load_returns_none_for_missing_or_unusable_file() {
    let tmp = TempDir::new().unwrap();
    assert!(CategoryModel::load(Some(&tmp.path().join("없는파일.joblib"))).is_none());

    let garbage = tmp.path().join("garbage.joblib");
    fs::write(&garbage, b"not a joblib payload").unwrap();
    assert!(CategoryModel::load(Some(&garbage)).is_none());

    // joblib.dump({"classifier": None}) 에 대응한다. 모델 형식은 달라도 필수 필드가 빠진 페이로드다.
    let wrong = tmp.path().join("wrong.joblib");
    fs::write(&wrong, serde_json::to_vec(&json!({"classifier": null})).unwrap()).unwrap();
    assert!(CategoryModel::load(Some(&wrong)).is_none());
}

// ---------------------------------------------------------------------------
// 공개 API
// ---------------------------------------------------------------------------

/// 구 `FakeEs { by_inode, by_path }`(`EsLookup`). `DummyEs`(`EsTransport`)의 `get`은 inode(id)로, `search`는 `query.term.file_path`로 `_source`를 찾는다.
fn fake_es(by_inode: HashMap<String, Value>, by_path: HashMap<String, Value>) -> Arc<DummyEs> {
    let es = DummyEs::with_index("books");
    es.on("get", move |kw| {
        let id = kw["id"].as_str().unwrap_or_default().to_string();
        by_inode.get(&id).map(|source| json!({"_id": id, "_source": source})).ok_or_else(|| not_found_error("not found"))
    });
    es.on("search", move |kw| {
        let path = kw["query"]["term"]["file_path"].as_str().unwrap_or_default();
        let hits: Vec<Value> = by_path.get(path).map(|source| json!({"_source": source})).into_iter().collect();
        Ok(json!({"hits": {"hits": hits}}))
    });
    es
}

fn opts(es: Option<Arc<DummyEs>>, library_root: Option<&Path>, min_confidence: Option<f64>) -> ClassifierOptions {
    ClassifierOptions { es: es.map(|e| e as Arc<dyn EsTransport>), library_root: library_root.map(Path::to_path_buf), min_confidence }
}

#[test]
fn test_classifier_without_a_model_refuses_to_answer() {
    let tmp = TempDir::new().unwrap();
    let clf = BookCategoryClassifier::new(None, ClassifierOptions::default());
    assert!(!clf.is_enabled());
    let pred = clf.classify_path(&tmp.path().join("책.epub"));
    assert_eq!(pred.category, None);
    assert!(pred.reason.contains("모델 파일이 없어"));
    let preds = clf.classify_documents(&[json!({}), json!({})], None);
    assert!(preds == vec![pred.clone(), pred.clone()] || clf.classify_documents(&[json!({}), json!({})], None).iter().all(|p| p.category.is_none()));
}

#[test]
fn test_build_document_prefers_elasticsearch_by_inode() {
    use std::os::unix::fs::MetadataExt;
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let f = tmp.path().join("책.epub");
    make_epub(&f, &EpubSpec::publisher("무시될출판사"));
    let inode = fs::metadata(&f).unwrap().ino().to_string();
    let source = json!({"category": "3_무협", "summary": vocab("3_무협"), "title": "무림", "author": "작가", "publisher": "을유문화사", "file_path": f.to_str().unwrap(), "file_type": "epub"});

    let es = fake_es(HashMap::from([(inode.clone(), source)]), HashMap::new());
    let clf = BookCategoryClassifier::new(Some(Arc::new(model)), opts(Some(es), None, None));
    let doc = clf.build_document(&f);
    assert_eq!(doc["id"], inode.as_str());
    assert_eq!(doc["text"], vocab("3_무협"));
    // ES 에 publisher 가 있으면 EPUB 을 다시 열지 않는다
    assert_eq!(doc["publisher"], "을유문화사");
}

#[test]
fn test_build_document_falls_back_to_path_lookup_then_to_the_file() {
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let f = tmp.path().join("무림맹_1.epub");
    make_epub(&f, &EpubSpec::publisher("파일에서읽은출판사"));
    let source = json!({"category": "3_무협", "summary": vocab("3_무협"), "title": "", "author": "", "publisher": "", "file_path": f.to_str().unwrap(), "file_type": "epub"});

    // inode 로는 못 찾고 경로로 찾는다. publisher 는 ES 가 비워 두었으니 파일에서 읽는다.
    let es = fake_es(HashMap::new(), HashMap::from([(f.to_str().unwrap().to_string(), source)]));
    let clf = BookCategoryClassifier::new(Some(Arc::new(model)), opts(Some(es), None, None));
    let doc = clf.build_document(&f);
    assert_eq!(doc["text"], vocab("3_무협"));
    assert_eq!(doc["publisher"], "파일에서읽은출판사");
}

#[test]
fn test_build_document_reads_the_file_when_elasticsearch_is_absent() {
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let f = tmp.path().join("무림맹_1.txt");
    fs::write(&f, vocab("3_무협").repeat(10)).unwrap();

    let clf = BookCategoryClassifier::new(Some(Arc::new(model)), opts(None, None, None));
    let doc = clf.build_document(&f);
    assert_eq!(doc["id"], "");
    assert!(doc["text"].as_str().unwrap().contains("무림맹"));
    assert_eq!(doc["type"], "txt");
    // TXT 에는 dc:publisher 가 없다
    assert_eq!(doc["publisher"], "");
}

#[test]
fn test_classify_path_uses_the_model() {
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let f = tmp.path().join("무림맹_1.txt");
    fs::write(&f, [vocab("3_무협"); 10].join(" ")).unwrap();

    let clf = BookCategoryClassifier::new(Some(Arc::new(model)), opts(None, None, Some(0.0)));
    let pred: Prediction = clf.classify_path(&f);
    assert_eq!(pred.category.as_deref(), Some("3_무협"));
}

#[test]
fn test_min_confidence_precedence() {
    let (model, _) = trained();
    assert_eq!(BookCategoryClassifier::new(Some(Arc::new(model.clone())), opts(None, None, Some(0.55))).min_confidence(), 0.55);
    assert_eq!(BookCategoryClassifier::new(Some(Arc::new(model.clone())), ClassifierOptions::default()).min_confidence(), model.min_confidence());
    // 모델이 없으면 판정하지 않는다
    assert_eq!(BookCategoryClassifier::new(None, ClassifierOptions::default()).min_confidence(), 1.0);
}

#[test]
fn test_threshold_comes_from_the_holdout_when_config_leaves_it_open() {
    // 확신도 척도는 데이터마다 달라 고정값을 못 쓴다. 학습이 홀드아웃에서 고른다.
    let (model, report) = trained();
    assert!(model.config["decision"]["min_confidence"].is_null());
    assert_eq!(model.min_confidence(), report["at_precision_90"]["threshold"].as_f64().unwrap());
    assert_eq!(model.meta["calibrated_target"], 0.90);
}

#[test]
fn test_uncalibrated_model_refuses_everything() {
    // 홀드아웃 기록이 없으면 오분류보다 무판정을 고른다.
    let (mut model, _) = trained();
    model.meta.as_object_mut().unwrap().remove("calibrated_threshold");
    assert_eq!(model.min_confidence(), 1.0);
}

#[test]
fn test_target_accuracy_95_picks_the_stricter_threshold() {
    // 목표 정답률 0.95 로 학습한 픽스처(trained_95)를 읽는다.
    let (model, report) = load_trained("trained_95");
    assert_eq!(model.meta["calibrated_threshold"], report["at_precision_95"]["threshold"]);
    assert_eq!(model.meta["calibrated_target"], 0.95);
}

#[test]
fn test_indexed_path_is_relative_to_the_library_root() {
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let root = tmp.path().join("text");
    fs::create_dir_all(root.join("3_무협")).unwrap();
    let f = root.join("3_무협").join("책.epub");
    fs::write(&f, b"x").unwrap();

    let clf = BookCategoryClassifier::new(Some(Arc::new(model)), opts(None, Some(&root), None));
    // ES 는 라이브러리 루트 기준 상대 경로를 저장한다. 절대 경로로 질의하면 하나도 안 맞는다.
    assert_eq!(clf.indexed_path(&f), "3_무협/책.epub");

    let outside = tmp.path().join("다른곳").join("책.epub");
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&outside, b"x").unwrap();
    assert_eq!(clf.indexed_path(&outside), outside.to_str().unwrap());
}

#[test]
fn test_build_document_finds_documents_by_relative_path() {
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let root = tmp.path().join("text");
    fs::create_dir_all(root.join("3_무협")).unwrap();
    let f = root.join("3_무협").join("무림맹_1.epub");
    make_epub(&f, &EpubSpec::publisher("출판사"));
    let source = json!({"category": "3_무협", "summary": vocab("3_무협"), "title": "", "author": "", "publisher": "을유문화사", "file_path": "3_무협/무림맹_1.epub", "file_type": "epub"});

    let es = fake_es(HashMap::new(), HashMap::from([("3_무협/무림맹_1.epub".to_string(), source)]));
    let clf = BookCategoryClassifier::new(Some(Arc::new(model)), opts(Some(es), Some(&root), None));
    let doc = clf.build_document(&f);
    assert_eq!(doc["text"], vocab("3_무협"));
    assert_eq!(doc["publisher"], "을유문화사");
}

#[test]
fn test_save_and_load_use_the_environment_path() {
    let mut env = EnvGuard::new();
    let tmp = TempDir::new().unwrap();
    let (model, _) = trained();
    let dest = tmp.path().join("어딘가").join("model.joblib");
    env.set(MODEL_DIR_ENV, dest.to_str().unwrap());

    // 경로를 안 줘도 환경변수가 가리키는 곳에 쓰고 거기서 읽는다
    assert_eq!(model.save(None).unwrap(), dest);
    assert!(dest.exists());
    assert!(CategoryModel::load(None).is_some());
}

mod expected_accuracy {
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use classifier::inference_model::CategoryModelApi;
    use classifier::{BookCategoryClassifier, ClassifierOptions};
    use tempfile::TempDir;

    fn classifier(tmp: &Path, model: Option<Arc<dyn CategoryModelApi>>) -> BookCategoryClassifier {
        BookCategoryClassifier::new(model, ClassifierOptions { library_root: Some(tmp.to_path_buf()), ..Default::default() })
    }

    /// `model.expected_accuracy.return_value`와 호출 기록을 흉내 내는 대역.
    struct FakeModel {
        value: f64,
        calls: Mutex<Vec<f64>>,
    }

    impl CategoryModelApi for FakeModel {
        fn expected_accuracy(&self, confidence: f64) -> Option<f64> {
            self.calls.lock().unwrap().push(confidence);
            Some(self.value)
        }
    }

    #[test]
    fn test_expected_accuracy_is_none_without_model() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(classifier(tmp.path(), None).expected_accuracy(0.9), None);
    }

    #[test]
    fn test_expected_accuracy_delegates_to_model() {
        let tmp = TempDir::new().unwrap();
        let model = Arc::new(FakeModel { value: 0.87, calls: Mutex::new(Vec::new()) });
        // 호출 기록을 보려면 모델을 공유해야 한다. 모델이 `Arc<dyn CategoryModelApi>`라 같은 Arc를 테스트가 쥔다.
        let clf = classifier(tmp.path(), Some(model.clone()));
        assert_eq!(clf.expected_accuracy(0.9), Some(0.87));
        // assert_called_once_with(0.9)
        assert_eq!(*model.calls.lock().unwrap(), vec![0.9]);
    }
}

mod build_document {
    use crate::common::shared_fixtures::{options};
    use super::common;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use classifier::inference_model::{CategoryModelApi, Prediction};
    use classifier::{BookCategoryClassifier, ClassifierOptions};
    use es_client::transport::EsTransport;
    use common::es_dummy::DummyEs;
    use serde_json::{json, Value};
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
    type Handler = Option<Box<dyn Fn(&Value) -> Value + Send + Sync>>;

    fn fake_es(get: Handler, search: Handler) -> Arc<DummyEs> {
        let es = DummyEs::with_index("books");
        es.on("get", move |params| Ok((get.as_ref().expect("get 을 부르면 안 된다"))(params)));
        es.on("search", move |params| Ok((search.as_ref().expect("search 를 부르면 안 된다"))(params)));
        es
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
}
