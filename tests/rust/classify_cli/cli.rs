use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use classify_cli::{CliIo, Deps, main_with};
use serde_json::{Value, json};
use tempfile::TempDir;

#[path = "../../../utils/classify_cli/src/shuffle.rs"]
mod shuffle;

#[path = "../../../utils/classify_cli/src/move_file.rs"]
mod move_file;

#[test]
fn move_file_preserves_content_time_and_collisions_across_filesystems() {
    let source = TempDir::new().unwrap();
    let destination = tempfile::tempdir_in("/dev/shm").unwrap();
    let from = source.path().join("book.txt");
    let to = destination.path().join("book.txt");
    fs::write(&from, "본문").unwrap();
    let time = fs::metadata(&from).unwrap().modified().unwrap();
    move_file::move_file(&from, &to).unwrap();
    assert!(!from.exists());
    assert_eq!(fs::read_to_string(&to).unwrap(), "본문");
    assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), time);
    fs::write(&from, "새 본문").unwrap();
    assert!(move_file::move_file(&from, &to).is_err());
    assert_eq!(fs::read_to_string(&from).unwrap(), "새 본문");
    assert_eq!(fs::read_to_string(&to).unwrap(), "본문");
}

#[test]
fn shuffle_matches_python_random_integer_seeds() {
    // Python 3.13 random.Random(seed).shuffle(list(range(20)))로 생성했다.
    for (seed, expected) in [
        (0, [10, 18, 16, 14, 0, 17, 11, 2, 3, 9, 5, 7, 4, 19, 6, 15, 8, 1, 13, 12]),
        (42, [19, 5, 14, 4, 9, 13, 15, 18, 6, 12, 17, 10, 1, 11, 2, 16, 7, 8, 0, 3]),
        (20260914, [7, 9, 6, 5, 1, 2, 16, 19, 12, 3, 8, 4, 10, 18, 15, 17, 0, 11, 14, 13]),
        (1099511627783, [17, 0, 10, 3, 1, 2, 9, 12, 7, 13, 5, 11, 14, 6, 16, 4, 8, 15, 18, 19]),
    ] {
        let mut values: Vec<_> = (0..20).collect();
        shuffle::shuffle(&mut values, seed);
        assert_eq!(values, expected);
    }
}

#[test]
fn random_range_matches_python_seeded_draws() {
    let mut random = shuffle::Random::new(42);
    assert_eq!(random.below(41), 40);
    assert_eq!(random.below(3), 0);
    assert_eq!(10 + random.below(99_999_990), 3_356_896);
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained")
}

fn run(args: &[&str], deps: &Deps) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn every_command_prints_help_and_no_args_fail_like_python() {
    let deps = Deps::system();
    for command in classify_cli::commands() {
        let (code, out, err) = run(&[command, "--help"], &deps);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains(command));
    }
    assert_eq!(run(&[], &deps).0, 1);
    assert_eq!(run(&["-v"], &deps).0, 1);
}

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
fn verbose_classify_emits_debug_diagnostics() {
    let dir = TempDir::new().unwrap();
    let epub = dir.path().join("broken.epub");
    fs::write(&epub, b"not a zip archive").unwrap();
    let run = |verbose: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_classify_cli"));
        if verbose {
            command.arg("-v");
        }
        command.args(["classify", "--no-es", "--model"]).arg(fixture()).arg(&epub).output().unwrap()
    };
    let quiet = run(false);
    assert!(quiet.status.success(), "{}", String::from_utf8_lossy(&quiet.stderr));
    assert!(!String::from_utf8_lossy(&quiet.stderr).contains("DEBUG"));
    let output = run(true);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("DEBUG") && stderr.contains("읽지 못했다"), "{stderr}");
}

