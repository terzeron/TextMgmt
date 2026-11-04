//! `classifier::corpus_collector` 단위 테스트.

mod common;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use classifier::corpus_collector::{*, absolute_path, attach_publishers, attach_publishers_with, AttachOptions, document, IndexedJsonl, is_trainable, MAX_JSONL_LINE_BYTES, read_jsonl, write_jsonl};
use common::es_dummy::DummyEs;
use serde_json::{json, Value};
use tempfile::TempDir;
use classifier::model_trainer;
use std::collections::HashSet;
use common::epub::{EpubSpec, make_epub};
use classifier::confidence_calibrator::{expected_accuracy, fit_expected_accuracy, from_coverage_report};
use crate::common::shared_fixtures::{VOCAB, floats, make_docs, parity, trained};

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

#[test]
fn indexed_holdout_indices_match_materialized_selection_when_keys_repeat() {
    let docs = vec![json!({"id":"same","cat":"A","text":"first"}), json!({"id":"same","cat":"A","text":"second"})];
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("duplicate-keys.jsonl");
    write_jsonl(&docs, &path).unwrap();
    let indexed = IndexedJsonl::open(&path).unwrap();
    let (kept, _) = model_trainer::filter_by_size(&docs, 2);
    let (_, expected_docs) = model_trainer::split_holdout(&kept, 0.5, 42);
    let expected: Vec<usize> = expected_docs.iter().map(|expected| docs.iter().position(|doc| doc == expected).unwrap()).collect();

    assert_eq!(model_trainer::holdout_indices_indexed(&indexed, 2, 0.5, 42), expected);
}

/// Python 테스트 생성기와 같은 LCG.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn assert_close(got: &[f64], want: &[f64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: 길이");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 1e-12, "{what}[{i}]: rust {g} python {w}");
    }
}

// ---------------------------------------------------------------------------
// corpus
// ---------------------------------------------------------------------------

#[test]
fn test_document_takes_top_level_category_and_filename() {
    let doc = document(
        &json!({"_id": "12345", "_source": {"category": "9_북스캔OCR/completed/2024", "summary": "본문", "title": "제목", "author": "저자", "publisher": "출판사", "file_path": "/mnt/data/text/9_북스캔OCR/책.epub", "file_type": "epub"}}),
    );
    assert_eq!(doc["id"], "12345");
    assert_eq!(doc["cat"], "9_북스캔OCR");
    assert_eq!(doc["name"], "책.epub");
    assert_eq!(doc["publisher"], "출판사");
}

#[test]
fn test_document_tolerates_missing_fields() {
    let doc = document(&json!({"_id": 7, "_source": {}}));
    assert_eq!(doc, json!({"id": "7", "cat": "", "text": "", "title": "", "author": "", "publisher": "", "type": "", "name": "", "path": ""}));
}

#[test]
fn test_is_trainable() {
    let cases = [
        (json!({"cat": "3_무협", "text": "가".repeat(200)}), true),
        (json!({"cat": "3_무협", "text": "가".repeat(199)}), false),
        (json!({"cat": "", "text": "가".repeat(500)}), false),
        // 0_* 는 미분류/격리라 레이블이 아니다
        (json!({"cat": "0_nf", "text": "가".repeat(500)}), false),
        // 책이 아니라 페이지 이미지다
        (json!({"cat": "9_북스캔OCR", "text": "가".repeat(500)}), false),
        // 카테고리가 아니라 운영용 디렉토리다. 실측에서 789건이 섞여 들어왔다.
        (json!({"cat": "trash", "text": "가".repeat(500)}), false),
        (json!({"cat": "_root", "text": "가".repeat(500)}), false),
        (json!({"cat": ".preview_cache", "text": "가".repeat(500)}), false),
    ];
    for (doc, expected) in cases {
        assert_eq!(is_trainable(&doc, 200, &["9_북스캔OCR"]), expected, "case: cat={} len={}", doc["cat"], doc["text"].as_str().unwrap().chars().count());
    }
}

#[test]
fn test_absolute_path_resolves_paths_relative_to_the_library_root() {
    // loader 가 라이브러리 루트 기준 상대 경로로 색인하므로 그대로 열면 못 찾는다
    assert_eq!(absolute_path(&json!({"path": "3_무협/책.epub"}), "/mnt/data/text"), Some(PathBuf::from("/mnt/data/text/3_무협/책.epub")));
    assert_eq!(absolute_path(&json!({"path": "/절대/경로/책.epub"}), "/mnt/data/text"), Some(PathBuf::from("/절대/경로/책.epub")));
    assert_eq!(absolute_path(&json!({"path": ""}), "/mnt/data/text"), None);
    assert_eq!(absolute_path(&json!({}), "/mnt/data/text"), None);
}

