//! 원본: tests/test_classifier.py
//!
//! 가정한 API (모듈 경로는 Python 경로를 따른다):
//! - `backend::classifier::config`
//!   - `default_config() -> Value` (Python `DEFAULT_CONFIG`), `series_to_parent() -> HashMap<String, String>` (`SERIES_TO_PARENT`)
//!   - `merge_config(base: &Value, over: &Value) -> Value`, `load_config(path: &Path) -> Value`, `save_config(cfg: &Value, path: &Path) -> io::Result<()>`
//!   - `config_hash(cfg: &Value) -> String`, `enabled_fields(cfg: &Value) -> Value`, `resolve_parent(Option<&str>) -> Option<String>`
//! - `backend::classifier::corpus`
//!   - `document(hit: &Value) -> Value` (Python `_document`), `is_trainable(doc: &Value, min_chars: usize, excluded: &[&str]) -> bool`
//!   - `absolute_path(doc: &Value, root: &str) -> Option<PathBuf>`, `read_jsonl(&Path) -> io::Result<Vec<Value>>`, `write_jsonl(&[Value], &Path) -> io::Result<usize>`
//!   - `attach_publishers(docs: &mut [Value], cache: &Path, root: &Path, workers: usize) -> Value`
//! - `backend::classifier::features`
//!   - `field_text(doc: &Value, spec: &Value) -> String`, `make_vectorizer(spec: &Value) -> Vectorizer` (`fit(&[&str]) -> &Self`, `vocabulary() -> &HashMap<String, usize>`)
//!   - `FeatureSpace::new(fields: &Value)`, `fit_transform(&mut self, docs: &[Value]) -> Result<SparseMatrix, ValueError>`,
//!     `fit_transform_with(&mut self, docs: &[Value], FitOptions { spill_dir: Option<PathBuf>, dtype: Dtype }) -> Result<SparseMatrix, ValueError>`,
//!     `transform(&self, docs: &[Value]) -> Result<SparseMatrix, ValueError>`, `has_vectorizer(&str) -> bool`, `vectorizer_names() -> Vec<String>`,
//!     `block_sizes() -> Vec<(String, usize)>`, `describe() -> Value`
//!   - `SparseMatrix { shape() -> (usize, usize), nnz() -> usize, dtype() -> Dtype, to_dense() -> Vec<Vec<f64>> }`
//!   - `shrink_sources(docs: &mut [Value], sources: &[&str], remaining: &[(&str, Value)])`
//! - `backend::classifier::model`
//!   - `DEFAULT_MODEL_PATH: &str`, `MODEL_DIR_ENV: &str`, `model_path() -> PathBuf`, `softmax_confidence(scores: &[Vec<f64>], temperature: f64) -> Vec<Vec<f64>>`
//!   - `CategoryModel` (Clone): `classes: Vec<String>`, `meta: Value`, `config: Value`, `predict_one(&self, doc: &Value, min_confidence: f64) -> Prediction`,
//!     `predict(&self, docs: &[Value]) -> Vec<Prediction>`, `save(&self, Option<&Path>) -> io::Result<PathBuf>`, `CategoryModel::load(Option<&Path>) -> Option<CategoryModel>`,
//!     `min_confidence(&self) -> f64`
//!   - `Prediction { category: Option<String>, ranked: Vec<(String, f64)>, reason: String }` (Clone, PartialEq, Debug)
//! - `backend::classifier::reader::read_epub_publisher(&Path) -> String`
//! - `backend::classifier::training`
//!   - `train(docs: &mut Vec<Value>, cfg: &Value, opts: TrainOptions { accept_parent: Option<HashMap<String, String>>, spill_dir: Option<PathBuf>, release_holdout_text: bool }) -> Result<(CategoryModel, Value), ValueError>`
//!   - `evaluate(model: &CategoryModel, docs: &[Value], accept_parent: Option<&HashMap<String, String>>) -> Value`, `format_report(&Value) -> String`
//!   - `filter_by_size(docs: &[Value], min: usize) -> (Vec<Value>, Vec<String>)`, `coverage_curve(correct: &[bool], conf: &[f64], points: &[f64]) -> Vec<Value>`
//!   - `max_coverage_at(correct: &[bool], conf: &[f64], target: f64) -> Value`, `split_holdout(docs: &[Value], ratio: f64, seed: u64) -> (Vec<Value>, Vec<Value>)`, `rss_note_with(&Path) -> String` (아래 참조)
//! - `backend::classifier::{BookCategoryClassifier, ClassifierOptions, ValueError}` (docs/rust-test-seams.md 3.12, 3.13)
//!   - `BookCategoryClassifier::new(model: Option<Arc<dyn CategoryModelApi>>, ClassifierOptions { es: Option<Arc<dyn EsTransport>>, library_root: Option<PathBuf>, min_confidence: Option<f64> })`
//!     (`CategoryModel`이 `CategoryModelApi`를 구현한다. 구 `es_manager: Box<dyn EsLookup>`은 `es: Arc<dyn EsTransport>`로 바뀌었다)
//!   - `is_enabled()`, `classify_path(&Path) -> Prediction`, `classify_documents(&[Value]) -> Vec<Prediction>`, `build_document(&Path) -> Value`, `min_confidence() -> f64`, `indexed_path(&Path) -> String`
//! - `backend::es_manager::EsTransport`: `index_name()`, `call(op, kwargs) -> Result<Value, EsError>` (`get`, `search` 기본 구현). 구 `EsLookup`(`get_by_id`, `search_by_path`) 대신
//!   `common::es_dummy::DummyEs`의 `get`(id 조회)·`search`(`query.term.file_path` 조회) 응답으로 같은 조회를 만든다.
//! - `backend::classifier::training::rss_note_with(status_path: &Path) -> String` (전역 `set_proc_status_path` 대신. 상태 파일 경로를 인자로 받는다)

