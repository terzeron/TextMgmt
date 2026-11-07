//! 학습 표본을 고른다. 카테고리마다 균등 비복원 추출(Fisher–Yates 셔플 후 앞 N개)을 한다.

use std::collections::{BTreeMap, HashSet};

use classifier::feature_hasher::fnv1a64;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

/// 홀드아웃을 떼지 않는 표본 크기 하한.
const MIN_SAMPLE_FOR_HOLDOUT: usize = 5;

/// `file_path`의 첫 조각이 카테고리다. `0_` 접두, `9_북스캔OCR`, 루트 낱개 파일은 학습하지 않는다.
pub fn category_of(file_path: &str) -> Option<&str> {
    let (category, rest) = file_path.split_once('/')?;
    if rest.is_empty() || category.starts_with("0_") || category == "9_북스캔OCR" {
        return None;
    }
    Some(category)
}

/// 카테고리별 inode 목록(오름차순).
#[derive(Debug, Default)]
pub struct Population {
    pub by_category: BTreeMap<String, Vec<i64>>,
}

impl Population {
    pub fn size_of(&self, category: &str) -> usize {
        self.by_category.get(category).map_or(0, Vec::len)
    }
}

/// 같은 inode는 처음 본 경로로 한 번만 센다.
pub fn group(rows: &[(i64, String)]) -> Population {
    let mut seen = HashSet::new();
    let mut by_category: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for (id, path) in rows {
        if let Some(category) = category_of(path)
            && seen.insert(*id)
        {
            by_category.entry(category.to_string()).or_default().push(*id);
        }
    }
    for ids in by_category.values_mut() {
        ids.sort_unstable();
    }
    Population { by_category }
}

/// `(inode, 카테고리)` 목록.
#[derive(Debug, Default)]
pub struct Sample {
    pub train: Vec<(i64, String)>,
    pub holdout: Vec<(i64, String)>,
}

fn holdout_size(sample_size: usize, ratio: f64) -> usize {
    if sample_size < MIN_SAMPLE_FOR_HOLDOUT {
        return 0;
    }
    ((sample_size as f64 * ratio).round() as usize).clamp(1, sample_size)
}

/// 카테고리마다 독립된 난수열(seed ^ 카테고리 해시)을 써서, 다른 카테고리가 늘거나 줄어도 표본이 바뀌지 않는다.
pub fn draw(pop: &Population, per_category: usize, holdout_ratio: f64, seed: u64) -> Sample {
    let mut sample = Sample::default();
    for (category, ids) in &pop.by_category {
        let mut rng = ChaCha8Rng::seed_from_u64(seed ^ fnv1a64(category.as_bytes()));
        let mut shuffled = ids.clone();
        shuffled.shuffle(&mut rng);
        shuffled.truncate(per_category);
        let held = if holdout_ratio > 0.0 { holdout_size(shuffled.len(), holdout_ratio) } else { 0 };
        let (train, holdout) = shuffled.split_at(shuffled.len() - held);
        sample.train.extend(train.iter().map(|id| (*id, category.clone())));
        sample.holdout.extend(holdout.iter().map(|id| (*id, category.clone())));
    }
    sample
}
