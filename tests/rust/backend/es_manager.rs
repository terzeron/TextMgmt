//! `backend::es_manager` 단위 테스트.

mod common;
#[path = "common/book_fixtures.rs"]
mod fx;
use backend::es_manager::{
    ConnectOptions, Docs, ESManager, ESManagerError, EsConnector, EsError, EsManagerApi, EsTransport, SimilarDocsArgs, UpdateFields, decode_category_cursor, encode_category_cursor,
};
use base64::Engine;
use common::clock::FakeClock;
use common::es_dummy::{DummyEs, runtime_error};
use common::faulty_fs::{FaultyFs, fail_always, fail_if_path_contains};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, atomic::AtomicBool, atomic::AtomicUsize, atomic::Ordering};
use tempfile::TempDir;

/// 메서드를 하나도 구현하지 않은 대역. trait 기본 구현이 `Unsupported`인지 확인한다.
struct Bare;

impl EsManagerApi for Bare {}

#[test]
fn trait_defaults_report_unsupported() {
    let bare = Bare;
    let docs = Docs::new();
    let fields = serde_json::json!({});
    let categories: Vec<String> = Vec::new();
    let paths: Vec<String> = Vec::new();
    let ids: Vec<i64> = Vec::new();
    let excluded: Vec<String> = Vec::new();
    let args = serde_json::json!({});
    let check = |result: Result<(), EsError>| assert_eq!(result.unwrap_err(), EsError::Unsupported);

    check(bare.create_index().map(|_| ()));
    check(bare.search_by_id(1).map(|_| ()));
    check(bare.update(1, &fields).map(|_| ()));
    check(bare.delete(1).map(|_| ()));
    check(bare.insert(&docs).map(|_| ()));
    check(bare.delete_by_file_paths(&paths, None).map(|_| ()));
    check(bare.delete_by_ids(&ids, 10).map(|_| ()));
    check(bare.refresh());
    check(bare.search_and_aggregate_by_category().map(|_| ()));
    check(bare.iter_all_category_file_paths(10).map(|_| ()));
    check(bare.search_categories_pdf_stats(&categories).map(|_| ()));
    check(bare.search_category_pdf_stats("cat").map(|_| ()));
    check(bare.get_by_id_source("1").map(|_| ()));
    check(bare.search_by_path_source("book.epub").map(|_| ()));
    check(bare.search_by_category("cat", 10).map(|_| ()));
    check(bare.search_by_category_paged("cat", 10, None).map(|_| ()));
    check(bare.count_by_categories(&categories, false).map(|_| ()));
    check(bare.count_by_category("cat", false).map(|_| ()));
    check(bare.rename_category("old", "new").map(|_| ()));
    check(bare.delete_by_category("cat", false).map(|_| ()));
    check(bare.search_by_keyword("word", 10).map(|_| ()));
    check(bare.search_by_keyword_paged(&args).map(|_| ()));
    check(bare.search_similar_docs(&args).map(|_| ()));
    check(bare.search_similar_docs_paged(&args).map(|_| ()));
    check(bare.search_latest_docs(10, None).map(|_| ()));
    check(bare.backfill_created_time(std::path::Path::new("/x")).map(|_| ()));
    check(bare.aggregate_cover_categories().map(|_| ()));
    check(bare.search_cover_files_paged(&categories, 10, None, &excluded).map(|_| ()));
    check(bare.search_group_representatives(&categories, &excluded).map(|_| ()));
}

type Hit = (i64, Value, f64);

/// `with_search_stub`이 받은 호출 하나: (query, sort, max_result_count).
type SearchCall = (Value, Option<Value>, i64);

/// 구 `set_search_override`의 함수 모양. `ESManager::with_search_stub`이 받는다.
type SearchStub = Arc<dyn Fn(&Value, Option<&Value>, i64, Option<&[String]>) -> Result<Vec<Hit>, EsError> + Send + Sync>;

// TODO(common): 클로저를 `EsConnector`로 감싸는 어댑터다. `connect` 테스트가 늘면 common/es_dummy로 옮긴다.
struct FnConnector<F>(F);

impl<F: Fn(&ConnectOptions) -> Arc<dyn EsTransport> + Send + Sync> EsConnector for FnConnector<F> {
    fn connect(&self, options: &ConnectOptions) -> Arc<dyn EsTransport> {
        (self.0)(options)
    }
}

/// `DummyEs`의 색인 이름은 `with_transport`가 `index_name()`으로 읽으므로 `idx`로 맞춘다(구 `with_client(es, "idx")`).
fn new_es() -> Arc<DummyEs> {
    DummyEs::with_index("idx")
}

/// 테스트 모듈 맨 위의 `os.environ.setdefault`에 대응하는 기본 환경 값.
fn default_env(key: &str) -> Option<String> {
    match key {
        "TM_ES_URL" => Some("http://localhost:9200".to_string()),
        "TM_ES_BOOK_INDEX" => Some("test".to_string()),
        "TM_ES_USER" => Some("test".to_string()),
        "TM_ES_PASSWORD" => Some("test".to_string()),
        _ => None,
    }
}

/// `_set_es_env`: 재시도 테스트가 쓰는 환경 값.
fn retry_env(key: &str) -> Option<String> {
    match key {
        "TM_ES_BOOK_INDEX" => Some("test_index".to_string()),
        "TM_ES_URL" => Some("http://localhost:9200".to_string()),
        "TM_ES_USER" => Some("elastic".to_string()),
        "TM_ES_PASSWORD" => Some("changeme".to_string()),
        _ => None,
    }
}

fn make_manager(es: &Arc<DummyEs>) -> ESManager {
    ESManager::with_transport(es.clone() as Arc<dyn EsTransport>)
}

/// `connect`로 매니저를 만든다. 재시도 대기는 `clock`에 쌓인다. 색인 이름은 `env`의 `TM_ES_BOOK_INDEX`다.
fn connect_with(env: &dyn Fn(&str) -> Option<String>, es: &Arc<DummyEs>, clock: &FakeClock) -> Result<ESManager, ESManagerError> {
    let es = es.clone();
    ESManager::connect(env, &FnConnector(move |_: &ConnectOptions| es.clone() as Arc<dyn EsTransport>), clock)
}

/// 구 `set_search_paged_override`: `_search_paged`가 부르는 `search` 응답을 스크립트한다.
/// 페이지 검색 응답은 점수 10.0, 기준 점수 100.0(정규화해도 10.0)이고 `size == 1`인 기준 점수 조회에는 100.0을 준다.
/// 마지막 페이지 검색의 `query`를 돌려준다.
fn script_paged_search(es: &DummyEs, total: i64) -> Arc<Mutex<Value>> {
    let captured = Arc::new(Mutex::new(Value::Null));
    let sink = captured.clone();
    es.on("search", move |kw| {
        if kw["size"] == json!(1) {
            return Ok(json!({"hits": {"hits": [{"_score": 100.0}]}}));
        }
        *sink.lock().unwrap() = kw["query"].clone();
        Ok(json!({"hits": {"total": {"value": total}, "max_score": 100.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 10.0}]}}))
    });
    captured
}

fn runtime(message: &str) -> EsError {
    runtime_error(message)
}

fn contains(array: &Value, item: &Value) -> bool {
    array.as_array().expect("배열").contains(item)
}

fn capture(es: &DummyEs, op: &str, response: Value) -> Arc<Mutex<Value>> {
    let captured = Arc::new(Mutex::new(json!({})));
    let sink = captured.clone();
    es.on(op, move |kw| {
        let mut guard = sink.lock().unwrap();
        if let (Some(dst), Some(src)) = (guard.as_object_mut(), kw.as_object()) {
            for (k, v) in src {
                dst.insert(k.clone(), v.clone());
            }
        }
        Ok(response.clone())
    });
    captured
}

/// `RecordingES.last_search_kwargs`: 마지막 `search` 호출의 인자.
fn last_search(es: &DummyEs) -> Value {
    es.calls_of("search").pop().expect("search 호출")
}

mod es {
    use super::*;

    #[test]
    fn test_init() {
        let es = new_es();
        let clock = FakeClock::new(0.0);
        let esm = connect_with(&default_env, &es, &clock);
        assert!(esm.is_ok());
    }
}

#[test]
fn test_do_exist_index_get_existing_ids_and_paths() {
    let es = new_es();
    let manager = make_manager(&es);
    es.set_mapping(json!({"exists": true}));
    assert!(manager.do_exist_index().unwrap());
    assert_eq!(manager.get_existing_paths(&[1, 2]).unwrap(), HashMap::from([(1, "/a".to_string())]));
}

#[test]
fn test_do_exist_index_uses_emptiness_for_all_json_shapes() {
    let cases = [
        (json!(null), false),
        (json!(false), false),
        (json!(0), false),
        (json!(""), false),
        (json!([]), false),
        (json!({}), false),
        (json!(true), true),
        (json!(-1), true),
        (json!("present"), true),
        (json!([0]), true),
        (json!({"index": true}), true),
    ];

    for (response, expected) in cases {
        let es = new_es();
        let response_for_es = response.clone();
        es.on("indices.exists", move |_| Ok(response_for_es.clone()));
        assert_eq!(make_manager(&es).do_exist_index().unwrap(), expected, "response={response}");
    }
}

#[test]
fn test_get_existing_created_time_metadata_omits_empty_metadata() {
    let es = new_es();
    es.on("mget", |_| Ok(json!({"docs": [{"_id": "3", "found": true, "_source": {}}]})));
    assert!(make_manager(&es).get_existing_created_time_metadata(&[3]).unwrap().is_empty());
}

