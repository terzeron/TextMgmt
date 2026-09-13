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

from utils.detect_mojibake import VERDICT_CORRUPTED, classify_text, korean_likeness  # noqa: E402
from backend.category_lexicon import (  # noqa: E402
    DEFAULT_LEXICON_PATH,
    FILENAME_LEXICON_PATH,
    SERIES_TO_PARENT,
    NB_ALPHA,
    extract_filename_features,
    resolve_parent,
    CategoryLexicon,
    analyze_texts,
    decode_best,
    extract_word_sets,
    get_kiwi,
    read_document_text,
    read_epub_text,
    read_txt_text,
)

LOGGER = logging.getLogger("corpus_lexicon")

DEFAULT_LIBRARY_ROOT = Path("/mnt/data/text")
DEFAULT_WORK_DIR = REPO_ROOT / "tmp" / "corpus_lexicon"

TEXT_EXTS = {".txt", ".epub"}

# `0_*`는 분류 입력 스테이징(0_telegram 등)이지 카테고리가 아니므로 제외한다.
CATEGORY_DIR_RE = re.compile(r"^[1-9]_")

# 샘플링 기본값. 3_판타지 78,027건 대 9_격언명언 7건의 불균형에 상한을 건다.
DEFAULT_MAX_FILES = 300
# 카테고리는 전부 사전을 갖는다. 읽을 수 있는 문서가 1건이라도 있으면 만든다.
# (파일이 0건인 카테고리만 물리적으로 불가능해 건너뛴다)
DEFAULT_MIN_FILES = 1
DEFAULT_HEAD_CHARS = 20000
DEFAULT_TAIL_CHARS = 10000
DEFAULT_SEED = 20260912
DEFAULT_BATCH_SIZE = 16

# 어휘 크기. 카이제곱으로 고른 30,000개가 정밀도 우선 구간에서 가장 좋았다.
# 홀드아웃 2,044건, Complement NB, 답한 비율별 정답률:
#   카이제곱 10,000 : 20% 구간 87.1%   15% 구간 90.4%
#   카이제곱 30,000 : 20% 구간 90.2%   15% 구간 92.5%
#   카이제곱 60,000 : 20% 구간 89.0%   15% 구간 91.8%
#   문서빈도 60,000 : 20% 구간 77.9%   15% 구간 79.7%
#   전체   262,994 : 20% 구간 85.0%   15% 구간 91.8%
# 같은 6만 개라도 고르는 기준이 다르면 77.9% 대 89.0% 이다. 자질 선택이 크게 작용한다.
# 그리고 전체 어휘보다 잘 고른 3만 개가 낫다. 저장도 9분의 1이다.
DEFAULT_VOCAB_SIZE = 30000

# Laplace 평활 계수
NB_ALPHA = 1.0

# 어휘에 넣기 위한 최소 문서 수. 1~2건짜리는 우연이다.
MIN_DOC_FREQ = 3

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
# 인코딩 손상 문서 제외
#
# 코퍼스에는 원본 텍스트를 CP949로 잘못 읽고 UTF-8로 다시 저장한 파일이 섞여 있다.
# 파일 자체는 유효한 UTF-8이라 디코딩으로는 걸러지지 않고 내용만 깨진다.
# 이런 문서가 들어가면 어휘가 통째로 잡음이 된다.
#
# 판정은 `utils.detect_mojibake` 로 일원화한다. 여기서 별도 기준을 두면
# 같은 파일에 사전 빌더와 전수 검사기가 다른 답을 낸다.
# ---------------------------------------------------------------------------


