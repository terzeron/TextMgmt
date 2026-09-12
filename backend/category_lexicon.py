#!/usr/bin/env python3
"""
CategoryLexicon - 코퍼스에서 추출한 카테고리별 어휘 사전 기반 본문 점수화

`utils/corpus_lexicon.py`가 /mnt/data/text 코퍼스를 샘플링해 만든 사전
(`backend/data/category_lexicon.json.gz`)을 읽어, 임의의 본문 텍스트에 대해
카테고리별 유사도(0~1)를 돌려준다.

사전 파일이 없거나 kiwipiepy 로드에 실패하면 빈 결과를 돌려주고,
호출부(book_classifier)는 기존 판정 로직만으로 동작한다.
"""

import gzip
import json
import logging
import math
import re
import threading
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Set

logger = logging.getLogger(__name__)

DEFAULT_LEXICON_PATH = Path(__file__).resolve().parent / "data" / "category_lexicon.json.gz"

# kiwi 품사 태그 중 내용어로 취급할 것: 일반명사, 고유명사, 외국어, 한자, 숫자
NOUN_TAGS = frozenset({"NNG", "NNP", "SL", "SH", "SN"})

# 고유 단어(해당 카테고리에서만 관찰되는 단어)에 곱하는 배수
UNIQUE_BONUS = 1.5

# 코사인 유사도가 이 값에 못 미치면 신호 없음으로 본다
MIN_SIMILARITY = 0.02

_HANGUL_RE = re.compile(r"[가-힣]")

_kiwi_lock = threading.Lock()
_kiwi_instance: Any = None
_kiwi_failed = False


def get_kiwi(num_workers: int = 1) -> Any:
    """kiwipiepy Kiwi 인스턴스를 지연 생성하여 재사용한다. 실패 시 None."""
    global _kiwi_instance, _kiwi_failed
    if _kiwi_instance is not None:
        return _kiwi_instance
    if _kiwi_failed:
        return None
    with _kiwi_lock:
        if _kiwi_instance is not None:
            return _kiwi_instance
        if _kiwi_failed:
            return None
        try:
            from kiwipiepy import Kiwi  # noqa: PLC0415  지연 import (모델 105MB 로드 회피)

            _kiwi_instance = Kiwi(num_workers=num_workers)
        except Exception as e:  # pragma: no cover - 설치 환경에 의존
            logger.warning(f"kiwipiepy 로드 실패, 어휘 사전 점수화를 건너뛴다: {e}")
            _kiwi_failed = True
            return None
    return _kiwi_instance


def is_meaningful_word(word: str) -> bool:
    """어휘 사전에 넣을 가치가 있는 단어인지 판정"""
    if _HANGUL_RE.search(word):
        return len(word) >= 2
    return len(word) >= 3 and word.isalpha()


def merge_contiguous_nouns(tokens: Iterable[Any]) -> List[str]:
    """
    kiwi 토큰 목록에서 명사 계열만 남기고, 원문 문자 위치가 맞닿은 토큰을 하나로 합친다.

    합치지 않으면 도메인 복합어가 부서진다.
    예) '운기조식' -> 운기 / 조 / 식,  '장문인' -> 장 / 문인
    """
    words: List[str] = []
    buf: List[str] = []
    next_start = -1

    for tok in tokens:
        if tok.tag in NOUN_TAGS:
            if buf and tok.start == next_start:
                buf.append(tok.form)
            else:
                if buf:
                    words.append("".join(buf))
                buf = [tok.form]
            next_start = tok.start + tok.len
        else:
            if buf:
                words.append("".join(buf))
                buf = []
            next_start = -1

    if buf:
        words.append("".join(buf))

    return [w for w in words if is_meaningful_word(w)]


def extract_nouns(text: str, kiwi: Any = None) -> List[str]:
    """본문 텍스트에서 명사 계열 단어를 순서대로 추출한다. kiwi 미가용 시 빈 목록."""
    if not text or not text.strip():
        return []
    k = kiwi if kiwi is not None else get_kiwi()
    if k is None:
        return []
    try:
        tokens = k.tokenize(text)
    except Exception as e:  # pragma: no cover - 입력 의존
        logger.warning(f"형태소 분석 실패: {e}")
        return []
    return merge_contiguous_nouns(tokens)


def extract_word_set(text: str, kiwi: Any = None) -> Set[str]:
    """본문의 단어 집합. 사전 통계가 문서빈도 기준이므로 집합으로 맞춘다."""
    return set(extract_nouns(text, kiwi=kiwi))


def extract_word_sets(texts: List[str], kiwi: Any = None) -> List[Set[str]]:
    """
    여러 본문을 한 번에 토큰화하여 단어 집합 목록을 돌려준다.

    kiwi는 문자열 하나를 넘기면 `num_workers` 설정과 무관하게 단일 스레드로 돈다.
    리스트를 넘겨야 워커가 실제로 병렬 처리한다(실측 1워커 57,754자/s -> 8워커 199,676자/s).
    """
    if not texts:
        return []
    k = kiwi if kiwi is not None else get_kiwi()
    if k is None:
        return [set() for _ in texts]
    try:
        results = list(k.tokenize(texts))
    except Exception as e:  # pragma: no cover - 입력 의존
        logger.warning(f"일괄 형태소 분석 실패, 개별 처리로 대체: {e}")
        return [extract_word_set(t, kiwi=k) for t in texts]
    return [set(merge_contiguous_nouns(tokens)) for tokens in results]


