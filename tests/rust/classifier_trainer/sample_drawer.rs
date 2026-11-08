//! 표본 추출: 카테고리 판정, 모집단 묶기, 균등 비복원 추출, 홀드아웃 분리.

use classifier_trainer::sample_drawer::{category_of, draw, group, Population};

fn rows(category: &str, ids: std::ops::Range<i64>) -> Vec<(i64, String)> {
    ids.map(|id| (id, format!("{category}/{id}.epub"))).collect()
}

#[test]
fn category_of_excludes_zero_prefix_ocr_and_root_files() {
    assert_eq!(category_of("3_판타지/a.epub"), Some("3_판타지"));
    assert_eq!(category_of("3_판타지/하위/a.epub"), Some("3_판타지"));
    assert_eq!(category_of("0_trash/a.epub"), None);
    assert_eq!(category_of("9_북스캔OCR/a.pdf"), None);
    assert_eq!(category_of("a.epub"), None);
}

#[test]
fn group_dedupes_inode_and_skips_excluded() {
    let mut input = rows("A", 1..4);
    input.push((1, "A/dup.epub".into()));
    input.push((9, "0_html/x.html".into()));
    let pop = group(&input);
    assert_eq!(pop.by_category.len(), 1);
    assert_eq!(pop.by_category["A"], vec![1, 2, 3]);
}

#[test]
fn same_seed_same_sample_and_different_seed_differs() {
    let pop = group(&rows("A", 0..100));
    let a = draw(&pop, 10, 0.1, 42);
    let b = draw(&pop, 10, 0.1, 42);
    let c = draw(&pop, 10, 0.1, 43);
    assert_eq!(a.train, b.train);
    assert_eq!(a.holdout, b.holdout);
    assert_ne!(a.train, c.train);
}

#[test]
fn small_category_uses_all_and_large_is_capped() {
    let mut input = rows("A", 0..3);
    input.extend(rows("B", 100..200));
    let sample = draw(&group(&input), 10, 0.0, 1);
    let count = |cat: &str| sample.train.iter().chain(&sample.holdout).filter(|(_, c)| c == cat).count();
    assert_eq!(count("A"), 3);
    assert_eq!(count("B"), 10);
}

#[test]
fn holdout_rules() {
    let holdout_of = |n: i64, per: usize| draw(&group(&rows("A", 0..n)), per, 0.1, 1).holdout.len();
    assert_eq!(holdout_of(4, 1000), 0);
    assert_eq!(holdout_of(10, 1000), 1);
    assert_eq!(holdout_of(5000, 1000), 100);
}

#[test]
fn train_and_holdout_do_not_overlap() {
    let sample = draw(&group(&rows("A", 0..50)), 20, 0.25, 3);
    assert_eq!(sample.train.len() + sample.holdout.len(), 20);
    assert!(sample.holdout.iter().all(|h| !sample.train.contains(h)));
}

#[test]
fn draw_is_uniform() {
    let pop = group(&rows("A", 0..10));
    let mut hits = [0usize; 10];
    let rounds = 20_000u64;
    for seed in 0..rounds {
        for (id, _) in draw(&pop, 3, 0.0, seed).train {
            hits[id as usize] += 1;
        }
    }
    for (id, n) in hits.iter().enumerate() {
        let rate = *n as f64 / rounds as f64;
        assert!((rate - 0.3).abs() < 0.02, "id {id}: {rate}");
    }
}

#[test]
fn sample_does_not_depend_on_other_categories() {
    let alone = draw(&group(&rows("A", 0..100)), 10, 0.0, 5);
    let mut input = rows("A", 0..100);
    input.extend(rows("B", 100..300));
    let with_other = draw(&group(&input), 10, 0.0, 5);
    let only_a = |s: &classifier_trainer::sample_drawer::Sample| s.train.iter().filter(|(_, c)| c == "A").cloned().collect::<Vec<_>>();
    assert_eq!(only_a(&alone), only_a(&with_other));
}

#[test]
fn population_reports_sizes() {
    let pop: Population = group(&rows("A", 0..7));
    assert_eq!(pop.size_of("A"), 7);
    assert_eq!(pop.size_of("Z"), 0);
}