#[test]
fn test_get_existing_created_time_metadata_returns_stored_values() {
    let es = new_es();
    let manager = make_manager(&es);

    es.on("mget", |kw| {
        assert_eq!(kw["docs"], json!([{"_index": "idx", "_id": "1"}, {"_index": "idx", "_id": "2"}]));
        assert_eq!(kw["source"], json!(["created_time", "created_time_source", "updated_time"]));
        Ok(json!({
            "docs": [
                {
                    "_id": "1",
                    "found": true,
                    "_source": {
                        "created_time": "2020-01-01T00:00:00",
                        "created_time_source": "statx_btime",
                        "updated_time": "2021-02-03T04:05:06",
                    },
                },
                {"_id": "2", "found": false},
            ]
        }))
    });

    assert_eq!(
        manager.get_existing_created_time_metadata(&[1, 2]).unwrap(),
        HashMap::from([(1, json!({"created_time": "2020-01-01T00:00:00", "created_time_source": "statx_btime", "updated_time": "2021-02-03T04:05:06"}))])
    );
}

#[test]
fn test_create_index_paths() {
    let es = new_es();
    let manager = make_manager(&es);

    // 구 set_ensure_category_nori_subfield_override: 이 ensure는 매핑을 읽는 `indices.get_mapping`을 한 번 부른다고 보고 그 호출을 센다.
    let ensure_calls = Arc::new(AtomicUsize::new(0));
    let counter = ensure_calls.clone();
    es.on("indices.get_mapping", move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"idx": {"mappings": {"properties": {"category": {"fields": {"nori": {"type": "text"}}}}}}}))
    });
    // 구 set_do_exist_index_override(true)
    es.respond("indices.exists", json!(true));
    assert_eq!(manager.create_index().unwrap()["acknowledged"], json!(true));
    assert_eq!(ensure_calls.load(Ordering::SeqCst), 1);

    es.on("indices.create", |_| Err(EsError::Status { status: 400, body: "resource_already_exists_exception".to_string() }));
    // 구 set_do_exist_index_override(false)
    es.respond("indices.exists", json!(false));
    assert_eq!(manager.create_index().unwrap()["acknowledged"], json!(true));
    assert_eq!(ensure_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn test_create_index_is_successful_when_existing_mapping_cannot_be_read() {
    let es = new_es();
    es.respond("indices.exists", json!(true));
    es.on("indices.get_mapping", |_| Err(runtime("mapping unavailable")));

    assert_eq!(make_manager(&es).create_index().unwrap(), json!({"acknowledged": true}));
}

#[test]
fn test_create_index_includes_created_time_mapping() {
    let es = new_es();
    let manager = make_manager(&es);
    // `indices.exists`의 기본 응답이 false라 색인이 없는 상황이다.

    assert_eq!(manager.create_index().unwrap()["acknowledged"], json!(true));

    let props = es.state(|s| s.created_body["mappings"]["properties"].clone());
    assert_eq!(props["created_time"], json!({"type": "date"}));
    assert_eq!(props["created_time_source"], json!({"type": "keyword"}));
}

#[test]
fn test_ensure_category_nori_subfield_adds() {
    let es = new_es();
    let manager = make_manager(&es);
    es.set_mapping(json!({"idx": {"mappings": {"properties": {"category": {}}}}}));
    manager._ensure_category_nori_subfield();
    assert!(es.state(|s| s.put_mapping_called));
}

#[test]
fn test_ensure_created_time_field_adds_missing_created_time_fields() {
    let es = new_es();
    let manager = make_manager(&es);
    es.set_mapping(json!({"idx": {"mappings": {"properties": {"category": {}}}}}));

    manager._ensure_created_time_field();

    assert!(es.state(|s| s.put_mapping_called));
    assert_eq!(es.state(|s| s.put_mapping_properties.clone()), json!({"created_time": {"type": "date"}, "created_time_source": {"type": "keyword"}}));
}

#[test]
fn test_ensure_created_time_field_swallows_mapping_read_error() {
    let es = new_es();
    es.on("indices.get_mapping", |_| Err(runtime("mapping unavailable")));

    make_manager(&es)._ensure_created_time_field();

    assert!(!es.state(|s| s.put_mapping_called));
}

#[test]
fn test_ensure_publisher_field_adds_it_when_missing() {
    let es = new_es();
    let manager = make_manager(&es);
    es.set_mapping(json!({"idx": {"mappings": {"properties": {"title": {}}}}}));

    manager._ensure_publisher_field();

    assert!(es.state(|s| s.put_mapping_called));
    // 자동분류가 전집을 가르는 데 쓰는 필드다. title/author 와 같은 모양으로 둔다.
    assert_eq!(es.state(|s| s.put_mapping_properties.clone()), json!({"publisher": {"type": "text", "analyzer": "nori_analyzer", "fields": {"keyword": {"type": "keyword"}}}}));
}

#[test]
fn test_ensure_publisher_field_is_a_no_op_when_already_present() {
    let es = new_es();
    let manager = make_manager(&es);
    es.set_mapping(json!({"idx": {"mappings": {"properties": {"publisher": {"type": "text"}}}}}));

    manager._ensure_publisher_field();

    assert!(!es.state(|s| s.put_mapping_called));
}

#[test]
fn test_ensure_publisher_field_swallows_errors() {
    let es = new_es();
    let manager = make_manager(&es);

    es.on("indices.get_mapping", |_| Err(runtime("ES 가 응답하지 않는다")));
    // 매핑 추가에 실패해도 색인 생성 자체를 막으면 안 된다
    manager._ensure_publisher_field();
    assert!(!es.state(|s| s.put_mapping_called));
}

#[test]
fn test_ensure_mapping_helpers_skip_incomplete_mapping_shapes() {
    let es = new_es();
    let manager = make_manager(&es);

    es.set_mapping(json!({"idx": {"mappings": {}}}));
    manager._ensure_category_nori_subfield();

    es.set_mapping(json!({"idx": {}}));
    manager._ensure_created_time_field();
    manager._ensure_publisher_field();

    assert!(!es.state(|s| s.put_mapping_called));
}

#[test]
fn test_ensure_mapping_helpers_swallow_put_mapping_errors() {
    let cases =
        [json!({"idx": {"mappings": {"properties": {"category": {}}}}}), json!({"idx": {"mappings": {"properties": {"title": {}}}}}), json!({"idx": {"mappings": {"properties": {"title": {}}}}})];
    let ensure: [fn(&ESManager); 3] = [ESManager::_ensure_category_nori_subfield, ESManager::_ensure_created_time_field, ESManager::_ensure_publisher_field];

    for (mapping, ensure) in cases.into_iter().zip(ensure) {
        let es = new_es();
        let manager = make_manager(&es);
        es.set_mapping(mapping);
        es.on("indices.put_mapping", |_| Err(runtime("mapping update rejected")));

        ensure(&manager);

        assert_eq!(es.calls_of("indices.put_mapping").len(), 1);
    }
}

#[test]
fn test_ensure_created_time_field_adds_missing_source_only() {
    let es = new_es();
    let manager = make_manager(&es);
    es.set_mapping(json!({"idx": {"mappings": {"properties": {"created_time": {"type": "date"}}}}}));

    manager._ensure_created_time_field();

    assert!(es.state(|s| s.put_mapping_called));
    assert_eq!(es.state(|s| s.put_mapping_properties.clone()), json!({"created_time_source": {"type": "keyword"}}));
}

#[test]
fn test_search_and_scroll_and_paged() {
    let es = new_es();
    let manager = make_manager(&es);
    let results = manager._search(&json!({"match": {"title": "a"}}), None, 5, None).unwrap();
    assert_eq!(results, vec![(1, json!({"a": 1}), 100.0)]);

    let results = manager._search(&json!({"match": {"title": "a"}}), None, 20000, None).unwrap();
    assert!(!results.is_empty());
    assert!(es.state(|s| s.cleared_scroll));

    let (paged, total) = manager._search_paged(&json!({"match": {"title": "a"}}), None, 10, 0, 0.0, None).unwrap();
    assert_eq!(total, 2);
    assert_eq!(paged[0].2, 100.0);
}

#[test]
fn test_search_paged_with_self_score_and_get_self_score() {
    let es = new_es();
    let manager = make_manager(&es);
    let (results, total) = manager._search_paged_with_self_score(&[json!({"match": {"title": {"query": "a"}}})], 1, 10, 0).unwrap();
    assert_eq!(total, 1);
    assert_eq!(results[0].2, 50.0);
}

#[test]
fn test_search_by_id_not_found() {
    let es = new_es();
    let manager = make_manager(&es);
    assert_eq!(manager.search_by_id(1).unwrap(), json!({}));
}

#[test]
fn test_search_by_id_propagates_unexpected_errors() {
    let es = new_es();
    es.on("get", |_| Err(runtime("backend unavailable")));
    let manager = make_manager(&es);
    assert!(matches!(manager.search_by_id(1), Err(EsError::Other(message)) if message == "backend unavailable"));
}

#[test]
fn test_aggregate_grouped_delete_insert_update_counts() {
    let es = new_es();
    let manager = make_manager(&es);
    assert_eq!(manager.search_and_aggregate_by_category().unwrap(), json!({"A": 2}));
    assert_eq!(manager.delete_by_file_paths(&["/a"], Some(&[1])), 2);

    es.on("delete_by_query", |_| Err(runtime("fail")));
    assert_eq!(manager.delete_by_file_paths(&["/a"], None), 0);
    es.reset("delete_by_query");

    let data = vec![(1, json!({"file_path": "/a"})), (2, json!({"file_path": "/b"}))];
    assert_eq!(manager.insert(&data, usize::MAX, 2).unwrap(), vec![1, 2]);

    assert!(!manager.update(1, &UpdateFields { title: "t".into(), ..Default::default() }).unwrap());
    assert_eq!(manager.count_by_category("A", true).unwrap(), 3);
    assert_eq!(manager.count_by_categories(&["A"], false).unwrap(), json!({"A": 3}));
    assert_eq!(manager.rename_category("A", "B").unwrap(), json!({"updated": 4, "failures": []}));
    assert_eq!(manager.delete_by_category("A", true).unwrap(), json!({"deleted": 2, "failures": []}));
    assert!(manager.delete(1).unwrap());
}

#[test]
fn test_rename_category_includes_subcategories() {
    // term 쿼리만 쓰면 하위 카테고리 문서가 남아 FS와 ES가 어긋난다.
    let es = new_es();
    let manager = make_manager(&es);
    let captured = capture(&es, "update_by_query", json!({"updated": 4, "failures": []}));

    manager.rename_category("A", "B").unwrap();

    let captured = captured.lock().unwrap().clone();
    let should = &captured["query"]["bool"]["should"];
    assert!(contains(should, &json!({"term": {"category": "A"}})));
    assert!(contains(should, &json!({"prefix": {"category": "A/"}})));
    let params = &captured["script"]["params"];
    assert_eq!(params["old_category"], json!("A"));
    assert_eq!(params["old_prefix"], json!("A/"));
    assert_eq!(params["new_prefix"], json!("B/"));
}

#[test]
fn test_search_by_keyword_paged_with_exclude_categories() {
    let es = new_es();
    let manager = make_manager(&es);

    let called = script_paged_search(&es, 1);
    let (results, total) = manager.search_by_keyword_paged("kw", 5, 1, Some(&["A", "B"]), None).unwrap();
    assert_eq!(total, 1);
    assert!(!results.is_empty());
    let query = called.lock().unwrap().clone();
    let must_not = &query["bool"]["must_not"];
    assert!(contains(must_not, &json!({"prefix": {"category": "A"}})));
    assert!(contains(must_not, &json!({"prefix": {"category": "B"}})));
}

#[test]
fn test_search_by_keyword_paged_filters_exact_category() {
    let es = new_es();
    let manager = make_manager(&es);
    let called = script_paged_search(&es, 1);

    manager.search_by_keyword_paged("kw", 10, 0, None, Some("과학")).unwrap();

    let query = called.lock().unwrap().clone();
    assert_eq!(query["bool"]["filter"], json!([{"term": {"category": "과학"}}]));
}

#[test]
fn test_search_similar_docs_paged_without_exclude() {
    let es = new_es();
    let manager = make_manager(&es);
    script_paged_search(&es, 2);

    let args = SimilarDocsArgs { title: "t".into(), author: "a".into(), file_size: 100, summary: "s".into(), size: 3, offset: 1, ..Default::default() };
    let (results, total) = manager.search_similar_docs_paged(&args).unwrap();
    assert_eq!(total, 2);
    assert_eq!(results, vec![(1, json!({"a": 1}), 10.0)]);
}

#[test]
#[allow(non_snake_case)]
fn test_search_builders_delegate_to__search() {
    let es = new_es();
    let captured: Arc<Mutex<Vec<SearchCall>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    let stub: SearchStub = Arc::new(move |query, sort, max_result_count, _fields| {
        sink.lock().unwrap().push((query.clone(), sort.cloned(), max_result_count));
        Ok(vec![(1, json!({"ok": true}), 100.0)])
    });
    let manager = make_manager(&es).with_search_stub(stub);
    assert!(!manager.search_by_title("hello", "epub", 123, 5).unwrap().is_empty());
    assert!(!manager.search_by_category("cat", 9).unwrap().is_empty());
    assert!(!manager.search_by_keyword("kw", 11).unwrap().is_empty());
    let args = SimilarDocsArgs { title: "t".into(), author: "a".into(), file_size: 100, summary: "s".into(), exclude_id: Some(3), max_result_count: 4, ..Default::default() };
    assert!(!manager.search_similar_docs(&args).unwrap().is_empty());

    let captured = captured.lock().unwrap();
    assert_eq!(captured[0].2, 5);
    assert_eq!(captured[1].1, Some(json!(["author.keyword", "title.keyword"])));
    assert!(contains(&captured[3].0["bool"]["must_not"], &json!({"term": {"_id": "3"}})));
}

#[test]
fn test_search_paged_returns_empty_when_base_score_non_positive() {
    let es = new_es();
    let manager = make_manager(&es);

    es.on("search", |_| Ok(json!({"hits": {"total": {"value": 3}, "max_score": 0, "hits": []}})));
    assert_eq!(manager._search_paged(&json!({"match_all": {}}), None, 10, 0, 0.0, None).unwrap(), (vec![], 3));
}

#[test]
fn test_count_by_categories_handles_msearch_error() {
    let es = new_es();
    let manager = make_manager(&es);
    es.on("msearch", |_| Ok(json!({"responses": [{"error": "boom"}, {"hits": {"total": {"value": 2}}}]})));
    assert_eq!(manager.count_by_categories(&["A", "B"], false).unwrap(), json!({"A": 0, "B": 2}));
}

#[test]
fn test_delete_returns_false_when_not_deleted() {
    let es = new_es();
    let manager = make_manager(&es);
    es.on("delete", |_| Ok(json!({"result": "noop"})));
    assert!(!manager.delete(1).unwrap());
}

#[test]
fn test_update_returns_true_when_no_failed_shards() {
    let es = new_es();
    let manager = make_manager(&es);
    es.on("update", |_| Ok(json!({"_shards": {"failed": 0}})));
    assert!(manager.update(1, &UpdateFields { title: "ok".into(), ..Default::default() }).unwrap());
}

#[test]
fn test_search_max_score_none_returns_empty() {
    // ESNoScore: search가 max_score None을 돌려준다.
    let es = new_es();
    es.on("search", |_| Ok(json!({"hits": {"max_score": null, "hits": []}})));
    let manager = make_manager(&es);
    assert_eq!(manager._search(&json!({"match": {"title": "a"}}), None, 10, None).unwrap(), vec![]);
}

#[test]
fn test_search_paged_base_score_zero() {
    // ESZeroScore
    let es = new_es();
    es.on("search", |_| Ok(json!({"hits": {"total": {"value": 3}, "max_score": 0.0, "hits": []}})));
    let manager = make_manager(&es);
    let (results, total) = manager._search_paged(&json!({"match": {"title": "a"}}), None, 10, 0, 0.0, None).unwrap();
    assert_eq!(results, vec![]);
    assert_eq!(total, 3);
}

#[test]
fn test_search_paged_with_self_score_errors() {
    // ESBad: msearch 응답이 모두 error다.
    let es = new_es();
    es.on("msearch", |_| Ok(json!({"responses": [{"error": {"reason": "bad"}}, {"error": {"reason": "bad2"}}]})));
    let manager = make_manager(&es);
    let (results, total) = manager._search_paged_with_self_score(&[json!({"match": {"title": {"query": "a"}}})], 1, 10, 0).unwrap();
    assert_eq!(results, vec![]);
    assert_eq!(total, 0);

    // ESBaseZero: 기준 점수 응답에 hit가 없다.
    let es = new_es();
    es.on("msearch", |_| Ok(json!({"responses": [{"hits": {"hits": []}}, {"hits": {"total": {"value": 2}, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 5.0}]}}]})));
    let manager = make_manager(&es);
    let (results, total) = manager._search_paged_with_self_score(&[json!({"match": {"title": {"query": "a"}}})], 1, 10, 0).unwrap();
    assert_eq!(results, vec![]);
    assert_eq!(total, 2);
}