#[test]
fn test_jsonl_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("sub").join("corpus.jsonl");
    let docs = make_docs(2);
    assert_eq!(write_jsonl(&docs, &path).unwrap(), docs.len());
    assert_eq!(read_jsonl(&path).unwrap(), docs);
}

#[test]
fn test_attach_publishers_only_fills_missing_values_and_caches() {
    let tmp = TempDir::new().unwrap();
    // ES 에 들어 있는 모양대로 라이브러리 루트 기준 상대 경로를 준다
    make_epub(&tmp.path().join("book.epub"), &EpubSpec::publisher("열린책들"));
    let cache = tmp.path().join("publisher.json");
    let mut docs = vec![
        json!({"id": "1", "type": "epub", "path": "book.epub", "publisher": ""}),
        // ES 가 이미 값을 갖고 있으면 파일을 안 읽는다
        json!({"id": "2", "type": "epub", "path": "없는파일.epub", "publisher": "이미있음"}),
        // EPUB 이 아니면 dc:publisher 가 없다
        json!({"id": "3", "type": "txt", "path": "a.txt", "publisher": ""}),
    ];
    let stats = attach_publishers(&mut docs, &cache, tmp.path(), 1);
    assert_eq!(stats["read"], 1);
    assert_eq!(stats["with_publisher"], 2);
    assert_eq!(docs[0]["publisher"], "열린책들");
    assert_eq!(docs[1]["publisher"], "이미있음");
    assert_eq!(docs[2]["publisher"], "");
    assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(&cache).unwrap()).unwrap(), json!({"1": "열린책들"}));

    // 두 번째 호출은 캐시만 쓰고 파일을 다시 안 읽는다
    fs::remove_file(tmp.path().join("book.epub")).unwrap();
    let mut again = vec![json!({"id": "1", "type": "epub", "path": "book.epub", "publisher": ""})];
    assert_eq!(attach_publishers(&mut again, &cache, tmp.path(), 1)["read"], 0);
    assert_eq!(again[0]["publisher"], "열린책들");
}

#[test]
fn test_train_learns_the_three_categories() {
    let (model, report) = trained();
    let classes: HashSet<&str> = model.classes.iter().map(String::as_str).collect();
    assert_eq!(classes, VOCAB.iter().map(|(c, _)| *c).collect::<HashSet<_>>());
    assert!(report["top1_accuracy"].as_f64().unwrap() > 0.9);
    assert_eq!(model.meta["documents"], 120);
    assert!(!model.meta["config_hash"].as_str().unwrap_or("").is_empty());
    assert!(model.meta["blocks"].get("publisher").is_none() || model.meta["blocks"]["publisher"]["weight"] == 3.0);
}

#[test]
fn isotonic_fit_matches_sklearn() {
    let p = parity();
    let mut rng = Lcg(1);
    let (mut conf, mut correct) = (Vec::new(), Vec::new());
    for _ in 0..p["isotonic"]["n"].as_u64().unwrap() {
        let c = 0.013 + 0.137 * rng.next();
        conf.push(c);
        correct.push(rng.next() < (0.4 + (c - 0.013) * 4.3).clamp(0.0, 0.99));
    }
    let cal = fit_expected_accuracy(&conf, &correct).unwrap().unwrap();
    let want = &p["isotonic"]["expected"];
    assert_eq!(cal["n"], want["n"]);
    assert_close(&floats(&cal["x"]), &floats(&want["x"]), "x");
    assert_close(&floats(&cal["y"]), &floats(&want["y"]), "y");
    let probes = floats(&p["isotonic"]["probes"]);
    let got: Vec<f64> = probes.iter().map(|x| expected_accuracy(Some(&cal), Some(*x)).unwrap()).collect();
    assert_close(&got, &floats(&p["isotonic"]["probe_values"]), "probe");
}

