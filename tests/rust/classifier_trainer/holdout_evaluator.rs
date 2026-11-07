//! 홀드아웃 평가: top-1·top-2, 보정 후 정답률, 실제 분포 가중 정답률, 임계값 표.

use classifier::feature_hasher::SparseVec;
use classifier::softmax_model::{ModelHeader, SoftmaxModel};
use classifier_trainer::holdout_evaluator::{evaluate, format_report};

/// 특징 없이 bias만으로 순위가 정해지는 모델.
fn bias_model(bias: &[f32], population: &[u64], sample: &[u64]) -> SoftmaxModel {
    let k = bias.len();
    SoftmaxModel {
        header: ModelHeader {
            version: 1,
            seed: 0,
            created_unix_secs: 0,
            hash_bits: 2,
            categories: (0..k).map(|i| ["a", "b", "c"][i].to_string()).collect(),
            population_counts: population.to_vec(),
            sample_counts: sample.to_vec(),
        },
        weights: vec![0.0; k << 2],
        bias: bias.to_vec(),
    }
}

fn rows(labels: &[usize]) -> Vec<(SparseVec, usize)> {
    labels.iter().map(|y| (Vec::new(), *y)).collect()
}

#[test]
fn top1_and_top2_counts() {
    let model = bias_model(&[3.0, 2.0, 1.0], &[1, 1, 1], &[1, 1, 1]);
    let report = evaluate(&model, &rows(&[0, 1, 2]));
    assert!((report.top1 - 1.0 / 3.0).abs() < 1e-9);
    assert!((report.top2 - 2.0 / 3.0).abs() < 1e-9);
    assert!((report.top1_prior - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(report.per_category, vec![("a".to_string(), 1, 1.0), ("b".to_string(), 1, 0.0), ("c".to_string(), 1, 0.0)]);
}

#[test]
fn weighted_accuracy_uses_population_over_sample() {
    // a: 실제 100권 중 10권 학습(가중치 10), b: 10권 중 10권(가중치 1). 모델은 늘 a를 고른다.
    let model = bias_model(&[1.0, 0.0], &[100, 10], &[10, 10]);
    let report = evaluate(&model, &rows(&[0, 1]));
    assert!((report.top1_prior - 0.5).abs() < 1e-9);
    assert!((report.top1_prior_weighted - 10.0 / 11.0).abs() < 1e-9);
}

#[test]
fn threshold_table_has_nine_rows_and_monotone_coverage() {
    let model = bias_model(&[3.0, 2.0, 1.0], &[1, 1, 1], &[1, 1, 1]);
    let report = evaluate(&model, &rows(&[0, 0, 1]));
    let table = &report.coverage_by_threshold;
    assert_eq!(table.len(), 9);
    assert!((table[0].0 - 0.1).abs() < 1e-9 && (table[8].0 - 0.9).abs() < 1e-9);
    assert!(table.windows(2).all(|w| w[0].1 >= w[1].1));
    // top-1 확률은 약 0.665라 0.6까지는 전부 답하고 0.7부터는 답하지 않는다.
    assert_eq!(table[5], (0.6, 1.0, 2.0 / 3.0));
    assert_eq!(table[6].1, 0.0);
}

#[test]
fn empty_holdout_reports_zero() {
    let model = bias_model(&[1.0, 0.0], &[1, 1], &[1, 1]);
    let report = evaluate(&model, &[]);
    assert_eq!(report.top1, 0.0);
    assert!(format_report(&report).contains("홀드아웃 0건"));
}

#[test]
fn format_report_lists_headline_numbers() {
    let model = bias_model(&[3.0, 2.0, 1.0], &[1, 1, 1], &[1, 1, 1]);
    let text = format_report(&evaluate(&model, &rows(&[0, 1, 2])));
    for needle in ["홀드아웃 3건", "top-1", "top-2", "보정 후 top-1", "실제 분포 가중 top-1", "임계", "a"] {
        assert!(text.contains(needle), "{needle} 없음:\n{text}");
    }
}
