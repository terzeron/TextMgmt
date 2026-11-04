
mod common;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use trainer::{CliIo, Deps, main_with};
use serde_json::{json, Value};
use tempfile::TempDir;
use crate::common::shared_fixtures::{fixture, run};

#[test]
fn cli_keeps_exit_codes_when_standard_output_or_error_is_closed() {
    use std::io::{self, Write};

    struct ClosedPipe;
    impl Write for ClosedPipe {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let deps = Deps::system();
    let mut err = Vec::new();
    let mut out = ClosedPipe;
    assert_eq!(main_with(&[], &mut CliIo { out: &mut out, err: &mut err }, &deps), 1);
    assert_eq!(main_with(&["info", "--help"], &mut CliIo { out: &mut out, err: &mut err }, &deps), 1);

    let mut out = Vec::new();
    assert_eq!(main_with(&["unknown"], &mut CliIo { out: &mut out, err: &mut ClosedPipe }, &deps), 2);
    assert_eq!(main_with(&["info", "--model", "/no/such/model"], &mut CliIo { out: &mut out, err: &mut ClosedPipe }, &deps), 1);
}

#[test]
fn malformed_arguments_fail_before_io() {
    for args in [vec!["unknown"], vec!["info", "--oops"], vec!["classify"], vec!["info", "--model"], vec!["classify", "x", "--min-confidence", "NaN"], vec!["reclassify", "x", "--limit", "-1"]] {
        let (code, _, err) = run(&args, &Deps::system());
        assert_eq!(code, 2, "{args:?}: {err}");
    }
}

#[test]
fn info_reads_exported_model_and_missing_model_explains_next_step() {
    let model = fixture();
    let (code, out, err) = run(&["info", "--model", model.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("학습 시각") && out.contains("판정 임계값"));
    let (code, _, err) = run(&["info", "--model", "/no/such/model"], &Deps::system());
    assert_eq!(code, 1);
    assert!(err.contains("collect") && err.contains("train"));
}

#[test]
fn classify_reads_files_and_reports_missing_paths() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("무림.txt");
    fs::write(&book, "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객").unwrap();
    let (code, out, err) = run(&["classify", book.to_str().unwrap(), "/no/such/file", "--model", fixture().to_str().unwrap(), "--no-es", "--min-confidence", "0"], &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("특징 출처: 파일") && out.contains("판정:") && out.contains("파일이 없다"));
}

#[test]
fn training_rejects_insufficient_corpus_without_creating_model() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    fs::write(&corpus, "{\"cat\":\"3_무협\"}\n").unwrap();
    let out = dir.path().join("export");
    let (code, _, err) = run(&["train", "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 1);
    assert!(err.contains("학습할 문서가 없다"));
    assert!(!out.exists());
}

#[test]
fn reband_clamps_threshold_to_measured_confidence() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("policy.json");
    fs::write(&path, json!({"sample_from":"refused", "rows":[{"confidence":0.04,"model_ok":false,"store_ok":true,"store_answered":true}]}).to_string()).unwrap();
    let (code, _, err) = run(&["bookstore-policy", "--reband", path.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["bookstore_override_below"], 0.04);
    assert_eq!(saved["sample_from"], "refused");
}

#[test]
fn reband_missing_file_reports_a_measurement_result_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("없는파일.json");
    let (code, _, err) = run(&["bookstore-policy", "--reband", path.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 1);
    assert!(err.contains("측정 결과 파일이 없다"), "{err}");
}

#[test]
fn reband_without_rows_names_the_file_and_next_step() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty-policy.json");
    fs::write(&path, json!({"bands": []}).to_string()).unwrap();
    let (code, _, err) = run(&["bookstore-policy", "--reband", path.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 1);
    assert!(err.contains(&format!("건별 결과가 없어 다시 나눌 수 없다: {}", path.display())), "{err}");
    assert!(err.contains("서점 조회부터 다시 해야 한다"), "{err}");
}

#[test]
fn reband_explains_when_the_model_beats_stores_in_every_band() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("policy.json");
    fs::write(&path, json!({"rows":[{"confidence":0.25,"model_ok":true,"store_ok":false,"store_answered":true}]}).to_string()).unwrap();
    let (code, out, err) = run(&["bookstore-policy", "--reband", path.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("서점이 모델을 이긴 구간이 없다"), "{out}");
}

#[test]
fn reclassify_preview_apply_and_collision_preserve_files() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("0_inbox");
    fs::create_dir(&source).unwrap();
    let book = source.join("book.txt");
    fs::write(&book, "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객").unwrap();
    let fixture = fixture();
    let mut args = vec!["reclassify", "0_inbox", "--library-root", dir.path().to_str().unwrap(), "--model", fixture.to_str().unwrap(), "--no-es", "--min-confidence", "0"];
    let (code, out, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("미리보기다"));
    assert!(book.exists());
    args.push("--apply");
    let (code, _, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(!book.exists());
    fs::write(&book, "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객").unwrap();
    assert_eq!(run(&args, &Deps::system()).0, 0);
    assert!(book.exists());
}

#[test]
fn reclassify_defaults_to_the_model_confidence_threshold() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("3_무협");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("무림맹_장문인.txt"), "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객").unwrap();

    let model_path = fixture();
    let model = classifier::inference_model::CategoryModel::load_dir(&model_path).unwrap();
    let common = ["reclassify", "3_무협", "--library-root", dir.path().to_str().unwrap(), "--model", model_path.to_str().unwrap(), "--no-es"];
    let (code, default_output, err) = run(&common, &Deps::system());
    assert_eq!(code, 0, "{err}");

    let threshold = model.min_confidence().to_string();
    let mut explicit = common.to_vec();
    explicit.extend(["--min-confidence", &threshold]);
    let (code, explicit_output, err) = run(&explicit, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert_eq!(default_output, explicit_output);
    assert_ne!(model.min_confidence(), 0.90);
}

#[test]
fn reclassify_prints_progress_every_500_files() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("0_inbox");
    fs::create_dir(&source).unwrap();
    for i in 0..500 {
        fs::write(source.join(format!("book-{i:03}.txt")), "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객").unwrap();
    }

    let model = fixture();
    let (code, out, err) = run(&["reclassify", "0_inbox", "--library-root", dir.path().to_str().unwrap(), "--model", model.to_str().unwrap(), "--no-es"], &Deps::system());

    assert_eq!(code, 0, "{err}");
    assert!(out.contains("500/500 처리"), "{out}");
}

