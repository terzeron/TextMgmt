//! softmax 분류 모델: 확률 계산, 사전확률 보정, 파일 저장·로드.

use std::path::PathBuf;

use classifier::Classify;
use classifier::feature_hasher::{BookFields, hash_features};
use classifier::softmax_model::{ModelHeader, SoftmaxModel};
use serde_json::json;

fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("softmax_model").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn model(categories: &[&str], population: &[u64], sample: &[u64], hash_bits: u32) -> SoftmaxModel {
    let k = categories.len();
    SoftmaxModel {
        header: ModelHeader {
            version: 1,
            seed: 7,
            created_unix_secs: 1,
            hash_bits,
            categories: categories.iter().map(|c| c.to_string()).collect(),
            population_counts: population.to_vec(),
            sample_counts: sample.to_vec(),
        },
        weights: vec![0.0; k << hash_bits],
        bias: vec![0.0; k],
    }
}

#[test]
fn save_then_load_round_trips() {
    let mut m = model(&["a", "b"], &[10, 20], &[5, 5], 4);
    m.weights[3] = 1.5;
    m.weights[17] = -2.0;
    m.bias[1] = 0.25;
    let path = temp_dir("round_trip").join("model.bin");
    m.save_atomic(&path).unwrap();
    assert_eq!(SoftmaxModel::load(&path).unwrap(), m);
}

#[test]
fn load_rejects_bad_magic_and_truncated_file() {
    let dir = temp_dir("bad");
    let path = dir.join("model.bin");
    model(&["a", "b"], &[1, 1], &[1, 1], 4).save_atomic(&path).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    let truncated = dir.join("truncated.bin");
    std::fs::write(&truncated, &bytes[..bytes.len() - 4]).unwrap();
    assert!(SoftmaxModel::load(&truncated).is_err());

    let mut bad = bytes.clone();
    bad[0] = b'X';
    let bad_magic = dir.join("bad_magic.bin");
    std::fs::write(&bad_magic, &bad).unwrap();
    assert!(SoftmaxModel::load(&bad_magic).is_err());
}

#[test]
fn save_atomic_leaves_no_tmp_and_replaces_existing() {
    let dir = temp_dir("atomic");
    let path = dir.join("model.bin");
    model(&["a"], &[1], &[1], 4).save_atomic(&path).unwrap();
    let second = model(&["a", "b", "c"], &[1, 1, 1], &[1, 1, 1], 4);
    second.save_atomic(&path).unwrap();
    assert_eq!(SoftmaxModel::load(&path).unwrap().header.categories.len(), 3);
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["model.bin".to_string()]);
}

#[test]
fn probabilities_sum_to_one_and_are_sorted() {
    let mut m = model(&["a", "b", "c"], &[1, 1, 1], &[1, 1, 1], 4);
    m.bias = vec![0.1, 2.0, -1.0];
    let ranked = m.predict_proba(&vec![(1, 1.0)], false);
    let total: f64 = ranked.iter().map(|(_, p)| p).sum();
    assert!((total - 1.0).abs() < 1e-9);
    assert_eq!(ranked.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), vec!["b", "a", "c"]);
}

#[test]
fn logits_add_weights_of_present_features() {
    let mut m = model(&["a", "b"], &[1, 1], &[1, 1], 4);
    // 카테고리 major: 카테고리 b의 버킷 3은 16 + 3.
    m.weights[16 + 3] = 2.0;
    m.bias[0] = 0.5;
    assert_eq!(m.logits(&vec![(3, 0.5)]), vec![0.5, 1.0]);
}

#[test]
fn prior_correction_breaks_tie_toward_larger_population() {
    let m = model(&["small", "large"], &[10, 1000], &[10, 10], 4);
    let raw = m.predict_proba(&vec![], false);
    assert!((raw[0].1 - 0.5).abs() < 1e-9);
    let corrected = m.predict_proba(&vec![], true);
    assert_eq!(corrected[0].0, "large");
    assert!((corrected[0].1 - 100.0 / 101.0).abs() < 1e-6);
}

#[test]
fn zero_sample_category_never_ranks_first() {
    let mut m = model(&["empty", "full"], &[100, 5], &[0, 5], 4);
    m.bias[0] = 50.0;
    let ranked = m.predict_proba(&vec![], true);
    assert_eq!(ranked[0].0, "full");
    assert_eq!(ranked[1].1, 0.0);
}

#[test]
fn classify_document_reads_es_source() {
    let mut m = model(&["a", "b"], &[1, 1], &[1, 1], 10);
    let x = hash_features(&BookFields { summary: String::new(), title: "검과 마법".into(), author: String::new() }, 10);
    for (i, _) in &x {
        m.weights[(1 << 10) + *i as usize] = 1.0;
    }
    let prediction = m.classify_document(&json!({"title": "검과 마법", "summary": ""})).unwrap();
    assert_eq!(prediction.ranked.len(), 2);
    assert_eq!(prediction.ranked[0].0, "b");
}