#[test]
fn classify_reports_missing_invalid_and_incomplete_es_environment_without_connecting() {
    use std::process::{Command, Output};

    fn run_with_es(command_name: &str, url: Option<&str>, index: Option<&str>, configure_optional_auth: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_classify_cli"));
        if command_name == "classify" {
            command.args(["classify", "/no/such/file", "--model"]).arg(fixture());
        } else {
            let output = TempDir::new().unwrap();
            command.args(["collect", "--out"]).arg(output.path().join("corpus.jsonl"));
        }
        for key in ["TM_ES_URL", "TM_ES_BOOK_INDEX", "TM_ES_USER", "TM_ES_PASSWORD", "TM_ES_CA_CERT"] {
            command.env_remove(key);
        }
        if let Some(url) = url {
            command.env("TM_ES_URL", url);
        }
        if let Some(index) = index {
            command.env("TM_ES_BOOK_INDEX", index);
        }
        if configure_optional_auth {
            command.env("TM_ES_USER", "test-user").env("TM_ES_PASSWORD", "test-password").env("TM_ES_CA_CERT", "/etc/ssl/certs/ca-certificates.crt");
        }
        command.output().unwrap()
    }

    let missing_url = run_with_es("classify", None, None, false);
    assert_eq!(missing_url.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_url.stderr).contains("TM_ES_URL이 없다"));

    let invalid_url = run_with_es("classify", Some("ftp://localhost:9200"), Some("books"), false);
    assert_eq!(invalid_url.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid_url.stderr).contains("http:// 또는 https://"));

    let missing_index = run_with_es("classify", Some("http://localhost:9200"), None, false);
    assert_eq!(missing_index.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing_index.stderr).contains("TM_ES_BOOK_INDEX가 없다"));

    let configured = run_with_es("classify", Some("https://localhost:9200"), Some("books"), true);
    assert!(configured.status.success(), "{}", String::from_utf8_lossy(&configured.stderr));
    assert!(String::from_utf8_lossy(&configured.stdout).contains("/no/such/file: 파일이 없다"));

    for (url, index, error) in [(None, None, "TM_ES_URL이 없다"), (Some("ftp://localhost"), Some("books"), "http:// 또는 https://"), (Some("http://localhost"), None, "TM_ES_BOOK_INDEX가 없다")]
    {
        let output = run_with_es("collect", url, index, false);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains(error));
    }
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
fn manifest_uses_exact_membership_and_rejects_invalid_or_missing_members() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("holdout.json");
    let docs = vec![json!({"id":"a"}), json!({"id":"b"})];
    fs::write(&path, json!({"document_keys":["b"]}).to_string()).unwrap();
    assert_eq!(classify_cli::load_manifest_holdout(&docs, &path).unwrap(), vec![docs[1].clone()]);
    for payload in [json!([]), json!({"document_keys":4}), json!({"document_keys":[4]}), json!({"document_keys":["missing"]})] {
        fs::write(&path, payload.to_string()).unwrap();
        assert!(classify_cli::load_manifest_holdout(&docs, &path).is_err());
    }
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

fn model_and_corpus(dir: &TempDir, count: usize) -> (PathBuf, PathBuf) {
    let path = dir.path().join("model");
    let model = backend::classifier::model::CategoryModel::load_dir(&fixture()).unwrap();
    model.save(Some(&path)).unwrap();
    let docs: Vec<Value> =
        (0..count).map(|i| json!({"id":i.to_string(),"cat":"3_무협","text":"무림맹 장문인 내공 운기조식 화산파 검법 강호 협객","name":format!("무협_{i}.txt"),"type":"txt"})).collect();
    let corpus = dir.path().join("corpus.jsonl");
    backend::classifier::corpus::write_jsonl(&docs, &corpus).unwrap();
    fs::write(classify_cli::holdout_manifest_path(&path), json!({"document_keys":docs.iter().map(|d| d["id"].clone()).collect::<Vec<_>>()} ).to_string()).unwrap();
    (path, corpus)
}

#[test]
fn evaluate_and_calibration_use_saved_holdout_and_dry_run_preserves_model() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 240);
    let before = fs::read(model.join("model.json")).unwrap();
    let common = ["--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap()];
    let mut args = vec!["evaluate"];
    args.extend(common);
    let (code, out, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("판정률"));
    args[0] = "score-calibration";
    args.push("--dry-run");
    let (code, out, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{out} {err}");
    assert_eq!(fs::read(model.join("model.json")).unwrap(), before);
    args.pop();
    assert_eq!(run(&args, &Deps::system()).0, 0);
    let loaded = backend::classifier::model::CategoryModel::load_dir(&model).unwrap();
    assert_eq!(loaded.meta["confidence_calibration"]["n"], 240);
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
    let model = backend::classifier::model::CategoryModel::load_dir(&model_path).unwrap();
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

struct Stores;
impl classify_cli::BookstoreQuery for Stores {
    fn query(&self, _: &str, _: &str, _: &str, _: f64) -> (Value, Value, Value) {
        (json!({"mapped":"3_무협"}), json!({"mapped":"3_무협"}), json!({}))
    }
}

struct VoteStores([Value; 3]);
impl classify_cli::BookstoreQuery for VoteStores {
    fn query(&self, _: &str, _: &str, _: &str, _: f64) -> (Value, Value, Value) {
        (self.0[0].clone(), self.0[1].clone(), self.0[2].clone())
    }
}

#[test]
fn bookstore_policy_samples_and_writes_per_document_rows() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 60);
    let out = dir.path().join("policy.json");
    let deps = Deps::system().with_bookstores(Arc::new(Stores));
    let (code, _, err) = run(
        &["bookstore-policy", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap(), "--sample", "12", "--sample-from", "all", "--delay", "0", "--out", out.to_str().unwrap()],
        &deps,
    );
    assert_eq!(code, 0, "{err}");
    let saved: Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();
    assert_eq!(saved["sample"], 12);
    assert_eq!(saved["rows"].as_array().unwrap().len(), 12);
    assert!(saved["rows"].as_array().unwrap().iter().all(|r| r["store_answered"] == true));
}