#[test]
fn reband_skips_bands_without_rows() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("policy.json");
    let rows: Vec<_> = (0..4).map(|_| json!({"confidence":0.1,"model_ok":true,"store_ok":true,"store_answered":true})).collect();
    fs::write(&path, json!({"rows":rows}).to_string()).unwrap();

    let (code, _, err) = run(&["bookstore-policy", "--reband", path.to_str().unwrap(), "--bands", "3"], &Deps::system());

    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["bands"].as_array().unwrap().iter().map(|band| band["n"].as_u64().unwrap()).collect::<Vec<_>>(), [4]);
}

struct CorpusEs;
impl es_client::transport::EsTransport for CorpusEs {
    fn index_name(&self) -> &str {
        "default-index"
    }
    fn call(&self, op: &str, kwargs: &Value) -> Result<Value, es_client::EsError> {
        assert_eq!(op, "search");
        assert_eq!(kwargs["index"], "chosen-index");
        Ok(json!({"hits":{"hits":[
            {"_id":"1","_source":{"category":"custom_cat","summary":"본문","file_path":"custom_cat/book.txt","file_type":"txt"}},
            {"_id":"2","_source":{"category":"3_무협","summary":"본문","file_path":"3_무협/book.txt","file_type":"txt"}},
            {"_id":"3","_source":{"category":"custom_skip","summary":"본문","file_type":"txt"}}
        ]}}))
    }
}

#[test]
fn collect_honors_index_label_pattern_and_prefixes_and_writes_corpus() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let config = classifier::classifier_config::merge_config(&classifier::classifier_config::default_config(), &json!({"corpus":{"label_pattern":"^custom_"}}));
    let deps = Deps::system().with_es(Arc::new(CorpusEs)).with_config(config);
    let (code, printed, err) = run(&["collect", "--out", out.to_str().unwrap(), "--index", "chosen-index", "--min-chars", "1", "--exclude-prefix", "custom_skip", "--with-publisher"], &deps);
    assert_eq!(code, 0, "{err}");
    let docs = classifier::corpus_collector::read_jsonl(&out).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["cat"], "custom_cat");
    assert!(printed.contains("제외 2건") && printed.contains("publisher:"));
}

