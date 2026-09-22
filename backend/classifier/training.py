#!/usr/bin/env python3
"""
학습과 평가

카테고리별로 층화해 홀드아웃을 떼고, 남은 것으로 학습한 뒤 홀드아웃으로 잰다.
성적은 "전체 정답률" 하나가 아니라 **판정률-정답률 곡선**으로 본다.
확신도 임계값을 올리면 답하는 비율이 줄고 정답률이 오르기 때문에, 둘을 같이
봐야 운영 지점을 고를 수 있다.
"""

import gc
import logging
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np
from joblib import parallel_backend
from sklearn.multiclass import OneVsRestClassifier
from sklearn.svm import LinearSVC

from backend.classifier.config import config_hash, enabled_fields
from backend.classifier.features import FeatureSpace
from backend.classifier.model import CategoryModel, softmax_confidence

logger = logging.getLogger(__name__)


def rss_note() -> str:
    """지금 이 프로세스가 쥐고 있는 메모리. 어느 단계가 피크인지 로그로 남긴다.

    두 번 죽은 뒤에 넣었다. 죽고 나서 보면 어느 단계였는지 알 길이 없었다.

    익명(anon)과 파일을 나눠 찍는다. 행렬을 파일로 두고 mmap 으로 넘기면 그만큼이
    파일 쪽으로 옮겨간다. 모자랄 때 커널이 회수하는 것은 파일 쪽뿐이라,
    `systemd-oomd` 나 OOM 킬러가 보는 숫자는 익명 쪽이다.
    """
    try:
        fields = {}
        with open("/proc/self/status", encoding="ascii") as f:
            for line in f:
                key = line.split(":", 1)[0]
                if key in ("VmHWM", "VmRSS", "RssAnon", "RssFile"):
                    fields[key] = int(line.split()[1])
        gb = lambda k: fields.get(k, 0) / 1024**2  # noqa: E731
        return f"[익명 {gb('RssAnon'):.1f}GB + 파일 {gb('RssFile'):.1f}GB = {gb('VmRSS'):.1f}GB, 최대 {gb('VmHWM'):.1f}GB]"
    except (OSError, ValueError):
        return ""


def filter_by_size(docs: Sequence[Dict[str, Any]], min_per_category: int) -> Tuple[List[Dict[str, Any]], List[str]]:
    """표본이 너무 적은 카테고리를 뺀다. 층화 분할이 성립하려면 최소 2건은 있어야 한다."""
    counts = Counter(d["cat"] for d in docs)
    floor = max(int(min_per_category), 2)
    dropped = sorted(c for c, n in counts.items() if n < floor)
    kept = [d for d in docs if counts[d["cat"]] >= floor]
    return kept, dropped


def document_key(doc: Dict[str, Any]) -> str:
    """분할에 쓸 문서의 고유 키. ES 의 _id(inode)가 없으면 경로를 쓴다."""
    return str(doc.get("id") or doc.get("path") or doc.get("name") or "")