mod common;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use backend::classifier::config::{config_hash, default_config, enabled_fields, load_config, merge_config, resolve_parent, save_config, series_to_parent};
use backend::classifier::corpus::{absolute_path, attach_publishers, document, is_trainable, read_jsonl, write_jsonl};
use backend::classifier::features::{Dtype, FeatureSpace, FitOptions, field_text, make_vectorizer, shrink_sources};
use backend::classifier::model::{CategoryModel, DEFAULT_MODEL_PATH, MODEL_DIR_ENV, Prediction, get_default_model, model_path, reset_default_model, softmax_confidence};
use backend::classifier::reader::read_epub_publisher;
use backend::classifier::training::{TrainOptions, coverage_curve, evaluate, filter_by_size, format_report, max_coverage_at, rss_note_with, split_holdout, train};
use backend::classifier::{BookCategoryClassifier, ClassifierOptions};
use backend::es_manager::EsTransport;
use common::env::EnvGuard;
use common::epub::{EpubSpec, make_epub};
use common::es_dummy::{DummyEs, not_found_error};
use common::log_capture::LogCapture;
use common::zipx::write_zip;
use serde_json::{Value, json};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// 시험용 데이터
//
// 카테고리마다 어휘를 확실히 갈라 둔다. 선형 모델이 배울 신호가 있어야
// 학습 경로 전체를 실제로 통과시킬 수 있다.
// ---------------------------------------------------------------------------

const VOCAB: [(&str, &str); 3] =
    [("3_무협", "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객"), ("3_판타지", "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕"), ("5_수학과학일반", "미분 적분 함수 벡터 행렬 확률 통계 방정식")];

fn vocab(cat: &str) -> &'static str {
    VOCAB.iter().find(|(c, _)| *c == cat).unwrap().1
}

fn make_docs(per_category: usize) -> Vec<Value> {
    let mut docs = Vec::new();
    for (cat, words) in VOCAB {
        let tokens: Vec<&str> = words.split_whitespace().collect();
        for i in 0..per_category {
            // 문서마다 어순을 바꿔 같은 문자열이 반복되지 않게 한다.
            let k = i % tokens.len();
            let shifted: Vec<&str> = tokens[k..].iter().chain(tokens[..k].iter()).copied().collect();
            let text = shifted.repeat(3).join(" ");
            docs.push(json!({
                "id": format!("{cat}-{i}"), "cat": cat, "text": text, "title": format!("{} 이야기 {i}", tokens[k]), "author": format!("작가{}", i % 5),
                "publisher": "테스트출판사", "type": if i % 2 == 0 { "epub" } else { "txt" }, "name": format!("{}_{i}.epub", tokens[k]), "path": format!("/mnt/data/text/{cat}/{i}.epub"),
            }));
        }
    }
    docs
}

fn small_fields() -> Value {
    json!({"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": false}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}})
}

fn train_opts() -> TrainOptions {
    TrainOptions { accept_parent: Some(series_to_parent()), spill_dir: None, release_holdout_text: false }
}

/// pytest fixture `trained`에 대응한다. Python sklearn 학습 결과와 비교하는 parity oracle을 읽는다
/// (`python tests/test_classifier_export.py trained` 로 다시 쓴다).
fn load_trained(name: &str) -> (CategoryModel, Value) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny").join(name);
    let model = CategoryModel::load(Some(&dir)).expect("학습 결과 픽스처를 읽어야 한다");
    let report: Value = serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
    (model, report)
}

fn trained() -> (CategoryModel, Value) {
    load_trained("trained")
}

fn approx(a: f64, b: f64, abs: f64) -> bool {
    (a - b).abs() <= abs
}

fn ids(docs: &[Value]) -> HashSet<String> {
    docs.iter().map(|d| d["id"].as_str().unwrap().to_string()).collect()
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[test]
fn test_load_config_falls_back_to_defaults_when_file_is_missing_or_broken() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(load_config(&tmp.path().join("없는파일.json")), default_config());

    let broken = tmp.path().join("broken.json");
    fs::write(&broken, "{ 이건 JSON 이 아니다").unwrap();
    assert_eq!(load_config(&broken), default_config());
}

#[test]
fn test_merge_config_overrides_only_given_keys() {
    let merged = merge_config(&default_config(), &json!({"model": {"C": 4.0}}));
    assert_eq!(merged["model"]["C"], 4.0);
    // 같은 딕셔너리의 다른 값은 남아야 한다
    assert_eq!(merged["model"]["class_weight"], default_config()["model"]["class_weight"]);
    assert_eq!(merged["fields"], default_config()["fields"]);
}

#[test]
fn test_save_and_load_config_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("config.json");
    let cfg = merge_config(&default_config(), &json!({"decision": {"min_confidence": 0.77}}));
    save_config(&cfg, &path).unwrap();
    assert_eq!(load_config(&path)["decision"]["min_confidence"], 0.77);
}