#[test]
fn test_count_by_categories_error_response() {
    let es = new_es();
    es.on("msearch", |_| Ok(json!({"responses": [{"error": {"reason": "bad"}}, {"hits": {"total": {"value": 5}}}]})));
    let manager = make_manager(&es);
    let result = manager.count_by_categories(&["A", "B"], false).unwrap();
    assert_eq!(result, json!({"A": 0, "B": 5}));
}

#[test]
fn test_ensure_category_nori_subfield_exception() {
    let es = new_es();
    let manager = make_manager(&es);

    es.on("indices.get_mapping", |_| Err(runtime("boom")));
    manager._ensure_category_nori_subfield();
}

#[test]
fn test_delete_by_file_paths_with_exclude_ids() {
    // ESDelete: 받은 body를 보관하고 deleted 3을 돌려준다.
    let es = new_es();
    let last_body = Arc::new(Mutex::new(Value::Null));
    let sink = last_body.clone();
    es.on("delete_by_query", move |kw| {
        *sink.lock().unwrap() = kw["body"].clone();
        Ok(json!({"deleted": 3}))
    });
    let manager = make_manager(&es);
    let deleted = manager.delete_by_file_paths(&["/a", "/b"], Some(&[1, 2]));
    assert_eq!(deleted, 3);
    let body = last_body.lock().unwrap().clone();
    let must_not = &body["query"]["bool"]["must_not"];
    assert!(contains(must_not, &json!({"ids": {"values": ["1", "2"]}})));
}

#[test]
fn test_search_scroll_max_score_none() {
    let es = new_es();
    es.on("search", |_| Ok(json!({"_scroll_id": "scroll1", "hits": {"max_score": 1.0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 1.0}]}})));
    es.on("scroll", |_| Ok(json!({"_scroll_id": "scroll1", "hits": {"max_score": null, "hits": [{"_id": "2", "_source": {"a": 2}, "_score": 1.0}]}})));
    let manager = make_manager(&es);
    assert_eq!(manager._search(&json!({"match": {"title": "a"}}), None, 20000, None).unwrap(), vec![]);
}

#[test]
fn test_search_scroll_clear_scroll_error() {
    let es = new_es();
    es.on("search", |_| Ok(json!({"_scroll_id": "scroll1", "hits": {"max_score": 1.0, "hits": []}})));
    es.on("clear_scroll", |_| Err(runtime("boom")));
    let manager = make_manager(&es);
    assert_eq!(manager._search(&json!({"match": {"title": "a"}}), None, 20000, None).unwrap(), vec![]);
}