def is_mojibake(text: str) -> bool:
    """인코딩 손상으로 내용이 깨진 텍스트인지 판정한다"""
    return classify_text(text)[0] == VERDICT_CORRUPTED


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
    n_corrupted = 0
    for start in range(0, len(train), batch_size):
        texts = [read_document_text(p, head_chars, tail_chars) for p in train[start : start + batch_size]]
        texts = [t for t in texts if t and len(t) >= 200]
        # 손상 판정은 문자 세기라서 형태소 분석보다 훨씬 싸다. 토큰화 전에 걸러낸다.
        clean = []
        for t in texts:
            if is_mojibake(t):
                n_corrupted += 1
            else:
                clean.append(t)
        if not clean:
            continue
        for words in extract_word_sets(clean, kiwi=kiwi):
            if not words:
                continue
            df.update(words)
            n_docs += 1

    if n_docs < min_files:
        LOGGER.info(f"{cat}: 본문 추출 성공 {n_docs}건 < 최소 {min_files}건, 건너뜀")
        return None

    result = {"category": cat, "total_files": len(files), "sampled_files": len(sampled), "doc_count": n_docs, "corrupted_docs": n_corrupted, "df": dict(df), "holdout": [str(p) for p in holdout], "elapsed_sec": round(time.time() - t0, 1)}
    work_dir.mkdir(parents=True, exist_ok=True)
    out_path = work_dir / f"{cat}.json.gz"
    with gzip.open(out_path, "wt", encoding="utf-8") as f:
        json.dump(result, f, ensure_ascii=False)
    LOGGER.info(f"{cat}: 문서 {n_docs}건(손상 제외 {n_corrupted}건), 고유단어 {len(df)}개, {result['elapsed_sec']}초 -> {out_path.name}")
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


def chi_square_scores(cats: List[str], merged: Dict[str, Dict[str, Any]], total_df: Counter[str], vocab: List[str], n_docs: int) -> Dict[str, float]:
    """
    단어마다 카테고리별 카이제곱의 최댓값. "이 단어가 나오는지 여부가 카테고리와
    통계적으로 얼마나 연관되는가" 를 잰다.

    TF-IDF 순위로 어휘를 고르면 안 된다. TF-IDF 는 검색에서 중요한 단어를 고르는
    척도지 분류에 도움되는 단어를 고르는 척도가 아니다. 실측에서 같은 6만 개를
    문서빈도순으로 고르면 77.9%, 카이제곱순으로 고르면 89.0% 였다.
    """
    scores: Dict[str, float] = {}
    for w in vocab:
        tot_t = total_df[w]
        best = 0.0
        for c in cats:
            n_c = merged[c]["doc_count"]
            n11 = merged[c]["df"].get(w, 0)      # c 에 속하고 w 를 포함
            n10 = tot_t - n11                     # c 가 아니고 w 를 포함
            n01 = n_c - n11                       # c 에 속하고 w 가 없음
            n00 = n_docs - n_c - n10              # c 도 아니고 w 도 없음
            den = (n11 + n01) * (n11 + n10) * (n10 + n00) * (n01 + n00)
            if den <= 0:
                continue
            best = max(best, n_docs * (n11 * n00 - n10 * n01) ** 2 / den)
        scores[w] = best
    return scores


