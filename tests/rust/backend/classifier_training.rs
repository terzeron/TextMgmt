use backend::classifier::{
    config,
    corpus::{IndexedJsonl, write_jsonl},
    model::CategoryModel,
    training::{self, TrainOptions},
};
use serde_json::{Value, json};
use tempfile::TempDir;

fn training_data() -> Vec<Value> {
    let mut docs = Vec::new();
    for (category, words) in [("3_무협", "무림 검법 강호 협객"), ("5_과학", "미분 행렬 확률 물리")] {
        for i in 0..12 {
            docs.push(json!({
                "id": format!("{category}-{i}"),
                "cat": category,
                "text": format!("{words} 공통본문 {i}"),
                "name": format!("{category}_{i}.txt"),
                "type": "txt"
            }));
        }
    }
    docs
}

fn config() -> Value {
    let mut cfg = config::default_config();
    cfg["corpus"]["min_per_category"] = json!(2);
    cfg["model"]["max_iter"] = json!(100);
    cfg["model"]["n_jobs"] = json!(1);
    cfg["holdout"] = json!(0.25);
    for (_, spec) in cfg["fields"].as_object_mut().unwrap() {
        spec["enabled"] = json!(false);
    }
    cfg["fields"]["body_word"]["enabled"] = json!(true);
    cfg["fields"]["body_word"]["min_df"] = json!(1);
    cfg["fields"]["body_word"]["max_features"] = json!(1000);
    cfg
}

fn python_fixture_docs() -> Vec<Value> {
    let categories = [
        ("3_무협", "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객"),
        ("3_판타지", "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕"),
        ("5_수학과학일반", "미분 적분 함수 벡터 행렬 확률 통계 방정식"),
    ];
    let mut docs = Vec::new();
    for (category, words) in categories {
        let tokens: Vec<_> = words.split_whitespace().collect();
        for i in 0..40 {
            let k = i % tokens.len();
            let text = tokens[k..].iter().chain(&tokens[..k]).copied().collect::<Vec<_>>().repeat(3).join(" ");
            docs.push(json!({
                "id": format!("{category}-{i}"),
                "cat": category,
                "text": text,
                "title": format!("{} 이야기 {i}", tokens[k]),
                "author": format!("작가{}", i % 5),
                "publisher": "테스트출판사",
                "type": if i % 2 == 0 { "epub" } else { "txt" },
                "name": format!("{}_{i}.epub", tokens[k]),
                "path": format!("/mnt/data/text/{category}/{i}.epub"),
            }));
        }
    }
    docs
}

fn python_fixture_config() -> Value {
    config::merge_config(
        &config::default_config(),
        &json!({
            "corpus": {"min_per_category": 5},
            "model": {"n_jobs": 1},
            "fields": {
                "body_word": {"min_df": 1, "max_features": 5000},
                "body_char": {"enabled": false},
                "filename": {"min_df": 1},
                "title": {"min_df": 1},
                "author": {"min_df": 1},
                "publisher": {"min_df": 1},
                "file_type": {"min_df": 1}
            },
            "holdout": 0.25
        }),
    )
}

#[test]
fn balanced_weights_match_one_vs_rest_binary_class_frequencies() {
    let labels = [0, 0, 0, 0, 1, 1, 2];
    assert_eq!(training::one_vs_rest_sample_weights(&labels, 0, 3.0, true), [2.625, 2.625, 2.625, 2.625, 3.5, 3.5, 3.5]);
    assert_eq!(training::one_vs_rest_sample_weights(&labels, 1, 3.0, true), [2.1, 2.1, 2.1, 2.1, 5.25, 5.25, 2.1]);
    assert_eq!(training::one_vs_rest_sample_weights(&labels, 1, 3.0, false), [3.0; 7]);
}

#[test]
fn trains_and_round_trips_a_rust_model_with_holdout_metrics() {
    let mut docs = training_data();
    let cfg = config();
    let (kept, _) = training::filter_by_size(&docs, cfg["corpus"]["min_per_category"].as_u64().unwrap() as usize);
    let (_, holdout_docs) = training::split_holdout(&kept, cfg["holdout"].as_f64().unwrap(), cfg["seed"].as_u64().unwrap());
    let (model, report) = training::train(&mut docs, &cfg, TrainOptions::default()).expect("Rust training should produce a model");
    assert_eq!(model.classes, ["3_무협", "5_과학"]);
    assert_eq!(report["n"], 6);
    assert_eq!(report, training::evaluate(&model, &holdout_docs, None));
    assert!(model.n_features() > 0);
    assert!(report["top1_accuracy"].as_f64().unwrap() >= 0.9);
    assert_eq!(model.scores(&json!({"cat":"3_무협", "text":"무림 검법 강호 협객"})).iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| model.classes[i].as_str()), Some("3_무협"));
    assert_eq!(model.scores(&json!({"cat":"5_과학", "text":"미분 행렬 확률 물리"})).iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| model.classes[i].as_str()), Some("5_과학"));

    let dir = TempDir::new().unwrap();
    model.save(Some(&dir.path().join("model"))).unwrap();
    let loaded = CategoryModel::load_dir(&dir.path().join("model")).unwrap();
    assert_eq!(loaded.classes, model.classes);
    assert_eq!(loaded.n_features(), model.n_features());
}

