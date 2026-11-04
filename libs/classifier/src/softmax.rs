//! 결정값 → 확신도. Python `softmax_confidence` 대응.

/// 결정값을 온도로 나눠 softmax 한다. 온도가 1e-6 보다 작으면 1e-6 으로 본다.
pub fn softmax_confidence(scores: &[f64], temperature: f64) -> Vec<f64> {
    let t = temperature.max(1e-6);
    let z: Vec<f64> = scores.iter().map(|s| s / t).collect();
    let max = z.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let e: Vec<f64> = z.iter().map(|v| (v - max).exp()).collect();
    let sum: f64 = e.iter().sum();
    e.iter().map(|v| v / sum).collect()
}

/// 내림차순 순위(동점이면 클래스 번호가 작은 쪽이 앞). Python `np.argsort(-d)` 의 안정 판이다.
pub fn rank_desc(scores: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    order
}

/// Python `f"{x:.{digits}f}"`. 정확히 반으로 갈리는 값은 짝수 쪽으로 반올림한다.
pub fn fixed(x: f64, digits: usize) -> String {
    format!("{x:.digits$}")
}