#[test]
fn test_config_hash_changes_with_content() {
    let other = merge_config(&default_config(), &json!({"model": {"C": 2.0}}));
    assert_ne!(config_hash(&default_config()), config_hash(&other));
    assert_eq!(config_hash(&default_config()), config_hash(&default_config().clone()));
}

#[test]
fn test_enabled_fields_drops_disabled_blocks() {
    let cfg = merge_config(&default_config(), &json!({"fields": {"body_char": {"enabled": false}}}));
    assert!(enabled_fields(&cfg).get("body_char").is_none());
    assert!(enabled_fields(&cfg).get("body_word").is_some());
}

#[test]
fn test_resolve_parent_folds_series_into_parent_genre() {
    assert_eq!(resolve_parent(Some("2_을유세계문학전집")).as_deref(), Some("2_소설외국"));
    assert_eq!(resolve_parent(Some("3_판타지")).as_deref(), Some("3_판타지"));
    assert_eq!(resolve_parent(None), None);
    assert_eq!(resolve_parent(Some("")).as_deref(), Some(""));
}

// ---------------------------------------------------------------------------
// corpus
// ---------------------------------------------------------------------------

#[test]
fn test_document_takes_top_level_category_and_filename() {
    let doc = document(
        &json!({"_id": "12345", "_source": {"category": "9_북스캔OCR/completed/2024", "summary": "본문", "title": "제목", "author": "저자", "publisher": "출판사", "file_path": "/mnt/data/text/9_북스캔OCR/책.epub", "file_type": "epub"}}),
    );
    assert_eq!(doc["id"], "12345");
    assert_eq!(doc["cat"], "9_북스캔OCR");
    assert_eq!(doc["name"], "책.epub");
    assert_eq!(doc["publisher"], "출판사");
}

#[test]
fn test_document_tolerates_missing_fields() {
    let doc = document(&json!({"_id": 7, "_source": {}}));
    assert_eq!(doc, json!({"id": "7", "cat": "", "text": "", "title": "", "author": "", "publisher": "", "type": "", "name": "", "path": ""}));
}

#[test]
fn test_is_trainable() {
    let cases = [
        (json!({"cat": "3_무협", "text": "가".repeat(200)}), true),
        (json!({"cat": "3_무협", "text": "가".repeat(199)}), false),
        (json!({"cat": "", "text": "가".repeat(500)}), false),
        // 0_* 는 미분류/격리라 레이블이 아니다
        (json!({"cat": "0_nf", "text": "가".repeat(500)}), false),
        // 책이 아니라 페이지 이미지다
        (json!({"cat": "9_북스캔OCR", "text": "가".repeat(500)}), false),
        // 카테고리가 아니라 운영용 디렉토리다. 실측에서 789건이 섞여 들어왔다.
        (json!({"cat": "trash", "text": "가".repeat(500)}), false),
        (json!({"cat": "_root", "text": "가".repeat(500)}), false),
        (json!({"cat": ".preview_cache", "text": "가".repeat(500)}), false),
    ];
    for (doc, expected) in cases {
        assert_eq!(is_trainable(&doc, 200, &["9_북스캔OCR"]), expected, "case: cat={} len={}", doc["cat"], doc["text"].as_str().unwrap().chars().count());
    }
}

#[test]
fn test_absolute_path_resolves_paths_relative_to_the_library_root() {
    // loader 가 라이브러리 루트 기준 상대 경로로 색인하므로 그대로 열면 못 찾는다
    assert_eq!(absolute_path(&json!({"path": "3_무협/책.epub"}), "/mnt/data/text"), Some(PathBuf::from("/mnt/data/text/3_무협/책.epub")));
    assert_eq!(absolute_path(&json!({"path": "/절대/경로/책.epub"}), "/mnt/data/text"), Some(PathBuf::from("/절대/경로/책.epub")));
    assert_eq!(absolute_path(&json!({"path": ""}), "/mnt/data/text"), None);
    assert_eq!(absolute_path(&json!({}), "/mnt/data/text"), None);
}

#[test]
fn test_jsonl_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("sub").join("corpus.jsonl");
    let docs = make_docs(2);
    assert_eq!(write_jsonl(&docs, &path).unwrap(), docs.len());
    assert_eq!(read_jsonl(&path).unwrap(), docs);
}

#[test]
fn test_read_epub_publisher_reads_opf_and_tolerates_broken_files() {
    let tmp = TempDir::new().unwrap();
    let good = make_epub(&tmp.path().join("good.epub"), &EpubSpec::publisher("을유문화사"));
    assert_eq!(read_epub_publisher(&good), "을유문화사");

    let no_opf = tmp.path().join("no_opf.epub");
    write_zip(&no_opf, &[("a.txt", "내용".as_bytes())]);
    assert_eq!(read_epub_publisher(&no_opf), "");

    let broken = tmp.path().join("broken.epub");
    fs::write(&broken, b"not a zip").unwrap();
    assert_eq!(read_epub_publisher(&broken), "");
}

