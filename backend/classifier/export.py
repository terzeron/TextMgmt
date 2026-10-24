#!/usr/bin/env python3
"""
학습된 모델을 Rust 추론용 파일로 내보낸다

학습은 Python(sklearn)이 맡고 추론은 Rust가 맡는다. joblib 은 pickle 이라 읽는 순간
코드가 실행되므로, Rust 는 pickle 을 읽지 않는다. 이 스크립트가 pickle 을 읽어 중립 형식
(JSON + 리틀엔디언 float32)으로 바꿔 둔다.

디렉토리 하나에 다음을 쓴다.

- `model.json`: 형식 버전, 클래스, 설정, 필드별 벡터라이저 옵션, 파일별 sha256
- `vocab.<field>.json`: 어휘 배열. 위치가 필드 안의 열 번호다.
- `idf.<field>.f32`: float32 `idf_`
- `coef.f32`: float32 계수, 전치 레이아웃 `[n_features, n_classes]` 행 우선.
  특징 하나의 클래스별 계수가 연속이라 희소 입력의 내적이 캐시 친화적이다.
- `golden.json`(선택): 합성 문서와 Python 판정 결과. Rust parity 테스트가 쓴다.

실행: `python -m backend.classifier.export --model <joblib> --out <dir> [--golden N --seed S]`
"""

import argparse
import hashlib
import json
import logging
import math
import random
import sys
import unicodedata
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

import numpy as np

from backend.classifier.model import CategoryModel

logger = logging.getLogger(__name__)

FORMAT_VERSION = 1
SUPPORTED_ANALYZERS = ("word", "char", "char_wb")
WORD_TOKEN_PATTERN = r"(?u)\b\w+\b"
COEF_FILE = "coef.f32"
_CHUNK = 16 * 1024 * 1024


def _jsonable(obj: Any) -> Any:
    """JSON 으로 쓸 수 있는 파이썬 기본형으로 바꾼다. numpy 타입과 NaN/Inf 를 정리한다."""
    if isinstance(obj, dict):
        return {str(k): _jsonable(v) for k, v in obj.items()}
    if isinstance(obj, (list, tuple, set, frozenset)):
        return [_jsonable(v) for v in obj]
    if isinstance(obj, np.ndarray):
        return _jsonable(obj.tolist())
    if isinstance(obj, np.generic):
        return _jsonable(obj.item())
    if isinstance(obj, float):
        return obj if math.isfinite(obj) else None
    if obj is None or isinstance(obj, (bool, int, str)):
        return obj
    if isinstance(obj, Path):
        return str(obj)
    if hasattr(obj, "isoformat"):
        return obj.isoformat()
    return str(obj)


