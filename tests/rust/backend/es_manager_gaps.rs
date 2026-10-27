//! 원본: tests/test_es_manager_gaps.py
//!
//! ESManager의 빈 입력, 페이지 끝, 유사 이름 검색 분기를 가짜 ES 클라이언트로 확인한다.
//!
//! 가정한 API (설계: docs/rust-test-seams.md 3.3, 3.13):
//! - `backend::es_manager::{ESManager, EsTransport, EsError}`. 구 `EsApi`는 `EsTransport`, 구 `ElasticsearchError`는 `EsError`다.
//! - `ESManager::with_transport(es: Arc<dyn EsTransport>) -> ESManager` (구 `with_client(es, index)`. 색인 이름은 `EsTransport::index_name()`이 준다)
//! - `ESManager::get_existing_created_time_metadata(&self, doc_ids: &[i64]) -> Result<HashMap<i64, Value>, EsError>`
//! - `ESManager::search_similar_names(&self, name: &str, author: &str, max_result_count: i64) -> Result<Vec<(i64, Value, f64)>, EsError>`
//! - `ESManager::with_search_stub(self, f: Arc<dyn Fn(&Value, Option<&Value>, i64, Option<&[String]>) -> Result<Vec<(i64, Value, f64)>, EsError> + Send + Sync>) -> ESManager`
//!   (구 `set_search_override`, Python `manager._search = fake`의 대응 seam. `_search` 반환을 직접 주입해야 하는 테스트에만 쓴다. 설계 3.3)
//! - `ESManager::search_category_pdf_stats(&self, category: &str) -> Result<Value, EsError>`
//! - `ESManager::search_categories_pdf_stats(&self, categories: &[&str]) -> Result<Value, EsError>`
//! - `ESManager::iter_all_category_file_paths(&self, batch_size: usize) -> Box<dyn Iterator<Item = Result<(String, String, i64), EsError>>>`
//! - 공용 가짜 ES는 `common::es_dummy::DummyEs`(`EsTransport` 구현)

mod common;

use std::sync::{Arc, Mutex};

use backend::es_manager::{ESManager, EsError, EsTransport};
use common::es_dummy::DummyEs;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

type SearchStub = Arc<dyn Fn(&Value, Option<&Value>, i64, Option<&[String]>) -> Result<Vec<(i64, Value, f64)>, backend::es_manager::EsError> + Send + Sync>;

/// `_manager()`: 가짜 ES와 인덱스 이름 `idx`를 가진 매니저.
fn manager() -> (ESManager, Arc<DummyEs>) {
    let es = DummyEs::with_index("idx");
    (ESManager::with_transport(es.clone() as Arc<dyn EsTransport>), es)
}

/// `manager._search = fake_search`: 넘어온 쿼리를 `captured`에 담고 빈 결과를 돌려주는 `_search` 대역.
fn capture_search() -> (SearchStub, Arc<Mutex<Value>>) {
    let captured = Arc::new(Mutex::new(Value::Null));
    let sink = captured.clone();
    let stub: SearchStub = Arc::new(move |query, _sort, _max, _fields| {
        *sink.lock().unwrap() = query.clone();
        Ok(Vec::new())
    });
    (stub, captured)
}

#[test]
fn es_manager_api_default_methods_return_unsupported() {
    struct UnsupportedManager;
    impl backend::es_manager::EsManagerApi for UnsupportedManager {}

    macro_rules! unsupported {
        ($call:expr) => {
            assert!(matches!($call, Err(EsError::Unsupported)))
        };
    }

    let manager = UnsupportedManager;
    let docs = BTreeMap::new();
    let strings = vec!["A".to_owned()];
    let similar = json!({});
    unsupported!(backend::es_manager::EsManagerApi::create_index(&manager));
    unsupported!(backend::es_manager::EsManagerApi::search_by_id(&manager, 1));
    unsupported!(backend::es_manager::EsManagerApi::update(&manager, 1, &Value::Null));
    unsupported!(backend::es_manager::EsManagerApi::delete(&manager, 1));
    unsupported!(backend::es_manager::EsManagerApi::insert(&manager, &docs));
    unsupported!(backend::es_manager::EsManagerApi::delete_by_file_paths(&manager, &strings, None));
    unsupported!(backend::es_manager::EsManagerApi::delete_by_ids(&manager, &[1], 1));
    unsupported!(backend::es_manager::EsManagerApi::refresh(&manager));
    unsupported!(backend::es_manager::EsManagerApi::search_and_aggregate_by_category(&manager));
    unsupported!(backend::es_manager::EsManagerApi::iter_all_category_file_paths(&manager, 1));
    unsupported!(backend::es_manager::EsManagerApi::search_categories_pdf_stats(&manager, &strings));
    unsupported!(backend::es_manager::EsManagerApi::search_category_pdf_stats(&manager, "A"));
    unsupported!(backend::es_manager::EsManagerApi::get_by_id_source(&manager, "1"));
    unsupported!(backend::es_manager::EsManagerApi::search_by_path_source(&manager, "/a"));
    unsupported!(backend::es_manager::EsManagerApi::search_by_category(&manager, "A", 10));
    unsupported!(backend::es_manager::EsManagerApi::search_by_category_paged(&manager, "A", 10, None));
    unsupported!(backend::es_manager::EsManagerApi::count_by_categories(&manager, &strings, false));
    unsupported!(backend::es_manager::EsManagerApi::count_by_category(&manager, "A", false));
    unsupported!(backend::es_manager::EsManagerApi::rename_category(&manager, "A", "B"));
    unsupported!(backend::es_manager::EsManagerApi::delete_by_category(&manager, "A", false));
    unsupported!(backend::es_manager::EsManagerApi::search_by_keyword(&manager, "q", 10));
    unsupported!(backend::es_manager::EsManagerApi::search_by_keyword_paged(&manager, &json!({})));
    unsupported!(backend::es_manager::EsManagerApi::search_similar_docs(&manager, &similar));
    unsupported!(backend::es_manager::EsManagerApi::search_similar_docs_paged(&manager, &similar));
    unsupported!(backend::es_manager::EsManagerApi::search_latest_docs(&manager, 10, None));
    unsupported!(backend::es_manager::EsManagerApi::backfill_created_time(&manager, Path::new("/books")));
    unsupported!(backend::es_manager::EsManagerApi::aggregate_cover_categories(&manager));
    unsupported!(backend::es_manager::EsManagerApi::search_cover_files_paged(&manager, &strings, 10, None, &[]));
    unsupported!(backend::es_manager::EsManagerApi::search_group_representatives(&manager, &strings, &[]));
}

