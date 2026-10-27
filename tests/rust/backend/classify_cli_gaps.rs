//! 원본: tests/test_classify_cli_gaps.py
//!
//! 자동분류 CLI의 오류 경로와 긴 작업의 진행 표시를 확인한다.
//!
//! 가정한 API (아직 구현 전이다. 공통 항목과 `ClassifyCliDeps`의 7개 주입 지점은 tests/rust/backend/classify_cli.rs 머리말과 같다. 설계: docs/rust-test-seams.md 3.12):
//! - `utils::classify_cli::load_manifest_holdout(&[Value], &Path) -> Result<Vec<Value>, ValueError>`
//! - `utils::classify_cli::ValueError { pub message: String }` (`Display`는 message)
//! - `utils::classify_cli::main_with(args: &[&str], io: &mut CliIo, deps: &ClassifyCliDeps) -> Result<i32, SystemExit>`, `CliIo { out, err }`
//!   (구 `main`, `capture_stdout`, `capture_logs`, `testing::override_*`는 없다. 로그는 `common::log_capture::LogCapture`로 모은다)
//! - `ClassifyCliDeps::{system, with_config_store, with_es_factory, with_corpus, with_service_factory, with_fs, with_clock, with_boundary_factor}`
//!   - `CorpusOps::holdout_docs(&self) -> Vec<Value>`(구 `override_holdout_docs`)와 `collect`를 이 파일에서 바꿔치기한다.
//!   - `ConfigStore::{load_config, save_config}`, `BookstoreQuery::query_bookstores`, `sysdeps::Clock`, `sysdeps::FsOps::read_dir`(구 `iter_files`)
//! - `backend::classifier::corpus::{read_jsonl, write_jsonl}`, `backend::classifier::training::train`, `backend::classifier::model::CategoryModel::save`
//!
//! tests/test_classify_cli.py의 `FakeBookstoreService`, `TEST_CONFIG`, `model_file`과 tests/test_classifier.py의 `VOCAB`, `make_docs`는
//! 테스트 바이너리가 파일마다 따로라 이 파일에 같은 헬퍼를 둔다.

mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use backend::classifier::config::{default_config, merge_config, series_to_parent};
use backend::classifier::corpus::{read_jsonl, write_jsonl};
use backend::classifier::training::train;
use backend::es_manager::EsManagerApi;
use common::{es_fake::FakeEsManager, log_capture::LogCapture};
use serde_json::{Value, json};
use sysdeps::{Clock, FsOps, RealFs};
use tempfile::TempDir;
use utils::classify_cli::{self, BookstoreQuery, ClassifyCliDeps, CliIo, ConfigStore, CorpusOps, SystemExit};

const CATEGORIES: [(&str, &str); 3] =
    [("3_무협", "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객"), ("3_판타지", "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕"), ("5_수학과학일반", "미분 적분 함수 벡터 행렬 확률 통계 방정식")];

fn vocab(category: &str) -> &'static str {
    CATEGORIES.iter().find(|(c, _)| *c == category).unwrap().1
}

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

struct FakeBookstoreService {
    answers: Vec<(String, String, String)>,
    calls: AtomicUsize,
}

impl FakeBookstoreService {
    fn new(answers: &[(&str, &str, &str)]) -> Arc<Self> {
        Arc::new(Self { answers: answers.iter().map(|(y, a, k)| (y.to_string(), a.to_string(), k.to_string())).collect(), calls: AtomicUsize::new(0) })
    }
}

impl BookstoreQuery for FakeBookstoreService {
    fn query_bookstores(&self, _search_title: &str, _raw_author: &str, _raw_title: &str) -> (Value, Value, Value) {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (y, a, k) = &self.answers[n % self.answers.len()];
        (json!({"mapped": y}), json!({"mapped": a}), json!({"mapped": k}))
    }
}

/// 구 `override_load_config`/`override_save_config`. 설정을 시험용으로 고정하고 저장 위치를 돌려준다.
// TODO(common): classify CLI 대역(`FixedConfig`, `StubCorpus`, `TickingClock`, `ListFs`)은 classify_cli와 이 파일이 따로 갖는다. 필요하면 common으로 올린다.
struct FixedConfig {
    cfg: Value,
    saved_to: PathBuf,
}