def split_holdout(docs: Sequence[Dict[str, Any]], ratio: float, seed: int) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    """문서 키의 해시로 홀드아웃을 뗀다. 카테고리별로 비율을 맞춘다.

    위치 인덱스로 나누면(`train_test_split(np.arange(n), random_state=seed)`) 코퍼스가
    한 건만 바뀌어도 전혀 다른 집합이 떨어진다. 그러면 나중에 같은 홀드아웃을
    되살릴 수 없다 - 되살렸다고 믿고 재면 학습 문서가 섞여 정답률이 부풀려진다.
    실측에서 문서 142건이 줄자 top-1 정답률이 86.5% 에서 93.8% 로 뛰었다.

    해시로 나누면 문서 하나의 소속이 그 문서의 키에만 달린다. 다른 문서가 들어오고
    나가도 안 바뀐다. 그래서 몇 달 뒤에도 같은 홀드아웃을 다시 뗄 수 있다.

    카테고리별로 따로 자르는 이유는 층화를 유지하기 위해서다. 카테고리 크기가
    78,520 대 30 까지 벌어져서, 전체를 한 번에 자르면 작은 카테고리가 통째로
    한쪽에 몰릴 수 있다.
    """
    import hashlib

    key = str(seed).encode()
    buckets: Dict[str, List[Tuple[bytes, Dict[str, Any]]]] = {}
    for doc in docs:
        digest = hashlib.blake2b(document_key(doc).encode("utf-8"), digest_size=8, key=key).digest()
        buckets.setdefault(doc["cat"], []).append((digest, doc))

    train_docs: List[Dict[str, Any]] = []
    hold_docs: List[Dict[str, Any]] = []
    for _cat, rows in buckets.items():
        rows.sort(key=lambda r: r[0])
        # 최소 1건은 홀드아웃에 남긴다. 학습 쪽도 최소 1건은 남아야 한다.
        n_hold = min(max(int(round(len(rows) * ratio)), 1), len(rows) - 1) if len(rows) > 1 else 0
        hold_docs.extend(doc for _d, doc in rows[:n_hold])
        train_docs.extend(doc for _d, doc in rows[n_hold:])
    return train_docs, hold_docs


def coverage_curve(correct: np.ndarray, confidence: np.ndarray, points: Sequence[float] = (0.5, 0.7, 0.8, 0.9, 0.95, 1.0)) -> List[Dict[str, float]]:
    """확신도가 높은 순으로 줄 세우고, 상위 몇 %까지 답했을 때의 정답률을 본다."""
    order = np.argsort(-confidence)
    running = np.cumsum(correct[order]) / np.arange(1, len(order) + 1)
    out = []
    for cov in points:
        k = max(int(len(order) * cov) - 1, 0)
        out.append({"coverage": float(cov), "precision": float(running[k]), "threshold": float(confidence[order][k])})
    return out


def max_coverage_at(correct: np.ndarray, confidence: np.ndarray, target_precision: float = 0.90) -> Dict[str, float]:
    """목표 정답률을 지키면서 답할 수 있는 최대 비율과, 그때의 임계값."""
    order = np.argsort(-confidence)
    running = np.cumsum(correct[order]) / np.arange(1, len(order) + 1)
    good = np.where(running >= target_precision)[0]
    if not len(good):
        return {"coverage": 0.0, "precision": 0.0, "threshold": 1.0}
    k = int(good[-1])
    return {"coverage": float((k + 1) / len(order)), "precision": float(running[k]), "threshold": float(confidence[order][k])}


def evaluate(model: CategoryModel, docs: Sequence[Dict[str, Any]], accept_parent: Optional[Dict[str, str]] = None) -> Dict[str, Any]:
    """
    홀드아웃으로 판정률-정답률 곡선을 낸다.

    `accept_parent` 는 하위 카테고리를 상위 장르로 접는 표다. 모델이 상위 장르를
    내놓고 정답이 하위 카테고리일 때 이를 오답으로 세면 안 된다.
    """
    parent = accept_parent or {}
    d = model.scores(docs)
    prob = softmax_confidence(d, model.temperature)
    best = np.argmax(d, axis=1)
    pred = model.classes[best]
    conf = prob[np.arange(len(docs)), best]
    truth = np.array([x["cat"] for x in docs])
    correct = np.fromiter((p == t or parent.get(p, p) == parent.get(t, t) for p, t in zip(pred, truth)), bool, len(truth))
    return {"n": len(docs), "top1_accuracy": float(correct.mean()), "curve": coverage_curve(correct, conf), "at_precision_90": max_coverage_at(correct, conf, 0.90), "at_precision_95": max_coverage_at(correct, conf, 0.95)}


