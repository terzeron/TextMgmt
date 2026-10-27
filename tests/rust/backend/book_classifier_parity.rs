//! 3단계 Python 대조: `backend/book_classifier.py` 의 순수 함수와 `bookstore_policy` 키워드 매핑.
//!
//! 기대값은 `tests/fixtures/classifier_tiny/book_classifier_parity.json` 이다. Python 구현이 만들고
//! (`python tests/test_classifier_export.py book_parity`), `test_book_parity_fixture_is_current` 가 최신인지 확인한다.
//! 서점 득표·정책·캐시의 동작은 `book_classifier.rs`, `bookstore_policy.rs`, `bookstore_cache.rs` 가 맡는다. 이 파일은 보안·경계와
//! `BookClassifierApi` 연결(book_manager 가 쓰는 모양)도 확인한다.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use backend::book_classifier::*;
use backend::classifier::bookstore_policy::{map_bookstore_candidates, match_keyword};
use backend::classifier::model::Prediction;
use backend::classifier::{BookClassifierApi, Classify};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixture() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/book_classifier_parity.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[test]
fn map_category_matches_python() {
    let f = fixture();
    let mut bad = Vec::new();
    for c in f["map_category"].as_array().unwrap() {
        let got = map_category(s(&c["cat"]), s(&c["title"]), s(&c["author"]));
        if got.as_deref() != c["expected"].as_str() {
            bad.push(format!("{:?} -> rust {got:?} python {}", c["cat"], c["expected"]));
        }
    }
    assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
}

#[test]
fn filename_parsing_matches_python() {
    let f = fixture();
    let mut bad = Vec::new();
    for c in f["filenames"].as_array().unwrap() {
        let name = s(&c["name"]);
        let (a, t, st) = clean_filename_to_author_title(name);
        let want: Vec<&str> = c["clean"].as_array().unwrap().iter().map(s).collect();
        if [a.as_str(), t.as_str(), st.as_str()] != want[..] {
            bad.push(format!("clean {name:?}: rust {:?} python {want:?}", (&a, &t, &st)));
        }
        if extract_explicit_genre(name).as_deref() != c["genre"].as_str() {
            bad.push(format!("genre {name:?}: rust {:?} python {}", extract_explicit_genre(name), c["genre"]));
        }
    }
    assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
}

#[test]
fn title_matching_matches_python() {
    let f = fixture();
    let mut bad = Vec::new();
    for c in f["titles"].as_array().unwrap() {
        let (a, b) = (s(&c["a"]), s(&c["b"]));
        let sim = title_similarity(a, b);
        if (sim - c["sim"].as_f64().unwrap()).abs() > 1e-12 || is_single_match_valid(a, b) != c["single"] || is_title_match_reliable(a, b) != c["reliable"] {
            bad.push(format!("{a:?} / {b:?}: rust sim {sim} python {}", c["sim"]));
        }
    }
    assert!(bad.is_empty(), "{} 건 불일치: {:?}", bad.len(), &bad[..bad.len().min(5)]);
}

#[test]
fn effective_filename_matches_python() {
    for c in fixture()["effective"].as_array().unwrap() {
        let got = get_effective_filename(Path::new(s(&c["path"])), Path::new(s(&c["target"])));
        assert_eq!(got, s(&c["expected"]), "{}", c["path"]);
    }
}

#[test]
fn keyword_mapping_matches_python() {
    let f = fixture();
    let kw = &f["keywords"];
    for c in f["keyword_cases"].as_array().unwrap() {
        let list = |v: &Value| v.as_array().unwrap().iter().map(|x| s(x).to_string()).collect::<Vec<_>>();
        assert_eq!(match_keyword(s(&c["cat"]), kw), list(&c["match"]), "match {}", c["cat"]);
        assert_eq!(map_bookstore_candidates(s(&c["cat"]), s(&c["title"]), s(&c["author"]), Some(kw)), list(&c["candidates"]), "cand {}", c["cat"]);
    }
}

// ---------------------------------------------------------------------------
// 보안과 경계
// ---------------------------------------------------------------------------

struct Fixed(Prediction);

impl Classify for Fixed {
    fn is_available(&self) -> bool {
        true
    }

    fn classify_path(&self, _p: &Path, _m: Option<f64>) -> Result<Prediction, String> {
        Ok(self.0.clone())
    }

    fn classify_document(&self, _d: &Value, _m: Option<f64>) -> Result<Prediction, String> {
        Ok(self.0.clone())
    }

    fn min_confidence(&self) -> Option<f64> {
        Some(0.056)
    }

    fn expected_accuracy(&self, confidence: f64) -> Option<f64> {
        Some(confidence * 2.0)
    }
}

fn service(tmp: &TempDir, category: &str, confidence: f64) -> BookClassifierService {
    BookClassifierService::new(tmp.path(), Some(&tmp.path().join("cache.json")), Some(Arc::new(Fixed(Prediction::new(Some(category), confidence, vec![(category.to_string(), confidence)], "stub")))))
        .with_delay(0.0)
}

