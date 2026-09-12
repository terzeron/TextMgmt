#!/usr/bin/env python3
"""
corpus_lexicon.py

/mnt/data/text 의 `번호_카테고리명` 디렉토리를 샘플링하여
카테고리별 단어 출현빈도(문서빈도 기준) top-N 사전과 고유 단어 세트를 만든다.

산출물은 `backend/data/category_lexicon.json.gz` 이며
`backend.category_lexicon.CategoryLexicon` 이 런타임에 읽는다.

사용 예:
    # 1단계: 카테고리별 문서빈도 수집 (중단 후 재개 가능)
    uv run python utils/corpus_lexicon.py collect

    # 2단계: 수집 결과로 사전 빌드
    uv run python utils/corpus_lexicon.py build

    # 3단계: 홀드아웃으로 분류 정확도 측정
    uv run python utils/corpus_lexicon.py evaluate
"""

import argparse
import gzip
import json
import logging
import math
import random
import re
import sys
import time
import zipfile
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple, TypeVar

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from backend.category_lexicon import (  # noqa: E402
    DEFAULT_LEXICON_PATH,
    UNIQUE_BONUS,
    CategoryLexicon,
    extract_word_sets,
    get_kiwi,
)

LOGGER = logging.getLogger("corpus_lexicon")

DEFAULT_LIBRARY_ROOT = Path("/mnt/data/text")
DEFAULT_WORK_DIR = REPO_ROOT / "tmp" / "corpus_lexicon"

TEXT_EXTS = {".txt", ".epub"}

# `0_*`는 분류 입력 스테이징(0_telegram 등)이지 카테고리가 아니므로 제외한다.
CATEGORY_DIR_RE = re.compile(r"^[1-9]_")

# 샘플링 기본값. 3_판타지 78,027건 대 9_격언명언 7건의 불균형에 상한을 건다.
DEFAULT_MAX_FILES = 300
DEFAULT_MIN_FILES = 20
DEFAULT_HEAD_CHARS = 20000
DEFAULT_TAIL_CHARS = 10000
DEFAULT_TOP_N = 1000
DEFAULT_SEED = 20260912
DEFAULT_BATCH_SIZE = 16

# 사전 선정 임계값
CF_PRESENCE = 0.03  # 이 비율 이상 등장해야 해당 카테고리에 "있다"고 본다
UNIQUE_EXCLUSIVITY = 0.8  # 전 카테고리 출현률 합에서 차지하는 몫
UNIQUE_MIN_PRESENCE = 0.05
HOLDOUT_RATIO = 0.2

_T = TypeVar("_T", str, Path)

_TAG_RE = re.compile(r"<[^>]+>")
_WS_RE = re.compile(r"\s+")
_HANGUL_RE = re.compile(r"[가-힣]")


# ---------------------------------------------------------------------------
# 코퍼스 읽기
# ---------------------------------------------------------------------------


def list_category_dirs(library_root: Path) -> List[str]:
    """`번호_카테고리명` 형식이면서 0번대가 아닌 디렉토리 이름 목록"""
    if not library_root.is_dir():
        return []
    return sorted(d.name for d in library_root.iterdir() if d.is_dir() and CATEGORY_DIR_RE.match(d.name))


def list_category_files(cat_dir: Path) -> List[Path]:
    """카테고리 디렉토리 아래의 txt/epub 파일 목록 (경로 정렬로 결정론 확보)"""
    files = [p for p in cat_dir.rglob("*") if p.is_file() and p.suffix.lower() in TEXT_EXTS and not p.name.startswith(".")]
    files.sort()
    return files


def read_txt_text(fpath: Path, head_chars: int, tail_chars: int) -> str:
    """TXT 앞부분과 뒷부분을 인코딩 추정하여 읽는다"""
    try:
        size = fpath.stat().st_size
    except OSError:
        return ""

    def decode(raw: bytes) -> str:
        for enc in ("utf-8", "cp949", "euc-kr"):
            try:
                text = raw.decode(enc, errors="ignore")
            except Exception:
                continue
            if _HANGUL_RE.search(text):
                return text
        return raw.decode("utf-8", errors="ignore")

    parts: List[str] = []
    try:
        with open(fpath, "rb") as f:
            parts.append(decode(f.read(head_chars * 3))[:head_chars])
            if tail_chars > 0 and size > head_chars * 3:
                f.seek(max(0, size - tail_chars * 3))
                parts.append(decode(f.read())[-tail_chars:])
    except OSError:
        return ""
    return " ".join(parts)


