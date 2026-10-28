//! BookManager 의 분류 제안이 서점 득표·정책·보정을 포함한 `BookClassifierService` 로 동작하는지 본다.
//!
//! - (a) 작은 모델 fixture + 가짜 서점 클라이언트로, 서점 득표가 제안의 등급·근거를 바꾼다(모델 단독과 다르다).
//! - (b) 모델 디렉토리가 없으면 분류기 없이 안전하게 동작하고, 이유는 경고 로그로 한 번만 남는다.
//! - (c) 동시 제안에서도 분류 캐시 파일이 깨지지 않는다.
//!
//! 서점은 모두 `BookstoreSearch` 대역이다. 실제 네트워크에는 접속하지 않는다.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use backend::book_classifier::{BookClassifierService, BookstorePolicy, SharedBookClassifier};
use backend::book_manager::{BookManager, BookManagerDeps, ProposeOptions};
use backend::bookstore::{BookstoreSearch, SearchRow};
use backend::classifier::model::CategoryModel;
use backend::classifier::{BookCategoryClassifier, ClassifierOptions, Classify};
use common::env::EnvGuard;
use common::es_fake::FakeEsManager;
use common::log_capture::LogCapture;
use serde_json::{Value, json};
use tempfile::TempDir;

/// `Result<Value, _>`를 Python 원본의 `(결과, 오류)`로 펼친다. 실패하면 원본처럼 빈 객체다.
fn outcome(result: Result<serde_json::Value, String>) -> (serde_json::Value, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::json!({}), Some(error)),
    }
}

fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

fn model_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/trained")
}

/// 책 루트(`books`)와 서비스 파일(`meta`)을 나눠 둔다. 캐시 파일이 책으로 스캔되면 안 된다.
struct Setup {
    _tmp: TempDir,
    books: PathBuf,
    meta: PathBuf,
}

fn setup(files: &[&str]) -> Setup {
    let tmp = TempDir::new().unwrap();
    let books = tmp.path().join("books");
    let meta = tmp.path().join("meta");
    fs::create_dir_all(books.join("0_inbox")).unwrap();
    fs::create_dir_all(&meta).unwrap();
    for name in files {
        fs::write(books.join("0_inbox").join(name), "본문 내용 ".repeat(20)).unwrap();
    }
    Setup { books, meta, _tmp: tmp }
}

/// 항상 같은 행을 돌려주는 서점. 부른 횟수를 센다.
struct Store {
    category: &'static str,
    calls: Arc<AtomicUsize>,
}

impl BookstoreSearch for Store {
    fn search(&self, _isbn: &str, title: &str, _author: &str) -> (Vec<SearchRow>, String, String) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (vec![(title.to_string(), "작가".into(), self.category.into(), "http://x".into(), "http://s".into(), String::new())], title.to_string(), "title".into())
    }
}

fn fixture_model_classifier(books: &Path) -> Arc<dyn Classify> {
    let model = CategoryModel::load(Some(&model_dir())).expect("fixture 모델");
    Arc::new(BookCategoryClassifier::new(Some(Arc::new(model)), ClassifierOptions { library_root: Some(books.to_path_buf()), ..Default::default() }))
}

