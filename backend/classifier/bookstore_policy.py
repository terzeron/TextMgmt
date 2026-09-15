#!/usr/bin/env python3
"""
서점 신호를 쓰는 규칙

두 가지를 담는다. 이름이 `calibrate` 였을 때는 확률 보정처럼 읽혔지만
확률 보정은 `training.py` 가 한다. 여기는 서점에 관한 규칙만 있다.

1. 매핑 - 서점 카테고리 문자열을 내부 카테고리로 옮긴다.
   운영이 DB(`category_keywords`)에서 키워드를 관리하므로 그것을 먼저 본다.
   실측에서 DB 91개 중 57개는 하드코딩 규칙과 같았고, 4개는 DB 가 더 구체적이었으며
   (`한문` -> `1_동양고전한문`), 30개는 하드코딩에 아예 없었다.

2. 결합 - 모델과 서점 중 어느 쪽을 따를지. `bookstore-policy` 명령이 확신도
   구간마다 두 쪽 정답률을 재서 경계 하나를 남긴다. 그 경계 미만이면 서점이 먼저다.

DB 가 없거나 죽어 있어도 판정은 계속돼야 한다. 그때는 하드코딩 규칙만 쓴다.
"""

import json
import logging
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple

logger = logging.getLogger(__name__)

# DB 를 파일마다 조회하면 재분류 2만 건에 2만 번 연결한다. 캐시하고 주기적으로만 다시 읽는다.
KEYWORD_CACHE_SECONDS = 300

_lock = threading.Lock()
_cached_keywords: Optional[Dict[str, List[str]]] = None
_cached_at = 0.0


def _load_from_db() -> Dict[str, List[str]]:
    """DB 의 카테고리-키워드 매핑을 {키워드: [카테고리...]} 로 뒤집는다.

    한 키워드가 여러 카테고리에 붙는 것은 정상이다. 테이블의 유일 키가
    (category, keyword, content_type) 세 값의 조합이고, `search_by_keyword` 도
    카테고리 목록을 돌려준다. `환상문학` 이 `3_SF` 와 `3_판타지` 에,
    `클래식` 이 `1_서양고전` 과 `5_음악` 에 있는 것은 어느 쪽도 틀리지 않다.

    그래서 버리지 않고 후보를 전부 남긴다. 어느 쪽인지는 서점 다수결이 가른다.
    """
    from backend.category_mapping import CategoryMapping

    mapping = CategoryMapping()
    out: Dict[str, List[str]] = {}
    for category, keywords in mapping.get_all_mappings("book").items():
        for keyword in keywords:
            word = (keyword or "").strip()
            if not word:
                continue
            bucket = out.setdefault(word, [])
            if category not in bucket:
                bucket.append(category)
    return out


def keyword_map(loader: Optional[Callable[[], Dict[str, List[str]]]] = None, force: bool = False) -> Dict[str, List[str]]:
    """DB 키워드 맵. 5분간 캐시한다. DB 를 못 읽으면 빈 맵을 준다(하드코딩 규칙으로 넘어간다)."""
    global _cached_keywords, _cached_at

    with _lock:
        fresh = _cached_keywords is not None and (time.time() - _cached_at) < KEYWORD_CACHE_SECONDS
        if fresh and not force:
            return _cached_keywords or {}
        try:
            _cached_keywords = (loader or _load_from_db)()
            _cached_at = time.time()
            logger.info("서점 매핑 키워드 %d개를 DB 에서 읽었다", len(_cached_keywords))
        except Exception as e:
            # DB 가 죽어도 판정은 계속돼야 한다. 다음 호출에서 다시 시도한다.
            logger.warning("DB 에서 키워드를 못 읽었다. 하드코딩 규칙만 쓴다: %s", e)
            _cached_keywords = {}
            _cached_at = time.time()
        return _cached_keywords or {}


def reset_keyword_cache() -> None:
    """캐시를 비운다. 키워드를 고친 뒤와 테스트에서 쓴다."""
    global _cached_keywords, _cached_at
    with _lock:
        _cached_keywords = None
        _cached_at = 0.0


def match_keyword(cat_str: str, keywords: Dict[str, List[str]]) -> List[str]:
    """서점 카테고리 문자열에 걸리는 DB 키워드의 카테고리 후보를 모은다.

    긴 키워드를 먼저 본다. '영어전문교육' 이 '영어' 보다 구체적이라 먼저 맞아야 한다.
    첫 키워드가 후보를 하나만 주면 거기서 끝낸다. 여러 개를 주면(`클래식` 처럼)
    후보를 그대로 돌려주고 서점 다수결이 가르게 둔다.
    """
    if not cat_str or not keywords:
        return []
    for word in sorted(keywords, key=len, reverse=True):
        if word in cat_str:
            return list(keywords[word])
    return []


def map_bookstore_category(cat_str: str, raw_title: str, raw_author: str, keywords: Optional[Dict[str, List[str]]] = None) -> Optional[str]:
    """서점이 준 카테고리를 내부 카테고리 하나로 옮긴다. 후보가 여럿이면 None 이다."""
    candidates = map_bookstore_candidates(cat_str, raw_title, raw_author, keywords)
    return candidates[0] if len(candidates) == 1 else None


