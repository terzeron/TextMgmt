#!/usr/bin/env python3
"""
detect_mojibake.py

라이브러리 전체에서 인코딩이 손상된 문서를 찾는다.

손상 유형은 원본 텍스트를 잘못된 코드페이지로 읽고 다시 UTF-8로 저장한 것이다.
파일 자체는 유효한 UTF-8이라 디코딩으로는 걸러지지 않고 내용만 깨진다.

    정상: 소림사 장로가 강호에서 내공을 운기조식하며
    손상: 轝듿닶쓽 蹂댁옣씠 솗蹂대릺吏 븘땲븯怨

판별은 `korean_likeness` 하나로 한다. 한글 음절 중 한국어에서 실제로 자주 쓰이는
음절이 차지하는 비율이다. 손상 텍스트의 한글은 희귀 음절이라 이 값이 0에 수렴한다.

    손상 확실       0.000 ~ 0.006
    정상 한국어      0.271 ~ 0.511
    한문 원전        None (한글 50자 미만이라 판정 대상 아님)

형태소 분석기를 쓰지 않아 가볍고, 20만 건 전수 검사에 쓸 수 있다.
한자가 많은 것 자체는 손상 근거가 아니다. 한문 원전은 한글이 애초에 없어
판정 대상에서 빠지고, 국한문 혼용은 한글이 정상이라 likeness 가 높게 나온다.

사용 예:
    uv run python utils/detect_mojibake.py scan
    uv run python utils/detect_mojibake.py scan --root /mnt/data/text/9_격언명언
    uv run python utils/detect_mojibake.py check --file '/mnt/data/text/9_격언명언/명언.txt'
"""

import argparse
import json
import logging
import os
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

LOGGER = logging.getLogger("detect_mojibake")

DEFAULT_LIBRARY_ROOT = Path("/mnt/data/text")
TEXT_EXTS = {".txt", ".epub"}

# 한국어에서 압도적으로 자주 쓰이는 음절. /mnt/data/text/nf_common.py 의
# COMMON_HANGUL_SYLLABLES 와 같은 집합을 쓴다. 그쪽 도구(inspect_epub.py)와
# 판정 기준을 어긋나게 두면 같은 파일에 서로 다른 진단이 나온다.
COMMON_HANGUL_SYLLABLES = frozenset("이다는을가지에그고하의아은어도로리나를한었기서들게니있라자사시만으해마보했수대것")

# 이 수보다 음절이 적으면 비율이 흔들려 판정하지 않는다.
MIN_SYLLABLES_FOR_LIKENESS = 50

# 손상 0.006 이하, 정상 0.271 이상. 사이가 비어 있어 경계를 넉넉히 잡는다.
MOJIBAKE_LIKENESS_THRESHOLD = 0.10

# 한글이 전체의 이 비율에 못 미치면 판정하지 않는다.
# 영문 OCR 문서에 한글 잡음이 60자쯤 섞이면 음절 수 하한은 넘지만, 문서의 0.2%를 보고
# 전체를 손상이라 부르는 셈이 된다. 실측에서 이 유형이 오탐 1,971건을 만들었다.
# 실제 손상 파일은 한글 비율 0.48~0.85, 정상 한국어는 0.40~0.70 이라 여유가 크다.
MIN_HANGUL_RATIO = 0.10

# 표본 크기. 앞부분만 보면 표지와 판권지에 속아 본문 손상을 놓친다.
DEFAULT_HEAD_CHARS = 20000
DEFAULT_TAIL_CHARS = 10000

# /mnt/data 는 회전 디스크라 워커를 늘려도 I/O 로 막힌다. 2개가 실측상 가장 빨랐다.
DEFAULT_WORKERS = 2

VERDICT_CORRUPTED = "corrupted"
VERDICT_CLEAN = "clean"
VERDICT_UNDETERMINED = "undetermined"
VERDICT_UNREADABLE = "unreadable"


def korean_likeness(text: str) -> Optional[float]:
    """이 글이 정상 한국어로 읽히는 정도. 잴 음절이 모자라면 None."""
    syllables = [ch for ch in text if "가" <= ch <= "힣"]
    if len(syllables) < MIN_SYLLABLES_FOR_LIKENESS:
        return None
    hits = sum(1 for ch in syllables if ch in COMMON_HANGUL_SYLLABLES)
    return hits / len(syllables)


def classify_text(text: str) -> Tuple[str, Optional[float]]:
    """본문 표본으로 손상 여부를 판정한다"""
    if not text or len(text) < 200:
        return VERDICT_UNREADABLE, None
    hangul = sum(1 for ch in text if "가" <= ch <= "힣")
    if hangul / len(text) < MIN_HANGUL_RATIO:
        # 한국어 문서가 아니다(한문 원전, 중국어, 일본어, 영문, 악보). 판정 대상이 아니다.
        return VERDICT_UNDETERMINED, None
    likeness = korean_likeness(text)
    if likeness is None:
        return VERDICT_UNDETERMINED, None
    if likeness < MOJIBAKE_LIKENESS_THRESHOLD:
        return VERDICT_CORRUPTED, likeness
    return VERDICT_CLEAN, likeness