def build_lexicon(collected: Dict[str, Dict[str, Any]], vocab_size: int = DEFAULT_VOCAB_SIZE, min_doc_freq: int = MIN_DOC_FREQ) -> Dict[str, Any]:
    """
    Complement Naive Bayes 모델을 만든다.

    코사인 유사도(TF-IDF 중심점) 방식에서 옮겨왔다. 그 방식은 "맞은 단어 전체의
    겹침 비율" 을 보기 때문에, `오우거` 같은 결정적 단어 하나가 `사람`·`시간` 같은
    흔한 단어 수백 개에 묻힌다. NB 는 단어마다 로그 확률비를 더하므로 결정적 단어
    하나가 판정을 뒤집을 수 있다. 사람이 장르를 알아보는 방식에 더 가깝다.

    Complement 변형을 쓰는 이유는 클래스 불균형이다. 3_판타지 78,027건과
    9_격언명언 7건을 같은 저울에 올리는 문제를 이 변형이 정면으로 다룬다.

    저장은 가중치가 아니라 원시 문서빈도로 한다. 가중치는 전 어휘에 대해 0 이 아니라
    조밀해지지만, 문서빈도는 희소해서 훨씬 작다. 가중치는 적재할 때 계산한다.
    """
    # 하위 카테고리(출판사 전집 등)는 상위 장르에 합친다. 따로 두면 같은 어휘로
    # 서로 경쟁하며 점수를 깎고, 어느 쪽도 이기지 못해 판정이 보류된다.
    merged: Dict[str, Dict[str, Any]] = {}
    for cat, entry in collected.items():
        target = SERIES_TO_PARENT.get(cat, cat)
        if target not in merged:
            merged[target] = {"category": target, "doc_count": 0, "df": Counter(), "merged_from": []}
        merged[target]["doc_count"] += int(entry["doc_count"])
        merged[target]["df"].update(entry["df"])
        if target != cat:
            merged[target]["merged_from"].append(cat)

    cats = sorted(merged)
    if not cats:
        raise ValueError("수집 결과가 비어 있다")
    n_docs = sum(merged[c]["doc_count"] for c in cats)

    total_df: Counter[str] = Counter()
    for c in cats:
        total_df.update(merged[c]["df"])
    base_vocab = [w for w, n in total_df.items() if n >= min_doc_freq]
    if not base_vocab:
        raise ValueError("어휘가 비어 있다")

    chi = chi_square_scores(cats, merged, total_df, base_vocab, n_docs)
    if vocab_size and vocab_size < len(base_vocab):
        vocab = sorted(base_vocab, key=lambda w: (-chi[w], w))[:vocab_size]
    else:
        vocab = sorted(base_vocab)
    vocab_set = set(vocab)

    categories_out: Dict[str, Any] = {}
    for cat in cats:
        own = merged[cat]["df"]
        categories_out[cat] = {
            "doc_count": int(merged[cat]["doc_count"]),
            "merged_from": merged[cat]["merged_from"],
            # 희소 저장. 이 카테고리에 실제로 나온 어휘만 남긴다.
            "df": {w: int(own[w]) for w in own if w in vocab_set},
        }

    return {
        "version": 2,
        "model": "complement_naive_bayes",
        "built_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "params": {"vocab_size": len(vocab), "min_doc_freq": min_doc_freq, "alpha": NB_ALPHA, "category_count": len(cats), "doc_count": n_docs},
        "total_df": {w: int(total_df[w]) for w in vocab},
        "categories": categories_out,
    }