#[test]
fn test_search_by_title_summary_category_defaults() {
    let es = new_es();
    let manager = make_manager(&es);
    assert!(!manager.search_by_title("t", "", 0, -1).unwrap().is_empty()); // default max_result_count path
    assert!(!manager.search_by_category("c", -1).unwrap().is_empty());
}

#[test]
fn test_search_paged_with_zero_score() {
    // manager.es를 DummyZeroScoreES로 바꾼다. 같은 효과로 search를 바꿔 건다.
    let es = new_es();
    let manager = make_manager(&es);
    es.on("search", |_| Ok(json!({"hits": {"total": {"value": 1}, "max_score": 0, "hits": [{"_id": "1", "_source": {"a": 1}, "_score": 0}]}})));
    let (results, total) = manager._search_paged(&json!({"match": {"title": "a"}}), None, 10, 0, 0.0, None).unwrap();
    assert_eq!(results, vec![]);
    assert_eq!(total, 1);
}

#[test]
fn test_get_existing_ids_and_paths_empty() {
    let es = new_es();
    let manager = make_manager(&es);
    assert_eq!(manager.get_existing_paths(&[]).unwrap(), HashMap::new());
}

#[test]
fn test_delete_index_when_exists() {
    let es = new_es();
    let manager = make_manager(&es);
    // `indices.exists`가 true를 돌려주게 해 색인이 있는 상황을 만든다.
    es.set_mapping(json!({"exists": true}));
    manager.delete_index().unwrap();
    assert!(es.state(|s| s.deleted));
}

#[test]
fn test_delete_index_is_a_noop_when_index_is_missing() {
    let es = new_es();
    make_manager(&es).delete_index().unwrap();
    assert!(es.calls_of("indices.delete").is_empty());
}

#[test]
fn test_update_success_and_delete_by_file_paths_empty() {
    // manager.es를 DummySuccessES로 바꾼다. 새 가짜 ES로 매니저를 다시 만든다.
    let success = new_es();
    success.on("update", |_| Ok(json!({"_shards": {"failed": 0}})));
    let manager = make_manager(&success);
    assert!(manager.update(1, &UpdateFields { title: "t".into(), ..Default::default() }).unwrap());
    assert_eq!(manager.delete_by_file_paths(&[], None), 0);
}

#[test]
fn test_count_by_categories_single() {
    // es.count 응답을 7로 두어 count_by_category 위임을 확인한다.
    let es = new_es();
    let manager = make_manager(&es);
    es.on("count", |_| Ok(json!({"count": 7})));
    assert_eq!(manager.count_by_categories(&["A"], false).unwrap(), json!({"A": 7}));
}

#[test]
fn test_count_by_categories_empty_is_a_noop() {
    let es = new_es();

    assert_eq!(make_manager(&es).count_by_categories(&[], false).unwrap(), json!({}));
    assert!(es.calls_of("msearch").is_empty());
    assert!(es.calls_of("count").is_empty());
}

#[test]
fn test_category_file_path_iterator_yields_an_error_once_then_stops() {
    let es = new_es();
    es.on("search", |_| Err(runtime("search failed")));
    let manager = make_manager(&es);
    let mut paths = manager.iter_all_category_file_paths(10);

    assert!(matches!(paths.next(), Some(Err(_))));
    assert!(paths.next().is_none());
}

// ---- coverage: uncovered lines ----

#[test]
fn test_init_missing_env_var() {
    // Lines 26-27: missing env var -> sys.exit. 환경 변수를 지우지 않고 조회 함수를 주입한다.
    let env = |key: &str| if key == "TM_ES_BOOK_INDEX" { None } else { default_env(key) };
    let clock = FakeClock::new(0.0);
    match connect_with(&env, &new_es(), &clock) {
        Err(ESManagerError::SystemExit(_)) => {}
        other => panic!("SystemExit를 기대했다: {:?}", other.map(|_| ())),
    }
}

#[test]
fn test_create_index_non_exists_bad_request() {
    // Line 191: BadRequestError without resource_already_exists -> re-raise
    let es = new_es();
    let manager = make_manager(&es);

    // `indices.exists`의 기본 응답이 false라 색인이 없는 상황이다.
    es.on("indices.create", |_| Err(EsError::Status { status: 400, body: "some_other_error".to_string() }));
    assert!(matches!(manager.create_index(), Err(EsError::Status { .. })));
}

#[test]
fn test_search_default_max_result_count() {
    // Line 220: _search with max_result_count < 0 uses default
    let es = new_es();
    let manager = make_manager(&es);
    let results = manager._search(&json!({"match": {"title": "a"}}), None, -1, None).unwrap();
    assert!(results.len() as i64 <= ESManager::DEFAULT_MAX_RESULT_COUNT);
}

#[test]
fn test_search_scroll_early_exit_on_max() {
    // Lines 248, 255, 264-268: scroll path with result_count >= max_result_count
    let es = new_es();
    let base = es.clone();
    es.on("search", move |kw| {
        if kw.get("scroll").is_some_and(|s| !s.is_null()) {
            let hits: Vec<Value> = (0..5).map(|i| json!({"_id": i.to_string(), "_source": {"a": i}, "_score": 2.0})).collect();
            return Ok(json!({"_scroll_id": "s1", "hits": {"max_score": 2.0, "hits": hits}}));
        }
        base.base_call("search", kw)
    });
    es.on("scroll", |_| {
        let hits: Vec<Value> = (0..5).map(|i| json!({"_id": (i + 10).to_string(), "_source": {"a": i}, "_score": 2.0})).collect();
        Ok(json!({"_scroll_id": "s2", "hits": {"max_score": 2.0, "hits": hits}}))
    });
    let manager = make_manager(&es);
    // max_result_count > 10000 forces scroll, but cap at 3
    let results = manager._search(&json!({"match": {"title": "a"}}), None, 10001, None).unwrap();
    assert!(!results.is_empty());
}

#[test]
fn test_search_by_title_negative_max() {
    // Lines 295-299: search_by_title with negative max_result_count
    let es = new_es();
    let manager = make_manager(&es);
    let results: Vec<Hit> = manager.search_by_title("t", "", 0, -1).unwrap();
    let _ = results;
}

#[test]
fn test_search_by_category_negative_max() {
    // Line 310: search_by_category with negative max_result_count
    let es = new_es();
    let manager = make_manager(&es);
    let results: Vec<Hit> = manager.search_by_category("c", -1).unwrap();
    let _ = results;
}

#[test]
fn test_search_by_keyword_negative_max() {
    // Line 319: search_by_keyword with negative max_result_count
    let es = new_es();
    let manager = make_manager(&es);
    let results: Vec<Hit> = manager.search_by_keyword("k", -1).unwrap();
    let _ = results;
}

#[test]
fn test_search_similar_docs_negative_max() {
    // Line 335: search_similar_docs with negative max_result_count
    let es = new_es();
    let stub: SearchStub = Arc::new(|_query, _sort, _max, _fields| Ok(vec![(1, json!({"a": 1}), 100.0)]));
    let manager = make_manager(&es).with_search_stub(stub);
    let args = SimilarDocsArgs { title: "t".into(), author: "a".into(), summary: "s".into(), file_size: 10, max_result_count: -1, ..Default::default() };
    let results: Vec<Hit> = manager.search_similar_docs(&args).unwrap();
    let _ = results;
}

#[test]
fn test_insert_final_failure() {
    // Lines 502-503: insert bulk retries exhausted -> raise
    let es = new_es();
    es.on("bulk", |_| Err(EsError::Transport("persistent failure".to_string())));
    // 재시도 사이의 sleep은 FakeClock이 건너뛴다. 시계는 `connect`로만 주입하므로 매니저를 `connect_with`로 만든다.
    let clock = FakeClock::new(0.0);
    let manager = connect_with(&retry_env, &es, &clock).unwrap();
    let result = manager.insert(&[(1, json!({"file_path": "/a"}))], usize::MAX, 2);
    assert!(matches!(result, Err(EsError::Transport(_))));
}

#[test]
fn test_insert_uses_injected_clock_and_preserves_bulk_failure_modes() {
    let retry_es = new_es();
    retry_es.on("bulk", |_| Err(EsError::Transport("connection refused".to_string())));
    let clock = FakeClock::new(0.0);
    let retry_manager = make_manager(&retry_es).with_clock(clock.clone());
    assert!(retry_manager.insert(&[(1, json!({}))], usize::MAX, 3).is_err());
    assert_eq!(clock.slept(), vec![1.0, 2.0]);

    let status_es = new_es();
    status_es.on("bulk", |_| Err(EsError::Status { status: 400, body: "bad request".into() }));
    let status_clock = FakeClock::new(0.0);
    let status_manager = make_manager(&status_es).with_clock(status_clock.clone());
    assert!(matches!(status_manager.insert(&[(1, json!({}))], usize::MAX, 3), Err(EsError::Status { status: 400, .. })));
    assert!(status_clock.slept().is_empty());

    let item_error_es = new_es();
    item_error_es.on("bulk", |_| Ok(json!({"errors": true})));
    let item_error_manager = make_manager(&item_error_es);
    assert_eq!(item_error_manager.insert(&[(1, json!({}))], usize::MAX, 1).unwrap(), vec![1]);
}

#[test]
fn test_insert_num_docs_limit() {
    // Line 507: insert stops when data_count >= num_docs
    let es = new_es();
    es.on("bulk", |_| Ok(json!({"errors": false})));
    let manager = make_manager(&es);
    let data: Vec<(i64, Value)> = (0..10).map(|i| (i, json!({"file_path": format!("/{i}")}))).collect();
    let result = manager.insert(&data, 3, 3).unwrap();
    // Should stop after processing first chunk that hits num_docs
    assert!(result.len() <= 10);
}