#[test]
fn indexed_holdout_evaluation_matches_materialized_report_and_calibration() {
    let docs = training_data();
    let cfg = config();
    let (model, _) = training::train(&mut docs.clone(), &cfg, TrainOptions::default()).unwrap();
    let expected_report = training::evaluate(&model, &docs, None);
    let tmp = TempDir::new().unwrap();
    let corpus_path = tmp.path().join("holdout.jsonl");
    write_jsonl(&docs, &corpus_path).unwrap();
    let mut indexed = IndexedJsonl::open(&corpus_path).unwrap();

    let (actual_report, actual_calibration) = training::evaluate_indexed(&model, &mut indexed, &(0..docs.len()).collect::<Vec<_>>(), None).unwrap();
    let expected_calibration = {
        let confidence_and_correct: Vec<_> = docs
            .iter()
            .map(|doc| {
                let scores = model.scores(doc);
                let probabilities = backend::classifier::model::softmax_confidence(std::slice::from_ref(&scores), model.temperature());
                let best = scores.iter().enumerate().fold(0, |best, (index, score)| if *score > scores[best] { index } else { best });
                (probabilities[0][best], model.classes[best] == doc["cat"].as_str().unwrap_or(""))
            })
            .collect();
        let (confidence, correct): (Vec<_>, Vec<_>) = confidence_and_correct.into_iter().unzip();
        backend::classifier::calibration::fit_expected_accuracy(&confidence, &correct).unwrap().unwrap_or(Value::Null)
    };

    assert_eq!(actual_report, expected_report);
    assert_eq!(actual_calibration, expected_calibration);
}

#[test]
fn confidence_calibration_uses_accept_parent_as_correct_and_requires_minimum_samples() {
    let model = CategoryModel::from_training(vec!["3_무협".into(), "5_과학".into()], config::default_config(), json!({}), Vec::new(), Vec::new(), vec![1.0, 0.0]).unwrap();
    let docs: Vec<Value> = (0..200).map(|_| json!({"cat": "3_무협/하위", "text": ""})).collect();
    let parents = std::collections::HashMap::from([("3_무협/하위".to_string(), "3_무협".to_string())]);

    let calibration = training::fit_confidence_calibration(&model, &docs, Some(&parents)).unwrap().unwrap();

    assert_eq!(calibration["n"], 200);
    assert_eq!(calibration["y"], json!([1.0]), "accept_parent에 속한 예측은 맞은 표본으로 계산한다");
    assert!(training::fit_confidence_calibration(&model, &docs[..199], Some(&parents)).unwrap().is_none());
}

#[test]
fn indexed_holdout_indices_match_materialized_selection_when_keys_repeat() {
    let docs = vec![json!({"id":"same","cat":"A","text":"first"}), json!({"id":"same","cat":"A","text":"second"})];
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("duplicate-keys.jsonl");
    write_jsonl(&docs, &path).unwrap();
    let indexed = IndexedJsonl::open(&path).unwrap();
    let (kept, _) = training::filter_by_size(&docs, 2);
    let (_, expected_docs) = training::split_holdout(&kept, 0.5, 42);
    let expected: Vec<usize> = expected_docs.iter().map(|expected| docs.iter().position(|doc| doc == expected).unwrap()).collect();

    assert_eq!(training::holdout_indices_indexed(&indexed, 2, 0.5, 42), expected);
}