impl ConfigStore for FixedConfig {
    fn load_config(&self, _path: &Path) -> Value {
        self.cfg.clone()
    }

    fn save_config(&self, _cfg: &Value) -> PathBuf {
        self.saved_to.clone()
    }
}

type CollectFn = Box<dyn Fn(&dyn EsManagerApi, &Path) -> Value + Send + Sync>;
type HoldoutFn = Box<dyn Fn() -> Vec<Value> + Send + Sync>;

/// 구 `override_collect`/`override_holdout_docs`. 쓰는 메서드만 채운다.
#[derive(Default)]
struct StubCorpus {
    collect: Option<CollectFn>,
    holdout_docs: Option<HoldoutFn>,
}

impl StubCorpus {
    fn with_collect(mut self, f: impl Fn(&dyn EsManagerApi, &Path) -> Value + Send + Sync + 'static) -> Self {
        self.collect = Some(Box::new(f));
        self
    }

    fn with_holdout_docs(mut self, f: impl Fn() -> Vec<Value> + Send + Sync + 'static) -> Self {
        self.holdout_docs = Some(Box::new(f));
        self
    }
}

impl CorpusOps for StubCorpus {
    fn collect(&self, es: &dyn EsManagerApi, out_path: &Path) -> Value {
        (self.collect.as_ref().expect("collect를 부르면 안 된다"))(es, out_path)
    }

    fn holdout_docs(&self) -> Vec<Value> {
        (self.holdout_docs.as_ref().expect("holdout_docs를 부르면 안 된다"))()
    }
}

/// `itertools.count(0.0, 1.0)`: 호출할 때마다 1초씩 흐르는 가짜 시계. `now`와 `monotonic`이 같은 시간선을 쓰고 `sleep`은 안 잔다.
// TODO(common): `common::clock::FakeClock`은 호출마다 시간을 자동으로 보내지 않는다. 자동 증가 모드가 생기면 이 로컬 구현을 지운다.
struct TickingClock {
    ticks: AtomicUsize,
}

impl Clock for TickingClock {
    fn now(&self) -> f64 {
        self.ticks.fetch_add(1, Ordering::SeqCst) as f64
    }

    fn monotonic(&self) -> f64 {
        self.now()
    }

    fn sleep(&self, _seconds: f64) {}
}

/// `read_dir`가 항상 같은 목록을 돌려주는 `FsOps`. 나머지는 실제 파일 시스템에 맡긴다.
// TODO(common): `common::faulty_fs::FaultyFs`는 `read_dir`을 실패시킬 수만 있고 반환값을 정할 수 없다. 목록을 정하는 기능이 생기면 지운다.
struct ListFs(Vec<PathBuf>);

impl FsOps for ListFs {
    fn read_dir(&self, _p: &Path) -> io::Result<Vec<PathBuf>> {
        Ok(self.0.clone())
    }

    fn is_file(&self, p: &Path) -> io::Result<bool> {
        RealFs.is_file(p)
    }
}

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

fn run(args: &[&str]) -> (Result<i32, SystemExit>, String) {
    run_with(args, &ClassifyCliDeps::system())
}

/// `main_with(args, &mut CliIo, deps)`를 돌리고 (결과, stdout)을 돌려준다. capsys 대응.
fn run_with(args: &[&str], deps: &ClassifyCliDeps) -> (Result<i32, SystemExit>, String) {
    let (mut out, mut err) = (Vec::<u8>::new(), Vec::<u8>::new());
    let result = classify_cli::main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (result, String::from_utf8_lossy(&out).into_owned())
}

