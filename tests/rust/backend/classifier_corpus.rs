//! 원본: tests/test_classifier_corpus.py
//!
//! backend/classifier/corpus.py 중 ES 를 거치는 경로와 publisher 뒷마감 테스트.
//! ES 조회와 EPUB 읽기는 통합 테스트에서 안 밟히는 구간이라 여기서 직접 확인한다.
//!
//! 가정한 API:
//! - `backend::es_manager::EsTransport` 트레이트: `index_name(&self) -> &str`, `call(&self, op: &str, kwargs: &Value) -> Result<Value, EsError>`
//!   (`get`, `search`는 `call("get")`, `call("search")`로 가는 기본 구현). 구 `EsLike`를 대체한다(docs/rust-test-seams.md 3.3, 3.13).
//!   대역은 `common::es_dummy::DummyEs`(`respond`, `fail`, `calls_of`)다.
//! - `backend::classifier::corpus::fetch_by_inode(es: &dyn EsTransport, inode: u64, index: Option<&str>) -> Option<Value>`
//! - `backend::classifier::corpus::fetch_by_path(es: &dyn EsTransport, path: &str, index: Option<&str>) -> Option<Value>`
//! - (mock.patch 테스트용 seam, 문서 4.1에 따라 `*_with` 클로저는 현행 유지) `scan_documents_with(es, ScanOptions { batch, index }, scan: &dyn Fn(&Value) -> Vec<Value>) -> Vec<Value>`,
//!   `collect_with(es, out: &Path, CollectOptions { min_chars, excluded_prefixes }, scan) -> Value`,
//!   `publisher_of_with(path: &Path, reader: &dyn Fn(&Path) -> Result<String, String>) -> String`,
//!   `attach_publishers_with(docs: &mut [Value], cache_path: &Path, root: &Path, AttachOptions { workers, progress_every, log }, publisher_of: &(dyn Fn(&Path) -> String + Sync)) -> AttachStats { read, with_publisher }`

mod common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use backend::classifier::corpus::*;
use common::es_dummy::DummyEs;
use serde_json::{Value, json};
use tempfile::TempDir;

/// 구 `FakeEs(get_result, search_result, raise_on)`. `common::es_dummy::DummyEs`(색인 `books`)로 같은 응답과 실패를 만든다.
fn fake_es(get_result: Value, search_result: Value, raise_on: &str) -> Arc<DummyEs> {
    let es = DummyEs::with_index("books");
    es.respond("get", get_result);
    es.respond("search", search_result);
    if !raise_on.is_empty() {
        es.fail(raise_on, "ES 없음");
    }
    es
}

fn hit_with(doc_id: &str, category: &str, summary: &str, file_path: &str, file_type: &str, publisher: &str) -> Value {
    json!({"_id": doc_id, "_source": {"category": category, "summary": summary, "file_path": file_path, "file_type": file_type, "publisher": publisher, "title": "제목", "author": "저자"}})
}

/// Python `_hit(doc_id)`의 기본 인자와 같다.
fn hit(doc_id: &str) -> Value {
    hit_with(doc_id, "2_소설한국", "본문", "2_소설한국/a.epub", "epub", "")
}

/// 가정한 seam: `scan_documents_with`가 `elasticsearch.helpers.scan` 자리에 함수를 주입받는다.
#[test]
fn test_scan_documents_yields_records() {
    let captured: Mutex<Value> = Mutex::new(json!({}));
    let fake_scan = |kwargs: &Value| -> Vec<Value> {
        *captured.lock().unwrap() = kwargs.clone();
        vec![hit("1"), hit_with("2", "4_역사인물/하위", "본문", "2_소설한국/a.epub", "epub", "")]
    };

    let es = fake_es(Value::Null, Value::Null, "");
    let docs = scan_documents_with(&*es, ScanOptions { batch: 7, index: None }, &fake_scan);

    let ids: Vec<&str> = docs.iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["1", "2"]);
    // 카테고리가 경로면 최상위만 레이블로 쓴다.
    assert_eq!(docs[1]["cat"], "4_역사인물");
    assert_eq!(captured.lock().unwrap()["index"], "books");
    assert_eq!(captured.lock().unwrap()["size"], 7);
}