def read_epub_text(fpath: Path, head_chars: int, tail_chars: int) -> str:
    """EPUB 본문 앞쪽/뒤쪽 챕터의 태그를 걷어낸 텍스트"""
    try:
        with zipfile.ZipFile(fpath, "r") as z:
            names = [n for n in z.namelist() if n.lower().endswith((".html", ".xhtml", ".htm"))]
            if not names:
                return ""
            body = [n for n in names if not any(k in n.lower() for k in ("cover", "nav", "toc", "titlepage"))]
            names = body or names

            def gather(chunk_names: List[str], budget: int) -> str:
                out: List[str] = []
                total = 0
                for n in chunk_names:
                    if total >= budget:
                        break
                    try:
                        raw = z.read(n).decode("utf-8", errors="ignore")
                    except Exception:
                        continue
                    clean = _WS_RE.sub(" ", _TAG_RE.sub(" ", raw)).strip()
                    if len(clean) < 50:
                        continue
                    out.append(clean)
                    total += len(clean)
                return " ".join(out)[:budget]

            head = gather(names, head_chars)
            tail = gather(list(reversed(names)), tail_chars) if tail_chars > 0 else ""
            return (head + " " + tail).strip()
    except (zipfile.BadZipFile, OSError, RuntimeError):
        return ""


def read_document_text(fpath: Path, head_chars: int = DEFAULT_HEAD_CHARS, tail_chars: int = DEFAULT_TAIL_CHARS) -> str:
    """확장자에 맞춰 본문 표본을 읽는다"""
    ext = fpath.suffix.lower()
    if ext == ".txt":
        return read_txt_text(fpath, head_chars, tail_chars)
    if ext == ".epub":
        return read_epub_text(fpath, head_chars, tail_chars)
    return ""


def sample_files(files: Sequence[_T], max_files: int, seed: int, salt: str) -> List[_T]:
    """카테고리별로 재현 가능한 무작위 표본을 뽑는다"""
    if max_files <= 0 or len(files) <= max_files:
        return list(files)
    rng = random.Random(f"{seed}:{salt}")
    return sorted(rng.sample(list(files), max_files))


def split_holdout(files: Sequence[_T], ratio: float, seed: int, salt: str) -> Tuple[List[_T], List[_T]]:
    """표본을 학습용과 홀드아웃으로 나눈다. 코퍼스 디렉토리 자체가 정답 레이블이다."""
    if ratio <= 0.0 or len(files) < 5:
        return list(files), []
    rng = random.Random(f"holdout:{seed}:{salt}")
    shuffled = list(files)
    rng.shuffle(shuffled)
    n_hold = max(1, int(len(shuffled) * ratio))
    return sorted(shuffled[n_hold:]), sorted(shuffled[:n_hold])


# ---------------------------------------------------------------------------
# 1단계: 카테고리별 문서빈도 수집
# ---------------------------------------------------------------------------