#[test]
fn test_attach_publishers_only_fills_missing_values_and_caches() {
    let tmp = TempDir::new().unwrap();
    // ES 에 들어 있는 모양대로 라이브러리 루트 기준 상대 경로를 준다
    make_epub(&tmp.path().join("book.epub"), &EpubSpec::publisher("열린책들"));
    let cache = tmp.path().join("publisher.json");
    let mut docs = vec![
        json!({"id": "1", "type": "epub", "path": "book.epub", "publisher": ""}),
        // ES 가 이미 값을 갖고 있으면 파일을 안 읽는다
        json!({"id": "2", "type": "epub", "path": "없는파일.epub", "publisher": "이미있음"}),
        // EPUB 이 아니면 dc:publisher 가 없다
        json!({"id": "3", "type": "txt", "path": "a.txt", "publisher": ""}),
    ];
    let stats = attach_publishers(&mut docs, &cache, tmp.path(), 1);
    assert_eq!(stats["read"], 1);
    assert_eq!(stats["with_publisher"], 2);
    assert_eq!(docs[0]["publisher"], "열린책들");
    assert_eq!(docs[1]["publisher"], "이미있음");
    assert_eq!(docs[2]["publisher"], "");
    assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&cache).unwrap()).unwrap(), json!({"1": "열린책들"}));

    // 두 번째 호출은 캐시만 쓰고 파일을 다시 안 읽는다
    fs::remove_file(tmp.path().join("book.epub")).unwrap();
    let mut again = vec![json!({"id": "1", "type": "epub", "path": "book.epub", "publisher": ""})];
    assert_eq!(attach_publishers(&mut again, &cache, tmp.path(), 1)["read"], 0);
    assert_eq!(again[0]["publisher"], "열린책들");
}

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
fn test_make_vectorizer_keeps_single_character_word_tokens() {
    let mut vec = make_vectorizer(&json!({"analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100}));
    // 기본 token_pattern 은 한 글자를 버린다. 한자 한 글자가 뜻을 갖는 문서가 있어 살려 둔다.
    assert!(vec.fit(&["天 地 人 무협"]).vocabulary().contains_key("天"));
}

fn word_field(source: &str, weight: f64) -> Value {
    json!({"source": source, "weight": weight, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0})
}

#[test]
fn test_feature_space_applies_per_field_weight() {
    let fields = json!({"body_word": word_field("text", 1.0)});
    let docs = vec![json!({"text": "무협 강호"}), json!({"text": "마법 던전"})];

    let plain = FeatureSpace::new(&fields).fit_transform(&docs).unwrap();
    let heavy_fields = json!({"body_word": word_field("text", 3.0)});
    let heavy = FeatureSpace::new(&heavy_fields).fit_transform(&docs).unwrap();

    assert_eq!(heavy.shape(), plain.shape());
    let (h, p) = (heavy.to_dense(), plain.to_dense());
    for (hr, pr) in h.iter().zip(&p) {
        for (a, b) in hr.iter().zip(pr) {
            assert!(approx(*a, b * 3.0, 1e-8), "heavy={a} plain*3={}", b * 3.0);
        }
    }
}

#[test]
fn test_feature_space_skips_fields_without_any_vocabulary() {
    let fields = json!({"body_word": word_field("text", 1.0), "publisher": word_field("publisher", 3.0)});
    let mut space = FeatureSpace::new(&fields);
    space.fit_transform(&[json!({"text": "무협", "publisher": ""}), json!({"text": "마법", "publisher": ""})]).unwrap();
    assert!(!space.has_vectorizer("publisher"));
    assert!(space.describe().get("body_word").is_some());
}

#[test]
fn test_feature_space_raises_when_no_field_yields_features() {
    let fields = json!({"publisher": word_field("publisher", 1.0)});
    assert!(FeatureSpace::new(&fields).fit_transform(&[json!({"publisher": ""}), json!({"publisher": ""})]).is_err());
}

#[test]
fn test_feature_space_transform_requires_fitted_vectorizers() {
    assert!(FeatureSpace::new(&json!({})).transform(&[json!({"text": "무협"})]).is_err());
}

// ---------------------------------------------------------------------------
// model
// ---------------------------------------------------------------------------

#[test]
fn test_softmax_confidence_temperature_flattens_probabilities() {
    let scores = vec![vec![3.0, 1.0, 0.0]];
    let sharp = &softmax_confidence(&scores, 0.5)[0];
    let flat = &softmax_confidence(&scores, 5.0)[0];
    assert!(approx(sharp.iter().sum::<f64>(), 1.0, 1e-8));
    assert!(sharp[0] > flat[0]);
}

#[test]
fn test_train_learns_the_three_categories() {
    let (model, report) = trained();
    let classes: HashSet<&str> = model.classes.iter().map(String::as_str).collect();
    assert_eq!(classes, VOCAB.iter().map(|(c, _)| *c).collect::<HashSet<_>>());
    assert!(report["top1_accuracy"].as_f64().unwrap() > 0.9);
    assert_eq!(model.meta["documents"], 120);
    assert!(!model.meta["config_hash"].as_str().unwrap_or("").is_empty());
    assert!(model.meta["blocks"].get("publisher").is_none() || model.meta["blocks"]["publisher"]["weight"] == 3.0);
}

#[test]
fn test_predict_returns_ranked_candidates_and_refuses_when_unsure() {
    let (model, _) = trained();
    let doc = json!({"text": vocab("3_무협"), "title": "무림 이야기", "author": "작가1", "publisher": "테스트출판사", "type": "epub", "name": "무림맹_1.epub"});

    let confident = model.predict_one(&doc, 0.0);
    assert_eq!(confident.category.as_deref(), Some("3_무협"));
    assert_eq!(confident.ranked[0].0, "3_무협");
    assert_eq!(confident.ranked.len(), 3);

    let refused = model.predict_one(&doc, 1.0);
    assert_eq!(refused.category, None);
    assert!(refused.reason.contains("판정하지 않음"));
}

#[test]
fn test_predict_on_empty_input_returns_empty_list() {
    let (model, _) = trained();
    assert!(model.predict(&[]).is_empty());
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

#[test]
fn test_default_model_is_cached_until_reset() {
    let mut env = EnvGuard::new();
    let tmp = TempDir::new().unwrap();
    let model_dir = tmp.path().join("model");
    env.set(MODEL_DIR_ENV, model_dir.to_str().unwrap());
    reset_default_model();

    assert!(get_default_model().is_none());
    fs::create_dir_all(&model_dir).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny");
    for entry in fs::read_dir(&fixture).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), model_dir.join(entry.file_name())).unwrap();
        }
    }
    assert!(get_default_model().is_none(), "negative result must stay cached until reset");

    let valid_model_dir = tmp.path().join("valid-model");
    fs::create_dir(&valid_model_dir).unwrap();
    for entry in fs::read_dir(&fixture).unwrap().flatten() {
        if entry.file_type().unwrap().is_file() {
            fs::copy(entry.path(), valid_model_dir.join(entry.file_name())).unwrap();
        }
    }
    env.set(MODEL_DIR_ENV, valid_model_dir.to_str().unwrap());
    let loaded = get_default_model().expect("reset 후 모델을 읽어야 한다");
    let cached = get_default_model().expect("캐시 모델이 있어야 한다");
    assert!(Arc::ptr_eq(&loaded, &cached));
    reset_default_model();
    let reloaded = get_default_model().expect("다시 reset한 뒤 모델을 읽어야 한다");
    assert!(!Arc::ptr_eq(&loaded, &reloaded));
    reset_default_model();
}