#[test]
fn bookstore_policy_requires_a_majority_when_multiple_stores_answer() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 60);
    let cases = [
        ([json!({"mapped":"3_무협"}), json!({}), json!({})], Some("3_무협")),
        ([json!({"mapped":"3_무협"}), json!({"mapped":"5_과학"}), json!({})], None),
        ([json!({"mapped":"3_무협"}), json!({"mapped":"3_무협"}), json!({"mapped":"5_과학"})], Some("3_무협")),
    ];

    for (index, (answers, expected)) in cases.into_iter().enumerate() {
        let output = dir.path().join(format!("votes-{index}.json"));
        let deps = Deps::system().with_bookstores(Arc::new(VoteStores(answers)));
        let args =
            ["bookstore-policy", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap(), "--sample", "1", "--sample-from", "all", "--delay", "0", "--out", output.to_str().unwrap()];
        let (code, _, err) = run(&args, &deps);
        assert_eq!(code, 0, "{err}");
        let saved: Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
        if let Some(expected) = expected {
            assert_eq!(saved["rows"][0]["store_cat"], expected);
            assert!(saved["rows"][0]["store_answered"].as_bool().unwrap());
        } else {
            assert!(saved["rows"][0]["store_cat"].is_null());
            assert!(!saved["rows"][0]["store_answered"].as_bool().unwrap());
        }
    }
}

#[test]
fn bookstore_policy_samples_the_boundary_above_the_refused_range() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 60);
    let out = dir.path().join("boundary-policy.json");
    let deps = Deps::system().with_bookstores(Arc::new(Stores));

    let (code, printed, err) = run(
        &["bookstore-policy", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap(), "--sample", "6", "--sample-from", "boundary", "--delay", "0", "--out", out.to_str().unwrap()],
        &deps,
    );

    assert_eq!(code, 0, "{err}");
    assert!(printed.contains("거부 구간과 그 위"), "{printed}");
    let saved: Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();
    assert_eq!(saved["sample_from"], "boundary");
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
impl backend::es_manager::EsTransport for CorpusEs {
    fn index_name(&self) -> &str {
        "default-index"
    }
    fn call(&self, op: &str, kwargs: &Value) -> Result<Value, backend::es_manager::EsError> {
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
    let config = backend::classifier::config::merge_config(&backend::classifier::config::default_config(), &json!({"corpus":{"label_pattern":"^custom_"}}));
    let deps = Deps::system().with_es(Arc::new(CorpusEs)).with_config(config);
    let (code, printed, err) = run(&["collect", "--out", out.to_str().unwrap(), "--index", "chosen-index", "--min-chars", "1", "--exclude-prefix", "custom_skip", "--with-publisher"], &deps);
    assert_eq!(code, 0, "{err}");
    let docs = backend::classifier::corpus::read_jsonl(&out).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["cat"], "custom_cat");
    assert!(printed.contains("제외 2건") && printed.contains("publisher:"));
}

#[test]
fn collect_supports_python_lookahead_label_patterns() {
    let dir = TempDir::new().unwrap();
    let out = dir.path().join("corpus.jsonl");
    let config = backend::classifier::config::merge_config(&backend::classifier::config::default_config(), &json!({"corpus":{"label_pattern":"^[1-9]_(?=.*무협)"}}));
    let deps = Deps::system().with_es(Arc::new(CorpusEs)).with_config(config);

    let (code, _, err) = run(&["collect", "--out", out.to_str().unwrap(), "--index", "chosen-index", "--min-chars", "1", "--exclude-prefix", "custom_skip"], &deps);

    assert_eq!(code, 0, "Python `re` accepts configured lookahead patterns: {err}");
    let docs = backend::classifier::corpus::read_jsonl(&out).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0]["cat"], "3_무협");
}

struct StreamingCollectEs {
    output: std::path::PathBuf,
}