#[test]
fn test_scan_documents_honors_explicit_index() {
    let captured: Mutex<Value> = Mutex::new(json!({}));
    let fake_scan = |kwargs: &Value| -> Vec<Value> {
        *captured.lock().unwrap() = kwargs.clone();
        Vec::new()
    };

    let es = fake_es(Value::Null, Value::Null, "");
    assert!(scan_documents_with(&*es, ScanOptions { batch: 1000, index: Some("other".to_string()) }, &fake_scan).is_empty());
    assert_eq!(captured.lock().unwrap()["index"], "other");
}

#[test]
fn test_fetch_by_inode_found_and_missing() {
    let es = fake_es(hit("42"), Value::Null, "");
    let doc = fetch_by_inode(&*es, 42, None);
    assert!(doc.is_some() && doc.as_ref().unwrap()["id"] == "42");
    assert_eq!(es.calls_of("get")[0]["id"], "42");

    // 못 찾으면 None 을 주고 호출부가 파일에서 직접 읽는 경로로 넘어간다.
    assert!(fetch_by_inode(&*fake_es(Value::Null, Value::Null, "get"), 42, None).is_none());
}

#[test]
fn test_fetch_by_inode_uses_explicit_index() {
    let es = fake_es(hit("42"), Value::Null, "");
    fetch_by_inode(&*es, 42, Some("other"));
    assert_eq!(es.calls_of("get")[0]["index"], "other");
}

#[test]
fn test_fetch_by_path_found_empty_and_error() {
    let es = fake_es(Value::Null, json!({"hits": {"hits": [hit("7")]}}), "");
    let doc = fetch_by_path(&*es, "2_소설한국/a.epub", None);
    assert!(doc.is_some() && doc.as_ref().unwrap()["id"] == "7");

    assert!(fetch_by_path(&*fake_es(Value::Null, json!({"hits": {"hits": []}}), ""), "x", None).is_none());
    // hits 키 자체가 없어도 죽지 않는다.
    assert!(fetch_by_path(&*fake_es(Value::Null, json!({}), ""), "x", None).is_none());
    assert!(fetch_by_path(&*fake_es(Value::Null, Value::Null, "search"), "x", None).is_none());
}

#[test]
fn test_fetch_by_path_uses_explicit_index() {
    let es = fake_es(Value::Null, json!({"hits": {"hits": []}}), "");
    fetch_by_path(&*es, "x", Some("other"));
    assert_eq!(es.calls_of("search")[0]["index"], "other");
}

#[test]
fn test_is_trainable_supports_python_lookahead_label_pattern() {
    let matching = json!({"cat": "3_무협", "text": "본문", "id": "1"});
    let nonmatching = json!({"cat": "3_판타지", "text": "본문", "id": "2"});
    let pattern = fancy_regex::Regex::new(r"^[1-9]_(?=.*무협)").unwrap();

    assert!(is_trainable_with(&matching, 1, &[], &pattern));
    assert!(!is_trainable_with(&nonmatching, 1, &[], &pattern));
}

#[test]
fn test_collect_writes_only_trainable_rows() {
    let fake_scan = |_: &Value| -> Vec<Value> {
        vec![
            hit_with("1", "2_소설한국", &"본문".repeat(20), "2_소설한국/a.epub", "epub", ""),
            hit_with("2", "2_소설한국", &"본문".repeat(20), "2_소설한국/a.epub", "epub", ""),
            // 레이블 꼴이 아니면 운영용 디렉토리라 학습에서 뺀다.
            hit_with("3", "trash", &"본문".repeat(20), "2_소설한국/a.epub", "epub", ""),
            // 본문이 짧으면 표본 수만 늘리고 품질을 떨어뜨린다.
            hit_with("4", "2_소설한국", "짧음", "2_소설한국/a.epub", "epub", ""),
            // 제외 접두사.
            hit_with("5", "9_북스캔OCR", &"본문".repeat(20), "2_소설한국/a.epub", "epub", ""),
        ]
    };

    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("corpus.jsonl");
    let es = fake_es(Value::Null, Value::Null, "");
    let stats = collect_with(&*es, &out, CollectOptions { min_chars: 10, excluded_prefixes: vec!["9_".to_string()], index: None }, &fake_scan).unwrap();

    assert_eq!(stats, json!({"kept": 2, "skipped": 3, "categories": 1, "per_category": {"2_소설한국": 2}}));
    assert_eq!(fs::read_to_string(&out).unwrap().trim().lines().count(), 2);
}

