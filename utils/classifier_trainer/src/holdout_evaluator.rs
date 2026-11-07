//! 학습에 쓰지 않은 홀드아웃으로 모델을 잰다.

use std::fmt::Write;

use classifier::feature_hasher::SparseVec;
use classifier::softmax_model::SoftmaxModel;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    pub holdout_count: usize,
    /// 보정 없는 top-1·top-2 정답률.
    pub top1: f64,
    pub top2: f64,
    /// 사전확률 보정 후 top-1 정답률(균등 홀드아웃 기준).
    pub top1_prior: f64,
    /// 보정 후 top-1을 실제 분포로 가중한 정답률. 문서 가중치는 그 카테고리의 실제 권수 / 학습 권수다.
    pub top1_prior_weighted: f64,
    /// (카테고리, 홀드아웃 수, 보정 후 top-1 정답률).
    pub per_category: Vec<(String, usize, f64)>,
    /// (임계값 t, 보정 후 top-1 확률 ≥ t인 비율, 그중 정답률). t = 0.1 … 0.9.
    pub coverage_by_threshold: Vec<(f64, f64, f64)>,
}

fn ratio(a: f64, b: f64) -> f64 {
    if b > 0.0 {
        a / b
    } else {
        0.0
    }
}

pub fn evaluate(model: &SoftmaxModel, holdout: &[(SparseVec, usize)]) -> Report {
    let categories = &model.header.categories;
    let k = categories.len();
    let weight_of: Vec<f64> = model.header.population_counts.iter().zip(&model.header.sample_counts).map(|(p, s)| ratio(*p as f64, *s as f64)).collect();

    let (mut top1, mut top2, mut prior_hits, mut weighted_hits, mut weight_total) = (0usize, 0usize, 0usize, 0.0f64, 0.0f64);
    let mut per_category = vec![(0usize, 0usize); k];
    let mut scored: Vec<(f64, bool)> = Vec::with_capacity(holdout.len());
    for (x, y) in holdout {
        let raw = model.predict_proba(x, false);
        let truth = &categories[*y];
        top1 += usize::from(raw.first().is_some_and(|(c, _)| c == truth));
        top2 += usize::from(raw.iter().take(2).any(|(c, _)| c == truth));
        let corrected = model.predict_proba(x, true);
        let (best, p) = corrected.first().map(|(c, p)| (c.as_str(), *p)).unwrap_or(("", 0.0));
        let hit = best == truth;
        prior_hits += usize::from(hit);
        weight_total += weight_of[*y];
        if hit {
            weighted_hits += weight_of[*y];
        }
        per_category[*y].0 += 1;
        per_category[*y].1 += usize::from(hit);
        scored.push((p, hit));
    }
    let n = holdout.len() as f64;
    let coverage_by_threshold = (1..=9)
        .map(|step| {
            let t = step as f64 / 10.0;
            let answered: Vec<bool> = scored.iter().filter(|(p, _)| *p >= t).map(|(_, hit)| *hit).collect();
            let correct = answered.iter().filter(|h| **h).count();
            (t, ratio(answered.len() as f64, n), ratio(correct as f64, answered.len() as f64))
        })
        .collect();
    Report {
        holdout_count: holdout.len(),
        top1: ratio(top1 as f64, n),
        top2: ratio(top2 as f64, n),
        top1_prior: ratio(prior_hits as f64, n),
        top1_prior_weighted: ratio(weighted_hits, weight_total),
        per_category: categories.iter().zip(per_category).filter(|(_, (count, _))| *count > 0).map(|(c, (count, hits))| (c.clone(), count, ratio(hits as f64, count as f64))).collect(),
        coverage_by_threshold,
    }
}

pub fn format_report(r: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "홀드아웃 {}건", r.holdout_count);
    let _ = writeln!(out, "top-1 {:.4} · top-2 {:.4}", r.top1, r.top2);
    let _ = writeln!(out, "보정 후 top-1 {:.4} · 실제 분포 가중 top-1 {:.4}", r.top1_prior, r.top1_prior_weighted);
    let _ = writeln!(out, "임계\t판정률\t정답률");
    for (t, coverage, accuracy) in &r.coverage_by_threshold {
        let _ = writeln!(out, "{t:.1}\t{coverage:.4}\t{accuracy:.4}");
    }
    let _ = writeln!(out, "카테고리\t홀드아웃\t보정 후 top-1");
    for (category, count, accuracy) in &r.per_category {
        let _ = writeln!(out, "{category}\t{count}\t{accuracy:.4}");
    }
    out
}