#[test]
fn test_update_with_file_size_and_summary() {
    // Lines 529, 531: update with file_size and summary
    let es = new_es();
    let last_body = Arc::new(Mutex::new(Value::Null));
    let sink = last_body.clone();
    es.on("update", move |kw| {
        *sink.lock().unwrap() = kw["body"].clone();
        Ok(json!({"_shards": {"failed": 0}}))
    });
    let manager = make_manager(&es);
    let result = manager.update(1, &UpdateFields { title: "t".into(), file_size: 1024, summary: "test summary".into(), ..Default::default() }).unwrap();
    assert!(result);
    let doc = last_body.lock().unwrap()["doc"].clone();
    assert_eq!(doc["file_size"], json!(1024));
    assert_eq!(doc["summary"], json!("test summary"));
}

#[test]
fn test_update_with_updated_time() {
    // updated_time 을 주면 최신 책 정렬 기준 필드로 색인한다. 주지 않으면 건드리지 않는다.
    let es = new_es();
    let bodies = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = bodies.clone();
    es.on("update", move |kw| {
        sink.lock().unwrap().push(kw["body"].clone());
        Ok(json!({"_shards": {"failed": 0}}))
    });
    let manager = make_manager(&es);
    assert!(manager.update(1, &UpdateFields { file_size: 10, updated_time: "2026-10-16T12:00:00.000000".into(), ..Default::default() }).unwrap());
    assert!(manager.update(1, &UpdateFields { file_size: 10, ..Default::default() }).unwrap());
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies[0]["doc"], json!({"file_size": 10, "updated_time": "2026-10-16T12:00:00.000000"}));
    assert!(bodies[1]["doc"].get("updated_time").is_none());
}

#[test]
fn test_delete_by_category_no_prefix() {
    // Line 612: delete_by_category with prefix=False
    let es = new_es();
    let last_query = Arc::new(Mutex::new(Value::Null));
    let sink = last_query.clone();
    es.on("delete_by_query", move |kw| {
        *sink.lock().unwrap() = kw["query"].clone();
        Ok(json!({"deleted": 1, "failures": []}))
    });
    let manager = make_manager(&es);
    let result = manager.delete_by_category("A", false).unwrap();
    assert_eq!(result["deleted"], json!(1));
    assert_eq!(*last_query.lock().unwrap(), json!({"term": {"category": "A"}}));
}

#[test]
fn test_delete_returns_false() {
    // Line 622: delete when result != 'deleted'
    let es = new_es();
    es.on("delete", |_| Ok(json!({"result": "not_found"})));
    let manager = make_manager(&es);
    assert!(!manager.delete(1).unwrap());
}

// ---- 연결 재시도 ----

#[test]
fn test_init_success_first_try() {
    let es = new_es();
    es.on("info", |_| Ok(json!({"cluster_name": "test"})));
    let clock = FakeClock::new(0.0);

    connect_with(&retry_env, &es, &clock).unwrap();

    assert_eq!(es.calls_of("info").len(), 1);
}

#[test]
fn test_init_retry_then_success() {
    let es = new_es();
    // side_effect=[연결 거부, 연결 거부, 성공]
    let responses = Mutex::new(vec![Err(EsError::Transport("connection refused".to_string())), Err(EsError::Transport("connection refused".to_string())), Ok(json!({"cluster_name": "test"}))]);
    es.on("info", move |_| responses.lock().unwrap().remove(0));
    let clock = FakeClock::new(0.0);

    connect_with(&retry_env, &es, &clock).unwrap();

    assert_eq!(es.calls_of("info").len(), 3);
    let sleeps = clock.slept();
    assert_eq!(sleeps.len(), 2);
    assert!(sleeps.contains(&2.0));
    assert!(sleeps.contains(&4.0));
}

#[test]
fn test_init_all_retries_fail() {
    let es = new_es();
    es.on("info", |_| Err(EsError::Transport("connection refused".to_string())));
    let clock = FakeClock::new(0.0);

    let result = connect_with(&retry_env, &es, &clock);
    assert!(matches!(result, Err(ESManagerError::Elasticsearch(EsError::Transport(_)))));

    assert_eq!(es.calls_of("info").len(), 5);
    assert_eq!(clock.slept().len(), 4);
}

#[test]
fn test_init_does_not_retry_non_connection_errors() {
    let es = new_es();
    es.on("info", |_| Err(EsError::Status { status: 401, body: "unauthorized".into() }));
    let clock = FakeClock::new(0.0);

    let result = connect_with(&retry_env, &es, &clock);

    assert!(matches!(result, Err(ESManagerError::Elasticsearch(EsError::Status { status: 401, .. }))));
    assert_eq!(es.calls_of("info").len(), 1);
    assert!(clock.slept().is_empty());
}

// ---- 파일 경로 묶음 조회 ----
// test_update_main, test_update_main_failure, test_update_main_delete_failure는
// tests/rust/update/update.rs가 이미 옮겼으므로 여기서는 옮기지 않는다.

// ---- coverage: _search scroll edge cases ----

#[test]
fn test_search_scroll_initial_max_score_none() {
    // Line 248: initial scroll response has max_score=None (requires max_result_count > 10000)
    let es = new_es();
    let base = es.clone();
    es.on("search", move |kw| {
        if kw.get("scroll").is_some_and(|s| !s.is_null()) {
            return Ok(json!({"_scroll_id": "s1", "hits": {"max_score": null, "hits": [{"_id": "1", "_source": {}, "_score": 0}]}}));
        }
        base.base_call("search", kw)
    });
    let manager = make_manager(&es);
    // max_result_count > 10000 triggers scroll path
    let result = manager._search(&json!({"match_all": {}}), None, 20000, None).unwrap();
    assert_eq!(result, vec![]);
}

#[test]
fn test_search_scroll_early_exit_first_batch() {
    // Line 255: result_count >= max_result_count in first scroll batch
    let es = new_es();
    let base = es.clone();
    es.on("search", move |kw| {
        if kw.get("scroll").is_some_and(|s| !s.is_null()) {
            let hits: Vec<Value> = (0..10).map(|i| json!({"_id": i.to_string(), "_source": {"a": i}, "_score": 5.0})).collect();
            return Ok(json!({"_scroll_id": "s1", "hits": {"max_score": 5.0, "hits": hits}}));
        }
        base.base_call("search", kw)
    });
    let manager = make_manager(&es);
    // max_result_count > 10000 but small enough to early exit in first batch
    let result = manager._search(&json!({"match_all": {}}), None, 10001, None).unwrap();
    assert_eq!(result.len(), 10);
}

// ---- coverage: update with author field ----

#[test]
fn test_update_with_author() {
    // Line 523: update with author field
    let es = new_es();
    es.on("update", |_| Ok(json!({"_shards": {"failed": 0}})));
    let manager = make_manager(&es);
    let fields = UpdateFields { category: "A".into(), title: "T".into(), author: "Author".into(), file_path: "a.txt".into(), file_type: "txt".into(), ..Default::default() };
    let result = manager.update(1, &fields).unwrap();
    assert!(result);
}

#[test]
fn test_search_scroll_early_return_at_max_result_count() {
    // scroll 경로에서 max_result_count 에 도달하면 즉시 반환한다.
    let es = new_es();
    let manager = make_manager(&es);

    // max_result_count > 10000 이어야 scroll 경로에 진입
    let max_rc: usize = 10001;
    let hits: Vec<Value> = (0..max_rc).map(|i| json!({"_id": i.to_string(), "_source": {"a": i}, "_score": 1.0})).collect();
    let response = json!({"_scroll_id": "scroll1", "hits": {"max_score": 1.0, "hits": hits}});
    es.on("search", move |_| Ok(response.clone()));
    es.on("clear_scroll", |_| Ok(json!({"succeeded": true})));

    let result = manager.search_by_title("test", "", 0, max_rc as i64).unwrap();
    assert_eq!(result.len(), max_rc);
    // finally 블록에서 clear_scroll 호출 확인
    assert_eq!(es.calls_of("clear_scroll"), vec![json!({"scroll_id": "scroll1"})]);
}

// --- C4: ES 연결 TLS 처리 ---

#[test]
fn test_https_url_keeps_certificate_verification_and_passes_private_ca() {
    let env = |key: &str| match key {
        "TM_ES_URL" => Some("https://elasticsearch-es-http:9200".to_string()),
        "TM_ES_CA_CERT" => Some("/etc/tm-es/ca.crt".to_string()),
        _ => default_env(key),
    };
    let captured: Mutex<Option<ConnectOptions>> = Mutex::new(None);
    let connector = FnConnector(|options: &ConnectOptions| {
        *captured.lock().unwrap() = Some(options.clone());
        new_es() as Arc<dyn EsTransport>
    });
    ESManager::connect(&env, &connector, FakeClock::new(0.0).as_ref()).unwrap();

    let options = captured.lock().unwrap().clone().expect("연결 옵션");
    assert_eq!(options.hosts, vec!["https://elasticsearch-es-http:9200".to_string()]);
    assert_eq!(options.verify_certs, Some(true));
    assert_eq!(options.ssl_show_warn, None);
    assert_eq!(options.ca_cert_path, Some(std::path::PathBuf::from("/etc/tm-es/ca.crt")));
}

#[test]
fn test_http_url_does_not_set_ssl_kwargs() {
    // http URL 이면 ssl 관련 kwargs 를 넣지 않는다 (기존 동작 유지).
    let env = |key: &str| if key == "TM_ES_URL" { Some("http://localhost:9200".to_string()) } else { default_env(key) };
    let captured: Mutex<Option<ConnectOptions>> = Mutex::new(None);
    let connector = FnConnector(|options: &ConnectOptions| {
        *captured.lock().unwrap() = Some(options.clone());
        new_es() as Arc<dyn EsTransport>
    });
    ESManager::connect(&env, &connector, FakeClock::new(0.0).as_ref()).unwrap();

    let options = captured.lock().unwrap().clone().expect("연결 옵션");
    assert_eq!(options.verify_certs, None);
    assert_eq!(options.ssl_show_warn, None);
    assert_eq!(options.ca_cert_path, None);
}