def _write_json(path: Path, obj: Any) -> None:
    text = json.dumps(obj, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
    try:
        path.write_bytes(text.encode("utf-8"))
    except UnicodeEncodeError as e:
        raise ValueError(f"{path.name} 에 UTF-8 로 쓸 수 없는 문자가 있다 (짝 없는 surrogate)") from e


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        while chunk := f.read(_CHUNK):
            h.update(chunk)
    return h.hexdigest()


def _check_vectorizer(name: str, vec: Any) -> None:
    """Rust 가 같게 재현하지 못하는 옵션이면 내보내기 전에 막는다."""
    if vec.analyzer not in SUPPORTED_ANALYZERS:
        raise ValueError(f"필드 '{name}': 지원하지 않는 analyzer {vec.analyzer!r}")
    for opt in ("strip_accents", "stop_words", "tokenizer", "preprocessor"):
        if getattr(vec, opt, None):
            raise ValueError(f"필드 '{name}': {opt} 는 지원하지 않는다 ({getattr(vec, opt)!r})")
    if getattr(vec, "binary", False):
        raise ValueError(f"필드 '{name}': binary=True 는 지원하지 않는다")
    if not getattr(vec, "use_idf", True):
        raise ValueError(f"필드 '{name}': use_idf=False 는 지원하지 않는다")
    if vec.norm != "l2":
        raise ValueError(f"필드 '{name}': norm {vec.norm!r} 는 지원하지 않는다 (l2 만)")
    if vec.analyzer == "word" and vec.token_pattern != WORD_TOKEN_PATTERN:
        raise ValueError(f"필드 '{name}': token_pattern {vec.token_pattern!r} 는 지원하지 않는다")


def _ordered_vocab(name: str, vec: Any) -> List[str]:
    """`vocabulary_` 를 열 번호 순서의 배열로 바꾼다. 번호가 0..n-1 을 정확히 덮어야 한다."""
    size = len(vec.vocabulary_)
    vocab: List[Optional[str]] = [None] * size
    for term, idx in vec.vocabulary_.items():
        i = int(idx)
        if not 0 <= i < size or vocab[i] is not None:
            raise ValueError(f"필드 '{name}': 어휘 번호가 0..{size - 1} 을 정확히 덮지 않는다")
        vocab[i] = str(term)
    return vocab  # type: ignore[return-value]


def _write_coef(model: CategoryModel, out: Path, n_features: int) -> Dict[str, Any]:
    estimators = getattr(model.classifier, "estimators_", None)
    n_classes = len(model.classes)
    if estimators is None or len(estimators) != n_classes:
        raise ValueError("classifier 가 클래스마다 추정기를 하나씩 가진 OneVsRest 가 아니다")
    classifier_classes = getattr(model.classifier, "classes_", None)
    if classifier_classes is not None and [str(c) for c in classifier_classes] != [str(c) for c in model.classes]:
        raise ValueError("classifier.classes_ 와 model.classes 의 순서가 다르다")
    coef = np.empty((n_features, n_classes), dtype="<f4")
    intercept: List[float] = []
    for k, est in enumerate(estimators):
        w = np.asarray(est.coef_)
        if w.shape != (1, n_features):
            raise ValueError(f"추정기 {k} 의 coef_ shape {w.shape} 가 (1, {n_features}) 가 아니다")
        w32 = w[0].astype("<f4")
        if not np.isfinite(w32).all():
            raise ValueError(f"추정기 {k} 의 계수에 NaN/Inf 가 있다 (float32 변환 뒤)")
        coef[:, k] = w32
        b = float(np.asarray(est.intercept_).ravel()[0])
        if not math.isfinite(b):
            raise ValueError(f"추정기 {k} 의 intercept 가 유한하지 않다")
        intercept.append(b)
    path = out / COEF_FILE
    coef.tofile(path)
    return {"coef_file": COEF_FILE, "coef_sha256": _sha256_file(path), "intercept": intercept}


def export_model(model: CategoryModel, out_dir: Path | str, golden: int = 0, seed: int = 0) -> Dict[str, Any]:
    """모델을 `out_dir` 에 내보내고 요약을 돌려준다. `golden > 0` 이면 `golden.json` 도 쓴다."""
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    space = model.feature_space
    fields: List[Dict[str, Any]] = []
    vocabs: Dict[str, List[str]] = {}
    offset = 0
    for name, vec in space.vectorizers.items():
        _check_vectorizer(name, vec)
        spec = space.fields[name]
        vocab = _ordered_vocab(name, vec)
        idf = np.asarray(vec.idf_).astype("<f4")
        if idf.shape != (len(vocab),) or not np.isfinite(idf).all():
            raise ValueError(f"필드 '{name}': idf 길이 또는 값이 맞지 않다")
        vocab_file, idf_file = f"vocab.{name}.json", f"idf.{name}.f32"
        _write_json(out / vocab_file, vocab)
        (out / idf_file).write_bytes(idf.tobytes())
        lo, hi = vec.ngram_range
        fields.append(
            {
                "name": name,
                "source": spec["source"],
                "weight": float(spec.get("weight", 1.0)),
                "max_chars": int(spec.get("max_chars") or 0),
                "analyzer": vec.analyzer,
                "ngram_range": [int(lo), int(hi)],
                "lowercase": bool(vec.lowercase),
                "sublinear_tf": bool(vec.sublinear_tf),
                "norm": vec.norm,
                "token_pattern": vec.token_pattern,
                "offset": offset,
                "size": len(vocab),
                "vocab_file": vocab_file,
                "idf_file": idf_file,
                "idf_sha256": _sha256_file(out / idf_file),
                "vocab_sha256": _sha256_file(out / vocab_file),
            }
        )
        vocabs[name] = vocab
        offset += len(vocab)
    if not fields:
        raise ValueError("벡터라이저가 하나도 없다")

    manifest: Dict[str, Any] = {
        "format_version": FORMAT_VERSION,
        "classes": [str(c) for c in model.classes],
        "config": _jsonable(model.config),
        "meta": _jsonable({k: v for k, v in model.meta.items() if not k.startswith("_")}),
        "n_features": offset,
        "n_classes": len(model.classes),
        "fields": fields,
    }
    manifest.update(_write_coef(model, out, offset))
    _write_json(out / "model.json", manifest)

    summary: Dict[str, Any] = {"out_dir": str(out), "n_features": offset, "n_classes": len(model.classes), "fields": [f["name"] for f in fields]}
    if golden > 0:
        cases = write_golden(model, out, vocabs, golden, seed)
        summary["golden_cases"] = cases
    summary["files"] = {p.name: p.stat().st_size for p in sorted(out.iterdir())}
    return summary


# ---------------------------------------------------------------------------
# golden: 합성 문서와 Python 판정 결과
# ---------------------------------------------------------------------------


def _special_docs(space_sources: Sequence[str]) -> List[Dict[str, Any]]:
    """경계 사례. 실제 도서 본문은 쓰지 않고 직접 쓴 짧은 문장만 쓴다."""

    def fill(text: str, name: str = "sample.epub") -> Dict[str, Any]:
        base = {"text": text, "title": text[:40], "author": text[:10], "publisher": text[:12], "type": "epub", "name": name}
        return {k: v for k, v in base.items() if k in space_sources or k == "name"}

    hangul = "대한민국의 역사와 문화 이야기 무림 용사의 모험"
    return [
        {"label": "empty", "doc": {}},
        {"label": "hangul", "doc": fill(hangul)},
        {"label": "english", "doc": fill("The Quick Brown FOX jumps over a lazy dog, again and again.")},
        {"label": "cjk", "doc": fill("漢字と ひらがな カタカナ 東京 上海 運氣調息")},
        {"label": "whitespace", "doc": fill("a \t\n  b\x0b\x0c 　c\x1f d\x00e   ")},
        {"label": "digits_name", "doc": fill("숫자 이름", name="book_0012345678_vol01_7_v2.epub")},
        {"label": "nfd", "doc": fill(unicodedata.normalize("NFD", hangul))},
        {"label": "emoji", "doc": fill("😀 café 🎉 naïve ÀÉÎ ΣΑΣ İstanbul ǅ 1٢３ x²")},
        {"label": "list_author", "doc": {**fill("목록 필드 시험"), "author": ["홍길동", "", "Kim"]}},
        {"label": "blank_fields", "doc": {"text": "   ", "title": "", "author": None, "publisher": "  ", "type": "", "name": ""}},
    ]


def _synthetic_doc(rng: random.Random, pools: Dict[str, List[str]]) -> Dict[str, Any]:
    """어휘에서 무작위로 뽑은 조각을 이어 만든 합성 문서."""
    doc: Dict[str, Any] = {}
    for source, terms in pools.items():
        if not terms:
            continue
        count = rng.randint(0, 40 if source == "text" else 4)
        pieces = [rng.choice(terms) for _ in range(count)]
        text = "".join(p + rng.choice(["", " ", " "]) for p in pieces).strip()
        if source == "name":
            text = f"{text}_{rng.randint(10, 99999999)}.epub"
        doc[source] = text
    return doc


def write_golden(model: CategoryModel, out: Path, vocabs: Dict[str, List[str]], n: int, seed: int) -> int:
    """합성 문서 `n` 개 + 경계 사례를 모델에 넣어 결정값·확신도·사유를 `golden.json` 에 쓴다."""
    rng = random.Random(seed)
    space = model.feature_space
    pools: Dict[str, List[str]] = {}
    for name, vocab in vocabs.items():
        pools.setdefault(space.fields[name]["source"], []).extend(t for t in vocab if t.strip())
    cases: List[Dict[str, Any]] = list(_special_docs(list(pools)))
    for i in range(n):
        cases.append({"label": f"synthetic_{i}", "doc": _synthetic_doc(rng, pools)})
    docs = [c["doc"] for c in cases]
    scores = model.scores(docs)
    preds = model.predict(docs)
    for case, row, pred in zip(cases, scores, preds):
        case["scores"] = [float(v) for v in row]
        case["category"] = pred.category
        case["confidence"] = pred.confidence
        case["top"] = [{"category": c, "confidence": p} for c, p in pred.ranked]
        case["reason"] = pred.reason
    payload = {"temperature": model.temperature, "min_confidence": model.min_confidence, "seed": seed, "cases": _jsonable(cases)}
    _write_json(out / "golden.json", payload)
    return len(cases)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="분류 모델을 Rust 추론용 파일로 내보낸다")
    parser.add_argument("--model", required=True, help="model.joblib 경로 (신뢰하는 파일만)")
    parser.add_argument("--out", required=True, help="출력 디렉토리")
    parser.add_argument("--golden", type=int, default=0, help="golden.json 에 쓸 합성 문서 수")
    parser.add_argument("--seed", type=int, default=0, help="합성 문서 시드")
    args = parser.parse_args(argv)
    model = CategoryModel.load(args.model)
    if model is None:
        print(f"모델을 읽지 못했다: {args.model}", file=sys.stderr)
        return 1
    summary = export_model(model, args.out, golden=args.golden, seed=args.seed)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