#[test]
fn process_file_refuses_unsafe_target_category_names() {
    for bad in ["../outside", "a/b", "..", "/abs", ""] {
        let tmp = TempDir::new().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        let f = lib.join("책.txt");
        fs::write(&f, "x").unwrap();
        let mut svc = BookClassifierService::new(&lib, Some(&tmp.path().join("cache.json")), Some(Arc::new(Fixed(Prediction::new(Some(bad), 0.9, vec![], "stub"))))).with_delay(0.0);
        let res = svc.process_file(&f, &lib, ProcessOptions { use_bookstore: false, ..Default::default() });
        // 라이브러리 밖으로 새거나 파일이 사라지면 안 된다.
        assert!(f.exists(), "{bad:?}");
        assert!(!tmp.path().join("outside").exists(), "{bad:?}");
        assert_ne!(res["action"], "moved", "{bad:?}");
    }
}

#[test]
fn cache_files_are_read_defensively_and_saved_atomically() {
    let tmp = TempDir::new().unwrap();
    // 객체가 아닌 JSON 과 심볼릭 링크는 빈 캐시로 본다.
    let list = tmp.path().join("list.json");
    fs::write(&list, "[1, 2]").unwrap();
    assert!(BookClassifierService::new(tmp.path(), Some(&list), None).cache().is_empty());
    let real = tmp.path().join("real.json");
    fs::write(&real, r#"{"a": {"search_title": "x"}}"#).unwrap();
    let link = tmp.path().join("link.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(BookClassifierService::new(tmp.path(), Some(&link), None).cache().is_empty());

    // 저장은 임시 파일 뒤 교체이고, 임시 파일이 남지 않는다.
    let mut svc = BookClassifierService::new(tmp.path(), Some(&tmp.path().join("c.json")), None);
    svc.set_cache(json!({"k": {"search_title": "삼국지"}}));
    svc.save_cache();
    svc.save_cache();
    let names: Vec<String> = fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    let saved: Value = serde_json::from_slice(&fs::read(tmp.path().join("c.json")).unwrap()).unwrap();
    assert_eq!(saved["k"]["search_title"], "삼국지");
}

// ---------------------------------------------------------------------------
// book_manager 연결: `BookClassifierApi`
// ---------------------------------------------------------------------------

#[test]
fn shared_service_serves_the_book_manager_facade() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("3_판타지").join("[남희성] 달빛조각사 1권.txt");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, "본문").unwrap();
    let shared: Arc<dyn BookClassifierApi> = Arc::new(SharedBookClassifier::new(service(&tmp, "3_판타지", 0.9).with_bookstore_policy(BookstorePolicy { override_below: 0.08, ..Default::default() })));

    let (category, method, reason, entry) = shared.classify_file(&f);
    assert_eq!((category.as_deref(), method.as_str()), (Some("3_판타지"), "model"));
    assert!(reason.contains("stub"));
    assert_eq!(entry["model_category"], "3_판타지");
    assert_eq!(entry["confidence"], json!(0.9));

    assert_eq!(shared.override_below(), 0.08);
    assert!(shared.prefers_bookstore(0.05) && !shared.prefers_bookstore(0.5));
    assert_eq!(shared.min_confidence(), Some(0.056));
    assert_eq!(shared.expected_accuracy(Some(0.25)), Some(Some(0.5)));
    assert_eq!(shared.expected_accuracy(None), Some(None));
}

#[test]
fn policy_and_cache_files_reject_symlinks_and_oversized_input() {
    use backend::bookstore_cache::BookstoreResponseCache;
    let tmp = TempDir::new().unwrap();

    // 정책 파일: 심볼릭 링크와 상한 초과는 기본값으로 본다.
    let real = tmp.path().join("real.json");
    fs::write(&real, json!({"bookstore_override_below": 0.08}).to_string()).unwrap();
    assert_eq!(BookstorePolicy::load(Some(&real)).override_below, 0.08);
    let link = tmp.path().join("bookstore_policy.json");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(BookstorePolicy::load(Some(&link)).override_below, 0.0);
    let big = tmp.path().join("big.json");
    fs::write(&big, format!("{{\"sample\": 1, \"pad\": \"{}\"}}", "x".repeat(backend::classifier::bookstore_policy::MAX_POLICY_BYTES as usize))).unwrap();
    assert_eq!(BookstorePolicy::load(Some(&big)).sample, 0);
    // 깊이가 한도를 넘는 JSON 도 기본값이다.
    let deep = tmp.path().join("deep.json");
    fs::write(&deep, format!("{}1{}", "[".repeat(5000), "]".repeat(5000))).unwrap();
    assert_eq!(BookstorePolicy::load(Some(&deep)), BookstorePolicy { ..Default::default() });

    // 서점 캐시: 링크를 따라 읽거나 쓰지 않는다.
    let stores = json!({"yes24": {"cat": "소설", "isbn": "9791162203484", "author": "남희성"}, "aladin": {}, "kyobo": {}});
    let target = tmp.path().join("target.jsonl");
    BookstoreResponseCache::new(&target).put(&stores, "달빛조각사", "남희성", "");
    let cache_link = tmp.path().join("bookstore_cache.jsonl");
    std::os::unix::fs::symlink(&target, &cache_link).unwrap();
    let before = fs::read(&target).unwrap();
    let mut cache = BookstoreResponseCache::new(&cache_link);
    assert_eq!(cache.get("", "달빛조각사", "남희성"), None, "링크는 읽지 않는다");
    cache.put(&stores, "다른책", "남희성", "");
    assert_eq!(fs::read(&target).unwrap(), before, "링크로 쓰지 않는다");
}