// ---------------------------------------------------------------------------
// training
// ---------------------------------------------------------------------------

#[test]
fn test_filter_by_size_drops_small_categories() {
    let mut docs = vec![json!({"cat": "3_무협"}); 10];
    docs.extend(vec![json!({"cat": "9_격언명언"}); 2]);
    let (kept, dropped) = filter_by_size(&docs, 5);
    assert_eq!(dropped, vec!["9_격언명언"]);
    assert_eq!(kept.len(), 10);
}

#[test]
fn test_filter_by_size_keeps_at_least_two_per_category_for_stratification() {
    let mut docs = vec![json!({"cat": "3_무협"}); 3];
    docs.push(json!({"cat": "3_판타지"}));
    let (kept, dropped) = filter_by_size(&docs, 1);
    // 층화 분할이 성립하려면 최소 2건이 있어야 한다. 1을 줘도 2로 올린다.
    assert_eq!(dropped, vec!["3_판타지"]);
    assert_eq!(kept.len(), 3);
}

#[test]
fn test_coverage_curve_precision_rises_as_coverage_falls() {
    // 확신도가 높은 쪽이 맞고 낮은 쪽이 틀리게 둔다
    let correct = [true, true, true, false, false];
    let confidence = [0.99, 0.95, 0.9, 0.5, 0.4];
    let curve = coverage_curve(&correct, &confidence, &[0.5, 1.0]);
    assert_eq!(curve[0]["precision"], 1.0);
    assert_eq!(curve[1]["precision"], 0.6);
}

#[test]
fn test_max_coverage_at_finds_the_widest_band_meeting_the_target() {
    let correct = [true, true, true, true, false];
    let confidence = [0.99, 0.95, 0.9, 0.85, 0.4];
    let best = max_coverage_at(&correct, &confidence, 0.9);
    assert!(approx(best["coverage"].as_f64().unwrap(), 0.8, 1e-6));
    assert_eq!(best["precision"], 1.0);
}

#[test]
fn test_max_coverage_at_returns_zero_when_target_is_unreachable() {
    let correct = [false, false];
    assert_eq!(max_coverage_at(&correct, &[0.9, 0.8], 0.9)["coverage"], 0.0);
}

#[test]
fn test_evaluate_accepts_parent_genre_as_correct() {
    let (model, _) = trained();
    let docs = vec![json!({"cat": "3_무협", "text": vocab("3_무협"), "title": "", "author": "", "publisher": "", "type": "epub", "name": "무림맹_1.epub"})];

    let strict = evaluate(&model, &docs, None);
    // 하위 카테고리를 상위 장르로 접으면 같은 예측이 정답이 된다
    let folded = evaluate(&model, &docs, Some(&HashMap::from([("3_무협".to_string(), "3_무협".to_string())])));
    assert_eq!(strict["n"], folded["n"]);
    assert_eq!(strict["n"], 1);
}

#[test]
fn test_train_raises_when_every_category_is_too_small() {
    let config = merge_config(&default_config(), &json!({"corpus": {"min_per_category": 1000}}));
    assert!(train(&mut make_docs(3), &config, TrainOptions::default()).is_err());
}