fn service(s: &Setup, categories: [&'static str; 3], calls: &Arc<AtomicUsize>) -> BookClassifierService {
    let mut svc = BookClassifierService::new(&s.books, Some(&s.meta.join("classification_cache.json")), Some(fixture_model_classifier(&s.books)))
        .with_delay(0.0)
        .with_bookstore_cache_file(&s.meta.join("bookstore_cache.jsonl"))
        .with_bookstore_policy(BookstorePolicy::default());
    svc.yes24 = Box::new(Store { category: categories[0], calls: Arc::clone(calls) });
    svc.aladin = Box::new(Store { category: categories[1], calls: Arc::clone(calls) });
    svc.kyobo = Box::new(Store { category: categories[2], calls: Arc::clone(calls) });
    svc
}

fn manager(s: &Setup, deps: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> BookManager {
    common::setup_env();
    BookManager::with_deps(deps(BookManagerDeps::new(s.books.clone(), FakeEsManager::new())))
}

fn propose(m: &BookManager, opts: ProposeOptions<'_>) -> Value {
    let (result, err) = outcome(run(m.propose_category_changes("0_inbox", Some(&json!({})), opts)));
    assert!(err.is_none(), "{err:?}");
    result
}

// ---------------------------------------------------------------------------
// (a) 서점 득표와 정책이 제안에 반영된다
// ---------------------------------------------------------------------------

#[test]
fn bookstore_votes_change_the_grade_compared_with_the_model_alone() {
    let s = setup(&["달빛조각사.txt"]);

    // 모델 단독(기존 훅). fixture 모델은 이 문서에 확신이 모자라 판정을 거부한다.
    let model_only = manager(&s, |d| {
        d.with_classifier_factory({
            let classifier = fixture_model_classifier(&s.books);
            Arc::new(move || Arc::clone(&classifier))
        })
    });
    let alone = propose(&model_only, ProposeOptions::default());
    assert_eq!(alone["items"][0]["grade"], "unknown");
    assert!(alone["items"][0]["target_category"].is_null());

    // 서비스: 서점 세 곳이 같은 카테고리를 가리키면 다수결이 이긴다.
    let calls = Arc::new(AtomicUsize::new(0));
    let shared = Arc::new(SharedBookClassifier::new(service(&s, ["소설 > 판타지", "국내도서 > 판타지", "소설 > 판타지"], &calls)));
    let with_service = manager(&s, |d| {
        d.with_classifier_service_factory(Arc::new({
            let shared = Arc::clone(&shared);
            move || Some(Arc::clone(&shared))
        }))
    });
    let result = propose(&with_service, ProposeOptions::default());
    let item = &result["items"][0];
    assert_eq!(item["target_category"], "3_판타지");
    assert_eq!(item["grade"], "certain");
    assert_eq!(item["source"], "bookstore_majority");
    assert!(item["reason"].as_str().unwrap().contains("서점"), "{}", item["reason"]);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    // 같은 서점 득표가 후보로 남는다.
    assert_eq!(item["candidates"][0]["category"], "3_판타지");
}

#[test]
fn use_bookstore_false_skips_the_stores_and_use_content_meta_false_uses_the_file_name_only() {
    let s = setup(&["달빛조각사.txt"]);
    let calls = Arc::new(AtomicUsize::new(0));
    let shared = Arc::new(SharedBookClassifier::new(service(&s, ["소설 > 판타지"; 3], &calls)));
    let m = manager(&s, |d| {
        d.with_classifier_service_factory(Arc::new({
            let shared = Arc::clone(&shared);
            move || Some(Arc::clone(&shared))
        }))
    });

    let no_stores = propose(&m, ProposeOptions { use_bookstore: Some(false), ..Default::default() });
    assert_eq!(calls.load(Ordering::SeqCst), 0, "서점을 부르면 안 된다");
    assert_eq!(no_stores["items"][0]["grade"], "unknown");

    // 둘 다 끄면 분류기를 만들지 않는다(Python 은 서비스를 만들지 않는다).
    let none = propose(&m, ProposeOptions { use_bookstore: Some(false), use_content_meta: Some(false), ..Default::default() });
    assert_eq!(none["items"][0]["grade"], "unknown");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn policy_boundary_and_calibration_come_from_the_service() {
    let s = setup(&["달빛조각사.txt"]);
    let calls = Arc::new(AtomicUsize::new(0));
    // 경계가 1.0 에 가까우면 모델 확신도가 경계 아래라 서점이 먼저다. 서점은 판타지로 일치한다.
    let svc = service(&s, ["소설 > 판타지"; 3], &calls).with_bookstore_policy(BookstorePolicy { override_below: 0.99, ..Default::default() });
    let shared = Arc::new(SharedBookClassifier::new(svc));
    let m = manager(&s, |d| {
        d.with_classifier_service_factory(Arc::new({
            let shared = Arc::clone(&shared);
            move || Some(Arc::clone(&shared))
        }))
    });
    let item = propose(&m, ProposeOptions::default())["items"][0].clone();
    assert_eq!(item["target_category"], "3_판타지");
    assert!(item["reason"].as_str().unwrap().contains("서점 우선"), "{}", item["reason"]);
}

// ---------------------------------------------------------------------------
// (b) 모델이 없을 때
// ---------------------------------------------------------------------------

#[test]
fn missing_model_directory_means_no_classifier_and_one_warning() {
    let s = setup(&["달빛조각사.txt", "다른책.txt"]);
    let mut env = EnvGuard::new();
    env.set("TM_CLASSIFIER_MODEL_DIR", s.meta.join("없는-모델").to_str().unwrap());
    let logs = LogCapture::start(log::Level::Info);

    let m = manager(&s, |d| d);
    let first = propose(&m, ProposeOptions::default());
    let second = propose(&m, ProposeOptions::default());

    for result in [&first, &second] {
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
        assert!(result["items"].as_array().unwrap().iter().all(|i| i["grade"] == "unknown" && i["target_category"].is_null()));
    }
    let warnings: Vec<String> = logs.records().iter().filter(|r| r.level == log::Level::Warn && r.message.contains("분류 모델")).map(|r| r.message.clone()).collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    // 로그에는 경로만 있고 책 이름은 없다.
    assert!(!logs.text().contains("달빛조각사"));
    drop(env);
}

#[test]
fn a_loadable_model_directory_builds_the_service_by_default() {
    let s = setup(&["달빛조각사.txt"]);
    let mut env = EnvGuard::new();
    env.set("TM_CLASSIFIER_MODEL_DIR", model_dir().to_str().unwrap());
    let m = manager(&s, |d| d);
    let result = propose(&m, ProposeOptions { use_bookstore: Some(false), ..Default::default() });
    // 모델이 읽혔으므로 확신도가 채워진다(서점은 끔).
    assert!(result["items"][0]["confidence"].is_number(), "{}", result["items"][0]);
    drop(env);
}

// ---------------------------------------------------------------------------
// (c) 동시 제안
// ---------------------------------------------------------------------------

#[test]
fn concurrent_proposals_keep_the_bookstore_cache_intact_and_do_not_save_the_classification_cache() {
    let names: Vec<String> = (0..6).map(|i| format!("책{i}번.txt")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let s = setup(&refs);
    let calls = Arc::new(AtomicUsize::new(0));
    let shared = Arc::new(SharedBookClassifier::new(service(&s, ["소설 > 판타지"; 3], &calls)));
    let m = Arc::new(manager(&s, |d| {
        d.with_classifier_service_factory(Arc::new({
            let shared = Arc::clone(&shared);
            move || Some(Arc::clone(&shared))
        }))
    }));

    let results = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                let r = propose(&m, ProposeOptions::default());
                results.lock().unwrap().push(r);
            });
        }
    });

    for r in results.lock().unwrap().iter() {
        assert_eq!(r["items"].as_array().unwrap().len(), 6);
    }
    // Python은 제안 경로에서 분류 캐시를 저장하지 않는다(저장은 직접 분류 CLI뿐이다). 서점 응답이 비었던 항목이 캐시에 남아
    // 같은 파일을 다시 제안해도 서점을 다시 조회하지 않는 부작용을 피하려는 것이다. 임시 파일도 남지 않는다.
    assert!(!s.meta.join("classification_cache.json").exists(), "제안 경로가 분류 캐시를 저장했다");
    let leftovers: Vec<String> = fs::read_dir(&s.meta).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // 서점 캐시는 조회마다 한 줄씩 붙으므로 모든 줄이 JSON 이어야 한다.
    for line in fs::read_to_string(s.meta.join("bookstore_cache.jsonl")).unwrap().lines() {
        serde_json::from_str::<Value>(line).unwrap();
    }
}
