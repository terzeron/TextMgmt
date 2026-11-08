//! 다중 클래스 softmax를 확률적 경사 하강으로 학습한다.
//!
//! 문서 하나씩 갱신한다. 미니배치로 기울기를 모으면 카테고리 × 해시 크기 배열을 매번 비워야 해서 느리다.
//! L2는 epoch마다 전체 가중치를 한 번 줄이는 방식으로 근사한다.

use classifier::feature_hasher::SparseVec;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

#[derive(Debug, Clone, PartialEq)]
pub struct TrainConfig {
    pub hash_bits: u32,
    pub epochs: usize,
    pub learning_rate: f32,
    pub l2: f32,
    pub seed: u64,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self { hash_bits: 17, epochs: 8, learning_rate: 0.5, l2: 1e-6, seed: 0 }
    }
}

/// 반환: (weights `K × 2^hash_bits` 카테고리 major, bias `K`).
pub fn train(rows: &[(SparseVec, usize)], categories: &[String], cfg: &TrainConfig) -> (Vec<f32>, Vec<f32>) {
    train_with_progress(rows, categories, cfg, &|_, _| {})
}

/// `on_epoch(epoch, 평균 손실)`을 epoch마다 부른다.
pub fn train_with_progress(rows: &[(SparseVec, usize)], categories: &[String], cfg: &TrainConfig, on_epoch: &dyn Fn(usize, f64)) -> (Vec<f32>, Vec<f32>) {
    let k = categories.len();
    let dim = 1usize << cfg.hash_bits;
    let mut weights = vec![0.0f32; k * dim];
    let mut bias = vec![0.0f32; k];
    let mut order: Vec<usize> = (0..rows.len()).collect();
    let mut logits = vec![0.0f32; k];

    for epoch in 0..cfg.epochs {
        order.shuffle(&mut ChaCha8Rng::seed_from_u64(cfg.seed.wrapping_add(epoch as u64)));
        let lr = cfg.learning_rate / (1.0 + epoch as f32);
        let mut loss = 0.0f64;
        for &n in &order {
            let (x, y) = &rows[n];
            for (c, z) in logits.iter_mut().enumerate() {
                let row = &weights[c * dim..(c + 1) * dim];
                *z = bias[c] + x.iter().map(|(i, v)| row[*i as usize] * v).sum::<f32>();
            }
            let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let total: f32 = logits.iter().map(|z| (z - max).exp()).sum();
            loss -= f64::from(logits[*y] - max - total.ln());
            for c in 0..k {
                let p = (logits[c] - max).exp() / total;
                let g = p - if c == *y { 1.0 } else { 0.0 };
                if g == 0.0 {
                    continue;
                }
                bias[c] -= lr * g;
                let row = &mut weights[c * dim..(c + 1) * dim];
                for (i, v) in x {
                    row[*i as usize] -= lr * g * v;
                }
            }
        }
        let decay = (1.0 - lr * cfg.l2 * rows.len() as f32).max(0.0);
        if decay < 1.0 {
            weights.iter_mut().for_each(|w| *w *= decay);
        }
        on_epoch(epoch, if rows.is_empty() { 0.0 } else { loss / rows.len() as f64 });
    }
    (weights, bias)
}
