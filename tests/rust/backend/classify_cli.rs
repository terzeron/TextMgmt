//! 원본: tests/test_classify_cli.py
//!
//! 자동분류 CLI는 운영 표면이라 한 번 터지면 긴 작업이 통째로 날아간다. 명령마다 최소 한 번은 실제로 돌린다.
//!
//! 가정한 API (아직 구현 전이다. 설계: docs/rust-test-seams.md 3.12 "classify CLI: 전역 가드 15종을 Deps 하나로", 3.10, 3.13):
//! - `utils::classify_cli::main_with(args: &[&str], io: &mut CliIo, deps: &ClassifyCliDeps) -> Result<i32, SystemExit>`  (Python `main` 반환값과 `SystemExit`)
//!   - `main(args)`는 `main_with(args, &mut CliIo::stdio(), &ClassifyCliDeps::system())`다. 구 `capture_stdout`/`capture_logs`와 `testing::override_*`는 없다.
//! - `utils::classify_cli::CliIo<'a> { pub out: &'a mut dyn Write, pub err: &'a mut dyn Write }`  (capsys 대응. 표준 출력은 `out`으로 캡처한다)
//! - `utils::classify_cli::SystemExit { pub code: i32, pub message: String }` (`Display`는 message)
//! - `utils::classify_cli::commands() -> Vec<&'static str>`  (Python `COMMANDS` 키)
//! - `utils::classify_cli::ClassifyCliDeps`: `system()`과 소비형 `with_*` 빌더. 전역 가드 15종을 아래 7개 주입 지점으로 묶는다.
//!   - `with_config_store(Arc<dyn ConfigStore>)`: 구 `load_config`, `save_config`. `ConfigStore { fn load_config(&self, path: &Path) -> Value; fn save_config(&self, cfg: &Value) -> PathBuf }`
//!   - `with_es_factory(Arc<dyn Fn(&str) -> Arc<dyn EsManagerApi> + Send + Sync>)`: 구 `es_manager`, `es_manager_ctor`. `deps.es_manager(Option<&str>) -> Arc<dyn EsManagerApi>`가 인덱스 이름(없으면 빈 문자열)을 넘긴다.
//!   - `with_corpus(Arc<dyn CorpusOps>)`: 구 `collect`, `attach_publishers`, `evaluate`, `format_report`, `fit_confidence_calibration`, `holdout_docs`.
//!     `CorpusOps`의 메서드는 모두 기본 구현을 가진다(바꿔치기하는 것만 구현한다).
//!   - `with_service_factory(Arc<dyn Fn() -> Arc<dyn BookstoreQuery> + Send + Sync>)`: 구 `book_classifier_service`. `BookstoreQuery { fn query_bookstores(&self, search_title: &str, raw_author: &str, raw_title: &str) -> (Value, Value, Value) }`
//!   - `with_fs(Arc<dyn sysdeps::FsOps>)`: 구 `move`(`rename`), `iter_files`(`read_dir`)
//!   - `with_clock(Arc<dyn sysdeps::Clock>)`: 구 `time`
//!   - `with_boundary_factor(f64)`: 구 `boundary_factor` (값 설정자)
//! - `utils::classify_cli::{load_model(&Path), load_corpus(&Path), publisher_cache_path(&Path) -> PathBuf, overrides(&Value) -> Value, holdout_manifest_path(&Path) -> PathBuf, holdout_docs(&[Value], &Value, Option<&Path>) -> Vec<Value>}`
//! - `backend::classifier::config::{default_config() -> Value, series_to_parent() -> HashMap<String, String>, merge_config(&Value, &Value) -> Value}`
//! - `backend::classifier::corpus::{write_jsonl(&[Value], &Path), read_jsonl(&Path) -> Vec<Value>}`
//! - `backend::classifier::training::{train(&[Value], &Value, Option<&HashMap<String, String>>) -> Result<(CategoryModel, Value), ValueError>, document_key(&Value) -> String}`
//! - `backend::classifier::model::CategoryModel::{load(&Path) -> Result<Self, _>, save(&self, &Path), meta: Value, min_confidence: f64}`
//! - 대역: `common::es_fake::FakeEsManager`(`EsManagerApi`), `common::faulty_fs::FaultyFs`(`FsOps`), `common::log_capture::LogCapture`(caplog).
//!
//! 학습이 필요한 테스트는 `#[ignore = "sklearn 대응 모델 필요"]`다. 전역 상태 재정의가 사라져 직렬 실행(`--test-threads=1`) 가정은 필요 없다.

mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use backend::classifier::config::{default_config, merge_config, series_to_parent};
use backend::classifier::corpus::{read_jsonl, write_jsonl};
use backend::classifier::model::CategoryModel;
use backend::classifier::training::{document_key, train};
use backend::es_manager::EsManagerApi;
use common::{es_fake::FakeEsManager, faulty_fs::FaultyFs, log_capture::LogCapture};
use serde_json::{Value, json};
use tempfile::TempDir;
use utils::classify_cli::{self, BookstoreQuery, ClassifyCliDeps, CliIo, ConfigStore, CorpusOps, SystemExit};