def collect_category(cat: str, cat_dir: Path, work_dir: Path, max_files: int = DEFAULT_MAX_FILES, min_files: int = DEFAULT_MIN_FILES, head_chars: int = DEFAULT_HEAD_CHARS, tail_chars: int = DEFAULT_TAIL_CHARS, seed: int = DEFAULT_SEED, holdout_ratio: float = HOLDOUT_RATIO, batch_size: int = DEFAULT_BATCH_SIZE, kiwi: Any = None) -> Optional[Dict[str, Any]]:
    """
    한 카테고리의 문서빈도를 세어 `{work_dir}/{cat}.json.gz` 로 저장하고 결과를 반환한다.
    파일 수가 min_files 미만이면 건너뛴다.
    """
    files = list_category_files(cat_dir)
    if len(files) < min_files:
        LOGGER.info(f"{cat}: 파일 {len(files)}건 < 최소 {min_files}건, 건너뜀")
        return None

    sampled = sample_files(files, max_files, seed, cat)
    train, holdout = split_holdout(sampled, holdout_ratio, seed, cat)

    df: Counter[str] = Counter()
    n_docs = 0
    t0 = time.time()
    # kiwi는 리스트를 넘겨야 워커가 병렬로 돈다. 배치 단위로 읽고 한 번에 토큰화한다.
    for start in range(0, len(train), batch_size):
        texts = [read_document_text(p, head_chars, tail_chars) for p in train[start : start + batch_size]]
        texts = [t for t in texts if t and len(t) >= 200]
        if not texts:
            continue
        for words in extract_word_sets(texts, kiwi=kiwi):
            if not words:
                continue
            df.update(words)
            n_docs += 1

    if n_docs < min_files:
        LOGGER.info(f"{cat}: 본문 추출 성공 {n_docs}건 < 최소 {min_files}건, 건너뜀")
        return None

    result = {"category": cat, "total_files": len(files), "sampled_files": len(sampled), "doc_count": n_docs, "df": dict(df), "holdout": [str(p) for p in holdout], "elapsed_sec": round(time.time() - t0, 1)}
    work_dir.mkdir(parents=True, exist_ok=True)
    out_path = work_dir / f"{cat}.json.gz"
    with gzip.open(out_path, "wt", encoding="utf-8") as f:
        json.dump(result, f, ensure_ascii=False)
    LOGGER.info(f"{cat}: 문서 {n_docs}건, 고유단어 {len(df)}개, {result['elapsed_sec']}초 -> {out_path.name}")
    return result


def collect_all(library_root: Path, work_dir: Path, categories: Optional[List[str]] = None, resume: bool = True, num_workers: int = 8, **kwargs: Any) -> List[str]:
    """전체 카테고리를 순회하며 수집한다. 이미 만들어진 카테고리는 건너뛴다."""
    cats = categories or list_category_dirs(library_root)
    kiwi = get_kiwi(num_workers=num_workers)
    if kiwi is None:
        raise RuntimeError("kiwipiepy 를 로드하지 못했다. `uv add kiwipiepy` 상태를 확인하라.")

    done: List[str] = []
    for i, cat in enumerate(cats, 1):
        cache = work_dir / f"{cat}.json.gz"
        if resume and cache.exists():
            LOGGER.info(f"[{i}/{len(cats)}] {cat}: 캐시 존재, 건너뜀")
            done.append(cat)
            continue
        LOGGER.info(f"[{i}/{len(cats)}] {cat}: 수집 시작")
        if collect_category(cat, library_root / cat, work_dir, kiwi=kiwi, **kwargs):
            done.append(cat)
    return done


def load_collected(work_dir: Path) -> Dict[str, Dict[str, Any]]:
    """수집 캐시를 모두 읽어들인다"""
    out: Dict[str, Dict[str, Any]] = {}
    if not work_dir.is_dir():
        return out
    for p in sorted(work_dir.glob("*.json.gz")):
        try:
            with gzip.open(p, "rt", encoding="utf-8") as f:
                entry = json.load(f)
        except Exception as e:
            LOGGER.warning(f"수집 캐시 손상 무시 ({p.name}): {e}")
            continue
        out[entry["category"]] = entry
    return out


# ---------------------------------------------------------------------------
# 2단계: 사전 빌드
# ---------------------------------------------------------------------------