#[test]
fn test_format_report_shows_curve_and_both_targets() {
    let (_, report) = trained();
    let text = format_report(&report);
    assert!(text.contains("홀드아웃"));
    assert!(text.contains("정답률 90% 유지 최대 판정률"));
    assert!(text.contains("정답률 95% 유지 최대 판정률"));
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
fn test_explicit_threshold_wins_over_the_calibrated_one() {
    let (mut model, _) = trained();
    model.config = merge_config(&model.config, &json!({"decision": {"min_confidence": 0.77}}));
    assert_eq!(model.min_confidence(), 0.77);
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
fn test_model_path_follows_the_environment_variable() {
    let mut env = EnvGuard::new();
    env.remove(MODEL_DIR_ENV);
    let expected_default = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("classifier/export");
    assert_eq!(PathBuf::from(DEFAULT_MODEL_PATH), expected_default);
    assert_eq!(model_path(), PathBuf::from(DEFAULT_MODEL_PATH));

    // pod 는 /books 로, 학습 머신은 /mnt/data/text 로 같은 파일을 본다. 복사가 없다.
    env.set(MODEL_DIR_ENV, "/books/.classifier/export");
    assert_eq!(model_path(), PathBuf::from("/books/.classifier/export"));
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

// ---------------------------------------------------------------------------
// 홀드아웃 분할 - 코퍼스가 바뀌어도 소속이 안 바뀐다
// ---------------------------------------------------------------------------

fn docs_n(n: usize, cat: &str, start: usize) -> Vec<Value> {
    (start..start + n).map(|i| json!({"id": i.to_string(), "cat": cat, "text": "x", "name": format!("{i}.txt")})).collect()
}

#[test]
fn test_split_holdout_keeps_the_requested_ratio() {
    let (train, hold) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 20260914);

    assert_eq!(hold.len(), 200);
    assert_eq!(train.len(), 800);
    assert!(ids(&train).intersection(&ids(&hold)).next().is_none());
}

#[test]
fn test_split_holdout_is_stable_when_documents_disappear() {
    // 코퍼스가 줄어도 남은 문서의 소속은 그대로여야 한다.
    //
    // 예전 위치 인덱스 분할은 문서 142건이 빠지자 홀드아웃이 통째로 뒤바뀌었다.
    // 그래서 재현한 홀드아웃의 80%가 학습 문서였고 정답률이 86.5% -> 93.8% 로 떴다.
    let docs = docs_n(1000, "3_SF", 0);
    let (_, before) = split_holdout(&docs, 0.2, 20260914);
    let survivors: Vec<Value> = docs.iter().filter(|d| d["id"].as_str().unwrap().parse::<u64>().unwrap() % 7 != 0).cloned().collect();
    let (_, after) = split_holdout(&survivors, 0.2, 20260914);

    let before_ids: HashSet<String> = before.iter().filter(|d| d["id"].as_str().unwrap().parse::<u64>().unwrap() % 7 != 0).map(|d| d["id"].as_str().unwrap().to_string()).collect();
    let after_ids = ids(&after);
    // 비율을 맞추느라 경계 몇 건은 움직인다. 대부분은 그대로여야 한다.
    let kept_ratio = before_ids.intersection(&after_ids).count() as f64 / before_ids.len() as f64;
    assert!(kept_ratio > 0.95, "kept_ratio={kept_ratio}");
}

#[test]
fn test_split_holdout_is_stable_when_documents_are_added() {
    let (_, before) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 20260914);
    let (_, after) = split_holdout(&docs_n(1200, "3_SF", 0), 0.2, 20260914);

    let (before_ids, after_ids) = (ids(&before), ids(&after));
    assert!(before_ids.intersection(&after_ids).count() as f64 / before_ids.len() as f64 > 0.95);
}

#[test]
fn test_split_holdout_keeps_every_category_represented() {
    // 카테고리 크기가 78,520 대 30 까지 벌어진다. 작은 쪽이 통째로 빠지면 안 된다.
    let mut docs = docs_n(5000, "2_소설한국", 0);
    docs.extend(docs_n(30, "1_문헌서지", 90000));
    let (train, hold) = split_holdout(&docs, 0.2, 20260914);

    let cats = |v: &[Value]| v.iter().map(|d| d["cat"].as_str().unwrap().to_string()).collect::<HashSet<_>>();
    let want: HashSet<String> = ["2_소설한국", "1_문헌서지"].iter().map(|s| s.to_string()).collect();
    assert_eq!(cats(&hold), want);
    assert_eq!(cats(&train), want);
}

#[test]
fn test_split_holdout_never_empties_either_side_for_tiny_categories() {
    let (train, hold) = split_holdout(&docs_n(2, "3_SF", 0), 0.2, 20260914);

    assert_eq!(hold.len(), 1);
    assert_eq!(train.len(), 1);
}

#[test]
fn test_split_holdout_changes_with_the_seed() {
    let (_, a) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 1);
    let (_, b) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 2);

    assert_ne!(ids(&a), ids(&b));
}

/// 가중치, analyzer, 빈 필드가 섞여야 합치는 순서와 열 오프셋이 검증된다.
///
/// `text` 를 보는 필드를 셋 두되 뒤로 갈수록 요구 길이를 늘렸다. 원문을 줄일 때
/// '남은 필드 중 가장 긴 요구'가 아니라 '바로 다음 필드'만 보면 여기서 깨진다.
fn spill_fields() -> Value {
    json!({
        "body_word": {"source": "text", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 50, "max_chars": 0},
        "body_char": {"source": "text", "weight": 0.3, "analyzer": "char_wb", "ngram": [2, 3], "min_df": 1, "max_features": 80, "max_chars": 8},
        "body_tail": {"source": "text", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 2], "min_df": 1, "max_features": 60, "max_chars": 18},
        "title": {"source": "title", "weight": 3.0, "analyzer": "char_wb", "ngram": [2, 2], "min_df": 1, "max_features": 40, "max_chars": 0},
        "publisher": {"source": "publisher", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 10, "max_chars": 0},
    })
}

