//! softmax 학습: 분리 가능한 데이터 학습, 재현성, L2 감쇠.

use classifier::feature_hasher::SparseVec;
use classifier_trainer::model_trainer::{train, TrainConfig};

const BITS: u32 = 6;

/// 카테고리마다 고유 버킷(k)과 공통 잡음 버킷(40)을 가진 문서들.
fn toy_rows() -> Vec<(SparseVec, usize)> {
    (0..30).map(|n| (vec![(n % 3, 1.0), (40, 0.5)], (n % 3) as usize)).collect()
}

fn categories() -> Vec<String> {
    vec!["a".into(), "b".into(), "c".into()]
}

fn config(l2: f32) -> TrainConfig {
    TrainConfig { hash_bits: BITS, epochs: 10, learning_rate: 0.5, l2, seed: 1 }
}

fn argmax(weights: &[f32], bias: &[f32], x: &SparseVec) -> usize {
    let dim = 1usize << BITS;
    (0..bias.len()).map(|k| bias[k] + x.iter().map(|(i, v)| weights[k * dim + *i as usize] * v).sum::<f32>()).enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0
}

#[test]
fn default_config_matches_plan() {
    let d = TrainConfig::default();
    assert_eq!((d.hash_bits, d.epochs, d.learning_rate, d.l2, d.seed), (17, 8, 0.5, 1e-6, 0));
}

#[test]
fn learns_linearly_separable_toy_data() {
    let rows = toy_rows();
    let (weights, bias) = train(&rows, &categories(), &config(0.0));
    assert_eq!(weights.len(), 3 << BITS);
    assert!(rows.iter().all(|(x, y)| argmax(&weights, &bias, x) == *y));
}

#[test]
fn same_seed_is_deterministic() {
    let rows = toy_rows();
    assert_eq!(train(&rows, &categories(), &config(0.0)), train(&rows, &categories(), &config(0.0)));
}

#[test]
fn l2_shrinks_weights() {
    let rows = toy_rows();
    let norm = |l2: f32| train(&rows, &categories(), &config(l2)).0.iter().map(|w| w * w).sum::<f32>();
    assert!(norm(0.05) < norm(0.0));
}