const CATEGORIES: [(&str, &str); 3] =
    [("3_무협", "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객"), ("3_판타지", "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕"), ("5_수학과학일반", "미분 적분 함수 벡터 행렬 확률 통계 방정식")];

fn vocab(category: &str) -> &'static str {
    CATEGORIES.iter().find(|(c, _)| *c == category).unwrap().1
}

/// tests/test_classifier.py의 `make_docs()`. 카테고리마다 40건, 문서마다 어순을 바꾼다.
fn make_docs() -> Vec<Value> {
    let mut docs = Vec::new();
    for (cat, words) in CATEGORIES {
        let tokens: Vec<&str> = words.split(' ').collect();
        for i in 0..40 {
            let k = i % tokens.len();
            let shifted: Vec<&str> = tokens[k..].iter().chain(tokens[..k].iter()).copied().collect();
            let text = shifted.join(" ");
            let text = [text.clone(), text.clone(), text].join(" ");
            let ext = if i % 2 == 0 { "epub" } else { "txt" };
            docs.push(json!({
                "id": format!("{cat}-{i}"), "cat": cat, "text": text, "title": format!("{} 이야기 {i}", tokens[k]), "author": format!("작가{}", i % 5),
                "publisher": "테스트출판사", "type": ext, "name": format!("{}_{i}.epub", tokens[k]), "path": format!("/mnt/data/text/{cat}/{i}.epub"),
            }));
        }
    }
    docs
}

