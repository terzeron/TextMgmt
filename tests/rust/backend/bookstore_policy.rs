//! 원본: tests/test_bookstore_policy.py
//!
//! 서점 신호 규칙 테스트. 두 가지를 검증한다.
//! 1. 매핑 우선순위 - 저자·시리즈 규칙 > DB 키워드 > 하드코딩 규칙
//! 2. 결합 규칙 - 측정된 경계 아래에서는 서점을 모델보다 먼저 본다
//!
//! 가정한 API (Python dict는 `serde_json::Value`로 받는다):
//! - `backend::classifier::bookstore_policy`
//!   - `keyword_map(loader: Option<&dyn Fn() -> Result<Value, String>>, force: bool) -> Value`
//!   - `reset_keyword_cache()`
//!   - `match_keyword(cat_str: &str, keywords: &Value) -> Vec<String>`
//!   - `map_bookstore_category(cat_str: &str, raw_title: &str, raw_author: &str, keywords: Option<&Value>) -> Option<String>`
//!   - `map_bookstore_candidates(cat_str: &str, raw_title: &str, raw_author: &str, keywords: Option<&Value>) -> Vec<String>`
//!   - `quantile_bands(confidences: &[f64], count: usize) -> Vec<f64>` (`window`는 기본값 0.5)
//!   - `bands_from_cuts(cuts: &[f64]) -> Vec<(f64, f64)>`
//!   - `BookstorePolicy { override_below: f64, sample: i64, bands: Vec<Value>, source: Option<String>, measured_max: f64 }` (`Default`)
//!     - `BookstorePolicy::load(path: Option<&Path>) -> Self`, `prefers_bookstore(&self, confidence: f64) -> bool`
//!   - `_load_from_db_with(get_all_mappings: &dyn Fn(&str) -> Value) -> Value` (Python이 `sys.modules`에 끼우던 `CategoryMapping.get_all_mappings(content_type)` 대역을 인자로 받는 seam)
//! - `backend::book_classifier::{BookClassifierService, DecideOptions}`
//!   - `BookClassifierService::new(library_root: &Path, cache_file: Option<&Path>, classifier: Option<Arc<dyn Classify>>) -> Self`
//!   - `with_bookstore_policy(self, BookstorePolicy) -> Self`, `classifier_library_root(&self) -> PathBuf`
//!   - `_decide(&self, fpath: Option<&Path>, effective_fname: &str, entry: &mut Value, DecideOptions) -> Decision { category: Option<String>, method: String, reason: String, .. }`
//!   - `with_bookstore_decider(self, Arc<dyn BookstoreDecider>) -> Self` (docs/rust-test-seams.md 3.12; 구 `wrap_decide_by_bookstore`)
//!   - `BookstoreDecider: Send + Sync`의 `decide(&self, entry: &mut Value, trust_single_match: bool, original: &dyn Fn(&mut Value, bool) -> (Option<String>, String, String)) -> (Option<String>, String, String)`
//!     (문서에 시그니처가 없어 기존 래퍼 클로저 모양을 그대로 옮긴 가정이다. 기본 구현은 `original`을 그대로 호출한다)
//! - `backend::classifier::{Classify, model::Prediction}`는 tests/rust/backend/book_classifier.rs와 같다.

mod common;

use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use backend::book_classifier::{BookClassifierService, BookstoreDecider, DecideOptions};
use backend::classifier::Classify;
use backend::classifier::bookstore_policy::{
    self, BookstorePolicy, bands_from_cuts, keyword_map, map_bookstore_candidates, map_bookstore_category, match_keyword, quantile_bands, reset_keyword_cache,
};
use backend::classifier::model::Prediction;
use serde_json::{Value, json};
use tempfile::TempDir;

/// 실제 DB 에 들어 있는 값에서 뽑았다. 하드코딩 규칙과 다른 4건이 포함돼 있다.
fn db_keywords() -> Value {
    json!({
        "한문": ["1_동양고전한문"],
        "영어전문교육": ["7_영어교육"],
        "자녀교육": ["9_어린이육아"],
        "반도체": ["8_IT"],
        "클래식": ["1_서양고전", "5_음악"],
        "환상문학": ["3_SF", "3_판타지"],
    })
}

static CACHE_LOCK: Mutex<()> = Mutex::new(());