#[test]
fn test_collect_streams_each_scroll_page_to_jsonl_before_fetching_the_next() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("corpus.jsonl");
    let es = DummyEs::with_index("books");
    es.on("search", |_| Ok(json!({"_scroll_id": "first-page", "hits": {"hits": [hit_with("1", "2_소설한국", "본문본문", "2_소설한국/a.epub", "epub", "")]}})));
    let scroll_path = out.clone();
    es.on("scroll", move |_| {
        let rows = fs::read_to_string(&scroll_path).expect("다음 페이지 요청 전에 JSONL 파일이 열려 있어야 한다");
        assert_eq!(rows.lines().count(), 1, "첫 페이지를 먼저 기록해야 한다");
        Ok(json!({"hits": {"hits": []}}))
    });

    let stats = collect(&*es, &out, CollectOptions { min_chars: 1, excluded_prefixes: Vec::new(), index: None }).unwrap();

    assert_eq!(stats["kept"], 1);
    assert_eq!(fs::read_to_string(out).unwrap().lines().count(), 1);
}

#[test]
fn test_publisher_of_swallows_reader_errors() {
    let tmp = TempDir::new().unwrap();
    let boom = |_: &Path| -> Result<String, String> { Err("EPUB 깨짐".to_string()) };
    // 한 권이 깨져도 전체 뒷마감이 멈추면 안 된다.
    assert_eq!(publisher_of_with(&tmp.path().join("x.epub"), &boom), "");
}

fn epub_docs() -> Vec<Value> {
    vec![
        json!({"id": "1", "type": "epub", "publisher": "", "path": "2_소설외국/a.epub"}),
        json!({"id": "2", "type": "epub", "publisher": "을유문화사", "path": "2_소설외국/b.epub"}),
        json!({"id": "3", "type": "pdf", "publisher": "", "path": "2_소설외국/c.pdf"}),
    ]
}

fn opts(workers: usize, progress_every: usize) -> AttachOptions<'static> {
    AttachOptions { workers, progress_every, log: None }
}

#[test]
fn test_attach_publishers_reads_only_missing_epubs() {
    let tmp = TempDir::new().unwrap();
    let publisher_of = |_: &Path| "민음사".to_string();
    let mut docs = epub_docs();
    let cache_path = tmp.path().join("cache").join("pub.json");

    let stats = attach_publishers_with(&mut docs, &cache_path, tmp.path(), opts(1, 0), &publisher_of);

    // ES 에 이미 값이 있는 문서와 EPUB 이 아닌 문서는 읽지 않는다.
    assert_eq!(stats.read, 1);
    assert_eq!(stats.with_publisher, 2);
    assert_eq!(docs[0]["publisher"], "민음사");
    let cached: Value = serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();
    assert_eq!(cached, json!({"1": "민음사"}));
}

#[test]
fn test_attach_publishers_uses_cache_and_skips_disk() {
    let tmp = TempDir::new().unwrap();
    let cache_path = tmp.path().join("pub.json");
    fs::write(&cache_path, json!({"1": "창비"}).to_string()).unwrap();

    let should_not_run = |_: &Path| -> String { panic!("캐시에 있으면 디스크를 읽으면 안 된다") };
    let mut docs = epub_docs();
    let stats = attach_publishers_with(&mut docs, &cache_path, tmp.path(), AttachOptions::default(), &should_not_run);

    assert_eq!(stats.read, 0);
    assert_eq!(docs[0]["publisher"], "창비");
}

#[test]
fn test_attach_publishers_rebuilds_broken_cache() {
    let tmp = TempDir::new().unwrap();
    let cache_path = tmp.path().join("pub.json");
    fs::write(&cache_path, "이건 JSON 이 아니다").unwrap();
    let publisher_of = |_: &Path| "문학동네".to_string();

    let mut docs = epub_docs();
    let stats = attach_publishers_with(&mut docs, &cache_path, tmp.path(), opts(1, 0), &publisher_of);

    // 캐시가 깨졌으면 새로 만든다. 뒷마감 자체는 계속한다.
    assert_eq!(stats.read, 1);
    assert_eq!(docs[0]["publisher"], "문학동네");
}

