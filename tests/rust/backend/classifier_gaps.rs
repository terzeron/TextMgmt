//! 원본: tests/test_classifier_gaps.py
//!
//! 가정한 API (docs/rust-test-seams.md 3.12, 3.13):
//! - `backend::book_classifier::{BookClassifierService, clean_empty_parent_dirs_with, inspect_epub_metadata, is_title_match_reliable}`
//!   - `BookClassifierService::new(library_root: &Path, cache_file: Option<&Path>, classifier: Option<Arc<dyn Classify>>) -> Self` (구 `Option<BookCategoryClassifier>`)
//!   - `BookClassifierService::_decide_by_bookstore(&self, entry: &mut Value) -> (String, String)` (Python은 튜플, 둘째 값이 판정 사유 코드; Rust 이름은 `decide_by_bookstore`)
//!   - `clean_empty_parent_dirs_with(child: &Path, root: &Path, fs: &dyn sysdeps::FsOps)` (경로 해석은 `FsOps::canonicalize`. 구 전역 `set_resolve_hook` 대체. 문서에 없는 가정이라 해당 테스트는 `#[ignore]` 유지)
//!   - `inspect_epub_metadata(path: &Path) -> Value`
//!   - `is_title_match_reliable(a: &str, b: &str) -> bool`
//! - `backend::classifier::{BookCategoryClassifier, ClassifierOptions}`: `BookCategoryClassifier::new(model: Option<Arc<dyn CategoryModelApi>>, ClassifierOptions) -> Self`
//!   - `ClassifierOptions { es: Option<Arc<dyn EsTransport>>, library_root: Option<PathBuf>, min_confidence: Option<f64> }` (Default 구현)
//!   - `BookCategoryClassifier::expected_accuracy(&self, f64) -> Option<f64>`
//! - `backend::classifier::model::CategoryModelApi` trait: `fn expected_accuracy(&self, f64) -> Option<f64>` (MagicMock 대체 seam. 구 `ClassifierModel`. 다른 메서드는 기본 구현)
//! - `backend::classifier::features::FeatureSpace::new(&Value) -> Self`, `FeatureSpace::names() -> Vec<String>`

mod common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use backend::book_classifier::{BookClassifierService, clean_empty_parent_dirs_with, inspect_epub_metadata, is_title_match_reliable};
use backend::classifier::features::FeatureSpace;
use backend::classifier::model::CategoryModelApi;
use backend::classifier::{BookCategoryClassifier, ClassifierOptions};
use common::faulty_fs::{FaultyFs, other_err};
use common::zipx::write_zip;
use serde_json::json;
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

#[test]
fn test_feature_space_names_follow_field_order() {
    let space = FeatureSpace::new(&json!({"title": {}, "body": {}}));
    assert_eq!(space.names(), vec!["title", "body"]);
}

#[test]
fn test_inspect_epub_metadata_returns_empty_meta_for_non_xml_opf() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("bad.epub");
    write_zip(&epub, &[("content.opf", "xml 이 아니다".as_bytes())]);
    assert_eq!(inspect_epub_metadata(&epub), json!({"title": "", "author": "", "subject": "", "description": ""}));
}

#[test]
fn test_title_match_accepts_reordered_words_with_high_bigram_similarity() {
    assert!(is_title_match_reliable("마법사의 돌 해리 포터", "해리 포터 마법사의 돌"));
}

#[test]
fn test_title_match_rejects_unrelated_titles() {
    assert!(!is_title_match_reliable("은하수를 여행하는 히치하이커", "조선시대 농업 경제사 연구"));
}

#[test]
fn test_clean_empty_parent_dirs_ignores_unresolvable_path() {
    let tmp = TempDir::new().unwrap();
    let child = tmp.path().join("a");
    fs::create_dir(&child).unwrap();

    // 가정한 seam: 경로 해석(`FsOps::canonicalize`)을 `common::FaultyFs`로 실패시킨다. 해석이 실패하면 아무것도 지우지 않아야 한다.
    let faulty = FaultyFs::new();
    faulty.set_resolve(|_p| Some(other_err("경로 해석 실패")));

    clean_empty_parent_dirs_with(&child, tmp.path(), faulty.as_ref());

    assert!(child.exists());
}

#[test]
fn test_bookstore_vote_skips_reliable_store_without_candidates() {
    let tmp = TempDir::new().unwrap();
    let service = BookClassifierService::new(tmp.path(), Some(&tmp.path().join("cache.json")), None);
    let mut entry = json!({"search_title": "해리 포터와 마법사의 돌", "yes24": {"title": "해리 포터와 마법사의 돌"}, "aladin": {}, "kyobo": {}});

    let result = service.decide_by_bookstore(&mut entry);

    assert_eq!(result.1, "not_found");
    assert_eq!(entry["bookstore_candidates"], json!([]));
}