#[test]
fn es_error_display_keeps_each_error_kind_distinguishable() {
    let cases = [
        (EsError::MissingEnv("TM_ES_URL".into()), "환경 변수 TM_ES_URL이(가) 없다"),
        (EsError::Transport("offline".into()), "ES 연결 오류: offline"),
        (EsError::Status { status: 503, body: "busy".into() }, "ES 응답 오류 503: busy"),
        (EsError::Parse("invalid json".into()), "ES 응답 해석 실패: invalid json"),
        (EsError::Unsupported, "지원하지 않는 연산"),
        (EsError::Other("custom".into()), "custom"),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn es_manager_api_implementation_delegates_to_manager_and_normalizes_results() {
    use backend::es_manager::EsManagerApi;

    let (manager, es) = manager();
    es.on("search", |_| {
        Ok(json!({
            "hits": {"total": {"value": 0}, "max_score": 0, "hits": []},
            "aggregations": {
                "unique_values": {"buckets": [{"key": "A", "doc_count": 2}]},
                "paths": {"buckets": []},
                "pdfs": {"doc_count": 0, "total_pages": {"value": 0}, "total_file_size": {"value": 0}},
                "by_category": {"buckets": {"A": {"pdfs": {"doc_count": 1, "total_pages": {"value": 3}, "total_file_size": {"value": 4}}}}},
                "categories": {"buckets": [{"key": {"category": "A"}, "doc_count": 1, "updated": {"value": null}, "created": {"value": null}}]}
            }
        }))
    });
    let api: &dyn EsManagerApi = &manager;
    let categories = vec!["A".to_owned()];
    let exclude = vec!["hidden".to_owned()];
    let docs = BTreeMap::from([(1, json!({"title": "sample"}))]);

    assert!(api.create_index().is_ok());
    assert_eq!(api.search_by_id(1).unwrap(), None);
    let _ = api.update(1, &json!({"title": "updated"}));
    let _ = api.delete(1);
    let _ = api.insert(&docs);
    let _ = api.delete_by_file_paths(&["/books/a.epub".into()], None);
    let _ = api.delete_by_ids(&[1], 1);
    assert!(api.refresh().is_ok());
    assert_eq!(api.search_and_aggregate_by_category().unwrap(), BTreeMap::from([("A".into(), 2)]));
    assert!(api.iter_all_category_file_paths(10).unwrap().is_empty());
    assert_eq!(api.search_categories_pdf_stats(&categories).unwrap()["A"]["page_count"], 3);
    let _ = api.search_category_pdf_stats("A");
    assert_eq!(api.get_by_id_source("1").unwrap(), None);
    assert_eq!(api.search_by_path_source("/books/a.epub").unwrap(), None);
    let _ = api.search_by_category("A", 10);
    let _ = api.search_by_category_paged("A", 10, None);
    assert_eq!(api.count_by_categories(&categories, false).unwrap()["A"], 3);
    let _ = api.count_by_category("A", true);
    let _ = api.rename_category("A", "B");
    let _ = api.delete_by_category("A", false);
    let _ = api.search_by_keyword("sample", 10);
    let _ = api.search_by_keyword_paged(&json!({"keyword": "sample", "exclude_categories": ["hidden"]}));
    let _ = api.search_similar_docs(&json!({"title": "sample"}));
    let _ = api.search_similar_docs_paged(&json!({"title": "sample"}));
    let _ = api.search_latest_docs(10, Some(&exclude));
    let _ = api.backfill_created_time(Path::new("/books"));
    let aggregate = api.aggregate_cover_categories().unwrap();
    assert_eq!(aggregate["A"].0, 1);
    let _ = api.search_cover_files_paged(&categories, 10, Some((2, 1, "/cover".into())), &exclude);
    let _ = api.search_group_representatives(&categories, &exclude);
}

#[test]
fn test_existing_created_time_metadata_is_empty_for_no_ids() {
    let (manager, es) = manager();
    assert!(manager.get_existing_created_time_metadata(&[]).unwrap().is_empty());
    assert!(es.calls_of("mget").is_empty(), "빈 ID 목록은 mget을 부르면 안 된다");
}

#[test]
fn test_search_similar_names_escapes_wildcards_and_adds_author_clause() {
    let (stub, captured) = capture_search();
    let manager = manager().0.with_search_stub(stub);

    manager.search_similar_names("a*b?c\\d", "저자", 20).unwrap();

    let query = captured.lock().unwrap().clone();
    let should = &query["bool"]["should"];
    assert_eq!(should[1]["wildcard"]["file_path"]["value"], json!("*a\\*b\\?c\\\\d*"));
    let last = should.as_array().unwrap().last().unwrap();
    assert_eq!(last["match"]["author"]["query"], json!("저자"));
}

#[test]
fn test_search_similar_names_without_author_has_no_author_clause() {
    let (stub, captured) = capture_search();
    let manager = manager().0.with_search_stub(stub);

    manager.search_similar_names("제목", "", 20).unwrap();

    let query = captured.lock().unwrap().clone();
    assert_eq!(query["bool"]["should"].as_array().unwrap().len(), 2);
}

#[test]
fn test_search_category_pdf_stats_sums_pdf_documents() {
    let (manager, es) = manager();
    es.on("search", |_| Ok(json!({"aggregations": {"pdfs": {"doc_count": 3, "total_pages": {"value": 120.0}, "total_file_size": {"value": null}}}})));

    assert_eq!(manager.search_category_pdf_stats("A").unwrap(), json!({"file_count": 3, "page_count": 120, "total_file_size": 0}));
}

#[test]
fn test_search_categories_pdf_stats_returns_empty_for_no_categories() {
    let (manager, es) = manager();
    assert_eq!(manager.search_categories_pdf_stats(&[]).unwrap(), json!({}));
    assert!(es.calls_of("search").is_empty(), "빈 카테고리 목록은 검색을 부르면 안 된다");
}

#[test]
fn test_search_categories_pdf_stats_maps_each_bucket() {
    let (manager, es) = manager();
    es.on(
        "search",
        |_| Ok(json!({"aggregations": {"by_category": {"buckets": {"A": {"pdfs": {"doc_count": 2, "total_pages": {"value": 10}, "total_file_size": {"value": 99}}}, "B": {"pdfs": {"doc_count": 0, "total_pages": {"value": null}, "total_file_size": {"value": null}}}}}}})),
    );

    assert_eq!(
        manager.search_categories_pdf_stats(&["A", "B"]).unwrap(),
        json!({"A": {"file_count": 2, "page_count": 10, "total_file_size": 99}, "B": {"file_count": 0, "page_count": 0, "total_file_size": 0}})
    );
}

#[test]
fn test_iter_all_category_file_paths_stops_when_last_page_has_no_after_key() {
    let (manager, es) = manager();
    es.on("search", |_| Ok(json!({"aggregations": {"paths": {"buckets": [{"key": {"category": null, "file_path": "x.txt"}, "doc_count": 2}]}}})));

    let rows: Vec<(String, String, i64)> = manager.iter_all_category_file_paths(10000).collect::<Result<_, _>>().unwrap();

    assert_eq!(rows, vec![("".to_string(), "x.txt".to_string(), 2)]);
    assert_eq!(es.calls_of("search").len(), 1);
}

#[test]
fn test_iter_all_category_file_paths_follows_after_key_until_empty_page() {
    let (manager, es) = manager();
    // side_effect=[첫 페이지, 빈 페이지]
    let pages = Mutex::new(vec![
        json!({"aggregations": {"paths": {"buckets": [{"key": {"category": "A", "file_path": "A/1"}, "doc_count": 1}], "after_key": {"category": "A", "file_path": "A/1"}}}}),
        json!({"aggregations": {"paths": {"buckets": []}}}),
    ]);
    es.on("search", move |_| Ok(pages.lock().unwrap().remove(0)));

    let rows: Vec<(String, String, i64)> = manager.iter_all_category_file_paths(1).collect::<Result<_, _>>().unwrap();

    assert_eq!(rows, vec![("A".to_string(), "A/1".to_string(), 1)]);
    assert_eq!(es.calls_of("search")[1]["aggs"]["paths"]["composite"]["after"], json!({"category": "A", "file_path": "A/1"}));
}
