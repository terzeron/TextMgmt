#!/usr/bin/env python3
"""
학습 데이터 수집 - Elasticsearch 에서 읽는다

ES 는 inode 를 _id 로 써서 /mnt/data/text 의 모든 문서를 인덱싱해 두었다.
본문 앞부분(summary, 최대 4,096자), 제목, 저자, 출판사, 확장자, 카테고리가 모두 들어 있다.

디스크(/mnt/data)는 회전 디스크라 23만 건을 읽으면 몇 시간이 걸리지만
ES 에서 긁으면 45초다. 학습도 판정도 ES 를 먼저 본다.
"""

import json
import logging
import re
from pathlib import Path
from typing import Any, Dict, Iterable, Iterator, List, Optional

logger = logging.getLogger(__name__)

# ES 에서 가져올 필드. summary 가 본문 앞부분이다.
SOURCE_FIELDS = ["category", "summary", "title", "author", "publisher", "file_path", "file_type"]


def _document(hit: Dict[str, Any]) -> Dict[str, Any]:
    """ES 히트 하나를 학습/판정용 레코드로 바꾼다."""
    src = hit.get("_source") or {}
    # 카테고리는 경로일 수 있다(9_북스캔OCR/completed/...). 최상위만 레이블로 쓴다.
    category = (src.get("category") or "").split("/")[0]
    path = src.get("file_path") or ""
    return {
        "id": str(hit.get("_id") or ""),
        "cat": category,
        "text": src.get("summary") or "",
        "title": src.get("title") or "",
        "author": src.get("author") or "",
        "publisher": src.get("publisher") or "",
        "type": src.get("file_type") or "",
        "name": Path(path).name if path else "",
        "path": path,
    }


DEFAULT_LABEL_PATTERN = r"^[1-9]_"


def is_trainable(doc: Dict[str, Any], min_chars: int, excluded_prefixes: Iterable[str], label_pattern: str = DEFAULT_LABEL_PATTERN) -> bool:
    """
    학습에 쓸 수 있는 레코드인가. 레이블이 없거나 본문이 없으면 못 쓴다.

    레이블은 '숫자_이름' 꼴 디렉토리다. 이 꼴이 아닌 trash, _root, .preview_cache 는
    카테고리가 아니라 운영용 디렉토리라 학습에 넣으면 레이블을 오염시킨다.
    """
    cat = doc.get("cat") or ""
    if not cat or not re.match(label_pattern, cat) or cat.startswith(tuple(excluded_prefixes)):
        return False
    return len(doc.get("text") or "") >= min_chars


def absolute_path(doc: Dict[str, Any], library_root: Path | str) -> Optional[Path]:
    """
    레코드의 file_path 를 실제 경로로 바꾼다.

    loader 가 `file_path.relative_to(prefix)` 로 넣기 때문에 ES 의 값은 상대 경로다.
    그대로 열면 아무 파일도 못 찾는다(실측: EPUB 58,766건에서 publisher 0건).
    """
    raw = doc.get("path") or ""
    if not raw:
        return None
    p = Path(raw)
    return p if p.is_absolute() else Path(library_root) / p


def scan_documents(es_manager: Any, index: Optional[str] = None, batch: int = 2000) -> Iterator[Dict[str, Any]]:
    """인덱스 전체를 훑는다. 메모리에 다 올리지 않고 하나씩 내보낸다."""
    from elasticsearch import helpers

    es = es_manager.es
    idx = index or es_manager.index_name
    for hit in helpers.scan(es, index=idx, query={"query": {"match_all": {}}}, _source=SOURCE_FIELDS, size=batch, preserve_order=False):
        yield _document(hit)


def fetch_by_inode(es_manager: Any, inode: int | str, index: Optional[str] = None) -> Optional[Dict[str, Any]]:
    """
    inode 하나로 문서를 집어 온다. ES 의 _id 가 inode 라 곧바로 찾는다.

    판정할 때 이걸 쓰면 디스크를 안 읽는다. 없으면 None 을 돌려주고
    호출부가 파일에서 직접 읽는 경로로 넘어간다.
    """
    idx = index or es_manager.index_name
    try:
        hit = es_manager.es.get(index=idx, id=str(inode), _source=SOURCE_FIELDS)
    except Exception as e:
        logger.debug("inode %s 를 ES 에서 못 찾았다: %s", inode, e)
        return None
    return _document(hit)


def fetch_by_path(es_manager: Any, file_path: str, index: Optional[str] = None) -> Optional[Dict[str, Any]]:
    """경로로 찾는다. inode 가 바뀐 경우의 대비책이다. file_path 는 keyword 타입이다."""
    idx = index or es_manager.index_name
    try:
        res = es_manager.es.search(index=idx, query={"term": {"file_path": file_path}}, size=1, _source=SOURCE_FIELDS)
    except Exception as e:
        logger.debug("경로 %s 를 ES 에서 못 찾았다: %s", file_path, e)
        return None
    hits = (res.get("hits") or {}).get("hits") or []
    return _document(hits[0]) if hits else None