impl backend::es_manager::EsTransport for StreamingCollectEs {
    fn index_name(&self) -> &str {
        "streaming"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, backend::es_manager::EsError> {
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
    assert_eq!(backend::classifier::corpus::read_jsonl(&out).unwrap().len(), 1);
}

struct PublisherCollectEs;

impl backend::es_manager::EsTransport for PublisherCollectEs {
    fn index_name(&self) -> &str {
        "publisher-collect"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, backend::es_manager::EsError> {
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
    assert_eq!(backend::classifier::corpus::read_jsonl(&out).unwrap()[0]["publisher"], "");

    let (code, second, err) = run(&args, &deps);
    assert_eq!(code, 0, "{err}");
    assert!(second.contains("publisher: 새로 읽음 0건"), "{second}");
    assert_eq!(backend::classifier::corpus::read_jsonl(&out).unwrap()[0]["publisher"], "");
}

struct TiedCategoryEs;

impl backend::es_manager::EsTransport for TiedCategoryEs {
    fn index_name(&self) -> &str {
        "tied-categories"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, backend::es_manager::EsError> {
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

impl backend::es_manager::EsTransport for ManyCategoriesEs {
    fn index_name(&self) -> &str {
        "many-categories"
    }

    fn call(&self, op: &str, _: &Value) -> Result<Value, backend::es_manager::EsError> {
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
fn score_calibration_refuses_too_few_samples_without_writing() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 3);
    let before = fs::read(model.join("model.json")).unwrap();
    let (code, out, err) = run(&["score-calibration", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap()], &Deps::system());
    assert_eq!(code, 1, "{err}");
    assert!(out.contains("표본이 모자라"));
    assert_eq!(fs::read(model.join("model.json")).unwrap(), before);
}

#[test]
fn train_runs_in_rust_and_writes_model_and_holdout_manifest() {
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
    let out = dir.path().join("model with spaces;$(no-shell)");
    let out_arg = format!("--out={}", out.display());
    let (code, report, err) =
        run(&["train", "--corpus", corpus.to_str().unwrap(), &out_arg, "--min-per-category", "2", "--body-min-df", "1", "--n-jobs", "1", "--no-char-ngram", "--save-config"], &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(report.contains("Rust 모델 저장"));
    assert!(report.contains(&format!("설정 저장: {}", out.join("config.json").display())), "{report}");
    assert!(out.join("config.json").is_file());
    let model = backend::classifier::model::CategoryModel::load_dir(&out).unwrap();
    assert_eq!(model.classes.len(), 2);
    assert_eq!(model.config["model"]["n_jobs"], 1);
    let original_docs = backend::classifier::corpus::read_jsonl(&corpus).unwrap();
    let (expected_model, expected_report) = backend::classifier::training::train(&mut original_docs.clone(), &model.config, backend::classifier::training::TrainOptions::default()).unwrap();
    assert_eq!(expected_model.classes, model.classes);
    assert_eq!(model.meta["holdout"], expected_report);
    assert!(!classify_cli::load_manifest_holdout(&backend::classifier::corpus::read_jsonl(&corpus).unwrap(), &classify_cli::holdout_manifest_path(&out)).unwrap().is_empty());
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
    let cfg = backend::classifier::config::merge_config(
        &backend::classifier::config::default_config(),
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
    let cfg = backend::classifier::config::merge_config(
        &backend::classifier::config::default_config(),
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
    let cfg = backend::classifier::config::merge_config(
        &backend::classifier::config::default_config(),
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
fn executable_exit_codes_and_help_match_the_library() {
    let binary = env!("CARGO_BIN_EXE_classify_cli");
    let output = std::process::Command::new(binary).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("사용법"));
    let output = std::process::Command::new(binary).arg("unknown").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
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
fn evaluate_reconstructs_missing_manifest_and_reports_partially_missing_keys() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 20);
    let args = ["evaluate", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap()];
    fs::remove_file(classify_cli::holdout_manifest_path(&model)).unwrap();
    let (code, _, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("해시 분할"));
    fs::write(classify_cli::holdout_manifest_path(&model), json!({"document_keys":["0","missing"]}).to_string()).unwrap();
    let (code, _, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("1건"));
    fs::write(&corpus, "").unwrap();
    assert_eq!(run(&args, &Deps::system()).0, 1);
    assert_eq!(run(&["train", "--corpus", corpus.to_str().unwrap()], &Deps::system()).0, 1);
}

#[test]
fn info_shows_dropped_categories_without_dumping_the_entire_list() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("model");
    let mut model = backend::classifier::model::CategoryModel::load_dir(&fixture()).unwrap();
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
    let m = backend::classifier::model::CategoryModel::load_dir(&model).unwrap();
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
fn bookstore_policy_reports_progress_and_refuses_an_empty_sampling_band() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 120);
    let out = dir.path().join("policy.json");
    let deps = Deps::system().with_bookstores(Arc::new(Stores));
    let common = ["bookstore-policy", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap(), "--out", out.to_str().unwrap(), "--delay", "0"];
    let mut args = common.to_vec();
    args.extend(["--sample-from", "all", "--sample", "120"]);
    let (code, printed, err) = run(&args, &deps);
    assert_eq!(code, 0, "{err}");
    assert!(printed.contains("100/120"));
    let manifest = model.join("model.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["config"]["decision"]["min_confidence"] = json!(0.0);
    fs::write(manifest, value.to_string()).unwrap();
    let (code, _, err) = run(&common, &deps);
    assert_eq!(code, 1);
    assert!(err.contains("해당하는 문서가 없다"));
}