def build_lexicon(collected: Dict[str, Dict[str, Any]], top_n: int = DEFAULT_TOP_N, cf_presence: float = CF_PRESENCE, unique_exclusivity: float = UNIQUE_EXCLUSIVITY, unique_min_presence: float = UNIQUE_MIN_PRESENCE) -> Dict[str, Any]:
    """
    카테고리별 문서빈도에서 카테고리 단위 TF-IDF 가중치를 계산해 사전을 만든다.

        p(c,w)      = w를 포함한 c의 문서 수 / c의 문서 수
        cf(w)       = p(c,w) >= cf_presence 인 카테고리 수
        weight(c,w) = p(c,w) * log(C / cf(w))

    `사람`, `생각` 같은 범용 명사는 모든 카테고리에 나타나 log(C/C)=0 으로 자동 소거된다.
    """
    cats = sorted(collected)
    n_cats = len(cats)
    if n_cats == 0:
        raise ValueError("수집 결과가 비어 있다")

    presence: Dict[str, Dict[str, float]] = {}
    for cat in cats:
        entry = collected[cat]
        n_docs = int(entry["doc_count"])
        presence[cat] = {w: c / n_docs for w, c in entry["df"].items()}

    # cf(w): 유의미하게 등장하는 카테고리 수, presence_sum(w): 전 카테고리 출현률 합
    cf: Counter[str] = Counter()
    presence_sum: Dict[str, float] = defaultdict(float)
    for cat in cats:
        for w, p in presence[cat].items():
            presence_sum[w] += p
            if p >= cf_presence:
                cf[w] += 1

    categories_out: Dict[str, Any] = {}
    for cat in cats:
        scored: List[Tuple[str, float]] = []
        for w, p in presence[cat].items():
            n_present = cf.get(w, 0)
            if n_present == 0:
                # 어느 카테고리에서도 임계치를 못 넘은 희귀어. 자기 카테고리에만 존재한다고 본다.
                n_present = 1
            idf = math.log(n_cats / n_present)
            if idf <= 0.0:
                continue
            weight = p * idf
            if weight <= 0.0:
                continue
            scored.append((w, weight))

        scored.sort(key=lambda kv: (-kv[1], kv[0]))
        top = scored[:top_n]
        if not top:
            LOGGER.warning(f"{cat}: 변별력 있는 단어가 없어 사전에서 제외")
            continue

        words = {w: round(v, 6) for w, v in top}
        unique = sorted(w for w, _ in top if presence[cat][w] >= unique_min_presence and presence_sum[w] > 0 and (presence[cat][w] / presence_sum[w]) >= unique_exclusivity)
        norm = math.sqrt(sum(v * v for v in words.values())) or 1.0

        categories_out[cat] = {"doc_count": int(collected[cat]["doc_count"]), "total_files": int(collected[cat].get("total_files", 0)), "norm": round(norm, 6), "words": words, "unique": unique}

    return {
        "version": 1,
        "built_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "params": {"top_n": top_n, "cf_presence": cf_presence, "unique_exclusivity": unique_exclusivity, "unique_min_presence": unique_min_presence, "unique_bonus": UNIQUE_BONUS, "category_count": len(categories_out)},
        "categories": categories_out,
    }


def write_lexicon(lexicon: Dict[str, Any], out_path: Path) -> Path:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(out_path, "wt", encoding="utf-8") as f:
        json.dump(lexicon, f, ensure_ascii=False, separators=(",", ":"))
    return out_path


# ---------------------------------------------------------------------------
# 3단계: 홀드아웃 평가
# ---------------------------------------------------------------------------


def evaluate_holdout(lexicon_path: Path, collected: Dict[str, Dict[str, Any]], limit_per_category: int = 30, head_chars: int = DEFAULT_HEAD_CHARS, tail_chars: int = DEFAULT_TAIL_CHARS, num_workers: int = 8) -> Dict[str, Any]:
    """
    학습에서 제외한 홀드아웃 파일로 기존 로직과 신규 어휘 사전의 top-1 정확도를 비교한다.
    서점 조회 없이 파일 내용만 쓴다.
    """
    from backend.book_classifier import classify_5_genres_from_content  # noqa: PLC0415

    lex = CategoryLexicon.load(lexicon_path)
    if lex is None:
        raise RuntimeError(f"사전을 읽지 못했다: {lexicon_path}")
    kiwi = get_kiwi(num_workers=num_workers)

    per_cat: Dict[str, Dict[str, int]] = {}
    totals: Counter[str] = Counter()
    confusion: Counter[Tuple[str, str]] = Counter()

    for cat in sorted(collected):
        holdout = [Path(p) for p in collected[cat].get("holdout", [])][:limit_per_category]
        if not holdout:
            continue
        stat: Counter[str] = Counter()
        for fpath in holdout:
            if not fpath.exists():
                continue
            text = read_document_text(fpath, head_chars, tail_chars)
            if not text or len(text) < 200:
                continue
            stat["n"] += 1
            totals["n"] += 1

            ranked = lex.rank(text, top_k=1, kiwi=kiwi)
            lex_cat = ranked[0][0] if ranked else None
            if lex_cat == cat:
                stat["lexicon_hit"] += 1
                totals["lexicon_hit"] += 1
            elif lex_cat:
                confusion[(cat, lex_cat)] += 1
            else:
                stat["lexicon_abstain"] += 1
                totals["lexicon_abstain"] += 1

            old_cat, _score, _reason = classify_5_genres_from_content(fpath.name, text[:8000])
            if old_cat == cat:
                stat["legacy_hit"] += 1
                totals["legacy_hit"] += 1
            elif not old_cat:
                stat["legacy_abstain"] += 1
                totals["legacy_abstain"] += 1

        if stat["n"]:
            per_cat[cat] = dict(stat)

    n = totals["n"] or 1
    return {
        "total_docs": totals["n"],
        "lexicon_top1_accuracy": round(totals["lexicon_hit"] / n, 4),
        "lexicon_abstain_rate": round(totals["lexicon_abstain"] / n, 4),
        "legacy_top1_accuracy": round(totals["legacy_hit"] / n, 4),
        "legacy_abstain_rate": round(totals["legacy_abstain"] / n, 4),
        "per_category": per_cat,
        "top_confusions": [{"true": t, "predicted": p, "count": c} for (t, p), c in confusion.most_common(30)],
    }


