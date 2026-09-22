#!/usr/bin/env python3
"""확신도 보정 테스트.

보정의 쓸모는 하나다. 화면에 뜨는 숫자가 '맞을 확률' 로 읽혀도 되는가.
그래서 단조성, 표본 부족, 보정 없음 세 가지를 본다.
"""

import numpy as np
import pytest

from backend.classifier.calibration import MIN_SAMPLES, expected_accuracy, fit_expected_accuracy, from_coverage_report


def _sample(n: int = 2000, seed: int = 0):
    """확신도가 높을수록 잘 맞는 가짜 홀드아웃. 실제 눈금(0.01~0.15)을 흉내 낸다."""
    rng = np.random.default_rng(seed)
    conf = rng.uniform(0.013, 0.15, n)
    # 0.013 에서 약 40%, 0.15 에서 약 99%
    prob = np.clip(0.4 + (conf - 0.013) * 4.3, 0.0, 0.99)
    correct = rng.random(n) < prob
    return conf, correct


def test_fit_returns_a_monotone_mapping():
    conf, correct = _sample()

    cal = fit_expected_accuracy(conf, correct)

    assert cal is not None
    ys = cal["y"]
    assert ys == sorted(ys)
    assert cal["n"] == len(conf)


def test_expected_accuracy_rises_with_confidence():
    """작은 확신도 차이가 정답률 차이로 읽혀야 점수 열이 쓸모 있다."""
    conf, correct = _sample()
    cal = fit_expected_accuracy(conf, correct)

    low = expected_accuracy(cal, 0.02)
    high = expected_accuracy(cal, 0.13)

    assert low is not None and high is not None
    assert low < high
    assert 0.0 <= low <= 1.0 and 0.0 <= high <= 1.0


def test_expected_accuracy_is_close_to_the_true_rate():
    """보정이 맞다면 0.076 근처의 실제 정답률과 크게 어긋나면 안 된다."""
    conf, correct = _sample()
    cal = fit_expected_accuracy(conf, correct)
    near = (conf > 0.07) & (conf < 0.082)

    predicted = expected_accuracy(cal, 0.076)

    assert predicted == pytest.approx(float(correct[near].mean()), abs=0.05)


def test_too_few_samples_produce_no_calibration():
    """200건 미만이면 계단이 표본을 암기한다. 숫자를 안 만드는 쪽이 낫다."""
    conf, correct = _sample(n=MIN_SAMPLES - 1)

    assert fit_expected_accuracy(conf, correct) is None


def test_expected_accuracy_without_calibration_is_none():
    """보정이 없는 옛 모델에서는 아무 값도 주장하지 않는다."""
    assert expected_accuracy(None, 0.076) is None
    assert expected_accuracy({}, 0.076) is None
    assert expected_accuracy({"x": [0.1], "y": [0.9]}, 0.076) is None


def test_expected_accuracy_without_confidence_is_none():
    conf, correct = _sample()
    cal = fit_expected_accuracy(conf, correct)

    assert expected_accuracy(cal, None) is None


def test_values_outside_the_measured_range_are_clipped():
    """홀드아웃에서 본 적 없는 점수는 끝값으로 자른다. 밖으로 외삽하지 않는다."""
    conf, correct = _sample()
    cal = fit_expected_accuracy(conf, correct)

    assert expected_accuracy(cal, 0.0) == pytest.approx(min(cal["y"]))
    assert expected_accuracy(cal, 0.99) == pytest.approx(max(cal["y"]))


def test_mismatched_lengths_are_rejected():
    with pytest.raises(ValueError):
        fit_expected_accuracy([0.1, 0.2], [True])


# ---------------------------------------------------------------------------
# 학습 기록(판정률-정답률 곡선)에서 만드는 눈금
# ---------------------------------------------------------------------------

# 실제 모델의 meta["holdout"] 모양. 누적 곡선이다.
REAL_HOLDOUT = {
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
}


def test_coverage_curve_becomes_a_monotone_scale():
    cal = from_coverage_report(REAL_HOLDOUT)

    assert cal is not None
    assert cal["source"] == "coverage_curve"
    assert cal["x"] == sorted(cal["x"])
    assert cal["y"] == sorted(cal["y"])


def test_coverage_curve_spreads_the_crowded_range():
    """0.05 와 0.07 이 같은 값으로 뭉개지면 눈금 구실을 못 한다."""
    cal = from_coverage_report(REAL_HOLDOUT)

    low = expected_accuracy(cal, 0.054)
    high = expected_accuracy(cal, 0.076)

    assert low is not None and high is not None
    assert high - low > 0.1


def test_coverage_curve_matches_the_recorded_cumulative_numbers():
    """누적 곡선을 차분한 값이다. 상위 50% 구간은 누적값이 곧 구간값이다."""
    cal = from_coverage_report(REAL_HOLDOUT)

    assert expected_accuracy(cal, 0.1281779488151631) == pytest.approx(0.974, abs=0.001)
    # 전체 정답률(0.865)보다 낮은 구간이 있어야 한다 - 그래야 낮은 점수를 걸러낼 수 있다.
    assert expected_accuracy(cal, 0.02) < 0.5


def test_coverage_curve_without_a_report_is_none():
    assert from_coverage_report(None) is None
    assert from_coverage_report({}) is None
    assert from_coverage_report({"curve": [{"coverage": 0.5, "precision": 0.9, "threshold": 0.1}]}) is None


def test_model_falls_back_to_the_coverage_curve():
    """isotonic 보정이 없는 옛 모델도 눈금을 갖는다. 재학습 없이."""
    from backend.classifier.model import CategoryModel

    model = CategoryModel.__new__(CategoryModel)
    model.meta = {"holdout": REAL_HOLDOUT}

    assert model.confidence_calibration["source"] == "coverage_curve"
    assert model.expected_accuracy(0.076) == pytest.approx(0.79, abs=0.02)


def test_model_prefers_the_stored_calibration_over_the_curve():
    """학습이 넣어 둔 isotonic 보정이 있으면 그것이 먼저다. 더 촘촘하다."""
    from backend.classifier.model import CategoryModel

    model = CategoryModel.__new__(CategoryModel)
    stored = {"x": [0.0, 1.0], "y": [0.5, 0.5], "source": "isotonic"}
    model.meta = {"holdout": REAL_HOLDOUT, "confidence_calibration": stored}

    assert model.confidence_calibration is stored
    assert model.expected_accuracy(0.076) == pytest.approx(0.5)