#[test]
fn test_path_prefix_query_variants() {
    for empty in ["", ".", "/"] {
        assert_eq!(ESManager::_path_prefix_query(empty), json!({"match_all": {}}), "빈 접두사: {empty:?}");
    }
    let q = ESManager::_path_prefix_query("foo/bar/");
    assert_eq!(q["bool"]["should"], json!([{"term": {"file_path": "foo/bar"}}, {"prefix": {"file_path": "foo/bar/"}}]));
}

#[test]
fn test_get_doc_ids_by_path_prefix_scrolls_and_clears() {
    // ScrollES: 첫 검색 2건, 스크롤 1페이지에 1건, 그다음 빈 페이지.
    let es = new_es();
    let cleared = Arc::new(AtomicBool::new(false));
    let page = Arc::new(AtomicUsize::new(0));
    es.on("search", |_| Ok(json!({"_scroll_id": "s1", "hits": {"hits": [{"_id": "1"}, {"_id": "2"}]}})));
    es.on("scroll", move |_| {
        if page.fetch_add(1, Ordering::SeqCst) + 1 == 1 {
            return Ok(json!({"_scroll_id": "s2", "hits": {"hits": [{"_id": "3"}]}}));
        }
        Ok(json!({"_scroll_id": "s3", "hits": {"hits": []}}))
    });
    let flag = cleared.clone();
    es.on("clear_scroll", move |_| {
        flag.store(true, Ordering::SeqCst);
        Ok(Value::Null)
    });
    let manager = make_manager(&es);
    assert_eq!(manager.get_doc_ids_by_path_prefix("foo"), HashSet::from([1, 2, 3]));
    assert!(cleared.load(Ordering::SeqCst));
}

#[test]
fn test_get_doc_ids_by_path_prefix_error_returns_empty() {
    let es = new_es();
    es.on("search", |_| Err(runtime("boom")));
    let manager = make_manager(&es);
    assert_eq!(manager.get_doc_ids_by_path_prefix("foo"), HashSet::new());
}

#[test]
fn test_delete_by_ids_empty_and_chunked() {
    // CountES: 요청한 ID 수만큼 deleted를 돌려준다.
    let es = new_es();
    es.on("delete_by_query", |kw| {
        let count = kw["body"]["query"]["ids"]["values"].as_array().unwrap().len();
        Ok(json!({"deleted": count}))
    });
    let manager = make_manager(&es);
    assert_eq!(manager.delete_by_ids(&[], 10000), 0);
    assert_eq!(es.calls_of("delete_by_query").len(), 0);
    assert_eq!(manager.delete_by_ids(&[1, 2, 3, 4, 5], 2), 5);
    assert_eq!(es.calls_of("delete_by_query").len(), 3);
}

#[test]
fn test_delete_by_ids_error_returns_partial() {
    // HalfES: 첫 호출은 2건 삭제, 이후는 예외.
    let es = new_es();
    let calls = AtomicUsize::new(0);
    es.on("delete_by_query", move |_| {
        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(json!({"deleted": 2}));
        }
        Err(runtime("boom"))
    });
    let manager = make_manager(&es);
    assert_eq!(manager.delete_by_ids(&[1, 2, 3, 4], 2), 2);
}

#[test]
fn test_search_by_category_paged_returns_cursor_and_total() {
    // 마지막 페이지가 아니면 다음 커서를, 마지막이면 None을 준다.
    let es = new_es();
    let manager = make_manager(&es);
    let response = json!({
        "hits": {
            "total": {"value": 32355},
            "hits": [
                {"_id": "1", "_source": {"title": "가"}, "sort": ["가", "/1.epub"]},
                {"_id": "2", "_source": {"title": "나"}, "sort": ["나", "/2.epub"]},
            ],
        }
    });
    let captured = capture(&es, "search", response);

    // size와 hit 수가 같으므로 다음 페이지가 있다고 본다
    let (result, total, next_after) = manager.search_by_category_paged("A", 2, None);
    let ids: Vec<i64> = result.iter().map(|(doc_id, _src, _score)| *doc_id).collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(total, 32355);
    assert_eq!(next_after, Some(vec![json!("나"), json!("/2.epub")]));
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured["query"], json!({"term": {"category": "A"}}));
    assert_eq!(captured["sort"], ESManager::category_sort());
    assert_eq!(captured["track_total_hits"], json!(true));
    assert!(!captured.as_object().unwrap().contains_key("search_after"));

    // hit 수가 size보다 적으면 마지막 페이지
    let (_result, _total, next_after) = manager.search_by_category_paged("A", 10, None);
    assert_eq!(next_after, None);
}

#[test]
fn test_search_by_category_paged_passes_search_after_and_handles_error() {
    let es = new_es();
    let manager = make_manager(&es);
    let captured = capture(&es, "search", json!({"hits": {"total": {"value": 0}, "hits": []}}));
    manager.search_by_category_paged("A", 5, Some(&[json!("가"), json!("/1.epub")]));
    assert_eq!(captured.lock().unwrap()["search_after"], json!(["가", "/1.epub"]));

    es.on("search", |_| Err(runtime("es down")));
    assert_eq!(manager.search_by_category_paged("A", 5, None), (vec![], 0, None));
}

#[test]
fn test_search_latest_docs_sorts_by_updated_time_and_excludes_hidden_categories() {
    let es = new_es();
    let manager = make_manager(&es);
    let response = json!({
        "hits": {
            "total": {"value": 1},
            "hits": [{
                "_id": "7",
                "_source": {
                    "category": "B",
                    "title": "New",
                    "author": "",
                    "file_path": "B/new.txt",
                    "file_type": "txt",
                    "file_size": 1,
                    "created_time": "2026-08-29T00:00:00.000000",
                    "updated_time": "2026-08-30T00:00:00.000000",
                },
            }],
        }
    });
    let captured = capture(&es, "search", response);

    let (result, total) = manager.search_latest_docs(100, Some(&["A"]));

    assert_eq!(total, 1);
    assert_eq!(result[0].0, 7);
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured["size"], json!(100));
    assert_eq!(captured["track_total_hits"], json!(true));
    assert_eq!(captured["sort"][0], json!({"updated_time": {"order": "desc", "missing": "_last"}}));
    let should = &captured["query"]["bool"]["must_not"][0]["bool"]["should"];
    assert!(contains(should, &json!({"term": {"category": "A"}})));
    assert!(contains(should, &json!({"prefix": {"category": "A/"}})));
}

