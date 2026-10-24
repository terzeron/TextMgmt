//! 확신도 보정: 점수를 "예상 정답률" 로 바꾼다. Python `backend/classifier/calibration.py` 대응.
//!
//! 홀드아웃의 (확신도, 맞았는가)로 isotonic 회귀 계단을 만든다. 판정은 이 값을 쓰지 않는다. 표시 전용이다.
//! isotonic 구현은 sklearn `IsotonicRegression(y_min=0, y_max=1, increasing=True, out_of_bounds="clip")` 의
//! `_build_y` 와 같다(동률 x 병합, 풀링, 중복 점 제거).

use serde_json::{Value, json};

use super::ValueError;

/// 계단이 표본 하나하나를 따라가면 보정이 아니라 암기다. 최소 이만큼은 모아야 한다.
pub const MIN_SAMPLES: usize = 200;

/// 확신도에서 정답률로 가는 단조 증가 계단을 학습한다. 표본이 모자라면 `Ok(None)`.
pub fn fit_expected_accuracy(confidences: &[f64], correct: &[bool]) -> Result<Option<Value>, ValueError> {
    if confidences.len() != correct.len() {
        return Err(ValueError("확신도와 정오 배열의 길이가 다르다".into()));
    }
    if confidences.len() < MIN_SAMPLES {
        log::warn!("표본이 {}건뿐이라 확신도 보정을 만들지 않는다 (최소 {MIN_SAMPLES}건)", confidences.len());
        return Ok(None);
    }
    if confidences.iter().any(|c| !c.is_finite()) {
        return Err(ValueError("확신도에 NaN 또는 Inf 가 있다".into()));
    }
    // x, y 순으로 정렬한다(np.lexsort((y, X))).
    let mut rows: Vec<(f64, f64)> = confidences.iter().zip(correct).map(|(c, ok)| (*c, if *ok { 1.0 } else { 0.0 })).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let (xs, ys, ws) = make_unique(&rows);
    let pooled = isotonic_increasing(&ys, &ws);
    let pooled: Vec<f64> = pooled.into_iter().map(|v| v.clamp(0.0, 1.0)).collect();
    // 앞뒤 점과 y 가 모두 같은 가운데 점은 뺀다(양 끝은 남긴다).
    let n = pooled.len();
    let keep: Vec<usize> = (0..n).filter(|&i| i == 0 || i == n - 1 || pooled[i] != pooled[i - 1] || pooled[i] != pooled[i + 1]).collect();
    Ok(Some(json!({"x": keep.iter().map(|&i| xs[i]).collect::<Vec<_>>(), "y": keep.iter().map(|&i| pooled[i]).collect::<Vec<_>>(), "n": confidences.len()})))
}

/// 같은 x(차이가 1e-15 미만)를 하나로 묶고 y 는 가중 평균, 가중치는 합이다. sklearn `_make_unique`.
fn make_unique(rows: &[(f64, f64)]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let eps = 1e-15;
    let (mut xs, mut ys, mut ws) = (Vec::new(), Vec::new(), Vec::new());
    let mut current_x = rows[0].0;
    let (mut current_y, mut current_w) = (0.0, 0.0);
    for &(x, y) in rows {
        if x - current_x >= eps {
            xs.push(current_x);
            ws.push(current_w);
            ys.push(current_y / current_w);
            current_x = x;
            current_w = 1.0;
            current_y = y;
        } else {
            current_w += 1.0;
            current_y += y;
        }
    }
    xs.push(current_x);
    ws.push(current_w);
    ys.push(current_y / current_w);
    (xs, ys, ws)
}

/// 가중 PAV(인접 위반 풀링). 증가 방향이다.
fn isotonic_increasing(y: &[f64], w: &[f64]) -> Vec<f64> {
    // 블록: (가중 합, 가중치, 길이)
    let mut blocks: Vec<(f64, f64, usize)> = Vec::with_capacity(y.len());
    for (&yi, &wi) in y.iter().zip(w) {
        blocks.push((yi * wi, wi, 1));
        while blocks.len() > 1 {
            let (s2, w2, n2) = blocks[blocks.len() - 1];
            let (s1, w1, n1) = blocks[blocks.len() - 2];
            if s1 / w1 >= s2 / w2 {
                blocks.truncate(blocks.len() - 2);
                blocks.push((s1 + s2, w1 + w2, n1 + n2));
            } else {
                break;
            }
        }
    }
    blocks.into_iter().flat_map(|(s, w, n)| std::iter::repeat_n(s / w, n)).collect()
}