def map_bookstore_candidates(cat_str: str, raw_title: str, raw_author: str, keywords: Optional[Dict[str, List[str]]] = None) -> List[str]:
    """서점이 준 카테고리를 내부 카테고리 후보 목록으로 옮긴다.

    순서가 중요하다.
    1. 저자·시리즈 전용 규칙 - 가장 구체적이다. 히가시노 게이고는 '일본소설' 이기 전에
       `2_소설일본게이고` 다. 카테고리 문자열을 비워 호출하면 그 절만 탄다.
    2. DB 키워드 - 운영이 관리하고 하드코딩보다 정밀하다. 한 키워드가 여러 카테고리에
       붙을 수 있고 그것이 정상이다. 후보를 전부 돌려준다.
    3. 하드코딩 규칙 전체 - DB 가 안 덮는 38개 카테고리를 메운다.
    """
    from backend.book_classifier import map_category

    specific = map_category("", raw_title, raw_author)
    if specific:
        return [specific]

    hits = match_keyword(cat_str, keyword_map() if keywords is None else keywords)
    if hits:
        return hits

    fallback = map_category(cat_str, raw_title, raw_author)
    return [fallback] if fallback else []


def quantile_bands(confidences: Sequence[float], count: int = 6, window: float = 0.5) -> List[float]:
    """확신도 분포를 보고 구간 경계를 고른다.

    고정값(0.5, 0.7, 0.8)으로 자르면 안 된다. 실측에서 2,000건 중 1,884건이 첫 구간
    하나에 몰렸다. 확신도는 79개 클래스에 softmax 를 씌운 값이라 중앙값이 0.13,
    판정 임계값이 0.056 이다. 0~1 에 고루 퍼진다는 가정이 틀렸다.

    그렇다고 분위수를 그대로 쓰면 값이 촘촘한 자리에서 경계가 그어져, 사실상 같은
    문서들이 두 구간으로 갈린다. 그래서 분위수 자리 근처에서 이웃 값 사이가 가장 크게
    벌어진 곳(밀도가 낮은 곳)으로 경계를 옮긴다.

    `window` 는 분위수 한 칸의 몇 배만큼 좌우를 살필지다. 0.5 면 이웃 분위수의 중간까지다.
    """
    values = sorted(float(c) for c in confidences)
    n = len(values)
    if n < 2 or count < 2:
        return []

    step = n / count
    reach = max(1, int(step * window))
    cuts: List[float] = []
    for i in range(1, count):
        center = int(round(i * step))
        lo = max(1, center - reach)
        hi = min(n - 1, center + reach)
        if lo >= hi:
            continue
        # 이웃한 두 값의 차이가 가장 큰 자리가 밀도가 가장 낮은 자리다.
        best = max(range(lo, hi + 1), key=lambda k: values[k] - values[k - 1])
        cut = (values[best] + values[best - 1]) / 2
        if not cuts or cut > cuts[-1]:
            cuts.append(cut)
    return cuts


def bands_from_cuts(cuts: Sequence[float]) -> List[Tuple[float, float]]:
    """경계값 목록을 (하한, 상한) 구간으로 바꾼다. 마지막 구간의 상한은 1 을 넘겨 둔다."""
    edges = [0.0] + [float(c) for c in cuts] + [1.01]
    return [(edges[i], edges[i + 1]) for i in range(len(edges) - 1)]


@dataclass
class BookstorePolicy:
    """모델과 서점 중 어느 쪽을 따를지 정하는 규칙.

    `override_below` 가 0.0 이면 서점이 이긴 구간이 없었다는 뜻이다. 그때는
    모델이 판정을 거부한 건에만 서점을 쓴다(예전 동작).
    """

    override_below: float = 0.0
    sample: int = 0
    bands: List[Dict[str, Any]] = field(default_factory=list)
    source: Optional[str] = None
    # 표본에서 실제로 본 가장 높은 확신도. 경계는 여기를 넘을 수 없다.
    measured_max: float = 0.0

    @classmethod
    def load(cls, path: Optional[Path | str] = None) -> "BookstorePolicy":
        p = Path(path) if path else default_policy_path()
        if not p.exists():
            logger.info("서점 규칙 파일이 없다. 서점은 모델이 거부한 건에만 쓴다: %s", p)
            return cls()
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as e:
            logger.warning("서점 규칙 파일을 못 읽었다 (%s): %s", p, e)
            return cls()
        measured_max = float(data.get("measured_max_confidence") or 0.0)
        override = float(data.get("bookstore_override_below") or 0.0)
        if measured_max and override > measured_max:
            # 옛 파일이거나 잘못 계산된 경우. 재지 않은 구간까지 서점에 넘기지 않는다.
            logger.warning("서점 경계 %.3f 가 측정 범위 %.3f 를 넘어 잘라 쓴다", override, measured_max)
            override = measured_max
        return cls(override_below=override, sample=int(data.get("sample") or 0), bands=data.get("bands") or [], source=str(p), measured_max=measured_max)

    def prefers_bookstore(self, confidence: float) -> bool:
        """이 확신도에서는 모델보다 서점을 믿는 것이 나은가."""
        return self.override_below > 0.0 and confidence < self.override_below


def default_policy_path() -> Path:
    """모델 파일 옆에 둔다. 학습 머신과 pod 가 같은 파일을 본다."""
    from backend.classifier.model import model_path

    return model_path().parent / "bookstore_policy.json"