#[test]
fn test_backfill_created_time_updates_missing_docs() {
    // BackfillES: 검색은 문서 2건을 돌려주고 bulk 본문을 모은다.
    let es = new_es();
    let bulk_body = Arc::new(Mutex::new(Vec::<Value>::new()));
    let cleared = Arc::new(AtomicBool::new(false));
    es.on("search", |kw| {
        assert_eq!(
            kw["query"],
            json!({
                "bool": {
                    "should": [
                        {"bool": {"must_not": [{"exists": {"field": "created_time"}}]}},
                        {"bool": {"must_not": [{"exists": {"field": "created_time_source"}}]}},
                    ],
                    "minimum_should_match": 1,
                }
            })
        );
        Ok(json!({"_scroll_id": "s1", "hits": {"hits": [{"_id": "1", "_source": {"file_path": "A/new.txt"}}, {"_id": "2", "_source": {"file_path": "missing.txt"}}]}}))
    });
    es.on("scroll", |_| Ok(json!({"_scroll_id": "s2", "hits": {"hits": []}})));
    let sink = bulk_body.clone();
    es.on("bulk", move |kw| {
        sink.lock().unwrap().extend(kw["body"].as_array().unwrap().iter().cloned());
        Ok(json!({"errors": false}))
    });
    let flag = cleared.clone();
    es.on("clear_scroll", move |_| {
        flag.store(true, Ordering::SeqCst);
        Ok(Value::Null)
    });

    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("A")).unwrap();
    std::fs::write(tmp.path().join("A").join("new.txt"), "new").unwrap();
    let manager = make_manager(&es);

    let result = manager.backfill_created_time(tmp.path(), 500);

    assert_eq!(result["updated"], json!(1));
    assert_eq!(result["skipped"], json!(1));
    assert_eq!(result["failed"], json!(0));
    let bulk_body = bulk_body.lock().unwrap();
    assert_eq!(bulk_body[0], json!({"update": {"_index": "idx", "_id": "1"}}));
    assert!(bulk_body[1]["doc"].get("created_time").is_some());
    assert!(bulk_body[1]["doc"]["created_time_source"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(cleared.load(Ordering::SeqCst));
}

#[test]
fn test_category_cursor_roundtrip_and_rejects_garbage() {
    // 커서는 불투명 문자열로 왕복되어야 하고, 깨진 값은 None으로 거부한다.
    let sort_values = vec![json!("철도 네트워크 제국 01 - 레일 헤드"), json!("0_telegram/철도.epub")];
    assert_eq!(decode_category_cursor(&encode_category_cursor(&sort_values)), Some(sort_values));
    assert_eq!(decode_category_cursor("not-a-cursor"), None);
    assert_eq!(decode_category_cursor(""), None);
    // 리스트가 아닌 JSON은 sort 값이 될 수 없다
    let not_a_list = base64::engine::general_purpose::URL_SAFE.encode(json!({"a": 1}).to_string());
    assert_eq!(decode_category_cursor(&not_a_list), None);
}

#[test]
fn test_category_cursor_rejects_invalid_padding() {
    // Base64 padding is valid only at the end, and a quartet cannot contain
    // more than two padding bytes.
    assert_eq!(decode_category_cursor("AA==AAAA"), None);
    assert_eq!(decode_category_cursor("A==="), None);
}

#[test]
fn test_search_latest_docs_edge_cases() {
    let es = new_es();
    let manager = make_manager(&es);

    // 342: None categories
    assert_eq!(ESManager::_exclude_category_queries(None), Vec::<Value>::new());
    // 346: empty category string skipped
    let queries = ESManager::_exclude_category_queries(Some(&["", "valid_cat"]));
    assert_eq!(queries.len(), 1);

    // 359-361: es.search exception
    es.on("search", |_| Err(runtime("ES connection error")));
    let (docs, total) = manager.search_latest_docs(100, None);
    assert_eq!(docs, vec![]);
    assert_eq!(total, 0);
}

/// `MockBackfillES`: 오류 지점을 켜고 끌 수 있는 backfill용 가짜 ES.
struct MockBackfill {
    search_error: AtomicBool,
    bulk_error: AtomicBool,
    bulk_items_error: AtomicBool,
    clear_scroll_error: AtomicBool,
    hits: Mutex<Vec<Value>>,
}

fn mock_backfill_es() -> (Arc<DummyEs>, Arc<MockBackfill>) {
    let es = new_es();
    let mock = Arc::new(MockBackfill {
        search_error: AtomicBool::new(false),
        bulk_error: AtomicBool::new(false),
        bulk_items_error: AtomicBool::new(false),
        clear_scroll_error: AtomicBool::new(false),
        hits: Mutex::new(Vec::new()),
    });
    let m = mock.clone();
    es.on("search", move |_| {
        if m.search_error.load(Ordering::SeqCst) {
            return Err(runtime("Search error"));
        }
        Ok(json!({"_scroll_id": "s1", "hits": {"hits": m.hits.lock().unwrap().clone()}}))
    });
    es.on("scroll", |_| Ok(json!({"_scroll_id": "s1", "hits": {"hits": []}})));
    let m = mock.clone();
    es.on("bulk", move |_| {
        if m.bulk_error.load(Ordering::SeqCst) {
            return Err(runtime("Bulk error"));
        }
        if m.bulk_items_error.load(Ordering::SeqCst) {
            return Ok(json!({"errors": true, "items": [{"update": {"error": "Item error"}}]}));
        }
        Ok(json!({"errors": false, "items": []}))
    });
    let m = mock.clone();
    es.on("clear_scroll", move |_| {
        if m.clear_scroll_error.load(Ordering::SeqCst) {
            return Err(runtime("Clear scroll error"));
        }
        Ok(Value::Null)
    });
    (es, mock)
}

#[test]
fn test_backfill_created_time_all_error_branches() {
    let tmp = TempDir::new().unwrap();

    // 1. search_error (440-442) & clear_scroll_error (447-448)
    let (es, mock) = mock_backfill_es();
    mock.search_error.store(true, Ordering::SeqCst);
    mock.clear_scroll_error.store(true, Ordering::SeqCst);
    let manager = make_manager(&es);
    let res = manager.backfill_created_time(tmp.path(), 500);
    assert_eq!(res["failed"], json!(1));

    // 2. bulk_body 비어있을 때 return (414) - 유효하지 않은 파일만 있는 경우 (384, 390-392)
    let (es2, mock2) = mock_backfill_es();
    *mock2.hits.lock().unwrap() = vec![
        json!({"_id": "1", "_source": {"file_path": ""}}),                                                   // 384: not stored_path
        json!({"_id": "2", "_source": {"file_path": "/outside/prefix/file.txt"}}),                           // 390-392: outside prefix
        json!({"_id": "3", "_source": {"file_path": tmp.path().join("nonexistent.txt").to_str().unwrap()}}), // not file
    ];
    let manager2 = make_manager(&es2);
    let res2 = manager2.backfill_created_time(tmp.path(), 500);
    assert_eq!(res2["skipped"], json!(3));
    assert_eq!(res2["updated"], json!(0));

    // 3. path_created_time_with_source OSError (407-409)
    let valid_file = tmp.path().join("test_file.txt");
    std::fs::write(&valid_file, "content").unwrap();
    let (es3, mock3) = mock_backfill_es();
    *mock3.hits.lock().unwrap() = vec![json!({"_id": "1", "_source": {"file_path": valid_file.to_str().unwrap()}})];
    // 구 set_path_created_time_override: 생성 시각 조회(stat)가 OSError를 낸다.
    let stat_fail = FaultyFs::new();
    stat_fail.set_metadata(fail_always(std::io::ErrorKind::Other, "stat error"));
    let manager3 = make_manager(&es3).with_fs(stat_fail);
    let res3 = manager3.backfill_created_time(tmp.path(), 500);
    assert_eq!(res3["failed"], json!(1));

    // 4. bulk_error (417-420)
    let (es4, mock4) = mock_backfill_es();
    *mock4.hits.lock().unwrap() = vec![json!({"_id": "1", "_source": {"file_path": valid_file.to_str().unwrap()}})];
    mock4.bulk_error.store(true, Ordering::SeqCst);
    let manager4 = make_manager(&es4);
    let res4 = manager4.backfill_created_time(tmp.path(), 500);
    assert_eq!(res4["failed"], json!(1));

    // 5. bulk_items_error (422-427)
    let (es5, mock5) = mock_backfill_es();
    *mock5.hits.lock().unwrap() = vec![json!({"_id": "1", "_source": {"file_path": valid_file.to_str().unwrap()}})];
    mock5.bulk_items_error.store(true, Ordering::SeqCst);
    let manager5 = make_manager(&es5);
    let res5 = manager5.backfill_created_time(tmp.path(), 500);
    assert_eq!(res5["failed"], json!(1));
    assert_eq!(res5["updated"], json!(0));
}

#[test]
fn test_get_doc_ids_by_path_prefix_clear_scroll_error() {
    // ScrollErrES
    let es = new_es();
    es.on("search", |_| Ok(json!({"_scroll_id": "s_err", "hits": {"hits": []}})));
    es.on("clear_scroll", |_| Err(runtime("clear scroll error")));
    let manager = make_manager(&es);
    // 586-587: clear_scroll exception handled silently
    let ids = manager.get_doc_ids_by_path_prefix("some/prefix");
    assert_eq!(ids, HashSet::new());
}

#[test]
fn test_ensure_category_nori_and_created_time_fields_already_present() {
    // ExistingFieldsES: 필드가 이미 다 있는 매핑
    let es = new_es();
    es.set_mapping(json!({
        "idx": {
            "mappings": {
                "properties": {
                    "category": {"type": "keyword", "fields": {"nori": {"type": "text"}}},
                    "created_time": {"type": "date"},
                    "created_time_source": {"type": "keyword"},
                }
            }
        }
    }));
    let manager = make_manager(&es);
    manager._ensure_category_nori_subfield();
    assert!(!es.state(|s| s.put_mapping_called));
    manager._ensure_created_time_field();
    assert!(!es.state(|s| s.put_mapping_called));
}

#[test]
fn test_backfill_created_time_resolve_oserror_and_clear_scroll_error() {
    // ClearScrollErrES: 검색 1건, 스크롤은 빈 페이지, clear_scroll은 기록 후 예외.
    let es = new_es();
    let cleared = Arc::new(AtomicBool::new(false));
    es.on("search", |_| Ok(json!({"_scroll_id": "scroll_123", "hits": {"hits": [{"_id": "1", "_source": {"file_path": "some/bad/path"}}]}})));
    es.on("scroll", |_| Ok(json!({"_scroll_id": "scroll_123", "hits": {"hits": []}})));
    let flag = cleared.clone();
    es.on("clear_scroll", move |_| {
        flag.store(true, Ordering::SeqCst);
        Err(runtime("clear_scroll failed"))
    });

    let tmp = TempDir::new().unwrap();
    // 경로에 "bad"가 들어 있으면 canonicalize(구 resolve)가 OSError를 던진다. 나머지는 안쪽 RealFs를 따른다.
    let resolve_fail = FaultyFs::new();
    resolve_fail.set_canonicalize(fail_if_path_contains("bad", std::io::ErrorKind::Other));
    let manager = make_manager(&es).with_fs(resolve_fail);
    let res = manager.backfill_created_time(tmp.path(), 500);
    assert!(cleared.load(Ordering::SeqCst));
    assert_eq!(res["skipped"], json!(1));
}

#[test]
fn test_search_similar_docs_paged_with_exclude_id() {
    let es = new_es();
    es.on("msearch", |_| Ok(json!({"responses": [{"hits": {"hits": [{"_score": 10.0}]}}, {"hits": {"total": {"value": 1}, "hits": [{"_id": "2", "_score": 8.0, "_source": {"title": "foo"}}]}}]})));
    let manager = make_manager(&es);
    let args = SimilarDocsArgs { title: "test".into(), exclude_id: Some(1), ..Default::default() };
    let (results, total) = manager.search_similar_docs_paged(&args).unwrap();
    assert_eq!(total, 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 2);
    assert_eq!(results[0].2, 80.0);
}

#[test]
fn test_search_by_id_success() {
    let es = new_es();
    es.on("get", |kw| Ok(json!({"_id": kw["id"], "_source": {"title": "found_book"}})));
    let manager = make_manager(&es);
    let source = manager.search_by_id(42).unwrap();
    assert_eq!(source, json!({"title": "found_book"}));
}

/// `_summary_clause`: should 절에서 summary 를 대상으로 하는 절 하나를 찾아 돌려준다.
fn summary_clause(should_clauses: &Value) -> Value {
    for clause in should_clauses.as_array().expect("should 배열") {
        if clause.get("more_like_this").is_some_and(|m| m["fields"] == json!(["summary"])) {
            return clause.clone();
        }
        if clause.get("match").is_some_and(|m| m.get("summary").is_some()) {
            return clause.clone();
        }
    }
    panic!("summary 절을 찾지 못했다: {should_clauses}");
}

/// `RecordingES.last_msearch`: 마지막 `msearch`의 searches.
fn last_msearch(es: &DummyEs) -> Vec<Value> {
    es.calls_of("msearch").pop().expect("msearch 호출")["searches"].as_array().expect("searches").clone()
}

#[test]
fn test_list_source_fields_excludes_summary() {
    // 목록 응답에 summary 가 없으므로 ES 에서도 받아오지 않는다.
    assert!(!ESManager::LIST_SOURCE_FIELDS.contains(&"summary"));
    // Book/Comics 가 생성자에서 읽는 필수 필드는 빠지면 안 된다.
    for required in ["category", "title", "author", "file_path", "file_type", "file_size", "updated_time"] {
        assert!(ESManager::LIST_SOURCE_FIELDS.contains(&required), "필수 필드 누락: {required}");
    }
}

#[test]
fn test_category_paged_requests_only_list_source_fields() {
    let es = new_es();
    let manager = make_manager(&es);
    manager.search_by_category_paged("A", 2, None);
    assert_eq!(last_search(&es)["source"], json!(ESManager::LIST_SOURCE_FIELDS));
}

#[test]
fn test_keyword_paged_requests_only_list_source_fields() {
    let es = new_es();
    let manager = make_manager(&es);
    manager.search_by_keyword_paged("k", 2, 0, None, None).unwrap();
    assert_eq!(last_search(&es)["source"], json!(ESManager::LIST_SOURCE_FIELDS));
}

#[test]
fn test_latest_docs_requests_only_list_source_fields() {
    let es = new_es();
    let manager = make_manager(&es);
    manager.search_latest_docs(2, None);
    assert_eq!(last_search(&es)["source"], json!(ESManager::LIST_SOURCE_FIELDS));
}

#[test]
fn test_similar_docs_uses_more_like_this_not_raw_match() {
    // summary 원문을 match 로 넣으면 nori 토큰 1,000여 개 OR 쿼리가 되어 3.5초 걸린다.
    let es = new_es();
    let manager = make_manager(&es);
    let args = SimilarDocsArgs { title: "t".into(), author: "a".into(), file_size: 100, summary: "본문 ".repeat(500), ..Default::default() };
    manager.search_similar_docs(&args).unwrap();
    let should = &last_search(&es)["query"]["bool"]["should"];
    let clause = summary_clause(should);
    assert!(clause.get("more_like_this").is_some(), "summary 절이 match 로 되돌아갔다");
    assert_eq!(clause["more_like_this"]["max_query_terms"], json!(ESManager::SIMILAR_SUMMARY_MAX_QUERY_TERMS));
}

#[test]
fn test_similar_docs_paged_msearch_limits_source() {
    let es = new_es();
    let manager = make_manager(&es);
    let args = SimilarDocsArgs { title: "t".into(), author: "a".into(), file_size: 100, summary: "본문 ".repeat(500), exclude_id: Some(7), size: 5, ..Default::default() };
    manager.search_similar_docs_paged(&args).unwrap();
    let searches = last_msearch(&es);
    let (self_score_body, main_body) = (&searches[1], &searches[3]);
    // self-score 는 점수만 쓰므로 _source 가 통째로 꺼져 있어야 한다.
    assert_eq!(self_score_body["_source"], json!(false));
    assert_eq!(main_body["_source"], json!(ESManager::LIST_SOURCE_FIELDS));
    let clause = summary_clause(&main_body["query"]["bool"]["should"]);
    assert!(clause.get("more_like_this").is_some());
}

#[test]
fn test_aggregate_cover_categories_counts_only_cover_types_across_pages_with_latest_times() {
    let es = new_es();
    let manager = make_manager(&es);

    let bucket = |category: &str, count: i64, updated: Value, created: Value| json!({"key": {"category": category}, "doc_count": count, "updated": {"value": updated}, "created": {"value": created}});
    let first = json!({"aggregations": {"categories": {"buckets": [bucket("A", 2, json!(1000.0), json!(900.0))], "after_key": {"category": "A"}}}});
    let second = json!({"aggregations": {"categories": {"buckets": [bucket("B", 5, json!(2000.0), Value::Null)], "after_key": null}}});
    es.on("search", move |kw| if kw["aggs"]["categories"]["composite"].get("after").is_none() { Ok(first.clone()) } else { Ok(second.clone()) });

    assert_eq!(manager.aggregate_cover_categories(), HashMap::from([("A".to_string(), (2, Some(1000.0), Some(900.0))), ("B".to_string(), (5, Some(2000.0), None))]));
    let calls = es.calls_of("search");
    assert_eq!(calls[0]["query"], json!({"bool": {"filter": [{"terms": {"file_type": ["epub", "pdf"]}}]}}));
    assert_eq!(calls[0]["aggs"]["categories"]["aggs"], json!({"updated": {"max": {"field": "updated_time"}}, "created": {"max": {"field": "created_time"}}}));
    assert_eq!(calls[1]["aggs"]["categories"]["composite"]["after"], json!({"category": "A"}));
}

#[test]
fn test_aggregate_cover_categories_returns_empty_on_error_or_no_buckets() {
    let es = new_es();
    let manager = make_manager(&es);

    es.on("search", |_| Err(runtime("es down")));
    assert_eq!(manager.aggregate_cover_categories(), HashMap::new());

    es.on("search", |_| Ok(json!({"aggregations": {"categories": {"buckets": []}}})));
    assert_eq!(manager.aggregate_cover_categories(), HashMap::new());
}

#[test]
fn test_search_cover_files_paged_filters_types_and_categories_and_returns_sort_values() {
    let es = new_es();
    let manager = make_manager(&es);
    let captured = capture(&es, "search", json!({"hits": {"total": {"value": 9}, "hits": [{"_id": "1", "_source": {"title": "가"}, "sort": ["가", "/1.epub"]}]}}));

    let (hits, total) = manager.search_cover_files_paged(&["A", "B"], 1, Some(&[json!("x"), json!("y")]), Some(&["secret"]));

    assert_eq!(hits, vec![(1, json!({"title": "가"}), vec![json!("가"), json!("/1.epub")])]);
    assert_eq!(total, 9);
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured["query"]["bool"]["filter"], json!([{"terms": {"file_type": ["epub", "pdf"]}}, {"terms": {"category": ["A", "B"]}}]));
    assert_eq!(captured["query"]["bool"]["must_not"], json!(ESManager::_exclude_category_queries(Some(&["secret"]))));
    // 최신 탭과 같은 최신순이다.
    assert_eq!(captured["sort"], ESManager::latest_sort());
    assert_eq!(captured["search_after"], json!(["x", "y"]));
}

