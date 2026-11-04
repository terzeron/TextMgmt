use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use trainer::{CliIo, Deps, main_with};
use serde_json::{Value, json};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained")
}

fn run(args: &[&str], deps: &Deps) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(args, &mut CliIo { out: &mut out, err: &mut err }, deps);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn manifest_uses_exact_membership_and_rejects_invalid_or_missing_members() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("holdout.json");
    let docs = vec![json!({"id":"a"}), json!({"id":"b"})];
    fs::write(&path, json!({"document_keys":["b"]}).to_string()).unwrap();
    assert_eq!(trainer::load_manifest_holdout(&docs, &path).unwrap(), vec![docs[1].clone()]);
    for payload in [json!([]), json!({"document_keys":4}), json!({"document_keys":[4]}), json!({"document_keys":["missing"]})] {
        fs::write(&path, payload.to_string()).unwrap();
        assert!(trainer::load_manifest_holdout(&docs, &path).is_err());
    }
}

fn model_and_corpus(dir: &TempDir, count: usize) -> (PathBuf, PathBuf) {
    let path = dir.path().join("model");
    let model = classifier::inference_model::CategoryModel::load_dir(&fixture()).unwrap();
    model.save(Some(&path)).unwrap();
    let docs: Vec<Value> =
        (0..count).map(|i| json!({"id":i.to_string(),"cat":"3_무협","text":"무림맹 장문인 내공 운기조식 화산파 검법 강호 협객","name":format!("무협_{i}.txt"),"type":"txt"})).collect();
    let corpus = dir.path().join("corpus.jsonl");
    classifier::corpus_collector::write_jsonl(&docs, &corpus).unwrap();
    fs::write(trainer::holdout_manifest_path(&path), json!({"document_keys":docs.iter().map(|d| d["id"].clone()).collect::<Vec<_>>()} ).to_string()).unwrap();
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
    let loaded = classifier::inference_model::CategoryModel::load_dir(&model).unwrap();
    assert_eq!(loaded.meta["confidence_calibration"]["n"], 240);
}

struct Stores;
impl trainer::BookstoreQuery for Stores {
    fn query(&self, _: &str, _: &str, _: &str, _: f64) -> (Value, Value, Value) {
        (json!({"mapped":"3_무협"}), json!({"mapped":"3_무협"}), json!({}))
    }
}

struct VoteStores([Value; 3]);
impl trainer::BookstoreQuery for VoteStores {
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
    let model = classifier::inference_model::CategoryModel::load_dir(&out).unwrap();
    assert_eq!(model.classes.len(), 2);
    assert_eq!(model.config["model"]["n_jobs"], 1);
    let original_docs = classifier::corpus_collector::read_jsonl(&corpus).unwrap();
    let (expected_model, expected_report) = classifier::model_trainer::train(&mut original_docs.clone(), &model.config, classifier::model_trainer::TrainOptions::default()).unwrap();
    assert_eq!(expected_model.classes, model.classes);
    assert_eq!(model.meta["holdout"], expected_report);
    assert!(!trainer::load_manifest_holdout(&classifier::corpus_collector::read_jsonl(&corpus).unwrap(), &trainer::holdout_manifest_path(&out)).unwrap().is_empty());
}

#[test]
fn evaluate_reconstructs_missing_manifest_and_reports_partially_missing_keys() {
    let dir = TempDir::new().unwrap();
    let (model, corpus) = model_and_corpus(&dir, 20);
    let args = ["evaluate", "--model", model.to_str().unwrap(), "--corpus", corpus.to_str().unwrap()];
    fs::remove_file(trainer::holdout_manifest_path(&model)).unwrap();
    let (code, _, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("해시 분할"));
    fs::write(trainer::holdout_manifest_path(&model), json!({"document_keys":["0","missing"]}).to_string()).unwrap();
    let (code, _, err) = run(&args, &Deps::system());
    assert_eq!(code, 0, "{err}");
    assert!(err.contains("1건"));
    fs::write(&corpus, "").unwrap();
    assert_eq!(run(&args, &Deps::system()).0, 1);
    assert_eq!(run(&["train", "--corpus", corpus.to_str().unwrap()], &Deps::system()).0, 1);
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