#[test]
fn coverage_curve_scale_matches_python() {
    let p = parity();
    let cal = from_coverage_report(Some(&p["coverage"]["holdout"])).unwrap();
    let want = &p["coverage"]["expected"];
    assert_eq!(cal["source"], want["source"]);
    assert_eq!(cal["n"], want["n"]);
    assert_close(&floats(&cal["x"]), &floats(&want["x"]), "x");
    assert_close(&floats(&cal["y"]), &floats(&want["y"]), "y");
    let probes = floats(&p["coverage"]["probes"]);
    let got: Vec<f64> = probes.iter().map(|x| expected_accuracy(Some(&cal), Some(*x)).unwrap()).collect();
    assert_close(&got, &floats(&p["coverage"]["probe_values"]), "probe");
}

#[test]
fn jsonl_is_byte_compatible_with_python_and_round_trips() {
    let p = parity();
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("c.jsonl");
    write_jsonl(&[p["json_dumps"]["value"].clone()], &path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), p["json_dumps"]["line"].as_str().unwrap());
    // Python 이 쓴 줄(와 빈 줄)을 Rust 가 읽는다.
    fs::write(&path, format!("{}\n\n{}", p["json_dumps"]["line"].as_str().unwrap(), p["json_dumps"]["line"].as_str().unwrap())).unwrap();
    assert_eq!(read_jsonl(&path).unwrap().len(), 2);
}

#[test]
fn indexed_jsonl_retains_metadata_and_reads_documents_on_demand() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("indexed.jsonl");
    let first = json!({"id":"id-1","cat":"3_무협","text":"긴 본문","publisher":"출판사"});
    let second = json!({"id":7,"path":"5_과학/book.txt","cat":"5_과학","text":"더 긴 본문","publisher":""});
    fs::write(&path, format!("{}\n\n{}\n", first, second)).unwrap();

    let mut indexed = IndexedJsonl::open(&path).unwrap();

    assert_eq!(indexed.len(), 2);
    assert_eq!(indexed.category(0), Some("3_무협"));
    assert_eq!(indexed.document_key(0), Some("id-1"));
    assert!(indexed.has_publisher());
    assert_eq!(indexed.category(1), Some("5_과학"));
    assert_eq!(indexed.document_key(1), Some("5_과학/book.txt"));
    assert_eq!(indexed.read_document(0).unwrap(), first);
    assert_eq!(indexed.read_document(1).unwrap(), second);
}

#[test]
fn jsonl_rejects_oversized_lines_and_broken_json() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("big.jsonl");
    fs::write(&path, vec![b'a'; MAX_JSONL_LINE_BYTES as usize + 10]).unwrap();
    assert!(read_jsonl(&path).is_err());
    fs::write(&path, "{\"a\": 1}\n이건 JSON 이 아니다\n").unwrap();
    assert!(read_jsonl(&path).is_err());
    // 깊이가 한도를 넘는 JSON 도 오류다(스택을 쓰지 않는다).
    fs::write(&path, format!("{}1{}\n", "[".repeat(5000), "]".repeat(5000))).unwrap();
    assert!(read_jsonl(&path).is_err());
}

#[test]
fn absolute_path_rejects_parent_dir_escapes() {
    assert_eq!(absolute_path(&json!({"path": "../etc/passwd"}), "/mnt/data/text"), None);
    assert_eq!(absolute_path(&json!({"path": "a/../../b.epub"}), "/mnt/data/text"), None);
    assert!(absolute_path(&json!({"path": "a/b..c.epub"}), "/mnt/data/text").is_some());
}

#[test]
fn attach_publishers_skips_symlinks_that_leave_the_library() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("lib");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.epub"), b"x").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.epub"), root.join("link.epub")).unwrap();
    fs::write(root.join("ok.epub"), b"x").unwrap();

    let mut docs = vec![json!({"id": "1", "type": "epub", "path": "link.epub", "publisher": ""}), json!({"id": "2", "type": "epub", "path": "ok.epub", "publisher": ""})];
    let seen = std::sync::Mutex::new(Vec::new());
    let reader = |p: &Path| {
        seen.lock().unwrap().push(p.file_name().unwrap().to_string_lossy().into_owned());
        "출판사".to_string()
    };
    let stats = attach_publishers_with(&mut docs, &tmp.path().join("pub.json"), &root, AttachOptions { workers: 2, progress_every: 0, log: None }, &reader);
    assert_eq!(*seen.lock().unwrap(), vec!["ok.epub"]);
    assert_eq!(stats.read, 1);
    assert_eq!(docs[0]["publisher"], "");
    assert_eq!(docs[1]["publisher"], "출판사");
}