#[test]
fn test_search_cover_files_paged_skips_search_without_categories_and_handles_error() {
    let es = new_es();
    let manager = make_manager(&es);
    es.on("search", |_| panic!("검색하면 안 된다"));
    assert_eq!(manager.search_cover_files_paged(&[], 60, None, None), (vec![], 0));

    es.on("search", |_| Err(runtime("es down")));
    assert_eq!(manager.search_cover_files_paged(&["A"], 60, None, None), (vec![], 0));
}

#[test]
fn test_search_group_representatives_queries_each_work_directory_by_file_path() {
    let es = new_es();
    let manager = make_manager(&es);
    let captured = capture(&es, "msearch", json!({"responses": [{"hits": {"hits": [{"_id": "3", "_source": {"title": "작품 1권"}}]}}, {"error": {"type": "boom"}}]}));

    let result = manager.search_group_representatives(&["a/작품", "b/작품"], Some(&["secret"]));

    assert_eq!(result, HashMap::from([("a/작품".to_string(), vec![(3, json!({"title": "작품 1권"}))])]));
    let captured = captured.lock().unwrap().clone();
    let first_query = &captured["searches"][1];
    assert_eq!(captured["searches"][0], json!({}));
    assert_eq!(first_query["sort"], json!([{"file_path": "asc"}]));
    assert_eq!(first_query["size"], json!(ESManager::REPRESENTATIVE_CANDIDATES));
    assert_eq!(first_query["query"]["bool"]["should"], json!([{"term": {"category": "a/작품"}}, {"prefix": {"category": "a/작품/"}}]));
    assert_eq!(first_query["query"]["bool"]["filter"], json!([{"terms": {"file_type": ["epub", "pdf"]}}]));
    assert_eq!(first_query["query"]["bool"]["must_not"], json!(ESManager::_exclude_category_queries(Some(&["secret"]))));
}

#[test]
fn test_search_group_representatives_handles_empty_input_and_error() {
    let es = new_es();
    let manager = make_manager(&es);
    assert_eq!(manager.search_group_representatives(&[], None), HashMap::new());

    es.on("msearch", |_| Err(runtime("es down")));
    assert_eq!(manager.search_group_representatives(&["a/b"], None), HashMap::new());
}

#[test]
fn test_search_latest_docs_only_returns_cover_types() {
    let es = new_es();
    let manager = make_manager(&es);
    let captured = capture(&es, "search", json!({"hits": {"total": {"value": 0}, "hits": []}}));

    manager.search_latest_docs(5, None);
    assert_eq!(captured.lock().unwrap()["query"], json!({"bool": {"must": [{"match_all": {}}], "filter": [{"terms": {"file_type": ["epub", "pdf"]}}]}}));

    manager.search_latest_docs(5, Some(&["secret"]));
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured["query"]["bool"]["must_not"], json!(ESManager::_exclude_category_queries(Some(&["secret"]))));
    assert_eq!(captured["query"]["bool"]["filter"], json!([{"terms": {"file_type": ["epub", "pdf"]}}]));
}

mod api_facade {
    use super::common;
    use backend::es_manager::{ESManager, EsError, EsTransport};
    use common::es_dummy::DummyEs;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

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
}