def fit_confidence_calibration(model: CategoryModel, docs: Sequence[Dict[str, Any]], accept_parent: Optional[Dict[str, str]] = None) -> Optional[Dict[str, Any]]:
    """홀드아웃에서 (확신도 -> 정답률) 계단을 학습한다.

    `evaluate` 와 같은 정오 기준을 쓴다. 정답률 곡선과 화면의 예상 정답률이 서로
    다른 계산으로 갈리면, 같은 모델을 두고 두 숫자가 어긋난다.
    """
    from backend.classifier.calibration import fit_expected_accuracy

    parent = accept_parent or {}
    d = model.scores(docs)
    prob = softmax_confidence(d, model.temperature)
    best = np.argmax(d, axis=1)
    pred = model.classes[best]
    conf = prob[np.arange(len(docs)), best]
    truth = np.array([x["cat"] for x in docs])
    correct = np.fromiter((p == t or parent.get(p, p) == parent.get(t, t) for p, t in zip(pred, truth)), bool, len(truth))
    return fit_expected_accuracy(conf, correct)


def train(docs: Sequence[Dict[str, Any]], config: Dict[str, Any], accept_parent: Optional[Dict[str, str]] = None, spill_dir: Optional[Path] = None) -> Tuple[CategoryModel, Dict[str, Any]]:
    """학습하고 홀드아웃 성적을 함께 돌려준다.

    `spill_dir` 을 주면 벡터화 블록을 그 디렉토리로 흘려보내 피크 메모리를 낮춘다.
    23만 건 실측에서 블록을 다 안고 `hstack` 하는 순간이 피크였고, 여유 10GB 기계에서
    `systemd-oomd` 가 학습을 죽였다. 대신 본문을 놓아주므로 `docs` 가 제자리에서 바뀐다.
    """
    kept, dropped = filter_by_size(docs, config["corpus"]["min_per_category"])
    if not kept:
        raise ValueError("학습할 문서가 없다. min_per_category 를 낮추거나 collect 를 다시 하라")
    if dropped:
        logger.info("표본 부족으로 제외한 카테고리 %d개: %s", len(dropped), ", ".join(dropped[:10]) + (" ..." if len(dropped) > 10 else ""))

    y = np.array([d["cat"] for d in kept])
    train_docs, hold_docs = split_holdout(kept, float(config["holdout"]), int(config["seed"]))
    logger.info("문서 %d건, 카테고리 %d개, 학습 %d / 홀드아웃 %d", len(kept), len(set(y)), len(train_docs), len(hold_docs))

    # 레이블은 본문을 놓아주기 전에 뽑아 둔다.
    y_train = np.array([d["cat"] for d in train_docs])

    def release_source_text() -> None:
        """벡터화가 끝난 학습 문서의 원문을 놓아준다.

        합치는 단계가 피크다. 그 직전에 부른다. 홀드아웃은 evaluate 가 다시 읽으므로
        건드리지 않는다. `docs` 를 제자리에서 바꾸는 것이라 스필을 켰을 때만 한다.
        """
        held = {id(d) for d in hold_docs}
        for d in train_docs:
            if id(d) not in held:
                for key in ("text", "name", "title", "author", "publisher"):
                    d.pop(key, None)
        gc.collect()
        logger.info("학습 문서의 원문을 놓아줬다 %s", rss_note())

    t0 = time.time()
    space = FeatureSpace(enabled_fields(config))
    # liblinear 은 float64 만 받는다. 합칠 때 바로 float64 로 만든다. float32 로 합친 뒤
    # astype 을 부르면 5GB 짜리 사본이 하나 더 생긴다.
    X = space.fit_transform(train_docs, spill_dir=spill_dir, dtype=np.float64, after_blocks=release_source_text if spill_dir is not None else None)
    logger.info("특징 %d개, 벡터화 %.0f초 %s", X.shape[1], time.time() - t0, rss_note())
    logger.info("행렬 %.1fGB (%d x %d, 비영요소 %d개) %s", (X.data.nbytes + X.indices.nbytes + X.indptr.nbytes) / 1024**3, X.shape[0], X.shape[1], X.nnz, rss_note())

    t0 = time.time()
    mcfg = config["model"]
    base = LinearSVC(C=float(mcfg["C"]), dual="auto", class_weight=None if mcfg.get("class_weight") in (None, "none") else mcfg["class_weight"], max_iter=int(mcfg.get("max_iter", 1000)))
    clf = OneVsRestClassifier(base, n_jobs=int(mcfg.get("n_jobs", 8)))
    # 스레드 병렬을 쓴다. liblinear 이 `with nogil` 로 GIL 을 풀어 실제로 병렬이 된다.
    #
    # 다만 행렬을 하나만 쓰지는 않는다. liblinear 은 호출마다 자기 자료구조로 옮겨 담아,
    # 스레드로 돌려도 워커 수에 비례해 메모리가 는다. 표본 4만 건(행렬 0.28GB) 실측:
    #   n_jobs=1  fit 266초  +0.37GB      n_jobs=4  fit 131초  +1.71GB
    #   n_jobs=2  fit 176초  +0.87GB      n_jobs=8  fit 122초  +3.23GB
    # 즉 워커당 약 0.4GB, 행렬 크기의 1.4배다. 비영요소가 4.85배인 23만 건에 대입하면
    # n_jobs=8 은 fit 에만 약 15.7GB 가 들어 10GB 상한에서 죽는다.
    #
    # 워커를 8에서 2로 줄여도 fit 은 2.2배밖에 안 느리다. 메모리는 3.7배 적게 쓴다.
    with parallel_backend("threading", n_jobs=int(mcfg.get("n_jobs", 8))):
        clf.fit(X, y_train)
    elapsed = time.time() - t0
    logger.info("학습 %.0f초 %s", elapsed, rss_note())

    model = CategoryModel(
        {
            "feature_space": space,
            "classifier": clf,
            "classes": clf.classes_,
            "config": config,
            "meta": {"trained_at": datetime.now(timezone.utc).isoformat(), "config_hash": config_hash(config), "documents": len(kept), "categories": sorted(set(y)), "dropped_categories": dropped, "features": int(X.shape[1]), "blocks": space.describe(), "train_seconds": round(elapsed, 1)},
        }
    )
    report = evaluate(model, hold_docs, accept_parent=accept_parent)
    model.meta["holdout"] = report

    # 임계값을 홀드아웃에서 고른다. 확신도 척도는 데이터마다 달라서 고정값을 못 쓴다.
    target = float(config["decision"].get("target_accuracy") or 0.90)
    picked = report["at_precision_95"] if target >= 0.95 else report["at_precision_90"]
    model.meta["calibrated_threshold"] = picked["threshold"]
    model.meta["calibrated_target"] = target
    logger.info("임계값 %.3f 선택 (정답률 %.1f%% 목표, 그때 판정률 %.1f%%)", picked["threshold"], target * 100, picked["coverage"] * 100)

    # 화면에 쓸 눈금. 확신도 0.076 을 "예상 정답률 0.93" 으로 옮기는 계단이다.
    # 판정에는 안 쓴다 - 임계값과 서점 경계는 원래 확신도 눈금 위에 있다.
    model.meta["confidence_calibration"] = fit_confidence_calibration(model, hold_docs, accept_parent=accept_parent)

    return model, report


def format_report(report: Dict[str, Any]) -> str:
    lines = [f"홀드아웃 {report['n']:,}건  전체 top-1 정답률 {report['top1_accuracy']:.1%}", "", "  판정률   정답률   임계값"]
    for row in report["curve"]:
        lines.append(f"  {row['coverage']:>5.0%}   {row['precision']:>6.1%}   {row['threshold']:.3f}")
    a90, a95 = report["at_precision_90"], report["at_precision_95"]
    lines.append("")
    lines.append(f"  정답률 90% 유지 최대 판정률: {a90['coverage']:.1%}  (임계값 {a90['threshold']:.3f})")
    lines.append(f"  정답률 95% 유지 최대 판정률: {a95['coverage']:.1%}  (임계값 {a95['threshold']:.3f})")
    return "\n".join(lines)
