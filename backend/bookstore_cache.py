#!/usr/bin/env python3
"""
서점 응답 캐시 - 같은 책을 두 번 묻지 않는다

`classification_cache.json` 은 파일 경로로 묶여 있다. 같은 책이 다른 파일명으로
들어오면 서점 3곳을 처음부터 다시 부른다. 한 건에 3.6초(조회 3회 + 예의상 두는
간격)가 들고 서점 쪽에도 부담이다.

그래서 서점 응답만 따로 모은다. 키는 두 가지다.

  isbn:<isbn>    가장 확실하다. 제목 표기가 달라도 같은 책을 가리킨다.
  title:<제목>   ISBN 을 모를 때 쓴다. 서점 검색에 넣은 제목 그대로다.

ISBN 은 서점 응답에서 얻는다. 쓸 때 두 키에 모두 넣고, 읽을 때는 ISBN 을 먼저 본다.

파일은 JSON 이 아니라 JSONL 이다. 제안 생성은 서비스를 매번 새로 만들고 웹 경로는
`save_cache()` 를 부르지 않는다. 기록이 파일에 바로 남지 않으면 캐시 구실을 못 한다.
그렇다고 한 건마다 전체를 다시 쓰면 11만 건짜리 파일에서 감당이 안 된다. 줄 하나를
덧붙이는 것은 건수와 무관하게 일정하다.
"""

import json
import logging
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

logger = logging.getLogger(__name__)

STORE_NAMES = ("yes24", "aladin", "kyobo")


def _isbn_key(isbn: str) -> str:
    return f"isbn:{isbn.strip()}"


def _title_key(title: str) -> str:
    return f"title:{title.strip()}"


def _isbn_from_stores(stores: Dict[str, Dict[str, Any]]) -> str:
    """서점 응답에서 ISBN 을 고른다. 먼저 답한 쪽을 쓴다 - 서점끼리 다를 일이 드물다."""
    for name in STORE_NAMES:
        isbn = (stores.get(name) or {}).get("isbn") or ""
        if isbn.strip():
            return isbn.strip()
    return ""


def _has_any_answer(stores: Dict[str, Dict[str, Any]]) -> bool:
    """서점 중 한 곳이라도 카테고리를 줬는가."""
    return any((stores.get(name) or {}).get("cat") for name in STORE_NAMES)


class BookstoreResponseCache:
    """서점 3곳의 응답을 ISBN·제목으로 찾아 쓰는 캐시."""

    def __init__(self, path: Path | str):
        self.path = Path(path)
        self._entries: Dict[str, Dict[str, Dict[str, Any]]] = {}
        self._load()

    def _load(self) -> None:
        if not self.path.exists():
            return
        try:
            with self.path.open(encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        record = json.loads(line)
                    except json.JSONDecodeError:
                        # 쓰다 죽으면 마지막 줄이 잘린다. 그 줄만 버리고 나머지는 쓴다.
                        logger.debug("서점 캐시의 깨진 줄을 건너뛴다 (%s)", self.path)
                        continue
                    stores = record.get("stores")
                    if not isinstance(stores, dict):
                        continue
                    for key in record.get("keys") or []:
                        # 나중 줄이 이긴다. 같은 책을 다시 물었으면 새 답이 맞다.
                        self._entries[key] = stores
        except OSError as e:
            logger.warning("서점 캐시를 읽지 못했다 (%s): %s", self.path, e)

    def get(self, isbn: str = "", title: str = "") -> Optional[Dict[str, Dict[str, Any]]]:
        """ISBN 을 먼저 보고, 없으면 제목으로 찾는다. 없으면 None."""
        if isbn and isbn.strip():
            hit = self._entries.get(_isbn_key(isbn))
            if hit is not None:
                return hit
        if title and title.strip():
            return self._entries.get(_title_key(title))
        return None

    def put(self, stores: Dict[str, Dict[str, Any]], isbn: str = "", title: str = "") -> None:
        """서점 응답을 남긴다. 전부 빈손이면 남기지 않는다.

        다 비었을 때 그것이 '정말 없는 책' 인지 '지금 망이 끊긴 것' 인지 구분할 방법이
        없다. 남기면 일시적인 실패가 영구 오답이 된다.
        """
        if not _has_any_answer(stores):
            return

        keys: List[str] = []
        effective_isbn = isbn.strip() if isbn and isbn.strip() else _isbn_from_stores(stores)
        if effective_isbn:
            keys.append(_isbn_key(effective_isbn))
        if title and title.strip():
            keys.append(_title_key(title))
        if not keys:
            return

        for key in keys:
            self._entries[key] = stores
        self._append({"keys": keys, "stores": stores, "cached_at": datetime.now(timezone.utc).isoformat()})

    def _append(self, record: Dict[str, Any]) -> None:
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            with self.path.open("a", encoding="utf-8") as f:
                f.write(json.dumps(record, ensure_ascii=False) + "\n")
        except OSError as e:
            # 캐시를 못 써도 판정은 계속돼야 한다. 다음 번에 서점을 다시 부를 뿐이다.
            logger.warning("서점 캐시에 쓰지 못했다 (%s): %s", self.path, e)