fn spill_docs() -> Vec<Value> {
    vec![
        json!({"text": "무협 강호 절세 고수", "title": "천룡팔부", "publisher": ""}),
        json!({"text": "마법 던전 용사 모험", "title": "던전 일지", "publisher": ""}),
        json!({"text": "", "title": "빈 본문", "publisher": ""}),
        json!({"text": "무협 마법 혼합 세계", "title": "혼합", "publisher": ""}),
    ]
}

#[test]
fn test_feature_space_spill_matches_in_memory_matrix() {
    // 디스크로 흘려보내고 다시 합친 행렬은 한 번에 만든 것과 같아야 한다.
    //
    // 합치기를 직접 구현했다. 열 오프셋이나 행 경계가 한 칸만 어긋나도 모델이
    // 조용히 엉뚱한 특징을 학습한다. 같은 자리 같은 값인지 값으로 못 박는다.
    let tmp = TempDir::new().unwrap();
    let (fields, docs) = (spill_fields(), spill_docs());
    let plain = FeatureSpace::new(&fields).fit_transform_with(&docs, FitOptions { spill_dir: None, dtype: Dtype::F64 }).unwrap();
    let spilled = FeatureSpace::new(&fields).fit_transform_with(&docs, FitOptions { spill_dir: Some(tmp.path().to_path_buf()), dtype: Dtype::F64 }).unwrap();

    assert_eq!(spilled.shape(), plain.shape());
    assert_eq!(spilled.dtype(), Dtype::F64);
    assert_eq!(spilled.nnz(), plain.nnz());
    assert_eq!(spilled.to_dense(), plain.to_dense());
    assert!(fs::read_dir(tmp.path()).unwrap().next().is_some(), "spill 행렬은 사용 중 파일로 유지돼야 한다");
    drop(spilled);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 0, "spill 파일은 행렬 수명 종료 때 제거한다");
}

#[test]
fn test_feature_space_lazy_rows_match_materialized_rows() {
    let tmp = TempDir::new().unwrap();
    let (fields, docs) = (spill_fields(), spill_docs());
    let selected = [0usize, 1, 3];
    let materialized: Vec<Value> = selected.iter().map(|&index| docs[index].clone()).collect();
    let expected = FeatureSpace::new(&fields).fit_transform_with(&materialized, FitOptions { spill_dir: Some(tmp.path().join("expected")), dtype: Dtype::F64 }).unwrap();
    let actual =
        FeatureSpace::new(&fields).fit_transform_selected_by(&selected, FitOptions { spill_dir: Some(tmp.path().join("actual")), dtype: Dtype::F64 }, |index| Ok(docs[index].clone())).unwrap();

    assert_eq!(actual.shape(), expected.shape());
    assert_eq!(actual.nnz(), expected.nnz());
    assert_eq!(actual.to_dense(), expected.to_dense());
}

#[test]
fn test_feature_space_spill_keeps_same_block_layout() {
    // 스필 경로도 벡터라이저와 블록 크기를 똑같이 남겨야 판정 때 transform 이 맞는다.
    let tmp = TempDir::new().unwrap();
    let (fields, docs) = (spill_fields(), spill_docs());
    let (mut plain, mut spilled) = (FeatureSpace::new(&fields), FeatureSpace::new(&fields));
    plain.fit_transform(&docs).unwrap();
    spilled.fit_transform_with(&docs, FitOptions { spill_dir: Some(tmp.path().to_path_buf()), ..Default::default() }).unwrap();

    assert_eq!(spilled.vectorizer_names(), plain.vectorizer_names());
    assert_eq!(spilled.block_sizes(), plain.block_sizes());
    // publisher 는 전부 빈 문자열이라 양쪽 모두에서 빠져야 한다.
    assert!(!spilled.has_vectorizer("publisher"));
}

#[test]
fn test_feature_space_spill_raises_when_no_field_yields_features() {
    let tmp = TempDir::new().unwrap();
    let fields = json!({"publisher": word_field("publisher", 1.0)});
    let result = FeatureSpace::new(&fields).fit_transform_with(&[json!({"publisher": ""}), json!({"publisher": ""})], FitOptions { spill_dir: Some(tmp.path().to_path_buf()), ..Default::default() });
    assert!(result.is_err());
}

