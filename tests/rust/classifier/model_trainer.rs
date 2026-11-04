mod common;
use classifier::{classifier_config, model_trainer};
use classifier::corpus_collector::{IndexedJsonl, write_jsonl};
use classifier::inference_model::CategoryModel;
use classifier::model_trainer::{blake2b_keyed, coverage_curve, document_key, evaluate, filter_by_size, format_report, holdout_document_keys, max_coverage_at, rss_note_with, split_holdout, TrainOptions};
use serde_json::{json, Value};
use tempfile::TempDir;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use crate::common::shared_fixtures::{approx, parity, trained, vocab};

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
    let mut cfg = classifier_config::default_config();
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
    classifier_config::merge_config(
        &classifier_config::default_config(),
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
    assert_eq!(model_trainer::one_vs_rest_sample_weights(&labels, 0, 3.0, true), [2.625, 2.625, 2.625, 2.625, 3.5, 3.5, 3.5]);
    assert_eq!(model_trainer::one_vs_rest_sample_weights(&labels, 1, 3.0, true), [2.1, 2.1, 2.1, 2.1, 5.25, 5.25, 2.1]);
    assert_eq!(model_trainer::one_vs_rest_sample_weights(&labels, 1, 3.0, false), [3.0; 7]);
}

#[test]
fn trains_and_round_trips_a_rust_model_with_holdout_metrics() {
    let mut docs = training_data();
    let cfg = config();
    let (kept, _) = model_trainer::filter_by_size(&docs, cfg["corpus"]["min_per_category"].as_u64().unwrap() as usize);
    let (_, holdout_docs) = model_trainer::split_holdout(&kept, cfg["holdout"].as_f64().unwrap(), cfg["seed"].as_u64().unwrap());
    let (model, report) = model_trainer::train(&mut docs, &cfg, TrainOptions::default()).expect("Rust training should produce a model");
    assert_eq!(model.classes, ["3_무협", "5_과학"]);
    assert_eq!(report["n"], 6);
    assert_eq!(report, model_trainer::evaluate(&model, &holdout_docs, None));
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
    let (model, _) = model_trainer::train(&mut docs.clone(), &cfg, TrainOptions::default()).unwrap();
    let expected_report = model_trainer::evaluate(&model, &docs, None);
    let tmp = TempDir::new().unwrap();
    let corpus_path = tmp.path().join("holdout.jsonl");
    write_jsonl(&docs, &corpus_path).unwrap();
    let mut indexed = IndexedJsonl::open(&corpus_path).unwrap();

    let (actual_report, actual_calibration) = model_trainer::evaluate_indexed(&model, &mut indexed, &(0..docs.len()).collect::<Vec<_>>(), None).unwrap();
    let expected_calibration = {
        let confidence_and_correct: Vec<_> = docs
            .iter()
            .map(|doc| {
                let scores = model.scores(doc);
                let probabilities = classifier::inference_model::softmax_confidence(std::slice::from_ref(&scores), model.temperature());
                let best = scores.iter().enumerate().fold(0, |best, (index, score)| if *score > scores[best] { index } else { best });
                (probabilities[0][best], model.classes[best] == doc["cat"].as_str().unwrap_or(""))
            })
            .collect();
        let (confidence, correct): (Vec<_>, Vec<_>) = confidence_and_correct.into_iter().unzip();
        classifier::confidence_calibrator::fit_expected_accuracy(&confidence, &correct).unwrap().unwrap_or(Value::Null)
    };

    assert_eq!(actual_report, expected_report);
    assert_eq!(actual_calibration, expected_calibration);
}

