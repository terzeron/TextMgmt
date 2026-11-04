//! `backend::book_manager` 단위 테스트.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;
use crate::common::shared_fixtures::epub_with_missing_page;
use crate::common::shared_fixtures::manager;
use crate::common::shared_fixtures::{NCX_NS_STR, Setup, setup};
use crate::common::shared_fixtures::{model_dir, propose, tmp_dir};
use backend::book_manager::{_prune_dead_ncx_navpoints, ProposeOptions};
use bookstore::store_clients::{BookstoreSearch, SearchRow};
use classifier::book_classifier::{BookClassifierService, BookstorePolicy, SharedBookClassifier};
use classifier::inference_model::CategoryModel;
use classifier::{BookCategoryClassifier, ClassifierOptions, Classify};
use common::faulty_fs::FaultyFs;
use common::zipx::read_zip_entry;
use fx::{Opts, with_deps};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering};
use std::{fs, io};

/// `fx::env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
// TODO(common): `book_fixtures::Opts`가 Deps 설정 훅(`configure: Option<Box<dyn FnOnce(BookManagerDeps) -> BookManagerDeps>>`)을 받으면 이 헬퍼가 필요 없다.
#[tokio::test]
async fn test_os_error_while_saving_reports_the_system_reason() {
    let faulty = FaultyFs::new();
    // OSError(28): 편집 파일을 백업하기 전에 동기화 단계에서 디스크 부족이 난다.
    faulty.set_sync_file(|_| Some(io::Error::from_raw_os_error(28)));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("파일 처리 실패: No space left on device"), "{error:?}");
    e.assert_nothing_changed(&original);
}

#[tokio::test]
async fn test_unexpected_error_while_saving_does_not_leak_panic_details() {
    let faulty = FaultyFs::new();
    faulty.set_sync_file(|_| panic!("internal path /x/y"));
    let e = with_deps(fx::env(Opts::loader()), true, |d| d.with_fs(faulty));
    let original = fs::read(&e.pdf).unwrap();

    let (result, error) = e.run_pages(&[1]).await;

    assert!(result.is_none() && error.as_deref() == Some("PDF 페이지 삭제 실패 (RuntimeError)"), "{error:?}");
    e.assert_nothing_changed(&original);
}

fn ncx_srcs(data: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(data).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants().filter(|n| n.has_tag_name((NCX_NS_STR, "content"))).filter_map(|c| c.attribute("src").map(str::to_string)).collect()
}

#[test]
fn test_prune_dead_ncx_navpoints_drops_missing_targets_and_keeps_children() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("m.epub"), false);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml"]);
    let pruned = _prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).expect("pruned");
    // 판권 항목만 사라지고, 그 아래 자식은 부모 자리로 올라온다
    assert_eq!(ncx_srcs(&pruned), ["Text/a.xhtml#top", "Text/a.xhtml#kid"]);
}

#[test]
fn test_prune_dead_ncx_navpoints_returns_none_when_nothing_is_dead() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("f.epub"), true);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml", "OEBPS/Text/colophon page.xhtml"]);
    assert!(_prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).is_none());
}

#[test]
fn test_prune_dead_ncx_navpoints_ignores_unparsable_ncx_and_empty_src() {
    assert!(_prune_dead_ncx_navpoints(b"<ncx", "OEBPS", &|_path| false).is_none());
    let no_src = format!(r#"<ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="x"><content src=""/></navPoint><navPoint id="y"/></navMap></ncx>"#).into_bytes();
    assert!(_prune_dead_ncx_navpoints(&no_src, "", &|_path| false).is_none());
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
