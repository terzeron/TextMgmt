//! 원본: tests/test_classifier_calibration.py
//!
//! 가정한 API:
//! - `backend::classifier::calibration::MIN_SAMPLES: usize`
//! - `backend::classifier::calibration::fit_expected_accuracy(conf: &[f64], correct: &[bool]) -> Result<Option<Value>, ValueError>`
//! - `backend::classifier::calibration::expected_accuracy(cal: Option<&Value>, conf: Option<f64>) -> Option<f64>`
//! - `backend::classifier::calibration::from_coverage_report(holdout: Option<&Value>) -> Option<Value>`
//! - `backend::classifier::model::CategoryModel::with_meta(meta: Value) -> CategoryModel` (Python `__new__` 후 meta 대입에 대응)
//! - `CategoryModel::confidence_calibration(&self) -> Option<Value>`, `CategoryModel::expected_accuracy(&self, f64) -> Option<f64>`
//! - `backend::classifier::ValueError` (Python `ValueError`)
//!
//! 확신도 보정의 쓸모는 하나다. 화면에 뜨는 숫자가 '맞을 확률'로 읽혀도 되는가.
//! 그래서 단조성, 표본 부족, 보정 없음 세 가지를 본다.

mod common;

use backend::classifier::calibration::{MIN_SAMPLES, expected_accuracy, fit_expected_accuracy, from_coverage_report};
use backend::classifier::model::CategoryModel;
use serde_json::{Value, json};

/// 시드 고정 난수. numpy 대신 splitmix64를 쓴다.
struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// 확신도가 높을수록 잘 맞는 가짜 홀드아웃. 실제 눈금(0.01~0.15)을 흉내 낸다.
fn sample(n: usize, seed: u64) -> (Vec<f64>, Vec<bool>) {
    let mut rng = Rng(seed);
    let conf: Vec<f64> = (0..n).map(|_| 0.013 + (0.15 - 0.013) * rng.next_f64()).collect();
    // 0.013 에서 약 40%, 0.15 에서 약 99%
    let correct = conf.iter().map(|c| rng.next_f64() < (0.4 + (c - 0.013) * 4.3).clamp(0.0, 0.99)).collect();
    (conf, correct)
}

fn approx(a: f64, b: f64, abs: f64) -> bool {
    (a - b).abs() <= abs
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect()
}

fn is_sorted(v: &[f64]) -> bool {
    v.windows(2).all(|w| w[0] <= w[1])
}

#[test]
fn test_fit_returns_a_monotone_mapping() {
    let (conf, correct) = sample(2000, 0);

    let cal = fit_expected_accuracy(&conf, &correct).unwrap();

    let cal = cal.expect("보정이 있어야 한다");
    assert!(is_sorted(&floats(&cal["y"])));
    assert_eq!(cal["n"].as_u64().unwrap() as usize, conf.len());
}

#[test]
fn test_expected_accuracy_rises_with_confidence() {
    // 작은 확신도 차이가 정답률 차이로 읽혀야 점수 열이 쓸모 있다.
    let (conf, correct) = sample(2000, 0);
    let cal = fit_expected_accuracy(&conf, &correct).unwrap();

    let low = expected_accuracy(cal.as_ref(), Some(0.02));
    let high = expected_accuracy(cal.as_ref(), Some(0.13));

    let (low, high) = (low.unwrap(), high.unwrap());
    assert!(low < high);
    assert!((0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high));
}

#[test]
fn test_expected_accuracy_is_close_to_the_true_rate() {
    // 보정이 맞다면 0.076 근처의 실제 정답률과 크게 어긋나면 안 된다.
    let (conf, correct) = sample(2000, 0);
    let cal = fit_expected_accuracy(&conf, &correct).unwrap();
    let near: Vec<bool> = conf.iter().zip(&correct).filter(|(c, _)| **c > 0.07 && **c < 0.082).map(|(_, ok)| *ok).collect();
    let rate = near.iter().filter(|b| **b).count() as f64 / near.len() as f64;

    let predicted = expected_accuracy(cal.as_ref(), Some(0.076)).unwrap();

    assert!(approx(predicted, rate, 0.05), "predicted={predicted} rate={rate}");
}