def build_filename_lexicon(library_root: Path, max_files: int = 600, min_files: int = DEFAULT_MIN_FILES, seed: int = DEFAULT_SEED, holdout_ratio: float = HOLDOUT_RATIO, min_feature_docs: int = 5, kiwi: Any = None) -> Dict[str, Any]:
    """
    파일명만으로 카테고리를 맞히는 Complement NB 모델을 만든다.

    코퍼스 파일명이 곧 학습 데이터다. 파일을 열지 않으므로 본문 모델보다 훨씬 빠르고,
    실측에서 판정률과 정답률이 모두 높았다.
      본문   답한비율  7.9%  정답률 94.4%
      파일명 답한비율 16.4%  정답률 97.9%

    본문 모델과 같은 seed·같은 홀드아웃 분할을 써서 두 모델을 나란히 평가할 수 있게 한다.
    """
    k = kiwi if kiwi is not None else get_kiwi(num_workers=8)
    df: Counter[Tuple[str, str]] = Counter()
    doc_count: Counter[str] = Counter()
    holdout: Dict[str, List[str]] = {}

    for cat in list_category_dirs(library_root):
        files = list_category_files(library_root / cat)
        if len(files) < min_files:
            continue
        train, hold = split_holdout(sample_files(files, max_files, seed, cat), holdout_ratio, seed, cat)
        parent = SERIES_TO_PARENT.get(cat, cat)
        for p in train:
            doc_count[parent] += 1
            for w in extract_filename_features(p.name, kiwi=k):
                df[(parent, w)] += 1
        holdout[cat] = [str(p) for p in hold]
        LOGGER.info(f"{cat}: 학습 {len(train)}건, 홀드아웃 {len(hold)}건 -> {parent}")

    cats = sorted(doc_count)
    if not cats:
        raise ValueError("파일명을 모으지 못했다")
    total: Counter[str] = Counter()
    for (_c, w), n in df.items():
        total[w] += n
    vocab = {w for w, n in total.items() if n >= min_feature_docs}
    if not vocab:
        raise ValueError("특징이 비어 있다")

    return {
        "version": 2,
        "model": "complement_naive_bayes",
        "features": "filename",
        "built_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "params": {"vocab_size": len(vocab), "min_feature_docs": min_feature_docs, "alpha": NB_ALPHA, "category_count": len(cats), "max_files": max_files},
        "total_df": {w: int(total[w]) for w in vocab},
        "categories": {c: {"doc_count": int(doc_count[c]), "df": {w: int(df[(c, w)]) for w in vocab if (c, w) in df}} for c in cats},
        "holdout": holdout,
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
    학습에서 제외한 홀드아웃 파일로 세 가지 판정의 정확도를 비교한다.
    코퍼스 디렉토리 이름이 곧 정답 레이블이다. 서점 조회 없이 파일 내용만 쓴다.

      lexicon  : 신규 어휘 사전 단독 (top-1 과 top-3)
      legacy   : 기존 5대 장르 키워드 스코어링
      weighted : 최종 가중치 합 판정 (evaluate_category_decision)
    """
    from backend.book_classifier import classify_5_genres_from_content, evaluate_category_decision  # noqa: PLC0415

    lex = CategoryLexicon.load(lexicon_path)
    if lex is None:
        raise RuntimeError(f"사전을 읽지 못했다: {lexicon_path}")
    kiwi = get_kiwi(num_workers=num_workers)
    empty: Dict[str, Any] = {"mapped": None}

    per_cat: Dict[str, Dict[str, int]] = {}
    totals: Counter[str] = Counter()
    confusion: Counter[Tuple[str, str]] = Counter()

    for cat in sorted(collected):
        # 모델은 하위 카테고리를 상위 장르로 합쳐 학습했으므로 어휘 사전은 상위 장르만
        # 내놓는다. 반면 파일명 시리즈 신호는 하위 카테고리를 내놓는다.
        # 둘 다 맞는 답이므로 정확한 하위 카테고리와 그 상위 장르를 모두 정답으로 친다.
        # 이걸 빠뜨리면 한쪽이 자동으로 전부 오답이 된다.
        true_cat = resolve_parent(cat)
        accepted = {cat, true_cat}
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

            ranked = lex.rank(text, top_k=3, kiwi=kiwi)
            if not ranked:
                stat["lexicon_abstain"] += 1
                totals["lexicon_abstain"] += 1
            else:
                if ranked[0][0] in accepted:
                    stat["lexicon_hit"] += 1
                    totals["lexicon_hit"] += 1
                else:
                    confusion[(cat, ranked[0][0])] += 1
                if any(c in accepted for c, _n, _s in ranked):
                    stat["lexicon_top3_hit"] += 1
                    totals["lexicon_top3_hit"] += 1

            old_cat, _score, _reason = classify_5_genres_from_content(fpath.name, text[:8000])
            if old_cat in accepted:
                stat["legacy_hit"] += 1
                totals["legacy_hit"] += 1
            elif not old_cat:
                stat["legacy_abstain"] += 1
                totals["legacy_abstain"] += 1

            # 최종 판정. 서점 결과를 비워 파일 내용만으로 돌린다.
            final_cat, _method, _why = evaluate_category_decision(fpath.name, fpath, fpath.stem, "", fpath.stem, empty, empty, empty)
            if final_cat in accepted:
                stat["weighted_hit"] += 1
                totals["weighted_hit"] += 1
            elif not final_cat:
                stat["weighted_abstain"] += 1
                totals["weighted_abstain"] += 1

        if stat["n"]:
            per_cat[cat] = dict(stat)

    n = totals["n"] or 1
    return {
        "total_docs": totals["n"],
        "lexicon_decided_rate": round(1 - totals["lexicon_abstain"] / n, 4),
        "lexicon_precision": round(totals["lexicon_hit"] / max(1, n - totals["lexicon_abstain"]), 4),
        "weighted_precision": round(totals["weighted_hit"] / max(1, n - totals["weighted_abstain"]), 4),
        "lexicon_top1_accuracy": round(totals["lexicon_hit"] / n, 4),
        "lexicon_top3_accuracy": round(totals["lexicon_top3_hit"] / n, 4),
        "lexicon_abstain_rate": round(totals["lexicon_abstain"] / n, 4),
        "legacy_top1_accuracy": round(totals["legacy_hit"] / n, 4),
        "legacy_abstain_rate": round(totals["legacy_abstain"] / n, 4),
        "weighted_top1_accuracy": round(totals["weighted_hit"] / n, 4),
        "weighted_abstain_rate": round(totals["weighted_abstain"] / n, 4),
        "per_category": per_cat,
        "top_confusions": [{"true": t, "predicted": p, "count": c} for (t, p), c in confusion.most_common(40)],
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
    if is_mojibake(text):
        return {"file": str(fpath), "error": "인코딩이 손상된 문서"}

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
    parser.add_argument("command", choices=["collect", "build", "build-filename", "evaluate", "inspect", "classify"])
    parser.add_argument("--library-root", type=Path, default=DEFAULT_LIBRARY_ROOT)
    parser.add_argument("--work-dir", type=Path, default=DEFAULT_WORK_DIR)
    parser.add_argument("--out", type=Path, default=DEFAULT_LEXICON_PATH)
    parser.add_argument("--category", action="append", default=None, help="특정 카테고리만 (반복 지정 가능)")
    parser.add_argument("--max-files", type=int, default=DEFAULT_MAX_FILES)
    parser.add_argument("--min-files", type=int, default=DEFAULT_MIN_FILES)
    parser.add_argument("--head-chars", type=int, default=DEFAULT_HEAD_CHARS)
    parser.add_argument("--tail-chars", type=int, default=DEFAULT_TAIL_CHARS)
    parser.add_argument("--vocab-size", type=int, default=DEFAULT_VOCAB_SIZE)
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

    if args.command == "build-filename":
        model = build_filename_lexicon(args.library_root, kiwi=get_kiwi(num_workers=args.num_workers))
        path = write_lexicon(model, FILENAME_LEXICON_PATH)
        LOGGER.info(f"파일명 모델: {path} ({path.stat().st_size / 1024 / 1024:.2f}MB, 카테고리 {model['params']['category_count']}개, 특징 {model['params']['vocab_size']:,}개)")
        return 0

    if args.command == "build":
        collected = load_collected(args.work_dir)
        if not collected:
            LOGGER.error(f"수집 결과가 없다: {args.work_dir}. 먼저 collect 를 실행하라.")
            return 1
        lexicon = build_lexicon(collected, vocab_size=args.vocab_size)
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
        LOGGER.info(
            f"홀드아웃 {report['total_docs']}건\n"
            f"  어휘사전  답한비율 {report['lexicon_decided_rate']:.1%}  그중 정답률 {report['lexicon_precision']:.1%}  (전체 대비 {report['lexicon_top1_accuracy']:.1%})\n"
            f"  기존5장르 top-1 {report['legacy_top1_accuracy']:.1%}  무판정 {report['legacy_abstain_rate']:.1%}\n"
            f"  가중치합  답한비율 {1 - report['weighted_abstain_rate']:.1%}  그중 정답률 {report['weighted_precision']:.1%}  (전체 대비 {report['weighted_top1_accuracy']:.1%})"
        )
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
            print(f"\n[{cat}] 문서 {lex.doc_counts.get(cat)}건")
            print("  top   : " + ", ".join(f"{w}({v:.3f})" for w, v in top))
        return 0

    return 1


if __name__ == "__main__":
    sys.exit(main())