#[test]
fn spilled_training_releases_holdout_source_text_and_keeps_metrics() {
    let mut docs = training_data();
    let original_docs = docs.clone();
    let cfg = config();
    let held: std::collections::HashSet<String> =
        model_trainer::holdout_document_keys(&docs, cfg["corpus"]["min_per_category"].as_u64().unwrap() as usize, cfg["holdout"].as_f64().unwrap(), cfg["seed"].as_u64().unwrap()).into_iter().collect();
    let (_, expected_holdout) = model_trainer::split_holdout(&original_docs, cfg["holdout"].as_f64().unwrap(), cfg["seed"].as_u64().unwrap());
    let (expected_model, _) = model_trainer::train(&mut original_docs.clone(), &cfg, TrainOptions::default()).unwrap();
    let expected = model_trainer::evaluate(&expected_model, &expected_holdout, None);
    let spill = TempDir::new().unwrap();

    let (_, actual) = model_trainer::train(&mut docs, &cfg, TrainOptions { spill_dir: Some(spill.path().to_path_buf()), release_holdout_text: true, ..TrainOptions::default() }).unwrap();

    assert_eq!(actual, expected);
    for doc in &docs {
        if held.contains(&model_trainer::document_key(doc)) {
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
    let (serial, _) = model_trainer::train(&mut serial_docs, &serial_config, TrainOptions::default()).unwrap();
    let (parallel, _) = model_trainer::train(&mut parallel_docs, &parallel_config, TrainOptions::default()).unwrap();
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
    let expected: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/classifier_tiny/trained/report.json")).unwrap()).unwrap();
    let mut docs = python_fixture_docs();
    let options = TrainOptions { accept_parent: Some(classifier_config::series_to_parent()), spill_dir: None, release_holdout_text: false };
    let (model, actual) = model_trainer::train(&mut docs, &python_fixture_config(), options).unwrap();
    let expected_model = CategoryModel::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained")).unwrap();

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
    let expected: Value = serde_json::from_slice(&std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/classifier_tiny/trained_95/report.json")).unwrap()).unwrap();
    let mut cfg = python_fixture_config();
    cfg["decision"]["target_accuracy"] = json!(0.95);
    let mut docs = python_fixture_docs();
    let options = TrainOptions { accept_parent: Some(classifier_config::series_to_parent()), spill_dir: None, release_holdout_text: false };
    let (model, actual) = model_trainer::train(&mut docs, &cfg, options).unwrap();
    let expected_model = CategoryModel::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained_95")).unwrap();

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

fn ids(docs: &[Value]) -> HashSet<String> {
    docs.iter().map(|d| d["id"].as_str().unwrap().to_string()).collect()
}

// ---------------------------------------------------------------------------
// 홀드아웃 분할 - 코퍼스가 바뀌어도 소속이 안 바뀐다
// ---------------------------------------------------------------------------

fn docs_n(n: usize, cat: &str, start: usize) -> Vec<Value> {
    (start..start + n).map(|i| json!({"id": i.to_string(), "cat": cat, "text": "x", "name": format!("{i}.txt")})).collect()
}

// ---------------------------------------------------------------------------
// training
// ---------------------------------------------------------------------------

#[test]
fn test_filter_by_size_drops_small_categories() {
    let mut docs = vec![json!({"cat": "3_무협"}); 10];
    docs.extend(vec![json!({"cat": "9_격언명언"}); 2]);
    let (kept, dropped) = filter_by_size(&docs, 5);
    assert_eq!(dropped, vec!["9_격언명언"]);
    assert_eq!(kept.len(), 10);
}

#[test]
fn test_filter_by_size_keeps_at_least_two_per_category_for_stratification() {
    let mut docs = vec![json!({"cat": "3_무협"}); 3];
    docs.push(json!({"cat": "3_판타지"}));
    let (kept, dropped) = filter_by_size(&docs, 1);
    // 층화 분할이 성립하려면 최소 2건이 있어야 한다. 1을 줘도 2로 올린다.
    assert_eq!(dropped, vec!["3_판타지"]);
    assert_eq!(kept.len(), 3);
}

#[test]
fn test_coverage_curve_precision_rises_as_coverage_falls() {
    // 확신도가 높은 쪽이 맞고 낮은 쪽이 틀리게 둔다
    let correct = [true, true, true, false, false];
    let confidence = [0.99, 0.95, 0.9, 0.5, 0.4];
    let curve = coverage_curve(&correct, &confidence, &[0.5, 1.0]);
    assert_eq!(curve[0]["precision"], 1.0);
    assert_eq!(curve[1]["precision"], 0.6);
}

#[test]
fn test_max_coverage_at_finds_the_widest_band_meeting_the_target() {
    let correct = [true, true, true, true, false];
    let confidence = [0.99, 0.95, 0.9, 0.85, 0.4];
    let best = max_coverage_at(&correct, &confidence, 0.9);
    assert!(approx(best["coverage"].as_f64().unwrap(), 0.8, 1e-6));
    assert_eq!(best["precision"], 1.0);
}

#[test]
fn test_max_coverage_at_returns_zero_when_target_is_unreachable() {
    let correct = [false, false];
    assert_eq!(max_coverage_at(&correct, &[0.9, 0.8], 0.9)["coverage"], 0.0);
}

#[test]
fn test_evaluate_accepts_parent_genre_as_correct() {
    let (model, _) = trained();
    let docs = vec![json!({"cat": "3_무협", "text": vocab("3_무협"), "title": "", "author": "", "publisher": "", "type": "epub", "name": "무림맹_1.epub"})];

    let strict = evaluate(&model, &docs, None);
    // 하위 카테고리를 상위 장르로 접으면 같은 예측이 정답이 된다
    let folded = evaluate(&model, &docs, Some(&HashMap::from([("3_무협".to_string(), "3_무협".to_string())])));
    assert_eq!(strict["n"], folded["n"]);
    assert_eq!(strict["n"], 1);
}

#[test]
fn test_format_report_shows_curve_and_both_targets() {
    let (_, report) = trained();
    let text = format_report(&report);
    assert!(text.contains("홀드아웃"));
    assert!(text.contains("정답률 90% 유지 최대 판정률"));
    assert!(text.contains("정답률 95% 유지 최대 판정률"));
}

#[test]
fn test_split_holdout_keeps_the_requested_ratio() {
    let (train, hold) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 20260914);

    assert_eq!(hold.len(), 200);
    assert_eq!(train.len(), 800);
    assert!(ids(&train).intersection(&ids(&hold)).next().is_none());
}

#[test]
fn test_split_holdout_is_stable_when_documents_disappear() {
    // 코퍼스가 줄어도 남은 문서의 소속은 그대로여야 한다.
    //
    // 예전 위치 인덱스 분할은 문서 142건이 빠지자 홀드아웃이 통째로 뒤바뀌었다.
    // 그래서 재현한 홀드아웃의 80%가 학습 문서였고 정답률이 86.5% -> 93.8% 로 떴다.
    let docs = docs_n(1000, "3_SF", 0);
    let (_, before) = split_holdout(&docs, 0.2, 20260914);
    let survivors: Vec<Value> = docs.iter().filter(|d| d["id"].as_str().unwrap().parse::<u64>().unwrap() % 7 != 0).cloned().collect();
    let (_, after) = split_holdout(&survivors, 0.2, 20260914);

    let before_ids: HashSet<String> = before.iter().filter(|d| d["id"].as_str().unwrap().parse::<u64>().unwrap() % 7 != 0).map(|d| d["id"].as_str().unwrap().to_string()).collect();
    let after_ids = ids(&after);
    // 비율을 맞추느라 경계 몇 건은 움직인다. 대부분은 그대로여야 한다.
    let kept_ratio = before_ids.intersection(&after_ids).count() as f64 / before_ids.len() as f64;
    assert!(kept_ratio > 0.95, "kept_ratio={kept_ratio}");
}

#[test]
fn test_split_holdout_is_stable_when_documents_are_added() {
    let (_, before) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 20260914);
    let (_, after) = split_holdout(&docs_n(1200, "3_SF", 0), 0.2, 20260914);

    let (before_ids, after_ids) = (ids(&before), ids(&after));
    assert!(before_ids.intersection(&after_ids).count() as f64 / before_ids.len() as f64 > 0.95);
}

#[test]
fn test_split_holdout_keeps_every_category_represented() {
    // 카테고리 크기가 78,520 대 30 까지 벌어진다. 작은 쪽이 통째로 빠지면 안 된다.
    let mut docs = docs_n(5000, "2_소설한국", 0);
    docs.extend(docs_n(30, "1_문헌서지", 90000));
    let (train, hold) = split_holdout(&docs, 0.2, 20260914);

    let cats = |v: &[Value]| v.iter().map(|d| d["cat"].as_str().unwrap().to_string()).collect::<HashSet<_>>();
    let want: HashSet<String> = ["2_소설한국", "1_문헌서지"].iter().map(|s| s.to_string()).collect();
    assert_eq!(cats(&hold), want);
    assert_eq!(cats(&train), want);
}

#[test]
fn test_split_holdout_never_empties_either_side_for_tiny_categories() {
    let (train, hold) = split_holdout(&docs_n(2, "3_SF", 0), 0.2, 20260914);

    assert_eq!(hold.len(), 1);
    assert_eq!(train.len(), 1);
}

#[test]
fn test_split_holdout_changes_with_the_seed() {
    let (_, a) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 1);
    let (_, b) = split_holdout(&docs_n(1000, "3_SF", 0), 0.2, 2);

    assert_ne!(ids(&a), ids(&b));
}

#[test]
fn test_rss_note_is_empty_when_proc_status_is_unreadable() {
    // 가정한 seam: /proc 상태 파일 경로를 인자로 받는 `rss_note_with`. 읽을 수 없는 경로를 준다. (전역 `set_proc_status_path` 대체)
    assert_eq!(rss_note_with(&PathBuf::from("/no/such/proc/status")), "");
}

#[test]
fn blake2b_matches_hashlib() {
    for case in parity()["blake2b"].as_array().unwrap() {
        let digest = blake2b_keyed(case["data"].as_str().unwrap().as_bytes(), case["key"].as_str().unwrap().as_bytes(), 8);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, case["hex"].as_str().unwrap(), "data={:?}", case["data"].as_str().unwrap().chars().take(8).collect::<String>());
    }
}