#[test]
fn test_too_few_samples_produce_no_calibration() {
    // 200건 미만이면 계단이 표본을 암기한다. 숫자를 안 만드는 쪽이 낫다.
    let (conf, correct) = sample(MIN_SAMPLES - 1, 0);

    assert!(fit_expected_accuracy(&conf, &correct).unwrap().is_none());
}

#[test]
fn test_expected_accuracy_without_calibration_is_none() {
    // 보정이 없는 옛 모델에서는 아무 값도 주장하지 않는다.
    assert_eq!(expected_accuracy(None, Some(0.076)), None);
    assert_eq!(expected_accuracy(Some(&json!({})), Some(0.076)), None);
    assert_eq!(expected_accuracy(Some(&json!({"x": [0.1], "y": [0.9]})), Some(0.076)), None);
}

#[test]
fn test_expected_accuracy_without_confidence_is_none() {
    let (conf, correct) = sample(2000, 0);
    let cal = fit_expected_accuracy(&conf, &correct).unwrap();

    assert_eq!(expected_accuracy(cal.as_ref(), None), None);
}

#[test]
fn test_values_outside_the_measured_range_are_clipped() {
    // 홀드아웃에서 본 적 없는 점수는 끝값으로 자른다. 밖으로 외삽하지 않는다.
    let (conf, correct) = sample(2000, 0);
    let cal = fit_expected_accuracy(&conf, &correct).unwrap().unwrap();
    let ys = floats(&cal["y"]);
    let (min, max) = (ys.iter().cloned().fold(f64::INFINITY, f64::min), ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max));

    assert!(approx(expected_accuracy(Some(&cal), Some(0.0)).unwrap(), min, 1e-6));
    assert!(approx(expected_accuracy(Some(&cal), Some(0.99)).unwrap(), max, 1e-6));
}

#[test]
fn test_mismatched_lengths_are_rejected() {
    assert!(fit_expected_accuracy(&[0.1, 0.2], &[true]).is_err());
}

// ---------------------------------------------------------------------------
// 학습 기록(판정률-정답률 곡선)에서 만드는 눈금
// ---------------------------------------------------------------------------

/// 실제 모델의 meta["holdout"] 모양. 누적 곡선이다.
fn real_holdout() -> Value {
    json!({
        "n": 47008,
        "top1_accuracy": 0.864788972089857,
        "curve": [
            {"coverage": 0.5, "precision": 0.974004424778761, "threshold": 0.1281779488151631},
            {"coverage": 0.7, "precision": 0.942136453426531, "threshold": 0.10072196828305498},
            {"coverage": 0.8, "precision": 0.9265010902515556, "threshold": 0.08265990361070291},
            {"coverage": 0.9, "precision": 0.9044602548041695, "threshold": 0.060971974205370344},
            {"coverage": 0.95, "precision": 0.8889759724119399, "threshold": 0.04635731124736481},
            {"coverage": 1.0, "precision": 0.864788972089857, "threshold": 0.019518310797334633},
        ],
        "at_precision_90": {"coverage": 0.917290673927842, "precision": 0.9, "threshold": 0.056374553813004144},
        "at_precision_95": {"coverage": 0.6435287610619469, "precision": 0.9500181812171499, "threshold": 0.10885944805582125},
    })
}

#[test]
fn test_coverage_curve_becomes_a_monotone_scale() {
    let cal = from_coverage_report(Some(&real_holdout())).expect("눈금이 있어야 한다");

    assert_eq!(cal["source"], "coverage_curve");
    assert!(is_sorted(&floats(&cal["x"])));
    assert!(is_sorted(&floats(&cal["y"])));
}

#[test]
fn test_coverage_curve_spreads_the_crowded_range() {
    // 0.05 와 0.07 이 같은 값으로 뭉개지면 눈금 구실을 못 한다.
    let cal = from_coverage_report(Some(&real_holdout()));

    let low = expected_accuracy(cal.as_ref(), Some(0.054)).unwrap();
    let high = expected_accuracy(cal.as_ref(), Some(0.076)).unwrap();

    assert!(high - low > 0.1, "low={low} high={high}");
}