#[test]
fn test_attach_publishers_skips_documents_without_path() {
    let tmp = TempDir::new().unwrap();
    let publisher_of = |_: &Path| "민음사".to_string();
    let mut docs = vec![json!({"id": "1", "type": "epub", "publisher": "", "path": ""})];

    let stats = attach_publishers_with(&mut docs, &tmp.path().join("pub.json"), tmp.path(), AttachOptions::default(), &publisher_of);

    assert_eq!(stats.read, 0);
    assert_eq!(docs[0]["publisher"], "");
}

#[test]
fn test_attach_publishers_logs_progress() {
    let tmp = TempDir::new().unwrap();
    let publisher_of = |_: &Path| "민음사".to_string();
    let mut docs: Vec<Value> = (0..4).map(|i| json!({"id": i.to_string(), "type": "epub", "publisher": "", "path": format!("2_소설외국/{i}.epub")})).collect();

    // caplog(INFO, logger="backend.classifier.corpus") 대응: log 콜백이 INFO 메시지를 모은다.
    let messages: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let log = |m: &str| messages.lock().unwrap().push(m.to_string());
    let stats = attach_publishers_with(&mut docs, &tmp.path().join("pub.json"), tmp.path(), AttachOptions { workers: 1, progress_every: 2, log: Some(&log) }, &publisher_of);

    assert_eq!(stats.read, 4);
    // 오래 도는 작업이라 진행 상황을 남긴다.
    let messages = messages.lock().unwrap();
    assert!(messages.iter().any(|m| m.contains("publisher 2/4")), "messages={messages:?}");
    assert!(messages.iter().any(|m| m.contains("publisher 4/4")), "messages={messages:?}");
}

#[test]
fn attach_publishers_streams_jsonl_and_preserves_existing_publishers() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("corpus.jsonl");
    let cache = tmp.path().join("corpus.jsonl.publisher.json");
    write_jsonl(
        &[
            json!({"id":"one","type":"epub","path":"one.epub","publisher":""}),
            json!({"id":"two","type":"epub","path":"two.epub","publisher":"기존 출판사"}),
            json!({"id":"three","type":"txt","path":"three.txt","publisher":""}),
        ],
        &path,
    )
    .unwrap();
    let calls = Mutex::new(Vec::new());

    let stats = attach_publishers_jsonl_with(&path, &cache, tmp.path(), 1, &|source| {
        calls.lock().unwrap().push(source.file_name().unwrap().to_string_lossy().into_owned());
        "새 출판사".to_owned()
    })
    .unwrap();

    let docs = read_jsonl(&path).unwrap();
    assert_eq!(stats, json!({"read":1,"cached":1,"with_publisher":2}));
    assert_eq!(*calls.lock().unwrap(), ["one.epub"]);
    assert_eq!(docs[0]["publisher"], "새 출판사");
    assert_eq!(docs[1]["publisher"], "기존 출판사");
    assert_eq!(docs[2]["publisher"], "");
    assert_eq!(serde_json::from_slice::<Value>(&fs::read(&cache).unwrap()).unwrap()["one"], "새 출판사");
}

#[test]
fn attach_publishers_streams_across_batches_and_keeps_the_cache() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("many.jsonl");
    let cache = tmp.path().join("many.publisher.json");
    let docs: Vec<Value> = (0..260).map(|index| json!({"id":index.to_string(),"type":"epub","path":format!("{index}.epub"),"publisher":""})).collect();
    write_jsonl(&docs, &path).unwrap();

    let stats = attach_publishers_jsonl_with(&path, &cache, tmp.path(), 2, &|_| "출판사".to_owned()).unwrap();

    assert_eq!(stats, json!({"read":260,"cached":260,"with_publisher":260}));
    let updated = read_jsonl(&path).unwrap();
    assert_eq!(updated.len(), 260);
    assert!(updated.iter().all(|doc| doc["publisher"] == "출판사"));
    assert_eq!(serde_json::from_slice::<Value>(&fs::read(cache).unwrap()).unwrap().as_object().unwrap().len(), 260);
}

#[test]
fn attach_publishers_does_not_replace_a_corpus_with_invalid_jsonl() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("broken.jsonl");
    fs::write(&path, b"{\"id\":\"a\"}\nnot-json\n").unwrap();
    let before = fs::read(&path).unwrap();

    assert!(attach_publishers_jsonl_with(&path, &tmp.path().join("cache.json"), tmp.path(), 1, &|_| String::new()).is_err());

    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1, "임시 파일이 남으면 안 된다");
}