#[test]
fn split_holdout_matches_python_membership_and_order() {
    let p = parity();
    let docs: Vec<Value> = p["split"]["docs"].as_array().unwrap().clone();
    let (train, hold) = split_holdout(&docs, p["split"]["ratio"].as_f64().unwrap(), p["split"]["seed"].as_u64().unwrap());
    let ids = |v: &[Value]| v.iter().map(|d| d["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let want = |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&hold), want(&p["split"]["hold_ids"]));
    assert_eq!(ids(&train), want(&p["split"]["train_ids"]));
}

#[test]
fn holdout_manifest_keys_match_the_filtered_document_split_without_cloning_documents() {
    let mut docs = Vec::new();
    for (cat, count) in [("3_소설", 6), ("5_과학", 4), ("9_희소", 1)] {
        for index in 0..count {
            docs.push(json!({"id": format!("{cat}-{index}"), "cat": cat, "text": "large body"}));
        }
    }
    let (kept, _) = filter_by_size(&docs, 3);
    let (_, expected_holdout) = split_holdout(&kept, 0.25, 20261026);
    let expected: Vec<String> = expected_holdout.iter().map(document_key).collect();

    assert_eq!(holdout_document_keys(&docs, 3, 0.25, 20261026), expected);
}

#[test]
fn format_report_matches_python() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained");
    let report: Value = serde_json::from_slice(&fs::read(dir.join("report.json")).unwrap()).unwrap();
    assert_eq!(format_report(&report), parity()["report_text"].as_str().unwrap());
}

#[test]
fn indexed_holdout_keys_match_value_corpus_split() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("holdout.jsonl");
    let mut rows = Vec::new();
    for (category, count) in [("3_소설", 6), ("5_과학", 4), ("9_희소", 1)] {
        for index in 0..count {
            rows.push(json!({"id":format!("{category}-{index}"),"cat":category,"text":"본문"}));
        }
    }
    write_jsonl(&rows, &path).unwrap();
    let indexed = IndexedJsonl::open(&path).unwrap();

    assert_eq!(model_trainer::holdout_document_keys_indexed(&indexed, 3, 0.25, 20261026), model_trainer::holdout_document_keys(&rows, 3, 0.25, 20261026));
}