#[test]
fn collect_supports_python_lookahead_label_patterns() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let config = classifier::classifier_config::merge_config(&classifier::classifier_config::default_config(), &json!({"corpus":{"label_pattern":"^[1-9]_(?=.*무협)"}}));
    let deps = Deps::system().with_es(Arc::new(CorpusEs)).with_config(config);

    let (code, _, err) = run(&["collect", "--out", out.to_str().unwrap(), "--index", "chosen-index", "--min-chars", "1", "--exclude-prefix", "custom_skip"], &deps);

    assert_eq!(code, 0, "Python `re` accepts configured lookahead patterns: {err}");
    let docs = classifier::corpus_collector::read_jsonl(&out).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["cat"], "3_무협");
}

struct StreamingCollectEs {
    output: std::path::PathBuf,
}

impl es_client::transport::EsTransport for StreamingCollectEs {
    fn index_name(&self) -> &str {
        "streaming"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, es_client::EsError> {
        match op {
            "search" => Ok(json!({"_scroll_id":"page-1", "hits":{"hits":[
                {"_id":"1", "_source":{"category":"2_소설한국", "summary":"본문", "file_path":"2_소설한국/a.txt", "file_type":"txt"}}
            ]}})),
            "scroll" => {
                let rows = fs::read_to_string(&self.output).expect("다음 페이지 전에 corpus 파일이 있어야 한다");
                assert_eq!(rows.lines().count(), 1, "현재 ES 페이지는 다음 요청 전에 저장되어야 한다");
                Ok(json!({"hits":{"hits":[]}}))
            }
            "clear_scroll" => Ok(json!({"succeeded":true})),
            other => panic!("unexpected ES operation: {other}"),
        }
    }
}

#[test]
fn collect_streams_jsonl_instead_of_retaining_all_documents() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let deps = Deps::system().with_es(Arc::new(StreamingCollectEs { output: out.clone() }));

    let (code, _, err) = run(&["collect", "--out", out.to_str().unwrap(), "--min-chars", "1"], &deps);

    assert_eq!(code, 0, "{err}");
    assert_eq!(classifier::corpus_collector::read_jsonl(&out).unwrap().len(), 1);
}

struct PublisherCollectEs;

impl es_client::transport::EsTransport for PublisherCollectEs {
    fn index_name(&self) -> &str {
        "publisher-collect"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, es_client::EsError> {
        assert_eq!(op, "search");
        Ok(json!({"hits":{"hits":[
            {"_id":"epub-1", "_source":{"category":"3_무협", "summary":"본문", "file_path":"3_무협/missing.epub", "file_type":"epub"}}
        ]}}))
    }
}

#[test]
fn collect_with_publisher_saves_and_reuses_the_adjacent_cache() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let cache = dir.path().join("corpus.jsonl.publisher.json");
    let deps = Deps::system().with_es(Arc::new(PublisherCollectEs));
    let args = ["collect", "--out", out.to_str().unwrap(), "--library-root", dir.path().to_str().unwrap(), "--min-chars", "1", "--publisher-workers", "1", "--with-publisher"];

    let (code, first, err) = run(&args, &deps);
    assert_eq!(code, 0, "{err}");
    assert!(first.contains("publisher: 새로 읽음 1건"), "{first}");
    assert!(cache.exists());
    assert_eq!(classifier::corpus_collector::read_jsonl(&out).unwrap()[0]["publisher"], "");

    let (code, second, err) = run(&args, &deps);
    assert_eq!(code, 0, "{err}");
    assert!(second.contains("publisher: 새로 읽음 0건"), "{second}");
    assert_eq!(classifier::corpus_collector::read_jsonl(&out).unwrap()[0]["publisher"], "");
}

struct TiedCategoryEs;

impl es_client::transport::EsTransport for TiedCategoryEs {
    fn index_name(&self) -> &str {
        "tied-categories"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, es_client::EsError> {
        assert_eq!(op, "search");
        Ok(json!({"hits":{"hits":[
            {"_id":"1", "_source":{"category":"2_나중", "summary":"본문", "file_path":"2_나중/a.txt", "file_type":"txt"}},
            {"_id":"2", "_source":{"category":"2_먼저", "summary":"본문", "file_path":"2_먼저/a.txt", "file_type":"txt"}}
        ]}}))
    }
}

#[test]
fn collect_keeps_first_seen_order_for_equal_category_counts() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let deps = Deps::system().with_es(Arc::new(TiedCategoryEs));

