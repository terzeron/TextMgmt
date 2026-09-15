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
from typing import Any, Callable, Dict, List, Optional

logger = logging.getLogger(__name__)

# DB 를 파일마다 조회하면 재분류 2만 건에 2만 번 연결한다. 캐시하고 주기적으로만 다시 읽는다.
KEYWORD_CACHE_SECONDS = 300

_lock = threading.Lock()
_cached_keywords: Optional[Dict[str, str]] = None
_cached_at = 0.0


def _load_from_db() -> Dict[str, str]:
    """DB 의 카테고리-키워드 매핑을 {키워드: 카테고리} 로 뒤집는다.

    같은 키워드가 두 카테고리에 있으면 그 키워드는 버린다. 실측에서 `환상문학` 이
    `3_SF` 와 `3_판타지` 에, `클래식` 이 `1_서양고전` 과 `5_음악` 에 함께 있었다.
    둘 중 하나를 고르는 규칙은 자의적이라, 신호가 없는 것으로 보고 하드코딩 규칙에 넘긴다.
    """
    from backend.category_mapping import CategoryMapping

    mapping = CategoryMapping()
    seen: Dict[str, set] = {}
    for category, keywords in mapping.get_all_mappings("book").items():
        for keyword in keywords:
            word = (keyword or "").strip()
            if word:
                seen.setdefault(word, set()).add(category)

    out: Dict[str, str] = {}
    for word, categories in seen.items():
        if len(categories) > 1:
            logger.warning("키워드 '%s' 가 %s 에 함께 있다. 어느 쪽인지 정할 수 없어 쓰지 않는다", word, ", ".join(sorted(categories)))
            continue
        out[word] = next(iter(categories))
    return out


def keyword_map(loader: Optional[Callable[[], Dict[str, str]]] = None, force: bool = False) -> Dict[str, str]:
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


def match_keyword(cat_str: str, keywords: Dict[str, str]) -> Optional[str]:
    """서점 카테고리 문자열에서 DB 키워드를 찾는다.

    긴 키워드를 먼저 본다. '영어전문교육' 이 '영어' 보다 구체적이라 먼저 맞아야 한다.
    """
    if not cat_str or not keywords:
        return None
    for word in sorted(keywords, key=len, reverse=True):
        if word in cat_str:
            return keywords[word]
    return None


def map_bookstore_category(cat_str: str, raw_title: str, raw_author: str, keywords: Optional[Dict[str, str]] = None) -> Optional[str]:
    """서점이 준 카테고리를 내부 카테고리로 옮긴다.

    순서가 중요하다.
    1. 저자·시리즈 전용 규칙 - 가장 구체적이다. 히가시노 게이고는 '일본소설' 이기 전에
       `2_소설일본게이고` 다. 카테고리 문자열을 비워 호출하면 그 절만 탄다.
    2. DB 키워드 - 운영이 관리하고 하드코딩보다 정밀하다.
    3. 하드코딩 규칙 전체 - DB 가 안 덮는 38개 카테고리를 메운다.
    """
    from backend.book_classifier import map_category

    specific = map_category("", raw_title, raw_author)
    if specific:
        return specific

    hit = match_keyword(cat_str, keyword_map() if keywords is None else keywords)
    if hit:
        return hit

    return map_category(cat_str, raw_title, raw_author)


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
        return cls(override_below=float(data.get("bookstore_override_below") or 0.0), sample=int(data.get("sample") or 0), bands=data.get("bands") or [], source=str(p))

    def prefers_bookstore(self, confidence: float) -> bool:
        """이 확신도에서는 모델보다 서점을 믿는 것이 나은가."""
        return self.override_below > 0.0 and confidence < self.override_below


def default_policy_path() -> Path:
    """모델 파일 옆에 둔다. 학습 머신과 pod 가 같은 파일을 본다."""
    from backend.classifier.model import model_path

    return model_path().parent / "bookstore_policy.json"