def inspect_file(path_str: str) -> Dict[str, Any]:
    """파일 한 건을 읽어 판정한다. 워커 프로세스에서 호출된다."""
    from utils.corpus_lexicon import read_document_text  # noqa: PLC0415  워커에서 지연 import

    p = Path(path_str)
    try:
        text = read_document_text(p, DEFAULT_HEAD_CHARS, DEFAULT_TAIL_CHARS)
    except Exception as e:  # pragma: no cover - 손상 파일 의존
        return {"path": path_str, "verdict": VERDICT_UNREADABLE, "likeness": None, "error": str(e)}

    verdict, likeness = classify_text(text)
    try:
        size = p.stat().st_size
    except OSError:
        size = 0
    reason = ""
    if verdict == VERDICT_UNREADABLE:
        # 같은 '판독 불가'라도 원인이 다르다. 빈 파일과 본문 추출 실패를 구분해 둔다.
        reason = "missing_or_empty" if size == 0 else ("no_text_extracted" if not text else "too_short")
    return {"path": path_str, "category": p.relative_to(DEFAULT_LIBRARY_ROOT).parts[0] if str(p).startswith(str(DEFAULT_LIBRARY_ROOT)) else "", "verdict": verdict, "likeness": round(likeness, 4) if likeness is not None else None, "size": size, "sample_len": len(text), "reason": reason}


def list_library_files(root: Path) -> List[Path]:
    """라이브러리 전체의 txt/epub 목록. 도구 캐시 디렉토리는 뺀다."""
    skip_parts = {"__pycache__", ".preview_cache", ".ruff_cache", ".serena", ".agents", ".git"}
    files = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in skip_parts and not d.startswith(".")]
        for name in filenames:
            if name.startswith("."):
                continue
            if Path(name).suffix.lower() in TEXT_EXTS:
                files.append(Path(dirpath) / name)
    files.sort()
    return files


def scan(root: Path, workers: int = DEFAULT_WORKERS, limit: int = 0, progress_every: int = 2000) -> Dict[str, Any]:
    """라이브러리를 훑어 판정 결과를 모은다"""
    LOGGER.info(f"파일 목록 수집: {root}")
    files = list_library_files(root)
    if limit > 0:
        files = files[:limit]
    LOGGER.info(f"대상 {len(files):,}건, 워커 {workers}개")

    results: List[Dict[str, Any]] = []
    counts: Dict[str, int] = {}
    t0 = time.time()

    with ProcessPoolExecutor(max_workers=workers) as pool:
        for i, res in enumerate(pool.map(inspect_file, [str(p) for p in files], chunksize=64), 1):
            results.append(res)
            counts[res["verdict"]] = counts.get(res["verdict"], 0) + 1
            if i % progress_every == 0:
                rate = i / max(0.001, time.time() - t0)
                eta = (len(files) - i) / max(0.001, rate)
                LOGGER.info(f"  {i:,}/{len(files):,} ({i / len(files):.1%}) {rate:.0f}건/s ETA {eta / 60:.0f}분  손상 {counts.get(VERDICT_CORRUPTED, 0):,}")

    per_category: Dict[str, Dict[str, int]] = {}
    for r in results:
        cat = r.get("category") or "(기타)"
        bucket = per_category.setdefault(cat, {})
        bucket[r["verdict"]] = bucket.get(r["verdict"], 0) + 1

    return {
        "scanned_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "root": str(root),
        "threshold": MOJIBAKE_LIKENESS_THRESHOLD,
        "total": len(results),
        "counts": counts,
        "elapsed_sec": round(time.time() - t0, 1),
        "per_category": per_category,
        # clean 을 뺀 전부를 남긴다. 목록이 없으면 나중에 원인을 되짚을 수 없다.
        "corrupted": [r for r in results if r["verdict"] == VERDICT_CORRUPTED],
        "unreadable": [r for r in results if r["verdict"] == VERDICT_UNREADABLE],
        "undetermined": [r for r in results if r["verdict"] == VERDICT_UNDETERMINED],
    }


def print_summary(report: Dict[str, Any]) -> None:
    counts = report["counts"]
    total = report["total"] or 1
    print(f"\n검사 {report['total']:,}건 / {report['elapsed_sec'] / 60:.1f}분  (임계값 likeness < {report['threshold']})")
    for verdict in (VERDICT_CORRUPTED, VERDICT_CLEAN, VERDICT_UNDETERMINED, VERDICT_UNREADABLE):
        n = counts.get(verdict, 0)
        print(f"  {verdict:<14} {n:>8,}  ({n / total:.2%})")

    rows = []
    for cat, b in report["per_category"].items():
        bad = b.get(VERDICT_CORRUPTED, 0)
        if bad:
            rows.append((bad, sum(b.values()), cat))
    rows.sort(reverse=True)
    if rows:
        print(f"\n손상이 있는 카테고리 {len(rows)}개")
        print(f"  {'카테고리':<26}{'손상':>8}{'전체':>8}{'비율':>8}")
        for bad, tot, cat in rows[:40]:
            print(f"  {cat:<26}{bad:>8,}{tot:>8,}{bad / tot:>8.1%}")


def main() -> int:
    parser = argparse.ArgumentParser(description="인코딩 손상 문서 전수 검사")
    parser.add_argument("command", choices=["scan", "check"])
    parser.add_argument("--root", type=Path, default=DEFAULT_LIBRARY_ROOT)
    parser.add_argument("--file", type=Path, action="append", default=None)
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS)
    parser.add_argument("--limit", type=int, default=0, help="0 이면 전체")
    parser.add_argument("--report", type=Path, default=REPO_ROOT / "tmp" / "mojibake_report.json")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    if args.command == "check":
        if not args.file:
            LOGGER.error("--file 로 대상을 지정하라")
            return 1
        for f in args.file:
            print(json.dumps(inspect_file(str(f)), ensure_ascii=False))
        return 0

    report = scan(args.root, workers=args.workers, limit=args.limit)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print_summary(report)
    print(f"\n리포트: {args.report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