/// `_run_policy`. 서점 조회를 가짜로 바꾸고 `bookstore-policy`를 돌려 (종료 코드, 저장된 JSON)을 돌려준다.
/// `deps`는 시계·경계 배수처럼 테스트마다 다른 주입 지점을 미리 채운 `ClassifyCliDeps`다.
fn run_policy(dir: &TempDir, extra: &[&str], deps: ClassifyCliDeps, docs: Option<Vec<Value>>) -> (i32, Value, String) {
    let (path, corpus) = model_file(dir);
    let service = FakeBookstoreService::new(&[("3_무협", "3_무협", "")]);
    let svc = service.clone();
    let mut deps = deps.with_service_factory(Arc::new(move || svc.clone() as Arc<dyn BookstoreQuery>));
    if let Some(docs) = docs {
        deps = deps.with_corpus(Arc::new(StubCorpus::default().with_holdout_docs(move || docs.clone())));
    }
    let out = dir.path().join("policy.json");
    let mut args = vec!["bookstore-policy", "--model", &s(&path), "--corpus", &s(&corpus), "--delay", "0", "--out", &s(&out)];
    args.extend_from_slice(extra);
    let (result, printed) = run_with(&args, &deps);
    let saved = serde_json::from_str(&fs::read_to_string(&out).unwrap()).unwrap();
    (result.unwrap(), saved, printed)
}

// ---- holdout manifest ----

#[test]
fn test_manifest_with_wrong_shape_is_rejected() {
    let dir = TempDir::new().unwrap();
    let manifest = dir.path().join("holdout.json");
    fs::write(&manifest, json!({"document_keys": ["a", 3]}).to_string()).unwrap();

    let err = classify_cli::load_manifest_holdout(&make_docs(), &manifest).unwrap_err();
    assert!(err.message.contains("잘못된 holdout manifest"), "{}", err.message);
}

#[test]
fn test_manifest_keys_missing_from_corpus_are_warned_but_found_ones_kept() {
    let dir = TempDir::new().unwrap();
    let docs = make_docs();
    let manifest = dir.path().join("holdout.json");
    fs::write(&manifest, json!({"document_keys": [docs[0]["id"].as_str().unwrap(), "없는-문서"]}).to_string()).unwrap();

    // caplog.at_level("WARNING", logger="classify_cli")
    let logs = LogCapture::start(log::Level::Warn);
    let held = classify_cli::load_manifest_holdout(&docs, &manifest).unwrap();

    let ids: Vec<Value> = held.iter().map(|d| d["id"].clone()).collect();
    assert_eq!(ids, vec![docs[0]["id"].clone()]);
    assert!(logs.records().iter().any(|r| r.level == log::Level::Warn && r.message.contains("1건을 현재 코퍼스에서 찾지 못했다")), "{:?}", logs.records());
}

#[test]
fn test_manifest_with_no_matching_documents_is_rejected() {
    let dir = TempDir::new().unwrap();
    let manifest = dir.path().join("holdout.json");
    fs::write(&manifest, json!({"document_keys": ["없는-문서"]}).to_string()).unwrap();

    let err = classify_cli::load_manifest_holdout(&make_docs(), &manifest).unwrap_err();
    assert!(err.message.contains("찾지 못했다"), "{}", err.message);
}

// ---- collect / train 출력 ----