/// 학습 때 기록해 둔 판정률-정답률 곡선에서 눈금을 만든다. 누적 곡선의 인접 점을 차분해 구간 정답률을 얻는다.
pub fn from_coverage_report(holdout: Option<&Value>) -> Option<Value> {
    let holdout = holdout?;
    if !is_truthy(holdout) {
        return None;
    }
    let mut rows: Vec<&Value> = holdout.get("curve").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    for key in ["at_precision_90", "at_precision_95"] {
        if let Some(row) = holdout.get(key).filter(|r| r.get("coverage").is_some_and(is_truthy)) {
            rows.push(row);
        }
    }
    let mut points: Vec<(f64, f64, f64)> = rows
        .iter()
        .filter(|r| r.get("coverage").is_some_and(is_truthy))
        .filter_map(|r| Some((r["coverage"].as_f64()?, r["precision"].as_f64()?, r["threshold"].as_f64()?)))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)));
    points.dedup();
    if points.len() < 2 {
        return None;
    }

    let mut xs = vec![points[0].2];
    let mut ys = vec![points[0].1];
    let (mut prev_cov, mut prev_prec, _) = points[0];
    for &(coverage, precision, threshold) in &points[1..] {
        let width = coverage - prev_cov;
        if width <= 0.0 {
            continue;
        }
        let band = (coverage * precision - prev_cov * prev_prec) / width;
        xs.push(threshold);
        ys.push(band.clamp(0.0, 1.0));
        prev_cov = coverage;
        prev_prec = precision;
    }
    xs.reverse();
    ys.reverse();
    // 단조성을 강제한다(np.maximum.accumulate).
    let mut running = f64::NEG_INFINITY;
    let ys: Vec<f64> = ys
        .into_iter()
        .map(|v| {
            running = running.max(v);
            running
        })
        .collect();

    let (mut dx, mut dy): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for (x, y) in xs.into_iter().zip(ys) {
        if dx.last().is_some_and(|last| x <= *last) {
            continue;
        }
        dx.push(x);
        dy.push(y);
    }
    if dx.len() < 2 {
        return None;
    }
    let n = holdout.get("n").and_then(Value::as_i64).unwrap_or(0);
    Some(json!({"x": dx, "y": dy, "n": n, "source": "coverage_curve"}))
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// 보정 계단 위에서 확신도를 정답률로 읽는다. 계단 사이는 직선으로 잇고 양끝 밖은 끝값으로 자른다(`np.interp`).
/// 보정이 없거나 값을 못 읽으면 `None`.
pub fn expected_accuracy(calibration: Option<&Value>, confidence: Option<f64>) -> Option<f64> {
    let cal = calibration.filter(|c| is_truthy(c))?;
    let confidence = confidence?;
    let xs = floats(cal.get("x"))?;
    let ys = floats(cal.get("y"))?;
    if xs.len() < 2 || xs.len() != ys.len() {
        return None;
    }
    Some(interp(confidence, &xs, &ys).clamp(0.0, 1.0))
}

fn floats(v: Option<&Value>) -> Option<Vec<f64>> {
    match v {
        None | Some(Value::Null) => Some(Vec::new()),
        Some(Value::Array(a)) => a.iter().map(Value::as_f64).collect(),
        _ => None,
    }
}

/// `np.interp(x, xs, ys)`. xs 는 오름차순이다.
fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let last = xs.len() - 1;
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[last] {
        return ys[last];
    }
    let hi = xs.partition_point(|v| *v <= x);
    let lo = hi - 1;
    let slope = (ys[hi] - ys[lo]) / (xs[hi] - xs[lo]);
    slope * (x - xs[lo]) + ys[lo]
}