def explain_file(fpath: Path, lex: CategoryLexicon, top_k: int = 5, head_chars: int = DEFAULT_HEAD_CHARS, tail_chars: int = DEFAULT_TAIL_CHARS, kiwi: Any = None) -> Dict[str, Any]:
    """
    파일 한 건의 카테고리 판정 근거를 모아 돌려준다.

    - lexicon: 코퍼스 어휘 사전 상위 후보와 실제로 맞은 단어
    - legacy: 기존 5대 장르 스코어링 결과
    - weighted: 서점 조회 없이 파일 내용만으로 돌린 최종 가중치 합 판정
    """
    from backend.book_classifier import classify_5_genres_from_content, evaluate_category_decision  # noqa: PLC0415

    text = read_document_text(fpath, head_chars, tail_chars)
    if not text or len(text) < 200:
        return {"file": str(fpath), "error": "본문을 읽지 못했거나 너무 짧다"}

    words = extract_word_sets([text], kiwi=kiwi)[0]
    sims = lex.score_words(words)
    order = sorted(sims.items(), key=lambda kv: kv[1], reverse=True)[:top_k]
    best = order[0][1] if order else 0.0

    candidates = []
    for cat, sim in order:
        weights = lex._weights[cat]
        matched = sorted(((w, weights[w]) for w in words if w in weights), key=lambda kv: -kv[1])[:15]
        candidates.append({
            "category": cat,
            "similarity": round(sim, 6),
            "normalized": round(sim / best, 4) if best else 0.0,
            "matched_unique": sorted(w for w in words if w in lex._unique.get(cat, set()))[:15],
            "matched_top_words": [w for w, _v in matched],
        })

    legacy_cat, legacy_score, legacy_reason = classify_5_genres_from_content(fpath.name, text[:8000])
    empty: Dict[str, Any] = {"mapped": None}
    final_cat, method, reason = evaluate_category_decision(fpath.name, fpath, fpath.stem, "", fpath.stem, empty, empty, empty)

    return {
        "file": str(fpath),
        "word_count": len(words),
        "lexicon": candidates,
        "legacy": {"category": legacy_cat, "score": legacy_score, "reason": legacy_reason},
        "weighted": {"category": final_cat, "method": method, "reason": reason},
    }


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description="카테고리별 어휘 사전 빌더")
    parser.add_argument("command", choices=["collect", "build", "evaluate", "inspect", "classify"])
    parser.add_argument("--library-root", type=Path, default=DEFAULT_LIBRARY_ROOT)
    parser.add_argument("--work-dir", type=Path, default=DEFAULT_WORK_DIR)
    parser.add_argument("--out", type=Path, default=DEFAULT_LEXICON_PATH)
    parser.add_argument("--category", action="append", default=None, help="특정 카테고리만 (반복 지정 가능)")
    parser.add_argument("--max-files", type=int, default=DEFAULT_MAX_FILES)
    parser.add_argument("--min-files", type=int, default=DEFAULT_MIN_FILES)
    parser.add_argument("--head-chars", type=int, default=DEFAULT_HEAD_CHARS)
    parser.add_argument("--tail-chars", type=int, default=DEFAULT_TAIL_CHARS)
    parser.add_argument("--top-n", type=int, default=DEFAULT_TOP_N)
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED)
    parser.add_argument("--holdout-ratio", type=float, default=HOLDOUT_RATIO)
    parser.add_argument("--eval-limit", type=int, default=30, help="평가 시 카테고리당 홀드아웃 상한")
    parser.add_argument("--num-workers", type=int, default=8)
    parser.add_argument("--batch-size", type=int, default=DEFAULT_BATCH_SIZE)
    parser.add_argument("--no-resume", action="store_true")
    parser.add_argument("--report", type=Path, default=None)
    parser.add_argument("--file", type=Path, action="append", default=None, help="classify 대상 파일 (반복 지정 가능)")
    parser.add_argument("--top-k", type=int, default=5)
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

    if args.command == "collect":
        done = collect_all(args.library_root, args.work_dir, categories=args.category, resume=not args.no_resume, num_workers=args.num_workers, max_files=args.max_files, min_files=args.min_files, head_chars=args.head_chars, tail_chars=args.tail_chars, seed=args.seed, holdout_ratio=args.holdout_ratio, batch_size=args.batch_size)
        LOGGER.info(f"수집 완료: {len(done)}개 카테고리")
        return 0

    if args.command == "build":
        collected = load_collected(args.work_dir)
        if not collected:
            LOGGER.error(f"수집 결과가 없다: {args.work_dir}. 먼저 collect 를 실행하라.")
            return 1
        lexicon = build_lexicon(collected, top_n=args.top_n)
        path = write_lexicon(lexicon, args.out)
        size_mb = path.stat().st_size / 1024 / 1024
        LOGGER.info(f"사전 생성: {path} ({size_mb:.2f}MB, 카테고리 {lexicon['params']['category_count']}개)")
        return 0

    if args.command == "evaluate":
        collected = load_collected(args.work_dir)
        if not collected:
            LOGGER.error(f"수집 결과가 없다: {args.work_dir}")
            return 1
        report = evaluate_holdout(args.out, collected, limit_per_category=args.eval_limit, head_chars=args.head_chars, tail_chars=args.tail_chars, num_workers=args.num_workers)
        LOGGER.info(f"홀드아웃 {report['total_docs']}건: 어휘사전 top-1 {report['lexicon_top1_accuracy']:.1%} (무판정 {report['lexicon_abstain_rate']:.1%}), 기존 5대장르 top-1 {report['legacy_top1_accuracy']:.1%} (무판정 {report['legacy_abstain_rate']:.1%})")
        out = args.report or (args.work_dir / "holdout_report.json")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        LOGGER.info(f"리포트: {out}")
        return 0

    if args.command == "classify":
        if not args.file:
            LOGGER.error("--file 로 대상 파일을 지정하라")
            return 1
        lex = CategoryLexicon.load(args.out)
        if lex is None:
            LOGGER.error(f"사전을 읽지 못했다: {args.out}")
            return 1
        kiwi = get_kiwi(num_workers=args.num_workers)
        results = [explain_file(f, lex, top_k=args.top_k, head_chars=args.head_chars, tail_chars=args.tail_chars, kiwi=kiwi) for f in args.file]
        print(json.dumps(results, ensure_ascii=False, indent=2))
        return 0

    if args.command == "inspect":
        lex = CategoryLexicon.load(args.out)
        if lex is None:
            LOGGER.error(f"사전을 읽지 못했다: {args.out}")
            return 1
        targets = args.category or lex.categories
        for cat in targets:
            words = lex._weights.get(cat)
            if not words:
                continue
            top = sorted(words.items(), key=lambda kv: -kv[1])[:25]
            uniq = sorted(lex._unique.get(cat, set()))[:15]
            print(f"\n[{cat}] 문서 {lex.doc_counts.get(cat)}건, 단어 {len(words)}개, 고유 {len(lex._unique.get(cat, set()))}개")
            print("  top   : " + ", ".join(f"{w}({v:.3f})" for w, v in top))
            print("  unique: " + ", ".join(uniq))
        return 0

    return 1


if __name__ == "__main__":
    sys.exit(main())
