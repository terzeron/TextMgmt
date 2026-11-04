//! 자동분류기 Rust 추론 코어 테스트: 분석기 단위, sklearn parity(fixture·실제 모델), 로더 오류.
//!
//! fixture 는 `tests/fixtures/classifier_tiny/` 이고 `python tests/test_classifier_export.py` 가 다시 쓴다.
//! 실제 모델 parity 는 `TM_CLASSIFIER_EXPORT_DIR=<내보낸 디렉토리> cargo test -p backend --test classifier_model -- --ignored` 로 돌린다.

mod common;
use classifier::classifier_config::{CONFIG_ENV, config_hash, default_config, enabled_fields, load_config, load_config_from_env, MAX_CONFIG_BYTES, merge_config, resolve_parent, save_config, series_to_parent};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use classifier::inference_model::CategoryModel;
use classifier::model_trainer::{train, TrainOptions};
use common::log_capture::LogCapture;
use tempfile::TempDir;
use crate::common::shared_fixtures::{make_docs, parity, trained};

#[test]
fn unusable_config_files_fall_back_to_the_defaults() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(load_config(d.path()), default_config(), "디렉터리");
    let big = d.path().join("big.json");
    // 상한을 넘는 올바른 JSON 객체. 크기 때문에만 거부된다.
    let mut body = br#"{"pad": ""#.to_vec();
    body.extend(vec![b'x'; MAX_CONFIG_BYTES as usize]);
    body.extend(br#""}"#);
    std::fs::write(&big, body).unwrap();
    assert_eq!(load_config(&big), default_config(), "너무 큼");
    let array = d.path().join("array.json");
    std::fs::write(&array, "[1, 2]").unwrap();
    assert_eq!(load_config(&array), default_config(), "객체 아님");
}

#[test]
fn the_environment_variable_points_at_the_config_file() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("c.json");
    std::fs::write(&p, r#"{"decision": {"temperature": 3}}"#).unwrap();
    // SAFETY: 이 테스트 바이너리에서 CONFIG_ENV를 쓰는 테스트는 이것뿐이다.
    unsafe { std::env::set_var(CONFIG_ENV, &p) };
    let cfg = load_config_from_env();
    unsafe { std::env::remove_var(CONFIG_ENV) };
    assert_eq!(cfg["decision"]["temperature"], 3);
}

#[test]
fn merging_into_a_non_object_keeps_the_base_and_truthiness_follows_python() {
    assert_eq!(merge_config(&json!(1), &json!({"a": 1})), json!(1));
    let cfg = json!({"fields": {
        "null": {"enabled": null}, "zero": {"enabled": 0}, "one": {"enabled": 1},
        "empty": {"enabled": ""}, "text": {"enabled": "y"}, "list": {"enabled": [1]}, "nolist": {"enabled": []},
        "obj": {"enabled": {"a": 1}}, "noobj": {"enabled": {}},
    }});
    let on: Vec<String> = enabled_fields(&cfg).as_object().unwrap().keys().cloned().collect();
    assert_eq!(on, ["one", "text", "list", "obj"]);
}

fn train_opts() -> TrainOptions {
    TrainOptions { accept_parent: Some(series_to_parent()), spill_dir: None, release_holdout_text: false }
}

fn spill_train_config() -> Value {
    merge_config(
        &default_config(),
        &json!({"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"min_df": 1, "max_features": 5000}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}}, "holdout": 0.25}),
    )
}

fn small_fields() -> Value {
    json!({"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": false}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}})
}

// ---------------------------------------------------------------------------
// config
// ---------------------------------------------------------------------------

#[test]
fn test_load_config_falls_back_to_defaults_when_file_is_missing_or_broken() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(load_config(&tmp.path().join("없는파일.json")), default_config());

    let broken = tmp.path().join("broken.json");
    fs::write(&broken, "{ 이건 JSON 이 아니다").unwrap();
    assert_eq!(load_config(&broken), default_config());
}

#[test]
fn test_merge_config_overrides_only_given_keys() {
    let merged = merge_config(&default_config(), &json!({"model": {"C": 4.0}}));
    assert_eq!(merged["model"]["C"], 4.0);
    // 같은 딕셔너리의 다른 값은 남아야 한다
    assert_eq!(merged["model"]["class_weight"], default_config()["model"]["class_weight"]);
    assert_eq!(merged["fields"], default_config()["fields"]);
}

#[test]
fn test_save_and_load_config_round_trip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("config.json");
    let cfg = merge_config(&default_config(), &json!({"decision": {"min_confidence": 0.77}}));
    save_config(&cfg, &path).unwrap();
    assert_eq!(load_config(&path)["decision"]["min_confidence"], 0.77);
}

#[test]
fn test_config_hash_changes_with_content() {
    let other = merge_config(&default_config(), &json!({"model": {"C": 2.0}}));
    assert_ne!(config_hash(&default_config()), config_hash(&other));
    assert_eq!(config_hash(&default_config()), config_hash(&default_config().clone()));
}

#[test]
fn test_enabled_fields_drops_disabled_blocks() {
    let cfg = merge_config(&default_config(), &json!({"fields": {"body_char": {"enabled": false}}}));
    assert!(enabled_fields(&cfg).get("body_char").is_none());
    assert!(enabled_fields(&cfg).get("body_word").is_some());
}