#[test]
fn test_collect_prints_bottom_ten_categories_when_there_are_many() {
    let dir = TempDir::new().unwrap();
    let per_category: serde_json::Map<String, Value> = (0..12).map(|n| (format!("카테고리{n:02}"), json!(n + 1))).collect();
    let n_categories = per_category.len();

    let corpus_ops = StubCorpus::default().with_collect(move |_es, out_path: &Path| {
        write_jsonl(&make_docs(), out_path);
        json!({"kept": 12, "skipped": 0, "categories": n_categories, "per_category": per_category})
    });
    let deps = ClassifyCliDeps::system().with_corpus(Arc::new(corpus_ops)).with_es_factory(Arc::new(|_index: &str| -> Arc<dyn EsManagerApi> { FakeEsManager::new() }));

    let (result, printed) = run_with(&["collect", "--out", &s(&dir.path().join("corpus.jsonl"))], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(printed.contains("... 하위 10개"));
    assert!(printed.contains("카테고리11"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_train_prints_where_the_config_was_saved() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    write_jsonl(&make_docs(), &corpus);
    let saved_to = dir.path().join("config.json");
    let deps = ClassifyCliDeps::system().with_config_store(Arc::new(FixedConfig { cfg: test_config(), saved_to: saved_to.clone() }));

    let (result, printed) = run_with(&["train", "--corpus", &s(&corpus), "--out", &s(&dir.path().join("model.joblib")), "--save-config"], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(printed.contains(&format!("설정 저장: {}", saved_to.display())));
}

// ---- bookstore-policy ----

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_bookstore_policy_samples_the_boundary_above_the_refused_range() {
    // 시험용 모델은 확신도가 높아 거부 구간이 비어 있다. 경계 배수를 키워 모든 문서가 들어오게 한다.
    let deps = ClassifyCliDeps::system().with_boundary_factor(1000.0);
    let dir = TempDir::new().unwrap();

    let (code, saved, printed) = run_policy(&dir, &["--sample", "6", "--sample-from", "boundary"], deps, None);

    assert_eq!(code, 0);
    assert_eq!(saved["sample_from"], "boundary");
    assert!(printed.contains("거부 구간과 그 위"));
}

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_bookstore_policy_reports_progress_and_skips_empty_bands() {
    // 같은 문서를 120번 재면 확신도가 모두 같아 첫 구간이 비고, 100건째에 진행 상황이 찍힌다.
    let dir = TempDir::new().unwrap();
    let (_, corpus_probe) = model_file(&dir);
    let doc = read_jsonl(&corpus_probe)[0].clone();
    // itertools.count(0.0, 1.0): 호출할 때마다 1초씩 흐르는 가짜 시계
    let deps = ClassifyCliDeps::system().with_clock(Arc::new(TickingClock { ticks: AtomicUsize::new(0) }));

    let (code, saved, printed) = run_policy(&dir, &["--sample", "120", "--sample-from", "all", "--bands", "3"], deps, Some(vec![doc; 120]));

    assert_eq!(code, 0);
    assert_eq!(saved["sample"], 120);
    assert!(printed.contains("  100/120"));
    assert!(saved["bands"].as_array().unwrap().iter().all(|band| band["n"].as_i64().unwrap() > 0));
}

#[test]
fn test_reband_skips_bands_without_rows() {
    let dir = TempDir::new().unwrap();
    let rows: Vec<Value> = (0..4).map(|_| json!({"confidence": 0.1, "model_ok": true, "store_ok": true, "store_answered": true})).collect();
    let saved = dir.path().join("policy.json");
    fs::write(&saved, json!({"rows": rows}).to_string()).unwrap();

    let (result, _) = run(&["bookstore-policy", "--reband", &s(&saved), "--bands", "3"]);
    assert_eq!(result.unwrap(), 0);

    let out: Value = serde_json::from_str(&fs::read_to_string(&saved).unwrap()).unwrap();
    let ns: Vec<i64> = out["bands"].as_array().unwrap().iter().map(|band| band["n"].as_i64().unwrap()).collect();
    assert_eq!(ns, vec![4]);
}

// ---- reclassify 진행 표시 ----

#[test]
#[ignore = "sklearn 대응 모델 필요"]
fn test_reclassify_prints_progress_every_500_files() {
    let dir = TempDir::new().unwrap();
    let (path, _) = model_file(&dir);
    let library = dir.path().join("library");
    let source = library.join("0_inbox");
    fs::create_dir_all(&source).unwrap();
    let book = source.join("a.txt");
    fs::write(&book, [vocab("3_무협"); 3].join(" ")).unwrap();
    let listed = book.clone();
    // 구 `_iter_files` 바꿔치기: `FsOps::read_dir`이 같은 파일을 500번 돌려준다.
    let deps = ClassifyCliDeps::system().with_fs(Arc::new(ListFs(vec![listed; 500])));

    let (result, out) = run_with(&["reclassify", "0_inbox", "--model", &s(&path), "--library-root", &s(&library), "--no-es"], &deps);
    assert_eq!(result.unwrap(), 0);

    assert!(out.contains("500/500 처리"));
}