    let (code, printed, err) = run(&["collect", "--out", out.to_str().unwrap(), "--min-chars", "1"], &deps);

    assert_eq!(code, 0, "{err}");
    let first = printed.find("2_나중").unwrap();
    let second = printed.find("2_먼저").unwrap();
    assert!(first < second, "동률이면 처음 나온 카테고리를 먼저 출력해야 한다: {printed}");
}

struct ManyCategoriesEs;

impl es_client::transport::EsTransport for ManyCategoriesEs {
    fn index_name(&self) -> &str {
        "many-categories"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, es_client::EsError> {
        assert_eq!(op, "search");
        let hits: Vec<_> = (0..12)
            .map(|n| {
                let category = format!("3_카테고리{n:02}");
                json!({"_id": n.to_string(), "_source": {"category": category, "summary": "본문", "title": "제목", "file_path": format!("{category}/book.txt"), "file_type": "txt"}})
            })
            .collect();
        Ok(json!({"hits":{"hits":hits}}))
    }
}

#[test]
fn collect_prints_bottom_ten_categories_when_there_are_many() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let deps = Deps::system().with_es(Arc::new(ManyCategoriesEs));

    let (code, printed, err) = run(&["collect", "--out", out.to_str().unwrap(), "--min-chars", "1"], &deps);

    assert_eq!(code, 0, "{err}");
    assert!(printed.contains("... 하위 10개"), "{printed}");
    assert!(printed.contains("3_카테고리11"), "{printed}");
}

#[test]
fn reclassify_rejects_parent_traversal_and_symlink_categories() {
    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
    for category in ["..", "linked"] {
        let (code, _, _) = run(&["reclassify", category, "--library-root", dir.path().to_str().unwrap(), "--model", fixture().to_str().unwrap(), "--no-es", "--apply"], &Deps::system());
        assert_eq!(code, 1);
    }
}

