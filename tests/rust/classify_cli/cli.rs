use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use classify_cli::{CliIo, Deps, main_with};
use serde_json::{Value, json};
use sysdeps::{CommandError, CommandOutput, CommandRunner};
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

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained")
}

fn run(args: &[&str], deps: &Deps) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn every_command_prints_help_and_no_args_succeed() {
    let deps = Deps::system();
    for command in classify_cli::commands() {
        let (code, out, err) = run(&[command, "--help"], &deps);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains(command));
    }
    assert_eq!(run(&[], &deps).0, 0);
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

#[derive(Default)]
struct Runner {
    calls: Mutex<Vec<Vec<String>>>,
    fail: bool,
}
impl CommandRunner for Runner {
    fn run(&self, argv: &[String], _: Option<Duration>) -> Result<CommandOutput, CommandError> {
        self.calls.lock().unwrap().push(argv.to_vec());
        Ok(CommandOutput { returncode: if self.fail { 7 } else { 0 }, stdout: b"python output\n".to_vec(), stderr: vec![] })
    }
    fn which(&self, _: &str) -> Option<PathBuf> {
        None
    }
}

#[test]
fn training_failure_preserves_status_and_does_not_export() {
    let dir = TempDir::new().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    fs::write(&corpus, "{\"cat\":\"3_무협\"}\n").unwrap();
    let runner = Arc::new(Runner { fail: true, ..Runner::default() });
    let deps = Deps::system().with_runner(runner.clone());
    let (code, out, _) = run(&["train", "--corpus", corpus.to_str().unwrap(), "--out", dir.path().join("export").to_str().unwrap()], &deps);
    assert_eq!(code, 7);
    assert!(out.contains("python output"));
    assert_eq!(runner.calls.lock().unwrap().len(), 1);
}

#[test]
fn manifest_uses_exact_membership_and_rejects_invalid_or_missing_members() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("holdout.json");
    let docs = vec![json!({"id":"a"}), json!({"id":"b"})];
    fs::write(&path, json!({"document_keys":["b"]}).to_string()).unwrap();
    assert_eq!(classify_cli::load_manifest_holdout(&docs, &path).unwrap(), vec![docs[1].clone()]);
    for payload in [json!({"document_keys":[4]}), json!({"document_keys":["missing"]})] {
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

struct Stores;
impl classify_cli::BookstoreQuery for Stores {
    fn query(&self, _: &str, _: &str, _: &str, _: f64) -> (Value, Value, Value) {
        (json!({"mapped":"3_무협"}), json!({"mapped":"3_무협"}), json!({}))
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

struct TrainingRunner {
    calls: Mutex<Vec<Vec<String>>>,
    export_status: i32,
}
impl CommandRunner for TrainingRunner {
    fn run(&self, argv: &[String], _: Option<Duration>) -> Result<CommandOutput, CommandError> {
        self.calls.lock().unwrap().push(argv.to_vec());
        let out = PathBuf::from(&argv[argv.iter().position(|s| s == "--out").unwrap() + 1]);
        if argv[2].contains("from utils.classify_cli") {
            fs::write(classify_cli::holdout_manifest_path(&out), json!({"document_keys":["1"]}).to_string()).unwrap();
        } else {
            if self.export_status != 0 {
                return Ok(CommandOutput { returncode: self.export_status, ..Default::default() });
            }
            backend::classifier::model::CategoryModel::load_dir(&fixture()).unwrap().save(Some(&out)).unwrap();
        }
        Ok(CommandOutput::default())
    }
    fn which(&self, _: &str) -> Option<PathBuf> {
        None
    }
}

#[test]
fn train_forwards_arguments_without_shell_and_exports_the_holdout_manifest() {
    for export_status in [0, 9] {
        let dir = TempDir::new().unwrap();
        let corpus = dir.path().join("corpus.jsonl");
        fs::write(&corpus, "{\"id\":\"1\",\"cat\":\"3_무협\"}\n").unwrap();
        let out = dir.path().join("model with spaces;$(no-shell)");
        let runner = Arc::new(TrainingRunner { calls: Mutex::new(vec![]), export_status });
        let deps = Deps::system().with_runner(runner.clone());
        let out_arg = format!("--out={}", out.display());
        let (code, _, err) = run(&["train", "--corpus", corpus.to_str().unwrap(), &out_arg, "--n-jobs", "1", "--no-char-ngram"], &deps);
        assert_eq!(code, export_status, "{err}");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].iter().any(|s| s == "--no-char-ngram"));
        assert_eq!(calls[0].last().unwrap(), &format!("{}.joblib", out.display()));
        assert_eq!(calls[1].last().unwrap(), out.to_str().unwrap());
        assert_eq!(classify_cli::holdout_manifest_path(&out).exists(), export_status == 0);
    }
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
    assert!(output.status.success());
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
    assert_eq!(code, 1, "{err}");
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
