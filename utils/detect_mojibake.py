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

리포트는 대상 디렉토리마다 다른 이름으로 tmp/ 에 쓴다(예: tmp/epub_structure_report_2_소설외국.json).
--report 로 경로를 직접 줄 수도 있다.

EPUB 구조 검사(`epub-structure`)는 인코딩과 별개다. epubcheck 는 XML 규격만 보므로
`<html:html>` 처럼 접두사가 붙은 본문도 통과시키지만, 뷰어의 정제기는 이를 거부한다.
뷰어가 렌더링하지 못하는 구조를 찾는다.
단 bad_mimetype 은 표준 위반일 뿐 뷰어를 깨뜨리지 않는다. 미리보기와 표지·삽화 편집이 mimetype 을
맨 앞에 무압축으로 다시 쓰기 때문이다. 원본을 그대로 읽는 다른 리더를 위한 표준 적합성 검사다.

    uv run python utils/detect_mojibake.py epub-structure --root /mnt/data/text/2_소설외국
    uv run python utils/detect_mojibake.py epub-structure --file '/mnt/data/text/2_소설Abe전집/73 ... .epub'
"""

import argparse
import json
import logging
import os
import posixpath
import re
import sys
import time
import zipfile
from concurrent.futures import ProcessPoolExecutor
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import unquote
from xml.etree import ElementTree

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from backend.classifier.reader import COMMON_HANGUL_SYLLABLES, MIN_SYLLABLES_FOR_LIKENESS, TEXT_HEAD_CHARS, TEXT_TAIL_CHARS, korean_likeness  # noqa: E402,F401

LOGGER = logging.getLogger("detect_mojibake")

DEFAULT_LIBRARY_ROOT = Path("/mnt/data/text")
TEXT_EXTS = {".txt", ".epub"}

# 손상 0.006 이하, 정상 0.271 이상. 사이가 비어 있어 경계를 넉넉히 잡는다.
MOJIBAKE_LIKENESS_THRESHOLD = 0.10

# 한글이 전체의 이 비율에 못 미치면 판정하지 않는다.
# 영문 OCR 문서에 한글 잡음이 60자쯤 섞이면 음절 수 하한은 넘지만, 문서의 0.2%를 보고
# 전체를 손상이라 부르는 셈이 된다. 실측에서 이 유형이 오탐 1,971건을 만들었다.
# 실제 손상 파일은 한글 비율 0.48~0.85, 정상 한국어는 0.40~0.70 이라 여유가 크다.
MIN_HANGUL_RATIO = 0.10

# 표본 크기. 앞부분만 보면 표지와 판권지에 속아 본문 손상을 놓친다.
DEFAULT_HEAD_CHARS = TEXT_HEAD_CHARS
DEFAULT_TAIL_CHARS = TEXT_TAIL_CHARS

# /mnt/data 는 회전 디스크라 워커를 늘려도 I/O 로 막힌다. 2개가 실측상 가장 빨랐다.
DEFAULT_WORKERS = 2

VERDICT_CORRUPTED = "corrupted"
VERDICT_CLEAN = "clean"
VERDICT_UNDETERMINED = "undetermined"
VERDICT_UNREADABLE = "unreadable"

EPUB_MIMETYPE = b"application/epub+zip"
ISSUE_NOT_ZIP = "not_zip"
ISSUE_BAD_MIMETYPE = "bad_mimetype"
ISSUE_NO_CONTAINER = "no_container"
ISSUE_OPF_MISSING = "opf_missing"
ISSUE_OPF_UNPARSABLE = "opf_unparsable"
ISSUE_SPINE_MEMBER_MISSING = "spine_member_missing"
ISSUE_PREFIXED_ROOT = "prefixed_root"
EPUB_ISSUES = (ISSUE_NOT_ZIP, ISSUE_BAD_MIMETYPE, ISSUE_NO_CONTAINER, ISSUE_OPF_MISSING, ISSUE_OPF_UNPARSABLE, ISSUE_SPINE_MEMBER_MISSING, ISSUE_PREFIXED_ROOT)

# 루트 태그는 파일 앞부분에 있다. DOCTYPE 내부 선언(<!ELEMENT)과 주석은 건너뛴다.
ROOT_TAG_HEAD_BYTES = 4096
ROOT_TAG_RE = re.compile(rb"<(?![?!])([^\s/>]+)")
COMMENT_RE = re.compile(rb"<!--.*?-->", re.DOTALL)
# container.xml, OPF 는 작다. 비정상적으로 큰 멤버는 읽지 않는다.
MAX_XML_BYTES = 5 * 1024 * 1024


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
    from backend.classifier.reader import read_document_text  # noqa: PLC0415  워커에서 지연 import

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


def default_report_path(kind: str, root: Path) -> Path:
    """대상 디렉토리마다 다른 리포트 경로. 다른 디렉토리를 스캔해도 이전 리포트를 덮어쓰지 않는다.

    라이브러리 하위는 상대 경로(2_소설외국, 하위는 __ 로 잇는다), 라이브러리 루트는 all,
    라이브러리 밖은 디렉토리 이름을 쓴다.
    """
    root = Path(os.path.abspath(root))
    try:
        parts = root.relative_to(DEFAULT_LIBRARY_ROOT).parts
    except ValueError:
        parts = (root.name,)
    slug = "__".join(re.sub(r"[^\w.-]+", "_", part) for part in parts) or "all"
    return REPO_ROOT / "tmp" / f"{kind}_report_{slug}.json"


def _category_of(p: Path) -> str:
    return p.relative_to(DEFAULT_LIBRARY_ROOT).parts[0] if str(p).startswith(str(DEFAULT_LIBRARY_ROOT)) else ""


def _read_member(zf: zipfile.ZipFile, name: str, limit: int = MAX_XML_BYTES) -> Optional[bytes]:
    """ZIP 멤버를 읽는다. 없거나 깨졌거나 limit 를 넘으면 None"""
    try:
        with zf.open(name) as f:
            data = f.read(limit + 1)
    except Exception:  # noqa: BLE001  손상 ZIP 은 KeyError/BadZipFile/zlib.error/RuntimeError 등으로 제각각 실패한다
        return None
    return None if len(data) > limit else data


def _local_name(tag: str) -> str:
    return tag.rpartition("}")[2]


def _root_tag_name(head: bytes) -> Optional[str]:
    """XML 문서 루트 요소의 원래 이름(접두사 포함)"""
    m = ROOT_TAG_RE.search(COMMENT_RE.sub(b"", head))
    return m.group(1).decode("utf-8", errors="replace") if m else None


def _spine_members(opf: ElementTree.Element, opf_path: str) -> List[str]:
    """spine 순서대로 본문 문서의 ZIP 멤버 경로를 돌려준다"""
    manifest = {el.get("id"): el.get("href") for el in opf.iter() if _local_name(el.tag) == "item" and el.get("href")}
    base = posixpath.dirname(opf_path)
    members = []
    for el in opf.iter():
        if _local_name(el.tag) == "itemref" and manifest.get(el.get("idref")):
            href = unquote(manifest[el.get("idref")]).split("#")[0]
            members.append(posixpath.normpath(posixpath.join(base, href)))
    return members


def inspect_epub_structure(path_str: str) -> Dict[str, Any]:
    """EPUB 한 건의 구조를 검사한다. 뷰어가 렌더링하지 못할 구조만 찾는다.

    issues 는 중복 없는 코드 목록이고, examples 는 코드별 첫 사례 멤버다.
    """
    p = Path(path_str)
    issues: List[str] = []
    examples: Dict[str, str] = {}

    def flag(code: str, member: str = "") -> None:
        if code not in issues:
            issues.append(code)
            examples[code] = member

    def result() -> Dict[str, Any]:
        return {"path": path_str, "category": _category_of(p), "issues": issues, "examples": examples}

    try:
        zf = zipfile.ZipFile(path_str)
    except (zipfile.BadZipFile, OSError):
        flag(ISSUE_NOT_ZIP)
        return result()

    with zf:
        infos = zf.infolist()
        first = infos[0] if infos else None
        mimetype = _read_member(zf, "mimetype", 64) if first is not None and first.filename == "mimetype" else None
        if first is None or first.compress_type != zipfile.ZIP_STORED or (mimetype or b"").strip() != EPUB_MIMETYPE:
            flag(ISSUE_BAD_MIMETYPE, first.filename if first else "")

        container = _read_member(zf, "META-INF/container.xml")
        if container is None:
            flag(ISSUE_NO_CONTAINER, "META-INF/container.xml")
            return result()
        try:
            rootfile = next(el for el in ElementTree.fromstring(container).iter() if _local_name(el.tag) == "rootfile")
            opf_path = posixpath.normpath(unquote(rootfile.get("full-path", "")))
        except (ElementTree.ParseError, StopIteration):
            flag(ISSUE_NO_CONTAINER, "META-INF/container.xml")
            return result()

        opf_bytes = _read_member(zf, opf_path)
        if opf_bytes is None:
            flag(ISSUE_OPF_MISSING, opf_path)
            return result()
        try:
            members = _spine_members(ElementTree.fromstring(opf_bytes), opf_path)
        except ElementTree.ParseError:
            flag(ISSUE_OPF_UNPARSABLE, opf_path)
            return result()

        names = set(zf.namelist())
        for member in members:
            if member not in names:
                flag(ISSUE_SPINE_MEMBER_MISSING, member)
                continue
            with zf.open(member) as f:
                root_name = _root_tag_name(f.read(ROOT_TAG_HEAD_BYTES))
            if root_name and ":" in root_name:
                flag(ISSUE_PREFIXED_ROOT, member)

    return result()


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


def scan_epub_structure(root: Path, workers: int = DEFAULT_WORKERS, limit: int = 0) -> Dict[str, Any]:
    """라이브러리의 EPUB 구조를 훑어 문제가 있는 파일 목록을 모은다"""
    files = [p for p in list_library_files(root) if p.suffix.lower() == ".epub"]
    if limit > 0:
        files = files[:limit]
    LOGGER.info(f"EPUB {len(files):,}건, 워커 {workers}개")

    t0 = time.time()
    with ProcessPoolExecutor(max_workers=workers) as pool:
        results = list(pool.map(inspect_epub_structure, [str(p) for p in files], chunksize=64))

    flagged = [r for r in results if r["issues"]]
    counts = {code: sum(1 for r in flagged if code in r["issues"]) for code in EPUB_ISSUES}
    return {"scanned_at": datetime.now(timezone.utc).isoformat(timespec="seconds"), "root": str(root), "total": len(results), "flagged_count": len(flagged), "counts": counts, "elapsed_sec": round(time.time() - t0, 1), "flagged": flagged}


def print_epub_structure_summary(report: Dict[str, Any]) -> None:
    total = report["total"] or 1
    print(f"\nEPUB {report['total']:,}건 검사 / {report['elapsed_sec'] / 60:.1f}분, 문제 {report['flagged_count']:,}건 ({report['flagged_count'] / total:.2%})")
    for code, n in report["counts"].items():
        if n:
            print(f"  {code:<22}{n:>8,}")


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
    parser.add_argument("command", choices=["scan", "check", "epub-structure"])
    parser.add_argument("--root", type=Path, default=DEFAULT_LIBRARY_ROOT)
    parser.add_argument("--file", type=Path, action="append", default=None)
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS)
    parser.add_argument("--limit", type=int, default=0, help="0 이면 전체")
    parser.add_argument("--report", type=Path, default=None, help="기본값은 대상 디렉토리별 tmp/mojibake_report_<디렉토리>.json, epub-structure 는 tmp/epub_structure_report_<디렉토리>.json")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    if args.command == "epub-structure":
        if args.file:
            for f in args.file:
                print(json.dumps(inspect_epub_structure(str(f)), ensure_ascii=False))
            return 0
        report = scan_epub_structure(args.root, workers=args.workers, limit=args.limit)
        report_path = args.report or default_report_path("epub_structure", args.root)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print_epub_structure_summary(report)
        print(f"\n리포트: {report_path}")
        return 0

    if args.command == "check":
        if not args.file:
            LOGGER.error("--file 로 대상을 지정하라")
            return 1
        for f in args.file:
            print(json.dumps(inspect_file(str(f)), ensure_ascii=False))
        return 0

    report = scan(args.root, workers=args.workers, limit=args.limit)
    report_path = args.report or default_report_path("mojibake", args.root)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print_summary(report)
    print(f"\n리포트: {report_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