/// 학습을 몇 초 안에 끝내는 설정. 어휘가 갈려 있어 판정은 실제로 배운다.
fn test_config() -> Value {
    let min_df = json!({"min_df": 1});
    merge_config(
        &default_config(),
        &json!({
            "corpus": {"min_per_category": 5}, "model": {"n_jobs": 1},
            "fields": {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": false}, "filename": min_df, "title": min_df, "author": min_df, "publisher": min_df, "file_type": min_df},
            "holdout": 0.25,
        }),
    )
}

/// 서점 조회를 대신한다. 네트워크를 타지 않고 1초 간격도 안 쉰다.
struct FakeBookstoreService {
    answers: Vec<(String, String, String)>,
    calls: AtomicUsize,
}

impl FakeBookstoreService {
    fn new(answers: &[(&str, &str, &str)]) -> Arc<Self> {
        Arc::new(Self { answers: answers.iter().map(|(y, a, k)| (y.to_string(), a.to_string(), k.to_string())).collect(), calls: AtomicUsize::new(0) })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl BookstoreQuery for FakeBookstoreService {
    fn query_bookstores(&self, _search_title: &str, _raw_author: &str, _raw_title: &str) -> (Value, Value, Value) {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (y, a, k) = &self.answers[n % self.answers.len()];
        (json!({"mapped": y}), json!({"mapped": a}), json!({"mapped": k}))
    }
}

/// `load_config`만 바꿔치기하는 `ConfigStore`. 학습 설정을 시험용으로 고정한다.
// TODO(common): classify CLI 대역(`FixedConfig`, `StubCorpus`)은 classify_cli와 classify_cli_gaps가 따로 갖는다. 필요하면 common으로 올린다.
struct FixedConfig(Value);

impl ConfigStore for FixedConfig {
    fn load_config(&self, _path: &Path) -> Value {
        self.0.clone()
    }
}

type CollectFn = Box<dyn Fn(&dyn EsManagerApi, &Path) -> Value + Send + Sync>;
type AttachFn = Box<dyn Fn(&mut Vec<Value>, &Path, &Path, usize) -> Value + Send + Sync>;
type EvaluateFn = Box<dyn Fn(&CategoryModel, &[Value], Option<&HashMap<String, String>>) -> Value + Send + Sync>;
type ReportFn = Box<dyn Fn(&Value) -> String + Send + Sync>;
type CalibrationFn = Box<dyn Fn() -> Value + Send + Sync>;

/// 구 `testing::override_{collect, attach_publishers, evaluate, format_report, fit_confidence_calibration}`. 쓰는 메서드만 채운다.
#[derive(Default)]
struct StubCorpus {
    collect: Option<CollectFn>,
    attach_publishers: Option<AttachFn>,
    evaluate: Option<EvaluateFn>,
    format_report: Option<ReportFn>,
    fit_confidence_calibration: Option<CalibrationFn>,
}

impl StubCorpus {
    fn with_collect(mut self, f: impl Fn(&dyn EsManagerApi, &Path) -> Value + Send + Sync + 'static) -> Self {
        self.collect = Some(Box::new(f));
        self
    }

    fn with_attach_publishers(mut self, f: impl Fn(&mut Vec<Value>, &Path, &Path, usize) -> Value + Send + Sync + 'static) -> Self {
        self.attach_publishers = Some(Box::new(f));
        self
    }

    fn with_evaluate(mut self, f: impl Fn(&CategoryModel, &[Value], Option<&HashMap<String, String>>) -> Value + Send + Sync + 'static) -> Self {
        self.evaluate = Some(Box::new(f));
        self
    }

    fn with_format_report(mut self, f: impl Fn(&Value) -> String + Send + Sync + 'static) -> Self {
        self.format_report = Some(Box::new(f));
        self
    }

    fn with_fit_confidence_calibration(mut self, f: impl Fn() -> Value + Send + Sync + 'static) -> Self {
        self.fit_confidence_calibration = Some(Box::new(f));
        self
    }
}

impl CorpusOps for StubCorpus {
    fn collect(&self, es: &dyn EsManagerApi, out_path: &Path) -> Value {
        (self.collect.as_ref().expect("collect를 부르면 안 된다"))(es, out_path)
    }

    fn attach_publishers(&self, docs: &mut Vec<Value>, cache_path: &Path, library_root: &Path, workers: usize) -> Value {
        (self.attach_publishers.as_ref().expect("attach_publishers를 부르면 안 된다"))(docs, cache_path, library_root, workers)
    }

    fn evaluate(&self, model: &CategoryModel, docs: &[Value], accept_parent: Option<&HashMap<String, String>>) -> Value {
        (self.evaluate.as_ref().expect("evaluate를 부르면 안 된다"))(model, docs, accept_parent)
    }

    fn format_report(&self, report: &Value) -> String {
        (self.format_report.as_ref().expect("format_report를 부르면 안 된다"))(report)
    }

    fn fit_confidence_calibration(&self) -> Value {
        (self.fit_confidence_calibration.as_ref().expect("fit_confidence_calibration을 부르면 안 된다"))()
    }
}

/// `es_manager` 바꿔치기. 인덱스 이름을 `sink`에 쌓고 `FakeEsManager`를 돌려준다. `sink`를 안 주면 기록하지 않는다.
fn fake_es_factory(sink: Option<Arc<Mutex<Vec<String>>>>) -> Arc<dyn Fn(&str) -> Arc<dyn EsManagerApi> + Send + Sync> {
    Arc::new(move |index_name: &str| -> Arc<dyn EsManagerApi> {
        if let Some(sink) = &sink {
            sink.lock().unwrap().push(index_name.to_string());
        }
        FakeEsManager::new()
    })
}

/// `FakeBookstoreService`를 서비스 공장으로 감싼다.
fn bookstore_factory(service: &Arc<FakeBookstoreService>) -> Arc<dyn Fn() -> Arc<dyn BookstoreQuery> + Send + Sync> {
    let service = service.clone();
    Arc::new(move || service.clone() as Arc<dyn BookstoreQuery>)
}

/// `model_file` fixture. 학습한 모델과 그 학습 데이터를 파일로 남긴다. CLI는 파일만 받는다.
fn model_file(dir: &TempDir) -> (PathBuf, PathBuf) {
    common::setup_env();
    let docs = make_docs();
    let corpus = dir.path().join("corpus.jsonl");
    write_jsonl(&docs, &corpus);
    let (model, _) = train(&docs, &test_config(), Some(&series_to_parent())).unwrap();
    let path = dir.path().join("model.joblib");
    model.save(&path);
    (path, corpus)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `main_with(args, &mut CliIo, deps)`를 돌리고 (결과, stdout)을 돌려준다. capsys 대응.
fn run_with(args: &[&str], deps: &ClassifyCliDeps) -> (Result<i32, SystemExit>, String) {
    let (mut out, mut err) = (Vec::<u8>::new(), Vec::<u8>::new());
    let result = classify_cli::main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (result, String::from_utf8_lossy(&out).into_owned())
}

/// 실제 구현(`ClassifyCliDeps::system()`)으로 돌린다.
fn run(args: &[&str]) -> (Result<i32, SystemExit>, String) {
    run_with(args, &ClassifyCliDeps::system())
}

fn write_text(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// 인자 파서
// ---------------------------------------------------------------------------

#[test]
fn test_every_command_prints_help() {
    for command in classify_cli::commands() {
        let (result, out) = run(&[command, "-h"]);
        let exit = result.expect_err(&format!("{command}: -h는 SystemExit이어야 한다"));
        assert_eq!(exit.code, 0, "{command}");
        assert!(!out.trim().is_empty(), "{command}: 도움말이 비어 있다");
    }
}

#[test]
fn test_no_command_prints_help_and_fails() {
    assert_eq!(run(&[]).0.unwrap(), 1);
}

// ---------------------------------------------------------------------------
// 명령 실행
// ---------------------------------------------------------------------------

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_info_prints_training_metadata() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let (result, out) = run(&["info", "--model", &s(&path)]);
    assert_eq!(result.unwrap(), 0);
    assert!(out.contains("학습 시각"));
    assert!(out.contains("학습 문서  120건 / 카테고리 3개"));
    assert!(out.contains("판정 임계값"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_evaluate_prints_the_coverage_curve() {
    let dir = TempDir::new().unwrap();
    let (path, corpus) = model_file(&dir);
    let (result, out) = run(&["evaluate", "--model", &s(&path), "--corpus", &s(&corpus)]);
    assert_eq!(result.unwrap(), 0);
    assert!(out.contains("판정률"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_evaluate_uses_the_model_holdout_manifest() {
    let dir = TempDir::new().unwrap();
    let (model_path, corpus) = model_file(&dir);
    let selected = read_jsonl(&corpus).last().unwrap().clone();
    fs::write(classify_cli::holdout_manifest_path(&model_path), json!({"document_keys": [document_key(&selected)]}).to_string()).unwrap();
    let evaluated = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = evaluated.clone();
    let corpus_ops = StubCorpus::default()
        .with_evaluate(move |_model, docs, _accept_parent| {
            sink.lock().unwrap().extend(docs.iter().cloned());
            json!({})
        })
        .with_format_report(|_report| "holdout report".to_string());
    let deps = ClassifyCliDeps::system().with_corpus(Arc::new(corpus_ops));

    let (result, out) = run_with(&["evaluate", "--model", &s(&model_path), "--corpus", &s(&corpus)], &deps);
    assert_eq!(result.unwrap(), 0);
    let ids: Vec<Value> = evaluated.lock().unwrap().iter().map(|d| d["id"].clone()).collect();
    assert_eq!(ids, vec![selected["id"].clone()]);
    assert!(out.contains("holdout report"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_bookstore_policy_runs_to_the_end_and_writes_the_threshold() {
    // 서점 조회부터 집계·저장까지 실제로 통과시킨다. 이 경로가 `Counter` 하나가 없어 마지막 줄에서 NameError로 죽은 적이 있다.
    let dir = TempDir::new().unwrap();
    // 두 서점이 같은 카테고리를 내면 다수결이 성립한다. 세 번째는 응답 없음이다.
    let service = FakeBookstoreService::new(&[("3_무협", "3_무협", ""), ("3_판타지", "", ""), ("", "", "")]);
    let deps = ClassifyCliDeps::system().with_service_factory(bookstore_factory(&service));

    let (path, corpus) = model_file(&dir);
    let out = dir.path().join("policy.json");
    // 시험용 모델은 어휘가 뚜렷해 거부되는 문서가 없다. 집계 경로를 보려는 테스트라 전체에서 뽑는다.
    let (result, printed) = run_with(&["bookstore-policy", "--model", &s(&path), "--corpus", &s(&corpus), "--sample", "12", "--delay", "0", "--sample-from", "all", "--out", &s(&out)], &deps);
    assert_eq!(result.unwrap(), 0);

    let saved = read_json(&out);
    assert_eq!(saved["sample"], 12);
    let below = saved["bookstore_override_below"].as_f64().unwrap();
    assert!((0.0..=1.01).contains(&below));
    assert_eq!(service.calls(), 12);
    assert!(printed.contains("서점으로 갈아탈 확신도 상한"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_uses_the_model_threshold_when_none_is_given() {
    // 임계값을 안 주면 모델이 학습 때 고른 값을 쓴다. 이 기본값이 0.90으로 박혀 있어 실제 라이브러리에서 40건 전부 판정 거부가 났다.
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    write_text(&library.join("3_무협").join("무림맹_장문인.txt"), &[vocab("3_무협"); 3].join(" "));

    // 임계값을 안 주고 돌린 결과와, 모델이 고른 값을 직접 준 결과가 같아야 한다.
    // 결과 문자열 자체를 못 박으면 시험용 모델의 임계값이 조금만 달라져도 깨진다. 여기서 볼 것은 "CLI가 숫자를 정해 두지 않는다" 하나다.
    let (result, default_out) = run(&["reclassify", "3_무협", "--model", &s(&path), "--library-root", &s(&library), "--no-es"]);
    assert_eq!(result.unwrap(), 0);

    let model = CategoryModel::load(&path).unwrap();
    let (result, explicit_out) = run(&["reclassify", "3_무협", "--model", &s(&path), "--library-root", &s(&library), "--no-es", "--min-confidence", &model.min_confidence.to_string()]);
    assert_eq!(result.unwrap(), 0);
    assert_eq!(explicit_out, default_out);

    // 0.90이 박혀 있던 시절의 동작과는 달라야 한다 - 그때는 전부 거부됐다.
    assert_ne!(model.min_confidence, 0.90);
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_preview_does_not_move_files() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    let book = library.join("3_무협").join("무림맹_장문인.txt");
    write_text(&book, "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객");

    let (result, out) = run(&["reclassify", "3_무협", "--model", &s(&path), "--library-root", &s(&library), "--no-es", "--min-confidence", "0.0"]);
    assert_eq!(result.unwrap(), 0);

    // --apply가 없으면 파일은 그대로 있어야 한다
    assert!(book.exists());
    assert!(out.contains("미리보기다"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_bookstore_policy_refuses_to_run_without_refused_documents() {
    // 거부 구간에서 뽑으라고 했는데 그런 문서가 없으면 조용히 전체를 쓰지 않고 멈춘다.
    // 서점을 4시간 조회한 뒤 엉뚱한 표본이었음을 아는 것보다 낫다.
    let dir = TempDir::new().unwrap();
    let (path, corpus) = model_file(&dir);
    let err = run(&["bookstore-policy", "--model", &s(&path), "--corpus", &s(&corpus), "--sample", "5", "--delay", "0", "--out", &s(&dir.path().join("p.json"))]).0.unwrap_err();
    assert!(err.message.contains("거부한 구간"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_bookstore_policy_saves_rows_for_rebanding() {
    // 건별 결과를 남겨야 구간을 다시 나눌 때 서점을 다시 조회하지 않는다. 표본 2,000건 조회에 4시간 35분이 걸렸다.
    let dir = TempDir::new().unwrap();
    let service = FakeBookstoreService::new(&[("3_무협", "3_무협", "")]);
    let deps = ClassifyCliDeps::system().with_service_factory(bookstore_factory(&service));

    let (path, corpus) = model_file(&dir);
    let out = dir.path().join("policy.json");
    let (result, _) = run_with(&["bookstore-policy", "--model", &s(&path), "--corpus", &s(&corpus), "--sample", "10", "--delay", "0", "--sample-from", "all", "--out", &s(&out)], &deps);
    assert_eq!(result.unwrap(), 0);

    let saved = read_json(&out);
    let rows = saved["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    for key in ["confidence", "model_ok", "store_ok", "store_answered"] {
        assert!(rows[0].get(key).is_some(), "행에 {key}가 없다");
    }
}

// ---------------------------------------------------------------------------
// 입력 검증
// ---------------------------------------------------------------------------

#[test]
fn test_load_model_stops_with_a_next_step() {
    let dir = TempDir::new().unwrap();
    let err = classify_cli::load_model(&dir.path().join("없는모델.joblib")).unwrap_err();
    // 다음에 무엇을 해야 하는지 알려야 한다.
    assert!(err.message.contains("collect") && err.message.contains("train"));
}

#[test]
fn test_load_corpus_stops_when_missing_or_empty() {
    let dir = TempDir::new().unwrap();
    let err = classify_cli::load_corpus(&dir.path().join("없는코퍼스.jsonl")).unwrap_err();
    assert!(err.message.contains("collect"));

    let empty = dir.path().join("empty.jsonl");
    fs::write(&empty, "").unwrap();
    let err = classify_cli::load_corpus(&empty).unwrap_err();
    assert!(err.message.contains("비었다"));
}

#[test]
fn test_publisher_cache_path_sits_next_to_the_corpus() {
    let dir = TempDir::new().unwrap();
    let cache = classify_cli::publisher_cache_path(&dir.path().join("corpus.jsonl"));
    assert_eq!(cache.file_name().unwrap().to_str().unwrap(), "corpus.jsonl.publisher.json");
}

#[test]
fn test_es_manager_passes_the_index_name() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let deps = ClassifyCliDeps::system().with_es_factory(fake_es_factory(Some(seen.clone())));

    deps.es_manager(Some("books_v2"));
    assert_eq!(seen.lock().unwrap().last().unwrap(), "books_v2");
    seen.lock().unwrap().clear();
    deps.es_manager(None);
    // 인덱스를 안 주면 빈 문자열을 넘겨 ESManager가 환경변수를 쓰게 한다.
    assert_eq!(seen.lock().unwrap().last().unwrap(), "");
}

#[test]
fn test_overrides_only_carries_values_given_on_the_cli() {
    // argparse.Namespace는 JSON 객체로 옮긴다. 없는 키는 Python에서 속성이 없는(또는 None) 값이다.
    let empty = json!({});
    assert_eq!(classify_cli::overrides(&empty), json!({}));

    let args = json!({"min_per_category": 7, "C": 2.0, "n_jobs": 1, "max_iter": 500, "holdout": 0.3, "class_weight": "balanced", "body_max_features": 100, "body_min_df": 2, "char_ngram": 1});
    let over = classify_cli::overrides(&args);
    assert_eq!(over["corpus"]["min_per_category"], 7);
    assert_eq!(over["model"]["class_weight"], "balanced");
    assert_eq!(over["fields"]["body_char"]["enabled"], true);
    assert_eq!(over["holdout"], 0.3);

    // 현재 동작을 그대로 고정한다: `--class-weight none`은 덮어쓰기를 만들지 않는다.
    // put()이 None 값을 건너뛰기 때문이다. 결과적으로 config.json의 값이 그대로 남는다.
    assert_eq!(classify_cli::overrides(&json!({"class_weight": "none"})), json!({}));
    assert_eq!(classify_cli::overrides(&json!({"char_ngram": 0}))["fields"]["body_char"]["enabled"], false);
}

// ---------------------------------------------------------------------------
// collect / train / classify
// ---------------------------------------------------------------------------

#[test]
fn test_collect_writes_corpus_and_prints_top_categories() {
    let dir = TempDir::new().unwrap();
    let docs = make_docs();
    let fixture = docs.clone();
    let corpus_ops = StubCorpus::default().with_collect(move |_es, out_path: &Path| {
        write_jsonl(&fixture, out_path);
        let mut per: HashMap<String, usize> = HashMap::new();
        for d in &fixture {
            *per.entry(d["cat"].as_str().unwrap().to_string()).or_default() += 1;
        }
        json!({"kept": fixture.len(), "skipped": 3, "categories": per.len(), "per_category": per})
    });
    let deps = ClassifyCliDeps::system().with_corpus(Arc::new(corpus_ops)).with_es_factory(fake_es_factory(None));

    let out = dir.path().join("corpus.jsonl");
    let (result, printed) = run_with(&["collect", "--out", &s(&out)], &deps);
    assert_eq!(result.unwrap(), 0);

    // Python `f"{len(docs):,}"`: 120건이라 천 단위 구분이 없다.
    assert!(printed.contains(&format!("수집 {}건", docs.len())));
    assert!(printed.contains("상위 10개 카테고리"));
    assert!(out.exists());
}

#[test]
fn test_collect_with_publisher_backfills_and_rewrites() {
    let dir = TempDir::new().unwrap();
    let docs = make_docs();
    let n = docs.len();
    let fixture = docs.clone();
    let corpus_ops = StubCorpus::default()
        .with_collect(move |_es, out_path: &Path| {
            write_jsonl(&fixture, out_path);
            json!({"kept": n, "skipped": 0, "categories": 1, "per_category": {"3_무협": n}})
        })
        .with_attach_publishers(|docs_arg: &mut Vec<Value>, _cache_path: &Path, _library_root: &Path, _workers: usize| {
            for d in docs_arg.iter_mut() {
                d["publisher"] = json!("민음사");
            }
            json!({"read": 2, "cached": 2, "with_publisher": docs_arg.len()})
        });
    let deps = ClassifyCliDeps::system().with_corpus(Arc::new(corpus_ops)).with_es_factory(fake_es_factory(None));

    let out = dir.path().join("corpus.jsonl");
    let (result, printed) = run_with(&["collect", "--out", &s(&out), "--with-publisher", "--library-root", &s(dir.path())], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(printed.contains("publisher: 새로 읽음 2건"));
    assert!(read_jsonl(&out).iter().all(|d| d["publisher"] == "민음사"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_train_writes_a_model_and_reports_blocks() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    write_jsonl(&make_docs(), &corpus);
    let deps = ClassifyCliDeps::system().with_config_store(Arc::new(FixedConfig(test_config())));

    let out = dir.path().join("model.joblib");
    let (result, printed) = run_with(&["train", "--corpus", &s(&corpus), "--out", &s(&out)], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(printed.contains("모델 저장"));
    assert!(out.exists());
    let holdout = read_json(&dir.path().join("model.joblib.holdout.json"));
    assert!(!holdout["document_keys"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_train_spill_cleanup_preserves_other_files_in_requested_directory() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    write_jsonl(&make_docs(), &corpus);
    let deps = ClassifyCliDeps::system().with_config_store(Arc::new(FixedConfig(test_config())));
    let spill_dir = dir.path().join("spill");
    fs::create_dir(&spill_dir).unwrap();
    let marker = spill_dir.join("keep.txt");
    fs::write(&marker, "user data").unwrap();

    let (result, _) = run_with(&["train", "--corpus", &s(&corpus), "--out", &s(&dir.path().join("model.joblib")), "--spill-dir", &s(&spill_dir)], &deps);
    assert_eq!(result.unwrap(), 0);

    assert_eq!(fs::read_to_string(&marker).unwrap(), "user data");
    let entries: Vec<PathBuf> = fs::read_dir(&spill_dir).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(entries, vec![marker]);
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_holdout_docs_uses_saved_manifest_keys() {
    let dir = TempDir::new().unwrap();
    let (_, corpus) = model_file(&dir);
    let docs = read_jsonl(&corpus);
    let manifest = dir.path().join("holdout.json");
    let chosen = docs.last().unwrap().clone();
    fs::write(&manifest, json!({"document_keys": [chosen["id"].as_str().unwrap()]}).to_string()).unwrap();

    let hold = classify_cli::holdout_docs(&docs, &test_config(), Some(&manifest));

    let ids: Vec<Value> = hold.iter().map(|d| d["id"].clone()).collect();
    assert_eq!(ids, vec![chosen["id"].clone()]);
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_train_warns_when_corpus_has_no_publisher() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let docs: Vec<Value> = make_docs()
        .into_iter()
        .map(|mut d| {
            d["publisher"] = json!("");
            d
        })
        .collect();
    write_jsonl(&docs, &corpus);
    let deps = ClassifyCliDeps::system().with_config_store(Arc::new(FixedConfig(test_config())));

    // caplog 대응. 프로덕션은 `log` crate만 쓴다.
    let logs = LogCapture::start(log::Level::Trace);
    run_with(&["train", "--corpus", &s(&corpus), "--out", &s(&dir.path().join("m.joblib"))], &deps);

    // publisher가 없으면 전집 판정이 나빠진다. 조용히 넘어가면 안 된다.
    assert!(logs.messages().iter().any(|msg| msg.contains("publisher")));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_classify_prints_the_verdict_and_missing_files() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let book = dir.path().join("무림맹_장문인.txt");
    fs::write(&book, [vocab("3_무협"); 3].join(" ")).unwrap();
    let missing = dir.path().join("없는파일.txt");

    let (result, printed) = run(&["classify", &s(&book), &s(&missing), "--model", &s(&path), "--no-es", "--min-confidence", "0.0"]);
    assert_eq!(result.unwrap(), 0);

    assert!(printed.contains("특징 출처: 파일"));
    assert!(printed.contains("판정:"));
    // 한 건이 없어도 나머지는 계속 판정한다.
    assert!(printed.contains("없는파일.txt: 파일이 없다"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_apply_moves_files() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    let book = library.join("0_inbox").join("무림맹_장문인.txt");
    write_text(&book, &[vocab("3_무협"); 3].join(" "));

    let (result, out) = run(&["reclassify", "0_inbox", "--model", &s(&path), "--library-root", &s(&library), "--no-es", "--min-confidence", "0.0", "--apply"]);
    assert_eq!(result.unwrap(), 0);

    assert!(!book.exists());
    assert!(out.contains("실제로 파일을 옮긴다"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_skips_when_destination_already_has_the_file() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    let name = "무림맹_장문인.txt";
    write_text(&library.join("0_inbox").join(name), &[vocab("3_무협"); 3].join(" "));
    write_text(&library.join("3_무협").join(name), "이미 있는 파일");

    let (result, out) = run(&["reclassify", "0_inbox", "--model", &s(&path), "--library-root", &s(&library), "--no-es", "--min-confidence", "0.0", "--apply"]);
    assert_eq!(result.unwrap(), 0);

    // 덮어쓰지 않는다. 원본도 그대로 둔다.
    assert!(library.join("0_inbox").join(name).exists());
    assert!(out.contains("옮김 0건"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_stops_when_directory_is_missing() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let err = run(&["reclassify", "없는카테고리", "--model", &s(&path), "--library-root", &s(dir.path()), "--no-es"]).0.unwrap_err();
    assert!(err.message.contains("디렉토리가 없다"));
}

// ---------------------------------------------------------------------------
// reband
// ---------------------------------------------------------------------------

fn policy_rows() -> Vec<Value> {
    (0..20)
        .map(|i| {
            let conf = i as f64 / 20.0;
            json!({"confidence": conf, "model_ok": conf > 0.5, "store_ok": true, "store_answered": true})
        })
        .collect()
}

#[test]
fn test_reband_rebuilds_bands_without_querying_bookstores() {
    let dir = TempDir::new().unwrap();
    let saved = dir.path().join("policy.json");
    fs::write(&saved, json!({"rows": policy_rows(), "sample_from": "refused"}).to_string()).unwrap();

    let (result, printed) = run(&["bookstore-policy", "--reband", &s(&saved), "--bands", "4"]);
    assert_eq!(result.unwrap(), 0);

    let out = read_json(&saved);
    assert_eq!(out["sample_from"], "refused");
    assert!(!out["bands"].as_array().unwrap().is_empty());
    // 경계는 실제로 잰 범위를 넘으면 안 된다.
    assert!(out["bookstore_override_below"].as_f64().unwrap() <= out["measured_max_confidence"].as_f64().unwrap());
    assert!(printed.contains("서점으로 갈아탈 확신도 상한"));
}

#[test]
fn test_reband_writes_to_a_separate_file_when_asked() {
    let dir = TempDir::new().unwrap();
    let saved = dir.path().join("policy.json");
    fs::write(&saved, json!({"rows": policy_rows()}).to_string()).unwrap();
    let dest = dir.path().join("reband.json");

    let (result, _) = run(&["bookstore-policy", "--reband", &s(&saved), "--bands", "3", "--out", &s(&dest)]);
    assert_eq!(result.unwrap(), 0);
    assert!(dest.exists());
    assert_eq!(read_json(&dest)["sample_from"], "unknown");
}

#[test]
fn test_reband_stops_when_the_file_is_missing() {
    let dir = TempDir::new().unwrap();
    let err = run(&["bookstore-policy", "--reband", &s(&dir.path().join("없는파일.json"))]).0.unwrap_err();
    assert!(err.message.contains("측정 결과 파일이 없다"));
}

#[test]
fn test_reband_stops_when_there_are_no_rows() {
    let dir = TempDir::new().unwrap();
    let saved = dir.path().join("policy.json");
    fs::write(&saved, json!({"bands": []}).to_string()).unwrap();
    let err = run(&["bookstore-policy", "--reband", &s(&saved)]).0.unwrap_err();
    // 서점 조회부터 다시 해야 한다는 것을 알려야 한다.
    assert!(err.message.contains("서점 조회부터"));
}

// ---------------------------------------------------------------------------
// score-calibration / reclassify 의 남은 갈래 / 스크립트 실행
// ---------------------------------------------------------------------------

fn fake_calibration() -> Value {
    json!({"x": [0.01, 0.05, 0.15], "y": [0.5, 0.8, 0.99], "n": 123})
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_score_calibration_stops_when_holdout_is_too_small() {
    let dir = TempDir::new().unwrap();
    let (path, corpus) = model_file(&dir);
    let before = fs::read(&path).unwrap();

    let (result, out) = run(&["score-calibration", "--model", &s(&path), "--corpus", &s(&corpus)]);
    assert_eq!(result.unwrap(), 1);

    assert!(out.contains("표본이 모자라"));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_score_calibration_dry_run_leaves_the_model_file_alone() {
    let dir = TempDir::new().unwrap();
    let (path, corpus) = model_file(&dir);
    let before = fs::read(&path).unwrap();
    let deps = ClassifyCliDeps::system().with_corpus(Arc::new(StubCorpus::default().with_fit_confidence_calibration(fake_calibration)));

    let (result, out) = run_with(&["score-calibration", "--model", &s(&path), "--corpus", &s(&corpus), "--dry-run"], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(out.contains("계단 3칸, 표본 123건"));
    assert!(out.contains("--dry-run 이라 모델 파일은 그대로 둔다"));
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_score_calibration_saves_the_scale_into_the_model() {
    let dir = TempDir::new().unwrap();
    let (path, corpus) = model_file(&dir);
    let deps = ClassifyCliDeps::system().with_corpus(Arc::new(StubCorpus::default().with_fit_confidence_calibration(fake_calibration)));

    let (result, out) = run_with(&["score-calibration", "--model", &s(&path), "--corpus", &s(&corpus)], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(out.contains("모델에 눈금을 넣었다"));
    assert_eq!(CategoryModel::load(&path).unwrap().meta["confidence_calibration"], fake_calibration());
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_limit_caps_the_number_of_files() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    for name in ["a.txt", "b.txt", "c.txt"] {
        write_text(&library.join("0_inbox").join(name), &[vocab("3_무협"); 3].join(" "));
    }

    let (result, out) = run(&["reclassify", "0_inbox", "--model", &s(&path), "--library-root", &s(&library), "--no-es", "--limit", "2"]);
    assert_eq!(result.unwrap(), 0);

    assert!(out.contains("대상 2건"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_counts_files_that_cannot_be_moved() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    let book = library.join("0_inbox").join("무림맹_장문인.txt");
    write_text(&book, &[vocab("3_무협"); 3].join(" "));
    // shutil.move 실패 주입: `FsOps::rename`을 `common::FaultyFs`로 실패시킨다.
    let faulty = FaultyFs::new();
    faulty.set_rename(|_src: &Path, _dst: &Path| Some(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read-only")));
    let deps = ClassifyCliDeps::system().with_fs(faulty);

    let (result, out) = run_with(&["reclassify", "0_inbox", "--model", &s(&path), "--library-root", &s(&library), "--no-es", "--min-confidence", "0.0", "--apply"], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(book.exists());
    assert!(out.contains("실패 1건"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_info_lists_dropped_categories_and_truncates_after_ten() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let mut model = CategoryModel::load(&path).unwrap();
    model.meta["dropped_categories"] = Value::Array((0..12).map(|i| json!(format!("9_희귀{i}"))).collect());
    let patched = dir.path().join("dropped.joblib");
    model.save(&patched);

    let (result, out) = run(&["info", "--model", &s(&patched)]);
    assert_eq!(result.unwrap(), 0);

    assert!(out.contains("제외 카테고리 12개: 9_희귀0"));
    assert!(out.contains("9_희귀9 ..."));
    assert!(!out.contains("9_희귀10"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요; mock.patch: sys.argv + runpy(스크립트 직접 실행); 바이너리 실행 테스트로 대체 후 활성화"]
fn test_running_as_script_exits_with_main_status() {
    // Python은 `python -m utils.classify_cli info --model <path>`를 runpy로 돌려 종료 코드 0을 확인한다.
    // Rust에서는 classify_cli 바이너리를 자식 프로세스로 실행하는 것이 대응이다.
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_classify_cli")).args(["info", "--model", &s(&path)]).status().unwrap();
    assert_eq!(status.code(), Some(0));
}