#[test]
fn spilled_training_releases_holdout_source_text_and_keeps_metrics() {
    let mut docs = training_data();
    let original_docs = docs.clone();
    let cfg = config();
    let held: std::collections::HashSet<String> =
        training::holdout_document_keys(&docs, cfg["corpus"]["min_per_category"].as_u64().unwrap() as usize, cfg["holdout"].as_f64().unwrap(), cfg["seed"].as_u64().unwrap()).into_iter().collect();
    let (_, expected_holdout) = training::split_holdout(&original_docs, cfg["holdout"].as_f64().unwrap(), cfg["seed"].as_u64().unwrap());
    let (expected_model, _) = training::train(&mut original_docs.clone(), &cfg, TrainOptions::default()).unwrap();
    let expected = training::evaluate(&expected_model, &expected_holdout, None);
    let spill = TempDir::new().unwrap();

    let (_, actual) = training::train(&mut docs, &cfg, TrainOptions { spill_dir: Some(spill.path().to_path_buf()), release_holdout_text: true, ..TrainOptions::default() }).unwrap();

    assert_eq!(actual, expected);
    for doc in &docs {
        if held.contains(&training::document_key(doc)) {
            assert!(doc["text"].is_null(), "holdout 원문이 RAM 문서에 남아 있음");
        }
    }
    assert_eq!(std::fs::read_dir(spill.path()).unwrap().count(), 0, "spill 파일이 남아 있음");
}

#[test]
fn parallel_category_training_matches_serial_scores() {
    let docs = training_data();
    let mut serial_docs = docs.clone();
    let mut parallel_docs = docs;
    let mut serial_config = config();
    serial_config["model"]["n_jobs"] = json!(1);
    let mut parallel_config = serial_config.clone();
    parallel_config["model"]["n_jobs"] = json!(2);
    let (serial, _) = training::train(&mut serial_docs, &serial_config, TrainOptions::default()).unwrap();
    let (parallel, _) = training::train(&mut parallel_docs, &parallel_config, TrainOptions::default()).unwrap();
    assert_eq!(serial.classes, parallel.classes);
    for sample in [json!({"text":"무림 검법 강호 협객"}), json!({"text":"미분 행렬 확률 물리"})] {
        let expected = serial.scores(&sample);
        let actual = parallel.scores(&sample);
        assert_eq!(expected.len(), actual.len());
        for (expected, actual) in expected.iter().zip(actual) {
            assert!((expected - actual).abs() < 1e-7);
        }
    }
}

#[test]
fn rust_training_matches_python_trained_fixture_report() {
    let expected: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/classifier_tiny/trained/report.json")).unwrap()).unwrap();
    let mut docs = python_fixture_docs();
    let options = TrainOptions { accept_parent: Some(config::series_to_parent()), spill_dir: None, release_holdout_text: false };
    let (model, actual) = training::train(&mut docs, &python_fixture_config(), options).unwrap();
    let expected_model = CategoryModel::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/trained")).unwrap();

    assert_eq!(actual["n"], expected["n"]);
    assert_eq!(actual["top1_accuracy"], expected["top1_accuracy"]);
    for metric in ["at_precision_90", "at_precision_95"] {
        let got = actual[metric]["threshold"].as_f64().unwrap();
        let want = expected[metric]["threshold"].as_f64().unwrap();
        assert!((got - want).abs() <= 1e-3, "{metric} threshold differs: Rust={got}, Python={want}");
    }
    let max_score_error = docs.iter().map(|doc| model.scores(doc).iter().zip(expected_model.scores(doc)).map(|(got, want)| (got - want).abs()).fold(0.0f64, f64::max)).fold(0.0f64, f64::max);
    assert!(max_score_error <= 3e-4, "decision score parity error: {max_score_error}");
}

#[test]
fn rust_training_matches_python_ninety_five_percent_fixture() {
    let expected: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/classifier_tiny/trained_95/report.json")).unwrap()).unwrap();
    let mut cfg = python_fixture_config();
    cfg["decision"]["target_accuracy"] = json!(0.95);
    let mut docs = python_fixture_docs();
    let options = TrainOptions { accept_parent: Some(config::series_to_parent()), spill_dir: None, release_holdout_text: false };
    let (model, actual) = training::train(&mut docs, &cfg, options).unwrap();
    let expected_model = CategoryModel::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/classifier_tiny/trained_95")).unwrap();

    assert_eq!(actual["n"], expected["n"]);
    assert_eq!(actual["top1_accuracy"], expected["top1_accuracy"]);
    for metric in ["at_precision_90", "at_precision_95"] {
        let got = actual[metric]["threshold"].as_f64().unwrap();
        let want = expected[metric]["threshold"].as_f64().unwrap();
        assert!((got - want).abs() <= 1e-3, "{metric} threshold differs: Rust={got}, Python={want}");
    }
    assert_eq!(model.meta["calibrated_target"], 0.95);
    assert_eq!(model.meta["calibrated_threshold"], actual["at_precision_95"]["threshold"]);
    let max_score_error = docs.iter().map(|doc| model.scores(doc).iter().zip(expected_model.scores(doc)).map(|(got, want)| (got - want).abs()).fold(0.0f64, f64::max)).fold(0.0f64, f64::max);
    assert!(max_score_error <= 3e-4, "decision score parity error: {max_score_error}");
}
