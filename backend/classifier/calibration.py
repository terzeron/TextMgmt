#!/usr/bin/env python3
"""
확신도 보정 - 점수를 '예상 정답률' 로 바꾼다

확신도는 79개 클래스에 softmax 를 씌운 값이라 눈금이 사람 기준과 안 맞는다.
바닥이 1/79 = 0.013 이고, 판정 임계값이 0.056, 실제로 보이는 위쪽이 0.13 쯤이다.
화면에 0.05 와 0.07 이 나란히 있으면 어느 쪽이 얼마나 나은지 읽을 수가 없다.

그래서 홀드아웃에서 (확신도 -> 맞았는가) 를 isotonic 회귀로 학습해 둔다.
확신도가 높을수록 정답률도 높다는 것 하나만 가정하고, 그 안에서 데이터가 말하는
대로 계단을 그린다. 그 계단을 모델 파일에 함께 넣으면, 판정할 때 0.076 을
"이 정도 점수면 대체로 0.93" 으로 옮길 수 있다.

판정 자체는 이 값을 안 쓴다. 임계값(`min_confidence`)과 서점 경계는 원래 확신도
눈금 위에 있고, 눈금을 바꾸면 둘 다 다시 재야 한다. 여기는 표시 전용이다.
"""

import logging
from typing import Any, Dict, List, Optional, Sequence

import numpy as np

logger = logging.getLogger(__name__)

# 계단이 표본 하나하나를 따라가면 보정이 아니라 암기다. 최소 이만큼은 모아야 한다.
MIN_SAMPLES = 200


def fit_expected_accuracy(confidences: Sequence[float], correct: Sequence[bool]) -> Optional[Dict[str, Any]]:
    """확신도에서 정답률로 가는 단조 증가 계단을 학습한다.

    표본이 모자라면 None 을 돌려준다. 근거 없는 숫자를 화면에 띄우느니 안 띄우는 편이 낫다.
    """
    from sklearn.isotonic import IsotonicRegression

    x = np.asarray(confidences, dtype=float)
    y = np.asarray(correct, dtype=float)
    if len(x) != len(y):
        raise ValueError("확신도와 정오 배열의 길이가 다르다")
    if len(x) < MIN_SAMPLES:
        logger.warning("표본이 %d건뿐이라 확신도 보정을 만들지 않는다 (최소 %d건)", len(x), MIN_SAMPLES)
        return None

    iso = IsotonicRegression(y_min=0.0, y_max=1.0, increasing=True, out_of_bounds="clip")
    iso.fit(x, y)
    return {"x": [float(v) for v in iso.X_thresholds_], "y": [float(v) for v in iso.y_thresholds_], "n": int(len(x))}


def from_coverage_report(holdout: Optional[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
    """학습 때 기록해 둔 판정률-정답률 곡선에서 눈금을 만든다.

    isotonic 보정(`fit_expected_accuracy`)이 없는 모델을 위한 길이다. 보정을 다시
    재려면 학습 때와 똑같은 홀드아웃이 필요한데, 코퍼스는 계속 바뀌어서 나중에는
    그 집합을 되살릴 수 없다. 억지로 되살리면 학습에 쓴 문서가 섞여 정답률이
    부풀려진다(실측: 86.5% 가 93.8% 로).

    반면 `meta["holdout"]` 의 곡선은 학습 시점의 진짜 홀드아웃에서 잰 값이다.
    그 곡선은 누적이다 - "이 점수 이상인 것들의 정답률". 인접한 두 점을 차분하면
    구간 자체의 정답률이 나온다.

        상위 50%   정답률 0.974          -> 확신도 0.1282 이상 구간의 정답률 0.974
        상위 64.4% 정답률 0.950 (누적)   -> 0.1089~0.1282 구간의 정답률 0.866

    구간 정답률은 확신도가 오를수록 함께 오른다. 그래서 그대로 눈금이 된다.
    """
    if not holdout:
        return None
    rows = list(holdout.get("curve") or [])
    for key in ("at_precision_90", "at_precision_95"):
        row = holdout.get(key)
        if row and row.get("coverage"):
            rows.append(row)

    points = sorted({(float(r["coverage"]), float(r["precision"]), float(r["threshold"])) for r in rows if r.get("coverage")})
    if len(points) < 2:
        return None

    # 가장 높은 임계값 구간은 누적값이 곧 구간값이다. 그 위에 아무것도 없다.
    xs = [points[0][2]]
    ys = [points[0][1]]
    prev_cov, prev_prec, _ = points[0]
    for coverage, precision, threshold in points[1:]:
        width = coverage - prev_cov
        if width <= 0:
            continue
        # 누적 정답 건수의 차이를 구간 폭으로 나누면 그 구간만의 정답률이다.
        band = (coverage * precision - prev_cov * prev_prec) / width
        xs.append(threshold)
        ys.append(min(1.0, max(0.0, band)))
        prev_cov, prev_prec = coverage, precision

    # np.interp 는 x 가 오름차순이어야 한다. 임계값은 내림차순으로 쌓였다.
    xs = xs[::-1]
    ys = ys[::-1]
    # 표본이 적은 구간에서 정답률이 한 칸 뒤집힐 수 있다. 단조성을 강제한다.
    ys = [float(v) for v in np.maximum.accumulate(np.asarray(ys, dtype=float))]

    deduped_x: List[float] = []
    deduped_y: List[float] = []
    for x, y in zip(xs, ys):
        if deduped_x and x <= deduped_x[-1]:
            continue
        deduped_x.append(float(x))
        deduped_y.append(float(y))
    if len(deduped_x) < 2:
        return None
    return {"x": deduped_x, "y": deduped_y, "n": int(holdout.get("n") or 0), "source": "coverage_curve"}


def expected_accuracy(calibration: Optional[Dict[str, Any]], confidence: Optional[float]) -> Optional[float]:
    """보정 계단 위에서 확신도를 정답률로 읽는다. 보정이 없으면 None.

    계단 사이는 직선으로 잇고, 양끝 밖은 끝값으로 자른다(`np.interp` 의 기본 동작).
    """
    if not calibration or confidence is None:
        return None
    xs = calibration.get("x") or []
    ys = calibration.get("y") or []
    if len(xs) < 2 or len(xs) != len(ys):
        return None
    try:
        value = float(np.interp(float(confidence), xs, ys))
    except (TypeError, ValueError):
        return None
    return min(1.0, max(0.0, value))