#[test]
fn test_resolve_parent_folds_series_into_parent_genre() {
    assert_eq!(resolve_parent(Some("2_을유세계문학전집")).as_deref(), Some("2_소설외국"));
    assert_eq!(resolve_parent(Some("3_판타지")).as_deref(), Some("3_판타지"));
    assert_eq!(resolve_parent(None), None);
    assert_eq!(resolve_parent(Some("")).as_deref(), Some(""));
}

#[test]
fn test_train_raises_when_every_category_is_too_small() {
    let config = merge_config(&default_config(), &json!({"corpus": {"min_per_category": 1000}}));
    assert!(train(&mut make_docs(3), &config, TrainOptions::default()).is_err());
}

#[test]
fn test_explicit_threshold_wins_over_the_calibrated_one() {
    let (mut model, _) = trained();
    model.config = merge_config(&model.config, &json!({"decision": {"min_confidence": 0.77}}));
    assert_eq!(model.min_confidence(), 0.77);
}

#[test]
fn test_train_with_spill_gives_the_same_model_as_without() {
    // 스필은 메모리를 아끼는 길일 뿐, 성적이 달라지면 안 된다.
    //
    // 분할이 문서 키 해시라 두 번 돌려도 같은 홀드아웃이 나온다. 그래서 같은 문서를
    // 같은 수만큼 재고, 정답률도 같아야 한다.
    //
    // 임계값까지 소수점 끝자리로 비교하지는 않는다. `LinearSVC` 는 `random_state` 가
    // 없어 dual 좌표하강의 셔플이 실행마다 달라진다. 스필 없이 두 번 돌려도 임계값
    // 끝자리는 어긋난다(실측 0.7830560 대 0.7830587). 행렬이 같은지는
    // `test_feature_space_spill_matches_in_memory_matrix` 가 값으로 못 박는다.
    let tmp = TempDir::new().unwrap();
    let config = spill_train_config();
    let (_, plain) = train(&mut make_docs(40), &config, train_opts()).unwrap();
    let (_, spilled) = train(&mut make_docs(40), &config, TrainOptions { spill_dir: Some(tmp.path().join("spill")), ..train_opts() }).unwrap();

    assert_eq!(spilled["n"], plain["n"]);
    assert_eq!(spilled["top1_accuracy"], plain["top1_accuracy"]);
    for (got, want) in spilled["curve"].as_array().unwrap().iter().zip(plain["curve"].as_array().unwrap()) {
        assert_eq!(got["coverage"], want["coverage"]);
        assert_eq!(got["precision"], want["precision"]);
        let (g, w) = (got["threshold"].as_f64().unwrap(), want["threshold"].as_f64().unwrap());
        assert!((g - w).abs() <= w.abs() * 1e-3, "threshold got={g} want={w}");
    }
    assert_eq!(spilled["at_precision_90"]["coverage"], plain["at_precision_90"]["coverage"]);
}

#[test]
fn test_train_with_spill_releases_train_text_but_keeps_the_holdout() {
    // 원문 해제가 홀드아웃까지 건드리면 평가가 조용히 빈 문서로 돌아간다.
    let tmp = TempDir::new().unwrap();
    let mut docs = make_docs(40);
    let (model, report) = train(&mut docs, &spill_train_config(), TrainOptions { spill_dir: Some(tmp.path().join("spill")), ..train_opts() }).unwrap();

    let with_text = docs.iter().filter(|d| d.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty())).count();
    assert_eq!(with_text as u64, report["n"].as_u64().unwrap(), "본문이 남은 문서 수는 홀드아웃 크기와 같아야 한다");
    assert!(docs.iter().all(|d| d.get("cat").and_then(Value::as_str).is_some_and(|c| !c.is_empty())), "레이블은 해제하면 안 된다");
    // 해제 뒤에도 홀드아웃 평가가 성립했는지 값으로 확인한다.
    assert!(report["top1_accuracy"].as_f64().unwrap() > 0.9);
    assert_eq!(model.meta["documents"].as_u64().unwrap() as usize, docs.len());
}

#[test]
fn test_train_without_spill_leaves_the_documents_untouched() {
    let mut docs = make_docs(40);
    train(&mut docs, &spill_train_config(), train_opts()).unwrap();
    assert!(docs.iter().all(|d| d.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty())));
}

#[test]
fn test_train_logs_dropped_small_categories() {
    let config = merge_config(&default_config(), &json!({"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": small_fields(), "holdout": 0.25}));
    let mut docs = make_docs(40);
    docs.push(json!({"id": "tiny-0", "cat": "9_희귀", "text": "희귀 문서", "title": "희귀", "author": "작가", "publisher": "출판사", "type": "txt", "name": "희귀.txt", "path": "/x/희귀.txt"}));
    // caplog.at_level(INFO). 프로덕션은 `log` crate만 쓴다.
    let logs = LogCapture::start(log::Level::Info);
    let (model, _) = train(&mut docs, &config, train_opts()).unwrap();
    assert!(!model.classes.iter().any(|c| c == "9_희귀"));
    assert!(logs.messages().iter().any(|m| m.contains("표본 부족으로 제외한 카테고리 1개")));
}

#[test]
fn config_hash_matches_python() {
    let p = parity();
    assert_eq!(config_hash(&default_config()), p["config_hash_default"].as_str().unwrap());
    // Python 이 학습한 모델이 meta 에 적어 둔 해시를, 내보낸 config 에서 Rust 가 다시 계산해도 같다.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/classifier_tiny/trained");
    let model = CategoryModel::load(Some(&dir)).unwrap();
    assert_eq!(config_hash(&model.config), model.meta["config_hash"].as_str().unwrap());
}
