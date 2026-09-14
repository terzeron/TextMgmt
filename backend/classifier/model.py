#!/usr/bin/env python3
"""
학습된 모델 - 로드와 예측

선형 모델의 결정값(decision_function)은 확률이 아니다. 1위와 2위의 차이를
온도로 나눠 softmax 에 넣어 0~1 의 확신도로 바꾼다. 임계값 하나로 판정률과
정답률을 맞바꿀 수 있게 하려는 것이다.

모델 파일에는 학습 시각, 설정 해시, 홀드아웃 성적을 함께 넣는다.
어떤 설정으로 만든 모델인지 나중에 되짚고, 나빠지면 되돌리기 위해서다.
"""

import logging
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np

logger = logging.getLogger(__name__)

MODEL_PATH = Path(__file__).resolve().parent / "model.joblib"


@dataclass
class Prediction:
    """판정 하나의 결과. category 가 None 이면 확신이 모자라 판정하지 않은 것이다."""

    category: Optional[str]
    confidence: float
    ranked: List[Tuple[str, float]]
    reason: str


def softmax_confidence(scores: np.ndarray, temperature: float) -> np.ndarray:
    """결정값을 확률로 바꾼다. 온도가 크면 평탄해져 확신도가 내려간다."""
    t = max(float(temperature), 1e-6)
    z = scores / t
    z = z - z.max(axis=-1, keepdims=True)
    e = np.exp(z)
    return e / e.sum(axis=-1, keepdims=True)


class CategoryModel:
    """학습 산출물을 감싸 예측만 책임진다."""

    def __init__(self, payload: Dict[str, Any]):
        self.feature_space = payload["feature_space"]
        self.classifier = payload["classifier"]
        self.classes: np.ndarray = np.asarray(payload["classes"])
        self.config: Dict[str, Any] = payload["config"]
        self.meta: Dict[str, Any] = payload.get("meta", {})

    @property
    def temperature(self) -> float:
        return float(self.config["decision"].get("temperature", 1.0))

    @property
    def min_confidence(self) -> float:
        return float(self.config["decision"].get("min_confidence", 0.9))

    def scores(self, docs: Sequence[Dict[str, Any]]) -> np.ndarray:
        X = self.feature_space.transform(docs)
        d = self.classifier.decision_function(X)
        return d.reshape(len(docs), -1)

    def predict(self, docs: Sequence[Dict[str, Any]], min_confidence: Optional[float] = None, top_k: int = 3) -> List[Prediction]:
        if not docs:
            return []
        floor = self.min_confidence if min_confidence is None else float(min_confidence)
        d = self.scores(docs)
        prob = softmax_confidence(d, self.temperature)
        order = np.argsort(-d, axis=1)
        out: List[Prediction] = []
        for i in range(len(docs)):
            idx = order[i, :top_k]
            ranked = [(str(self.classes[j]), float(prob[i, j])) for j in idx]
            best_cat, best_p = ranked[0]
            runner = ranked[1][0] if len(ranked) > 1 else "-"
            reason = f"모델 확신도 {best_p:.3f} -> {best_cat} (2위 {runner} {ranked[1][1]:.3f})" if len(ranked) > 1 else f"모델 확신도 {best_p:.3f} -> {best_cat}"
            if best_p < floor:
                out.append(Prediction(None, best_p, ranked, f"확신도 {best_p:.3f} < 임계값 {floor:.2f} 이라 판정하지 않음 (1위 후보 {best_cat})"))
            else:
                out.append(Prediction(best_cat, best_p, ranked, reason))
        return out

    def predict_one(self, doc: Dict[str, Any], min_confidence: Optional[float] = None) -> Prediction:
        return self.predict([doc], min_confidence=min_confidence)[0]

    def save(self, path: Path | str | None = None) -> Path:
        import joblib

        p = Path(path) if path else MODEL_PATH
        p.parent.mkdir(parents=True, exist_ok=True)
        joblib.dump(
            {"feature_space": self.feature_space, "classifier": self.classifier, "classes": self.classes, "config": self.config, "meta": self.meta},
            p,
            compress=3,
        )
        return p

    @classmethod
    def load(cls, path: Path | str | None = None) -> Optional["CategoryModel"]:
        import joblib

        p = Path(path) if path else MODEL_PATH
        if not p.exists():
            logger.info("모델 파일이 없다: %s", p)
            return None
        try:
            payload = joblib.load(p)
        except Exception as e:
            logger.warning("모델을 읽지 못했다 (%s): %s", p, e)
            return None
        try:
            return cls(payload)
        except KeyError as e:
            logger.warning("모델 파일의 형식이 맞지 않다 (%s): 없는 항목 %s", p, e)
            return None


_lock = threading.Lock()
_cached: Optional[CategoryModel] = None
_loaded = False


def get_default_model() -> Optional[CategoryModel]:
    """기본 경로의 모델을 한 번만 읽어 캐시한다."""
    global _cached, _loaded
    if _loaded:
        return _cached
    with _lock:
        if not _loaded:
            _cached = CategoryModel.load()
            _loaded = True
    return _cached


def reset_default_model() -> None:
    """재학습 후 다시 읽게 만든다. 테스트에서도 쓴다."""
    global _cached, _loaded
    with _lock:
        _cached = None
        _loaded = False
