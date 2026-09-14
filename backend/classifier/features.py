#!/usr/bin/env python3
"""
필드별 특징 추출 - 설정대로 TF-IDF 블록을 만들어 가로로 잇는다

필드마다 벡터라이저를 따로 두고, 만들어진 블록에 설정의 `weight` 를 곱한 뒤
하나로 붙인다. 선형 모델이라 블록에 곱한 값이 그 필드의 영향력을 그대로 바꾼다.
가중치를 바꾸려면 `config.json` 만 고치고 다시 학습하면 된다.
"""

import logging
import re
from typing import Any, Dict, List, Sequence

import numpy as np
from scipy.sparse import csr_matrix, hstack
from sklearn.feature_extraction.text import TfidfVectorizer

logger = logging.getLogger(__name__)

# 파일명의 긴 숫자는 권 번호나 일련번호라 카테고리를 가리키지 않는다.
DIGIT_RUN = re.compile(r"\d{2,}")


def field_text(doc: Dict[str, Any], spec: Dict[str, Any]) -> str:
    """레코드 하나에서 한 필드의 원문을 꺼낸다."""
    raw = doc.get(spec["source"]) or ""
    if isinstance(raw, (list, tuple)):
        raw = " ".join(str(x) for x in raw if x)
    text = str(raw)
    if spec["source"] == "name":
        text = DIGIT_RUN.sub(" ", text)
    limit = int(spec.get("max_chars") or 0)
    return text[:limit] if limit > 0 else text


def make_vectorizer(spec: Dict[str, Any]) -> TfidfVectorizer:
    lo, hi = spec.get("ngram") or [1, 1]
    kwargs: Dict[str, Any] = {
        "sublinear_tf": True,
        "dtype": np.float32,
        "analyzer": spec.get("analyzer", "word"),
        "ngram_range": (int(lo), int(hi)),
        "min_df": int(spec.get("min_df", 1)),
        "max_features": int(spec["max_features"]) if spec.get("max_features") else None,
    }
    if kwargs["analyzer"] == "word":
        # 기본 패턴은 한 글자 토큰을 버린다. 한자 한 글자가 의미를 갖는 경우가 있어 살린다.
        kwargs["token_pattern"] = r"(?u)\b\w+\b"
    return TfidfVectorizer(**kwargs)


class FeatureSpace:
    """필드별 벡터라이저 묶음. 학습 때 fit 하고, 판정 때는 transform 만 한다."""

    def __init__(self, fields: Dict[str, Dict[str, Any]]):
        self.fields = fields
        self.vectorizers: Dict[str, TfidfVectorizer] = {}
        self.block_sizes: Dict[str, int] = {}

    @property
    def names(self) -> List[str]:
        return list(self.fields.keys())

    def fit_transform(self, docs: Sequence[Dict[str, Any]]) -> csr_matrix:
        blocks = []
        for name, spec in self.fields.items():
            texts = [field_text(d, spec) for d in docs]
            vec = make_vectorizer(spec)
            try:
                block = vec.fit_transform(texts)
            except ValueError as e:
                # 어휘가 하나도 안 남는 필드는 조용히 버린다. 전부 빈 문자열일 때 생긴다.
                logger.warning("필드 '%s' 에서 특징을 못 뽑아 건너뛴다: %s", name, e)
                continue
            self.vectorizers[name] = vec
            self.block_sizes[name] = block.shape[1]
            blocks.append(self._scaled(block, spec))
            logger.info("필드 '%s': 특징 %d개, nnz %d", name, block.shape[1], block.nnz)
        if not blocks:
            raise ValueError("어느 필드에서도 특징을 못 뽑았다")
        return hstack(blocks, format="csr")

    def transform(self, docs: Sequence[Dict[str, Any]]) -> csr_matrix:
        blocks = []
        for name, vec in self.vectorizers.items():
            spec = self.fields[name]
            texts = [field_text(d, spec) for d in docs]
            blocks.append(self._scaled(vec.transform(texts), spec))
        if not blocks:
            raise ValueError("학습된 벡터라이저가 없다")
        return hstack(blocks, format="csr")

    @staticmethod
    def _scaled(block: csr_matrix, spec: Dict[str, Any]) -> csr_matrix:
        w = float(spec.get("weight", 1.0))
        return block if w == 1.0 else block * w

    def describe(self) -> Dict[str, Dict[str, Any]]:
        return {name: {"features": self.block_sizes.get(name, 0), "weight": self.fields[name].get("weight", 1.0)} for name in self.vectorizers}