class CategoryLexicon:
    """카테고리별 top-N 단어 가중치와 고유 단어 세트를 담고 텍스트를 점수화한다."""

    def __init__(self, data: Dict[str, Any]) -> None:
        self.version: int = int(data.get("version", 1))
        self.params: Dict[str, Any] = data.get("params", {})
        self.built_at: str = data.get("built_at", "")
        self._weights: Dict[str, Dict[str, float]] = {}
        self._unique: Dict[str, Set[str]] = {}
        self._norms: Dict[str, float] = {}
        self.doc_counts: Dict[str, int] = {}

        for cat, entry in (data.get("categories") or {}).items():
            words = {w: float(v) for w, v in (entry.get("words") or {}).items()}
            if not words:
                continue
            self._weights[cat] = words
            self._unique[cat] = set(entry.get("unique") or [])
            self.doc_counts[cat] = int(entry.get("doc_count", 0))
            norm = entry.get("norm")
            self._norms[cat] = float(norm) if norm else math.sqrt(sum(v * v for v in words.values())) or 1.0

    @property
    def categories(self) -> List[str]:
        return sorted(self._weights)

    def __bool__(self) -> bool:
        return bool(self._weights)

    @classmethod
    def load(cls, path: Optional[Path] = None) -> Optional["CategoryLexicon"]:
        """gzip JSON 사전을 읽는다. 파일이 없거나 손상되면 None."""
        p = Path(path) if path else DEFAULT_LEXICON_PATH
        if not p.exists():
            logger.info(f"어휘 사전 파일 없음: {p}")
            return None
        try:
            opener = gzip.open if p.suffix == ".gz" else open
            with opener(p, "rt", encoding="utf-8") as f:  # type: ignore[operator]
                data = json.load(f)
        except Exception as e:
            logger.warning(f"어휘 사전 로드 실패 ({p}): {e}")
            return None
        lex = cls(data)
        if not lex:
            logger.warning(f"어휘 사전이 비어 있음: {p}")
            return None
        return lex

    def score_words(self, words: Set[str]) -> Dict[str, float]:
        """
        단어 집합에 대한 카테고리별 코사인 유사도.

        raw(c) = Σ weight(c,w) × (고유 단어면 1.5) 를 카테고리 벡터의 L2 노름으로 나눈다.
        노름으로 나누지 않으면 어휘 가중치 총량이 큰 카테고리가 항상 이긴다.
        """
        if not words:
            return {}
        doc_norm = math.sqrt(len(words))
        sims: Dict[str, float] = {}
        for cat, weights in self._weights.items():
            uniq = self._unique[cat]
            acc = 0.0
            for w in words:
                weight = weights.get(w)
                if weight is None:
                    continue
                acc += weight * (UNIQUE_BONUS if w in uniq else 1.0)
            if acc <= 0.0:
                continue
            sims[cat] = acc / (self._norms[cat] * doc_norm)
        return sims

    def score_text(self, text: str, kiwi: Any = None) -> Dict[str, float]:
        """본문 텍스트에 대한 카테고리별 코사인 유사도."""
        return self.score_words(extract_word_set(text, kiwi=kiwi))

    def rank(self, text: str, top_k: int = 5, kiwi: Any = None) -> List[tuple]:
        """
        (카테고리, 정규화점수 0~1, 원유사도) 를 점수 내림차순으로 최대 top_k개 반환.

        최고 유사도가 MIN_SIMILARITY 미만이면 신호 없음으로 보고 빈 목록.
        """
        sims = self.score_text(text, kiwi=kiwi)
        if not sims:
            return []
        ordered = sorted(sims.items(), key=lambda kv: kv[1], reverse=True)
        best = ordered[0][1]
        if best < MIN_SIMILARITY:
            return []
        return [(cat, round(sim / best, 4), round(sim, 6)) for cat, sim in ordered[:top_k]]


_default_lexicon: Optional[CategoryLexicon] = None
_default_loaded = False
_default_lock = threading.Lock()


def get_default_lexicon() -> Optional[CategoryLexicon]:
    """기본 경로의 사전을 한 번만 읽어 재사용한다."""
    global _default_lexicon, _default_loaded
    if _default_loaded:
        return _default_lexicon
    with _default_lock:
        if not _default_loaded:
            _default_lexicon = CategoryLexicon.load()
            _default_loaded = True
    return _default_lexicon


def reset_default_lexicon() -> None:
    """테스트용: 캐시된 기본 사전을 비운다."""
    global _default_lexicon, _default_loaded
    with _default_lock:
        _default_lexicon = None
        _default_loaded = False