def write_jsonl(docs: Iterable[Dict[str, Any]], out_path: Path | str) -> int:
    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    n = 0
    with out.open("w", encoding="utf-8") as f:
        for d in docs:
            f.write(json.dumps(d, ensure_ascii=False) + "\n")
            n += 1
    return n


def read_jsonl(path: Path | str) -> List[Dict[str, Any]]:
    with Path(path).open(encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def collect(es_manager: Any, out_path: Path | str, min_chars: int, excluded_prefixes: Iterable[str], index: Optional[str] = None, label_pattern: str = DEFAULT_LABEL_PATTERN) -> Dict[str, Any]:
    """ES 를 훑어 학습용 JSONL 을 만든다. 돌려주는 값은 요약 통계다."""
    from collections import Counter

    prefixes = tuple(excluded_prefixes)
    counts: Counter = Counter()
    skipped = 0

    def _gen() -> Iterator[Dict[str, Any]]:
        nonlocal skipped
        for doc in scan_documents(es_manager, index=index):
            if not is_trainable(doc, min_chars, prefixes, label_pattern):
                skipped += 1
                continue
            counts[doc["cat"]] += 1
            yield doc

    kept = write_jsonl(_gen(), out_path)
    return {"kept": kept, "skipped": skipped, "categories": len(counts), "per_category": dict(counts.most_common())}


# ---------------------------------------------------------------------------
# publisher 보강
#
# publisher 는 loader 가 색인할 때 EPUB 의 OPF 에서 읽어 ES 에 넣는다. 전집 판정에
# 결정적인 증거다(을유문화사가 2_을유세계문학전집의 98.6%, 상위 장르 2_소설외국의 0.0%).
#
# 아직 다시 색인하지 않은 문서는 ES 에 값이 비어 있다. 그런 문서만 골라 파일에서
# 직접 읽어 메운다. 다시 색인하고 나면 이 뒷마감은 할 일이 없어진다.
#
# EPUB 을 여는 일이라 /mnt/data(회전 디스크)에서 느리다. 결과를 캐시에 남기고
# 다시 돌리면 캐시에 없는 것만 읽는다. 워커는 2개가 상한이다.
# 실측에서 8개는 1개보다 느렸다(I/O 바운드).
# ---------------------------------------------------------------------------

PUBLISHER_WORKERS = 2


def _publisher_of(path: Path) -> str:
    from backend.classifier.reader import read_epub_publisher

    try:
        return read_epub_publisher(path)
    except Exception:
        return ""


def attach_publishers(
    docs: List[Dict[str, Any]],
    cache_path: Path | str,
    library_root: Path | str,
    workers: int = PUBLISHER_WORKERS,
    progress_every: int = 5000,
) -> Dict[str, Any]:
    """ES 에 publisher 가 없는 EPUB 만 파일에서 읽어 메운다. 캐시에 있는 것은 다시 안 읽는다."""
    from concurrent.futures import ThreadPoolExecutor

    cache: Dict[str, str] = {}
    cp = Path(cache_path)
    if cp.exists():
        try:
            with cp.open(encoding="utf-8") as f:
                cache = json.load(f)
        except Exception as e:
            logger.warning("publisher 캐시를 못 읽어 새로 만든다 (%s): %s", cp, e)

    todo = [(d, absolute_path(d, library_root)) for d in docs if d.get("type") == "epub" and not d.get("publisher") and d["id"] not in cache]
    todo = [(d, p) for d, p in todo if p is not None]
    logger.info("파일에서 publisher 를 읽을 EPUB %d개 (ES 에 이미 있거나 캐시 적중 %d개)", len(todo), len(docs) - len(todo))

    if todo:
        with ThreadPoolExecutor(max_workers=max(1, workers)) as pool:
            for i, ((doc, _), pub) in enumerate(zip(todo, pool.map(_publisher_of, (p for _, p in todo))), 1):
                cache[doc["id"]] = pub
                if progress_every and i % progress_every == 0:
                    logger.info("  publisher %d/%d", i, len(todo))
        cp.parent.mkdir(parents=True, exist_ok=True)
        with cp.open("w", encoding="utf-8") as f:
            json.dump(cache, f, ensure_ascii=False)

    found = 0
    for d in docs:
        if not d.get("publisher"):
            d["publisher"] = cache.get(d["id"], "")
        if d["publisher"]:
            found += 1
    return {"read": len(todo), "cached": len(cache), "with_publisher": found}