#[test]
fn class_weight_none_keeps_the_existing_config_value() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let mut rows = String::new();
    for (category, text) in [("3_무협", "무림 검법 강호 협객"), ("5_과학", "미분 행렬 확률 물리")] {
        for i in 0..8 {
            rows.push_str(&json!({"id":format!("{category}-{i}"),"cat":category,"text":text}).to_string());
            rows.push('\n');
        }
    }
    fs::write(&corpus, rows).unwrap();
    let cfg = classifier::classifier_config::merge_config(
        &classifier::classifier_config::default_config(),
        &json!({"corpus":{"min_per_category":2},"model":{"class_weight":"balanced","n_jobs":1},"fields":{"body_word":{"min_df":1},"body_char":{"enabled":false}}}),
    );
    let deps = Deps::system().with_config(cfg);
    let out = dir.path().join("model");
    let (code, _, err) = run(&["train", "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap(), "--class-weight", "none", "--save-config"], &deps);
    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(out.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["model"]["class_weight"], "balanced");
}

#[test]
fn last_char_ngram_option_wins() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let mut rows = String::new();
    for (category, text) in [("3_무협", "무림 검법 강호 협객"), ("5_과학", "미분 행렬 확률 물리")] {
        for i in 0..8 {
            rows.push_str(&json!({"id":format!("{category}-{i}"),"cat":category,"text":text}).to_string());
            rows.push('\n');
        }
    }
    fs::write(&corpus, rows).unwrap();
    let cfg = classifier::classifier_config::merge_config(
        &classifier::classifier_config::default_config(),
        &json!({"corpus":{"min_per_category":2},"model":{"n_jobs":1},"fields":{"body_word":{"min_df":1},"body_char":{"enabled":true,"min_df":1,"max_features":100}}}),
    );
    let deps = Deps::system().with_config(cfg);
    let out = dir.path().join("model");
    let (code, _, err) = run(&["train", "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap(), "--char-ngram", "--no-char-ngram", "--save-config"], &deps);
    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(out.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["fields"]["body_char"]["enabled"], false);
}

#[test]
fn body_feature_limits_apply_to_both_text_fields() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let mut rows = String::new();
    for (category, text) in [("3_무협", "무림 검법 강호 협객"), ("5_과학", "미분 행렬 확률 물리")] {
        for i in 0..8 {
            rows.push_str(&json!({"id":format!("{category}-{i}"),"cat":category,"text":text}).to_string());
            rows.push('\n');
        }
    }
    fs::write(&corpus, rows).unwrap();
    let cfg = classifier::classifier_config::merge_config(
        &classifier::classifier_config::default_config(),
        &json!({"corpus":{"min_per_category":2},"model":{"n_jobs":1},"fields":{"body_word":{"max_features":500,"min_df":5},"body_char":{"enabled":false,"max_features":1000,"min_df":6}}}),
    );
    let deps = Deps::system().with_config(cfg);
    let out = dir.path().join("model");
    let (code, _, err) =
        run(&["train", "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap(), "--body-max-features", "100", "--body-min-df", "2", "--no-char-ngram", "--save-config"], &deps);
    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(out.join("config.json")).unwrap()).unwrap();
    for field in ["body_word", "body_char"] {
        assert_eq!(saved["fields"][field]["max_features"], 100, "{field}");
        assert_eq!(saved["fields"][field]["min_df"], 2, "{field}");
    }
}

#[test]
fn train_warns_when_corpus_has_no_publisher() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let rows = (0..8)
        .flat_map(|i| {
            [json!({"id":format!("a-{i}"),"cat":"3_무협","text":"무림 검법 강호 협객","publisher":""}), json!({"id":format!("b-{i}"),"cat":"5_과학","text":"미분 행렬 확률 물리","publisher":""})]
        })
        .map(|row| format!("{}\n", row))
        .collect::<String>();
    fs::write(&corpus, rows).unwrap();
    let out = dir.path().join("model");

    let (code, _, err) =
        run(&["train", "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap(), "--min-per-category", "2", "--body-min-df", "1", "--n-jobs", "1", "--no-char-ngram"], &Deps::system());

    assert_eq!(code, 0, "{err}");
    assert!(err.contains("publisher"), "{err}");
}

#[test]
fn train_reports_missing_and_empty_corpus_like_python() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.jsonl");
    let out = dir.path().join("model");
    let (code, _, err) = run(&["train", "--corpus", missing.to_str().unwrap(), "--out", out.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 1);
    assert!(err.contains("학습 데이터가 없다") && err.contains("collect"), "{err}");

    let empty = dir.path().join("empty.jsonl");
    fs::write(&empty, "").unwrap();
    let (code, _, err) = run(&["train", "--corpus", empty.to_str().unwrap(), "--out", out.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 1);
    assert!(err.contains("학습 데이터가 비었다"), "{err}");
}

#[test]
fn train_accepts_n_jobs_minus_one_for_all_available_cpus() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let rows = (0..8)
        .flat_map(|i| [json!({"id":format!("a-{i}"),"cat":"3_무협","text":"무림 검법 강호 협객"}), json!({"id":format!("b-{i}"),"cat":"5_과학","text":"미분 행렬 확률 물리"})])
        .map(|row| format!("{}\n", row))
        .collect::<String>();
    fs::write(&corpus, rows).unwrap();
    let out = dir.path().join("model");
    let (code, _, err) = run(
        &["train", "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap(), "--min-per-category", "2", "--body-min-df", "1", "--n-jobs", "-1", "--no-char-ngram", "--save-config"],
        &Deps::system(),
    );
    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(out.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["model"]["n_jobs"].as_i64(), Some(-1));
}

#[test]
fn train_spill_cleanup_preserves_other_files_in_requested_directory() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let mut rows = String::new();
    for (cat, text) in [("3_무협", "무림 검법 강호 협객"), ("5_과학", "미분 행렬 확률 물리")] {
        for i in 0..8 {
            rows.push_str(&json!({"id":format!("{cat}-{i}"),"cat":cat,"text":text}).to_string());
            rows.push('\n');
        }
    }
    fs::write(&corpus, rows).unwrap();
    let spill_dir = dir.path().join("spill");
    fs::create_dir(&spill_dir).unwrap();
    let marker = spill_dir.join("keep.txt");
    fs::write(&marker, "user data").unwrap();
    let out = dir.path().join("model");

    let (code, _, err) = run(
        &[
            "train",
            "--corpus",
            corpus.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--spill-dir",
            spill_dir.to_str().unwrap(),
            "--min-per-category",
            "2",
            "--body-min-df",
            "1",
            "--no-char-ngram",
        ],
        &Deps::system(),
    );

    assert_eq!(code, 0, "{err}");
    assert_eq!(fs::read_to_string(&marker).unwrap(), "user data");
    let entries: Vec<PathBuf> = fs::read_dir(&spill_dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(entries, vec![marker]);
}

#[test]
fn reband_separate_output_preserves_input_and_invalid_rows_fail() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.json");
    let output = dir.path().join("output.json");
    let before = json!({"rows":[{"confidence":0.04,"model_ok":false,"store_ok":true,"store_answered":true}]}).to_string();
    fs::write(&input, &before).unwrap();
    assert_eq!(run(&["bookstore-policy", "--reband", input.to_str().unwrap(), "--out", output.to_str().unwrap()], &Deps::system()).0, 0);
    assert_eq!(fs::read_to_string(&input).unwrap(), before);
    for invalid in [json!({"rows":[]}), json!({"rows":[{"confidence":3.0}]}), json!({"rows":[{"confidence":0.1,"model_ok":1}]})] {
        fs::write(&input, invalid.to_string()).unwrap();
        assert_eq!(run(&["bookstore-policy", "--reband", input.to_str().unwrap()], &Deps::system()).0, 1);
    }
}

#[test]
fn parser_validates_numeric_choices_and_accepts_paths_after_separator() {
    for args in [
        vec!["train", "--C", "0"],
        vec!["train", "--class-weight", "bad"],
        vec!["train", "--holdout", "2"],
        vec!["classify", "x", "--top", "0"],
        vec!["classify", "x", "--no-es=true"],
        vec!["bookstore-policy", "--delay", "1e100"],
        vec!["bookstore-policy", "--sample-from", "bad"],
    ] {
        assert_eq!(run(&args, &Deps::system()).0, 2, "{args:?}");
    }
    assert_eq!(run(&["-v", "--help"], &Deps::system()).0, 0);
    let model = fixture();
    let (code, out, err) = run(&["classify", "--model", model.to_str().unwrap(), "--no-es", "--", "--not-an-option.txt"], &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("파일이 없다"));
}

#[test]
fn info_shows_dropped_categories_without_dumping_the_entire_list() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model");
    let mut model = classifier::inference_model::CategoryModel::load_dir(&fixture()).unwrap();
    model.meta["dropped_categories"] = json!((0..12).map(|i| format!("drop{i}")).collect::<Vec<_>>());
    model.save(Some(&path)).unwrap();
    let (code, out, err) = run(&["info", "--model", path.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("제외 카테고리 12개"));
    assert!(!out.contains("drop11"));
}

#[test]
fn reclassify_limits_skips_refused_and_reports_move_failure() {
    let dir = TempDir::new().unwrap();
    let source = dir.path().join("0_inbox");
    fs::create_dir(&source).unwrap();
    for name in ["a.txt", "b.txt"] {
        fs::write(source.join(name), "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객").unwrap();
    }
    let model = fixture();
    let common = ["reclassify", "0_inbox", "--library-root", dir.path().to_str().unwrap(), "--model", model.to_str().unwrap(), "--no-es"];
    let mut args = common.to_vec();
    args.extend(["--limit", "1", "--min-confidence", "1"]);
    let (code, out, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("대상 1건") && out.contains("판정 안 함 1건"));
    let m = classifier::inference_model::CategoryModel::load_dir(&model).unwrap();
    for cat in m.classes {
        fs::write(dir.path().join(cat), "디렉토리가 아닌 파일").unwrap();
    }
    let mut args = common.to_vec();
    args.extend(["--min-confidence", "0", "--apply"]);
    let (code, out, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "Python CLI returns success after reporting per-file move failures: {err}");
    assert!(out.contains("실패 2건"));
    assert!(source.join("a.txt").exists());
}

#[test]
fn every_command_prints_help_and_no_args_fail_like_python() {
    let deps = Deps::system();
    for command in trainer::commands() {
        let (code, out, err) = run(&[command, "--help"], &deps);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains(command));
    }
    assert_eq!(run(&[], &deps).0, 1);
    assert_eq!(run(&["-v"], &deps).0, 1);
}