fn spill_train_config() -> Value {
    merge_config(
        &default_config(),
        &json!({"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"min_df": 1, "max_features": 5000}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}}, "holdout": 0.25}),
    )
}

#[test]
fn test_train_with_spill_gives_the_same_model_as_without() {
    // 스필은 메모리를 아끼는 길일 뿐, 성적이 달라지면 안 된다.
    //
    // 분할이 문서 키 해시라 두 번 돌려도 같은 홀드아웃이 나온다. 그래서 같은 문서를
    // 같은 수만큼 재고, 정답률도 같아야 한다.
    //
    // 임계값까지 소수점 끝자리로 비교하지는 않는다. `LinearSVC` 는 `random_state` 가
    // 없어 dual 좌표하강의 셔플이 실행마다 달라진다. 스필 없이 두 번 돌려도 임계값
    // 끝자리는 어긋난다(실측 0.7830560 대 0.7830587). 행렬이 같은지는
    // `test_feature_space_spill_matches_in_memory_matrix` 가 값으로 못 박는다.
    let tmp = TempDir::new().unwrap();
    let config = spill_train_config();
    let (_, plain) = train(&mut make_docs(40), &config, train_opts()).unwrap();
    let (_, spilled) = train(&mut make_docs(40), &config, TrainOptions { spill_dir: Some(tmp.path().join("spill")), ..train_opts() }).unwrap();

    assert_eq!(spilled["n"], plain["n"]);
    assert_eq!(spilled["top1_accuracy"], plain["top1_accuracy"]);
    for (got, want) in spilled["curve"].as_array().unwrap().iter().zip(plain["curve"].as_array().unwrap()) {
        assert_eq!(got["coverage"], want["coverage"]);
        assert_eq!(got["precision"], want["precision"]);
        let (g, w) = (got["threshold"].as_f64().unwrap(), want["threshold"].as_f64().unwrap());
        assert!((g - w).abs() <= w.abs() * 1e-3, "threshold got={g} want={w}");
    }
    assert_eq!(spilled["at_precision_90"]["coverage"], plain["at_precision_90"]["coverage"]);
}

#[test]
fn test_train_with_spill_releases_train_text_but_keeps_the_holdout() {
    // 원문 해제가 홀드아웃까지 건드리면 평가가 조용히 빈 문서로 돌아간다.
    let tmp = TempDir::new().unwrap();
    let mut docs = make_docs(40);
    let (model, report) = train(&mut docs, &spill_train_config(), TrainOptions { spill_dir: Some(tmp.path().join("spill")), ..train_opts() }).unwrap();

    let with_text = docs.iter().filter(|d| d.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty())).count();
    assert_eq!(with_text as u64, report["n"].as_u64().unwrap(), "본문이 남은 문서 수는 홀드아웃 크기와 같아야 한다");
    assert!(docs.iter().all(|d| d.get("cat").and_then(Value::as_str).is_some_and(|c| !c.is_empty())), "레이블은 해제하면 안 된다");
    // 해제 뒤에도 홀드아웃 평가가 성립했는지 값으로 확인한다.
    assert!(report["top1_accuracy"].as_f64().unwrap() > 0.9);
    assert_eq!(model.meta["documents"].as_u64().unwrap() as usize, docs.len());
}

#[test]
fn test_train_without_spill_leaves_the_documents_untouched() {
    let mut docs = make_docs(40);
    train(&mut docs, &spill_train_config(), train_opts()).unwrap();
    assert!(docs.iter().all(|d| d.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty())));
}

#[test]
fn test_shrink_sources_keeps_the_longest_remaining_need() {
    // 남은 필드 중 가장 긴 요구만큼은 남겨야 한다. 짧은 쪽에 맞추면 특징이 사라진다.
    let mut docs = vec![json!({"text": "가나다라마바사아자차카타파하", "title": "제목", "cat": "3_무협"})];
    let remaining = [("body_char", json!({"source": "text", "max_chars": 5})), ("body_tail", json!({"source": "text", "max_chars": 9})), ("title", json!({"source": "title", "max_chars": 0}))];
    shrink_sources(&mut docs, &["text", "title"], &remaining);
    assert_eq!(docs[0]["text"], "가나다라마바사아자");
    assert_eq!(docs[0]["title"], "제목", "max_chars 0 은 전부 필요하다는 뜻이다");
    assert_eq!(docs[0]["cat"], "3_무협", "원문이 아닌 키는 건드리면 안 된다");
}

#[test]
fn test_shrink_sources_drops_a_source_no_field_reads_any_more() {
    let mut docs = vec![json!({"text": "본문", "title": "제목", "cat": "3_무협"})];
    shrink_sources(&mut docs, &["text", "title"], &[("title", json!({"source": "title", "max_chars": 0}))]);
    assert!(docs[0].get("text").is_none());
    assert_eq!(docs[0]["title"], "제목");
}

#[test]
fn test_shrink_sources_leaves_the_filename_alone() {
    // `field_text` 는 숫자를 지운 뒤에 자른다. 먼저 자르면 결과가 달라진다.
    let mut docs = vec![json!({"name": "무협 1234 강호 5678 협객.epub"})];
    shrink_sources(&mut docs, &["name"], &[("filename", json!({"source": "name", "max_chars": 5}))]);
    assert_eq!(docs[0]["name"], "무협 1234 강호 5678 협객.epub");
}

#[test]
fn test_rss_note_is_empty_when_proc_status_is_unreadable() {
    // 가정한 seam: /proc 상태 파일 경로를 인자로 받는 `rss_note_with`. 읽을 수 없는 경로를 준다. (전역 `set_proc_status_path` 대체)
    assert_eq!(rss_note_with(&PathBuf::from("/no/such/proc/status")), "");
}

#[test]
fn test_train_logs_dropped_small_categories() {
    let config = merge_config(&default_config(), &json!({"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": small_fields(), "holdout": 0.25}));
    let mut docs = make_docs(40);
    docs.push(json!({"id": "tiny-0", "cat": "9_희귀", "text": "희귀 문서", "title": "희귀", "author": "작가", "publisher": "출판사", "type": "txt", "name": "희귀.txt", "path": "/x/희귀.txt"}));
    // caplog.at_level(INFO). 프로덕션은 `log` crate만 쓴다.
    let logs = LogCapture::start(log::Level::Info);
    let (model, _) = train(&mut docs, &config, train_opts()).unwrap();
    assert!(!model.classes.iter().any(|c| c == "9_희귀"));
    assert!(logs.messages().iter().any(|m| m.contains("표본 부족으로 제외한 카테고리 1개")));
}