#[test]
fn test_coverage_curve_matches_the_recorded_cumulative_numbers() {
    // 누적 곡선을 차분한 값이다. 상위 50% 구간은 누적값이 곧 구간값이다.
    let cal = from_coverage_report(Some(&real_holdout()));

    assert!(approx(expected_accuracy(cal.as_ref(), Some(0.1281779488151631)).unwrap(), 0.974, 0.001));
    // 전체 정답률(0.865)보다 낮은 구간이 있어야 한다 - 그래야 낮은 점수를 걸러낼 수 있다.
    assert!(expected_accuracy(cal.as_ref(), Some(0.02)).unwrap() < 0.5);
}

#[test]
fn test_coverage_curve_without_a_report_is_none() {
    assert!(from_coverage_report(None).is_none());
    assert!(from_coverage_report(Some(&json!({}))).is_none());
    assert!(from_coverage_report(Some(&json!({"curve": [{"coverage": 0.5, "precision": 0.9, "threshold": 0.1}]}))).is_none());
}

#[test]
fn test_model_falls_back_to_the_coverage_curve() {
    // isotonic 보정이 없는 옛 모델도 눈금을 갖는다. 재학습 없이.
    let model = CategoryModel::with_meta(json!({"holdout": real_holdout()}));

    assert_eq!(model.confidence_calibration().unwrap()["source"], "coverage_curve");
    assert!(approx(model.expected_accuracy(0.076).unwrap(), 0.79, 0.02));
}

#[test]
fn test_model_prefers_the_stored_calibration_over_the_curve() {
    // 학습이 넣어 둔 isotonic 보정이 있으면 그것이 먼저다. 더 촘촘하다.
    let stored = json!({"x": [0.0, 1.0], "y": [0.5, 0.5], "source": "isotonic"});
    let model = CategoryModel::with_meta(json!({"holdout": real_holdout(), "confidence_calibration": stored}));

    // Python은 `is`로 같은 객체인지 본다. Rust는 값이 같은지로 대신한다.
    assert_eq!(model.confidence_calibration().unwrap(), stored);
    assert!(approx(model.expected_accuracy(0.076).unwrap(), 0.5, 1e-6));
}

// ---------------------------------------------------------------------------
// 곡선이 모자라거나 겹칠 때
// ---------------------------------------------------------------------------

#[test]
fn test_coverage_curve_with_a_single_point_is_none() {
    let holdout = json!({"n": 10, "curve": [{"coverage": 0.5, "precision": 0.9, "threshold": 0.1}]});
    assert!(from_coverage_report(Some(&holdout)).is_none());
}

#[test]
fn test_coverage_curve_skips_points_that_do_not_widen_coverage() {
    // 판정률이 같은 점이 둘이면 정렬상 뒤쪽 점은 차분할 폭이 0 이라 건너뛴다.
    let holdout = json!({
        "n": 10,
        "curve": [
            {"coverage": 0.5, "precision": 0.95, "threshold": 0.12},
            {"coverage": 0.5, "precision": 0.94, "threshold": 0.11},
            {"coverage": 1.0, "precision": 0.9, "threshold": 0.02},
        ],
    });
    let cal = from_coverage_report(Some(&holdout)).expect("눈금이 있어야 한다");
    assert_eq!(cal["x"], json!([0.02, 0.11]));
}

#[test]
fn test_coverage_curve_with_one_distinct_threshold_is_none() {
    // 임계값이 모두 같으면 눈금의 x 가 한 점으로 접힌다.
    let holdout = json!({
        "n": 10,
        "curve": [
            {"coverage": 0.5, "precision": 0.95, "threshold": 0.1},
            {"coverage": 1.0, "precision": 0.9, "threshold": 0.1},
        ],
    });
    assert!(from_coverage_report(Some(&holdout)).is_none());
}

#[test]
fn test_expected_accuracy_with_unreadable_values_is_none() {
    assert_eq!(expected_accuracy(Some(&json!({"x": [0.1, "high"], "y": [0.5, 0.9]})), Some(0.2)), None);
}