/// autouse fixture `clean_cache` 대응. 키워드 캐시가 프로세스 전역이라 테스트끼리 직렬화한다.
struct CleanCache(#[allow(dead_code)] MutexGuard<'static, ()>);

fn clean_cache() -> CleanCache {
    let guard = CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_keyword_cache();
    CleanCache(guard)
}

impl Drop for CleanCache {
    fn drop(&mut self) {
        reset_keyword_cache();
    }
}

// ---------------------------------------------------------------------------
// 키워드 맵
// ---------------------------------------------------------------------------

#[test]
fn test_keyword_map_caches_and_reloads_on_demand() {
    let _clean = clean_cache();
    let calls = Cell::new(0);
    let loader = || {
        calls.set(calls.get() + 1);
        Ok(db_keywords())
    };

    assert_eq!(keyword_map(Some(&loader), false), db_keywords());
    keyword_map(Some(&loader), false);
    assert_eq!(calls.get(), 1, "5분 안에는 DB 를 다시 읽지 않는다");

    keyword_map(Some(&loader), true);
    assert_eq!(calls.get(), 2);
}

/// DB 가 죽어도 판정은 계속돼야 한다. 빈 맵을 주고 하드코딩 규칙으로 넘어간다.
#[test]
fn test_keyword_map_survives_a_dead_database() {
    let _clean = clean_cache();
    let broken = || Err("MySQL 연결 실패".to_string());

    assert_eq!(keyword_map(Some(&broken), false), json!({}));
}

/// 한 키워드가 여러 카테고리에 붙는 것은 정상이다.
///
/// 테이블의 유일 키가 (category, keyword, content_type) 조합이고 search_by_keyword 도
/// 목록을 돌려준다. `환상문학`(3_SF, 3_판타지)과 `클래식`(1_서양고전, 5_음악)이 그렇다.
/// 어느 쪽인지는 서점 다수결이 가른다.
#[test]
fn test_keyword_with_two_categories_keeps_both() {
    let _clean = clean_cache();
    let fake_mapping = |_content_type: &str| json!({"3_SF": ["환상문학", "과학소설"], "3_판타지": ["환상문학"], "5_음악": ["클래식"]});

    let loaded = bookstore_policy::_load_from_db_with(&fake_mapping);
    let mut fantasy: Vec<&str> = loaded["환상문학"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    fantasy.sort();
    assert_eq!(fantasy, ["3_SF", "3_판타지"]);
    assert_eq!(loaded["과학소설"], json!(["3_SF"]));
}

#[test]
fn test_multi_category_keyword_gives_no_single_answer() {
    let _clean = clean_cache();
    let keywords = db_keywords();
    // 후보가 둘이면 한 서점만으로는 못 고른다. 다수결로 넘긴다.
    assert_eq!(map_bookstore_category("예술 > 클래식", "", "", Some(&keywords)), None);
    let mut candidates = map_bookstore_candidates("예술 > 클래식", "", "", Some(&keywords));
    candidates.sort();
    assert_eq!(candidates, ["1_서양고전", "5_음악"]);
}

#[test]
fn test_match_keyword_prefers_the_longer_word() {
    let _clean = clean_cache();
    // '영어전문교육' 이 '영어' 보다 구체적이다. 짧은 쪽이 먼저 맞으면 7_교육일반 으로 샌다.
    let keywords = json!({"영어": ["7_영어교육"], "영어전문교육": ["7_영어교육"], "교육": ["7_교육일반"]});
    assert_eq!(match_keyword("외국어 > 영어전문교육", &keywords), ["7_영어교육"]);
    assert_eq!(match_keyword("", &keywords), Vec::<String>::new());
    assert_eq!(match_keyword("소설 > 한국소설", &json!({})), Vec::<String>::new());
}

// ---------------------------------------------------------------------------
// 매핑 우선순위
// ---------------------------------------------------------------------------

/// 히가시노 게이고는 '일본소설' 이기 전에 2_소설일본게이고 다.
#[test]
fn test_series_rule_wins_over_db_keyword() {
    let _clean = clean_cache();
    let keywords = json!({"일본소설": "2_소설일본"});
    assert_eq!(map_bookstore_category("소설 > 일본소설", "용의자 X의 헌신", "히가시노 게이고", Some(&keywords)).as_deref(), Some("2_소설일본게이고"));
}

/// DB 가 더 구체적인 4건. 하드코딩은 한문을 1_동양고전 으로 보낸다.
#[test]
fn test_db_keyword_wins_over_the_hardcoded_rule() {
    let _clean = clean_cache();
    let keywords = db_keywords();
    assert_eq!(map_bookstore_category("고전 > 한문", "", "", Some(&keywords)).as_deref(), Some("1_동양고전한문"));
    assert_eq!(map_bookstore_category("외국어 > 영어전문교육", "", "", Some(&keywords)).as_deref(), Some("7_영어교육"));
    assert_eq!(map_bookstore_category("가정 > 자녀교육", "", "", Some(&keywords)).as_deref(), Some("9_어린이육아"));
}

#[test]
fn test_db_keyword_fills_what_the_hardcoded_rule_misses() {
    let _clean = clean_cache();
    // '반도체' 는 하드코딩 규칙에 없다. DB 가 메운다.
    assert_eq!(map_bookstore_category("공학 > 반도체", "", "", Some(&db_keywords())).as_deref(), Some("8_IT"));
}

/// DB 는 35개 카테고리뿐이다. 나머지 38개는 하드코딩 규칙이 맡는다.
#[test]
fn test_hardcoded_rule_covers_what_the_db_does_not() {
    let _clean = clean_cache();
    let keywords = db_keywords();
    assert_eq!(map_bookstore_category("소설 > 무협소설", "", "", Some(&keywords)).as_deref(), Some("3_무협"));
    assert_eq!(map_bookstore_category("소설 > 라이트노벨", "", "", Some(&keywords)).as_deref(), Some("3_라이트노벨"));
}

#[test]
fn test_unknown_category_maps_to_nothing() {
    let _clean = clean_cache();
    assert_eq!(map_bookstore_category("듣도 보도 못한 분야", "", "", Some(&db_keywords())), None);
}

// ---------------------------------------------------------------------------
// 결합 규칙
// ---------------------------------------------------------------------------

/// 규칙 파일이 없으면 예전 동작이다. 모델이 늘 먼저다.
#[test]
fn test_policy_without_a_file_never_prefers_the_bookstore() {
    let _clean = clean_cache();
    let policy = BookstorePolicy::load(Some(Path::new("/없는/경로/bookstore_policy.json")));
    assert_eq!(policy.override_below, 0.0);
    assert!(!policy.prefers_bookstore(0.001));
}

#[test]
fn test_policy_reads_the_measured_boundary() {
    let _clean = clean_cache();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bookstore_policy.json");
    fs::write(&path, json!({"sample": 2000, "bookstore_override_below": 0.08, "bands": [{"low": 0.0, "high": 0.5}]}).to_string()).unwrap();

    let policy = BookstorePolicy::load(Some(&path));
    assert_eq!(policy.override_below, 0.08);
    assert_eq!(policy.sample, 2000);
    assert!(policy.prefers_bookstore(0.05));
    assert!(!policy.prefers_bookstore(0.08), "경계값은 모델 쪽이다");
}

#[test]
fn test_policy_tolerates_a_broken_file() {
    let _clean = clean_cache();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bookstore_policy.json");
    fs::write(&path, "{ 이건 JSON 이 아니다").unwrap();
    assert_eq!(BookstorePolicy::load(Some(&path)).override_below, 0.0);
}

// ---------------------------------------------------------------------------
// 판정 경로에 실제로 적용되는가
// ---------------------------------------------------------------------------

/// 정해진 예측을 늘 돌려주는 모델 대역.
struct StubClassifier {
    pred: Prediction,
}

impl StubClassifier {
    /// `StubPrediction(category, confidence, reason="stub")`와 `ranked = [(category or "-", confidence)]` 대응.
    fn new(category: Option<&str>, confidence: f64) -> Self {
        Self::with_reason(category, confidence, "stub")
    }

    fn with_reason(category: Option<&str>, confidence: f64, reason: &str) -> Self {
        Self { pred: Prediction::new(category, confidence, vec![(category.unwrap_or("-").to_string(), confidence)], reason) }
    }
}

impl Classify for StubClassifier {
    fn is_available(&self) -> bool {
        true
    }

    fn classify_path(&self, _fpath: &Path, _min_confidence: Option<f64>) -> Result<Prediction, String> {
        Ok(self.pred.clone())
    }

    fn classify_document(&self, _doc: &Value, _min_confidence: Option<f64>) -> Result<Prediction, String> {
        Ok(self.pred.clone())
    }
}

fn service(tmp: &TempDir, classifier: StubClassifier, policy: BookstorePolicy) -> BookClassifierService {
    BookClassifierService::new(tmp.path(), Some(&tmp.path().join("cache.json")), Some(Arc::new(classifier))).with_bookstore_policy(policy)
}

fn entry_with(store_cat: &str, title: &str) -> Value {
    json!({"search_title": title, "yes24": {"mapped": store_cat, "title": title}, "aladin": {"mapped": store_cat, "title": title}, "kyobo": {}})
}

fn entry() -> Value {
    entry_with("3_무협", "달빛조각사")
}

/// 모델이 답을 내도 확신도가 경계 아래면 서점을 따른다.
#[test]
fn test_bookstore_wins_below_the_measured_boundary() {
    let tmp = TempDir::new().unwrap();
    let svc = service(&tmp, StubClassifier::new(Some("3_판타지"), 0.05), BookstorePolicy { override_below: 0.08, ..Default::default() });

    let d = svc._decide(None, "달빛조각사.txt", &mut entry(), DecideOptions::default());
    assert_eq!(d.category.as_deref(), Some("3_무협"));
    assert_eq!(d.method, "bookstore_majority");
    assert!(d.reason.contains("서점 우선"), "reason: {}", d.reason);
}

#[test]
fn test_model_wins_above_the_boundary() {
    let tmp = TempDir::new().unwrap();
    let svc = service(&tmp, StubClassifier::new(Some("3_판타지"), 0.20), BookstorePolicy { override_below: 0.08, ..Default::default() });

    let d = svc._decide(None, "달빛조각사.txt", &mut entry(), DecideOptions::default());
    assert_eq!(d.category.as_deref(), Some("3_판타지"));
    assert_eq!(d.method, "model");
}

/// 경계가 0 이어도 모델이 판정을 거부하면 서점이 메운다. 예전 동작이다.
#[test]
fn test_bookstore_still_fills_the_gap_when_the_model_refuses() {
    let tmp = TempDir::new().unwrap();
    let svc = service(&tmp, StubClassifier::with_reason(None, 0.03, "확신도 부족"), BookstorePolicy { override_below: 0.0, ..Default::default() });

    let d = svc._decide(None, "달빛조각사.txt", &mut entry(), DecideOptions::default());
    assert_eq!(d.category.as_deref(), Some("3_무협"));
    assert_eq!(d.method, "bookstore_majority");
}

/// `_decide_by_bookstore` 호출 횟수를 세고 원래 구현에 넘기는 `BookstoreDecider`.
struct CountingDecider {
    calls: Arc<AtomicUsize>,
}

impl BookstoreDecider for CountingDecider {
    fn decide(&self, entry: &mut Value, trust_single_match: bool, original: &dyn Fn(&mut Value, bool) -> (Option<String>, String, String)) -> (Option<String>, String, String) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        original(entry, trust_single_match)
    }
}

/// 서점 조회는 한 건에 3초 이상 걸린다. 규칙을 적용해도 두 번 부르면 안 된다.
#[test]
fn test_bookstore_is_queried_once_even_when_the_policy_prefers_it() {
    let tmp = TempDir::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let svc =
        service(&tmp, StubClassifier::new(None, 0.01), BookstorePolicy { override_below: 0.08, ..Default::default() }).with_bookstore_decider(Arc::new(CountingDecider { calls: Arc::clone(&calls) }));

    svc._decide(None, "달빛조각사.txt", &mut entry(), DecideOptions::default());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// 확신도 구간
// ---------------------------------------------------------------------------

/// 경계는 값이 촘촘한 곳이 아니라 벌어진 곳에 잡혀야 한다.
///
/// 고정 구간(0.5/0.7/0.8)으로 자르면 2,000건 중 1,884건이 한 칸에 몰렸다.
#[test]
fn test_quantile_bands_split_where_samples_are_sparse() {
    // 0.10 부근과 0.50 부근에 뭉쳐 있고 그 사이가 비어 있다
    let dense_low = (0..50).map(|i| 0.10 + i as f64 * 0.0001);
    let dense_high = (0..50).map(|i| 0.50 + i as f64 * 0.0001);
    let values: Vec<f64> = dense_low.chain(dense_high).collect();
    let cuts = quantile_bands(&values, 2);

    assert_eq!(cuts.len(), 1);
    // 빈 구간 안에서 잘라야 한다. 뭉친 값 사이를 가르면 안 된다.
    assert!(0.1050 < cuts[0] && cuts[0] < 0.50, "cuts: {cuts:?}");
}

#[test]
fn test_quantile_bands_handle_tiny_samples() {
    assert_eq!(quantile_bands(&[], 4), Vec::<f64>::new());
    assert_eq!(quantile_bands(&[0.3], 4), Vec::<f64>::new());
}

#[test]
fn test_bands_from_cuts_covers_the_whole_range() {
    let bands = bands_from_cuts(&[0.05, 0.12]);
    assert_eq!(bands[0].0, 0.0);
    assert!(bands.last().unwrap().1 > 1.0);
    // 구간이 끊기지 않고 이어져야 한다
    let highs: Vec<f64> = bands[..bands.len() - 1].iter().map(|(_, hi)| *hi).collect();
    let lows: Vec<f64> = bands[1..].iter().map(|(lo, _)| *lo).collect();
    assert_eq!(highs, lows);
}

/// 서점마다 후보를 여럿 내도 겹치는 카테고리가 이긴다.
///
/// `클래식` 은 1_서양고전 이자 5_음악 이다. 다른 서점이 5_음악 쪽을 가리키면 그쪽으로 굳는다.
#[test]
fn test_overlapping_candidate_wins_the_vote() {
    let tmp = TempDir::new().unwrap();
    let svc = service(&tmp, StubClassifier::new(None, 0.01), BookstorePolicy::default());

    let mut entry = json!({
        "search_title": "사계",
        "yes24": {"candidates": ["1_서양고전", "5_음악"], "title": "사계"},
        "aladin": {"candidates": ["5_음악"], "title": "사계"},
        "kyobo": {},
    });
    let d = svc._decide(None, "사계.epub", &mut entry, DecideOptions::default());
    assert_eq!(d.category.as_deref(), Some("5_음악"));
    assert_eq!(d.method, "bookstore_majority");
}

/// 한 곳만 답했는데 후보가 둘이면 고를 근거가 없다. 수동으로 넘긴다.
#[test]
fn test_single_store_with_two_candidates_refuses() {
    let tmp = TempDir::new().unwrap();
    let svc = service(&tmp, StubClassifier::new(None, 0.01), BookstorePolicy::default());

    let mut entry = json!({"search_title": "사계", "yes24": {"candidates": ["1_서양고전", "5_음악"], "title": "사계"}, "aladin": {}, "kyobo": {}});
    let d = svc._decide(None, "사계.epub", &mut entry, DecideOptions { trust_single_match: true });
    assert_eq!(d.category, None);
    assert_eq!(d.method, "conflict");
}

/// 재지 않은 구간까지 서점에 넘기면 안 된다.
///
/// 거부 구간에서만 뽑으면 마지막 구간의 상한이 1.01(열린 위쪽 끝)이다. 모든 구간에서
/// 서점이 이기면 경계가 1.01 로 나와 확신도와 무관하게 늘 서점을 따르게 된다.
/// 실제로 그 값이 나왔다.
#[test]
fn test_boundary_never_exceeds_the_measured_range() {
    let _clean = clean_cache();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bookstore_policy.json");
    fs::write(&path, json!({"sample": 800, "bookstore_override_below": 1.01, "measured_max_confidence": 0.056, "bands": []}).to_string()).unwrap();

    let policy = BookstorePolicy::load(Some(&path));
    assert_eq!(policy.override_below, 0.056);
    assert!(policy.prefers_bookstore(0.05));
    assert!(!policy.prefers_bookstore(0.2), "재지 않은 고확신 구간은 모델이 지킨다");
}

/// pod 는 책을 /books 로 마운트한다. 설정 기본값 /mnt/data/text 를 쓰면 경로 조회가 죽는다.
///
/// ES 의 file_path 는 라이브러리 루트 기준 상대 경로라, 루트가 어긋나면
/// 상대 경로를 못 만들고 절대 경로로 질의해 0건이 된다.
#[test]
fn test_service_passes_its_library_root_to_the_classifier() {
    let tmp = TempDir::new().unwrap();

    let service = BookClassifierService::new(&tmp.path().join("books"), Some(&tmp.path().join("cache.json")), None);
    assert_eq!(service.classifier_library_root(), tmp.path().join("books"));
}

#[test]
fn test_keyword_map_ignores_blank_keywords() {
    let _clean = clean_cache();
    let fake_mapping = |_content_type: &str| json!({"3_SF": ["  ", "", null, " 과학소설 "]});

    assert_eq!(bookstore_policy::_load_from_db_with(&fake_mapping), json!({"과학소설": ["3_SF"]}));
}

/// 값이 둘뿐이면 분위수 자리 좌우에 살필 칸이 없어 경계를 만들지 않는다.
#[test]
fn test_quantile_bands_skips_cut_when_search_window_is_empty() {
    assert_eq!(quantile_bands(&[0.1, 0.2], 2), Vec::<f64>::new());
}
