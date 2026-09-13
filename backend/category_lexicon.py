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
import zipfile
import logging
import math
import re
import threading
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Set, Tuple

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


# 조사(J*), 어미(E*), 용언(V*)은 한국어 문장을 굴러가게 하는 문법 형태소다.
# 정상 한국어 산문은 토큰의 40~60%가 여기 해당한다.
GRAMMAR_TAG_PREFIXES = ("J", "E", "V")


def grammar_ratio(tokens: Iterable[Any]) -> float:
    """토큰 중 문법 형태소(조사·어미·용언)가 차지하는 비율"""
    toks = list(tokens)
    if not toks:
        return 0.0
    return sum(1 for t in toks if t.tag[:1] in GRAMMAR_TAG_PREFIXES) / len(toks)


def analyze_texts(texts: List[str], kiwi: Any = None) -> List[Tuple[Set[str], float]]:
    """
    한 번의 토큰화로 단어 집합과 문법 형태소 비율을 함께 구한다.

    문법 형태소 비율은 인코딩 손상 판정에 쓴다. 손상 텍스트는 한글처럼 보여도
    조사와 어미가 없어 비율이 0.03 언저리로 떨어진다.
    """
    if not texts:
        return []
    k = kiwi if kiwi is not None else get_kiwi()
    if k is None:
        return [(set(), 0.0) for _ in texts]
    try:
        results = list(k.tokenize(texts))
    except Exception as e:  # pragma: no cover - 입력 의존
        logger.warning(f"일괄 형태소 분석 실패, 개별 처리로 대체: {e}")
        results = []
        for t in texts:
            try:
                results.append(k.tokenize(t))
            except Exception:
                results.append([])
    out: List[Tuple[Set[str], float]] = []
    for tokens in results:
        toks = list(tokens)
        out.append((set(merge_contiguous_nouns(toks)), grammar_ratio(toks)))
    return out


def extract_word_sets(texts: List[str], kiwi: Any = None) -> List[Set[str]]:
    """
    여러 본문을 한 번에 토큰화하여 단어 집합 목록을 돌려준다.

    kiwi는 문자열 하나를 넘기면 `num_workers` 설정과 무관하게 단일 스레드로 돈다.
    리스트를 넘겨야 워커가 실제로 병렬 처리한다(실측 1워커 57,754자/s -> 8워커 199,676자/s).
    """
    return [words for words, _gram in analyze_texts(texts, kiwi=kiwi)]


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


# ---------------------------------------------------------------------------
# 본문 추출
#
# 사전을 만들 때와 점수를 매길 때가 같은 코드로 같은 분량을 읽어야 한다.
# 분류기 쪽에 있던 추출기는 .txt 를 250줄까지만 읽어 사전이 학습한 분량의
# 3분의 1도 되지 않았고, 그만큼 판정이 나빠졌다.
# ---------------------------------------------------------------------------
TEXT_HEAD_CHARS = 20000
TEXT_TAIL_CHARS = 10000

_TAG_RE = re.compile(r"<[^>]+>")
_WS_RE = re.compile(r"\s+")


def decode_best(raw: bytes) -> str:
    """
    바이트 열을 올바른 인코딩으로 디코딩한다.

    UTF-8 은 자기검증 인코딩이다. CP949 로 저장된 한국어 텍스트는 바이트 쌍이
    유효한 UTF-8 시퀀스가 되는 일이 거의 없어 엄격 디코딩에서 실패한다.
    그래서 "엄격 UTF-8 성공 여부"가 가장 신뢰할 수 있는 판별이다.

    두 가지를 하면 안 된다.
    - "한글이 하나라도 나오면 채택": CP949 파일을 UTF-8로 잘못 읽은 결과에도 한글이
      우연히 섞인다. 실측에서 정상 CP949 소설 1,856건이 손상으로 오판됐다.
    - "likeness 가 가장 높은 인코딩 채택": 이미 손상된 파일을 CP949 로 다시 읽으면
      더 한국어처럼 보이는 다른 쓰레기가 나와 손상을 놓친다.
    """
    if not raw:
        return ""

    # UTF-16 은 BOM 이나 널 바이트 밀도로 먼저 알아본다. 후보에서 빼 두면 UTF-16 한국어
    # 텍스트를 통째로 깨진 것으로 읽는다(실측 97건).
    if raw[:2] in (b"\xff\xfe", b"\xfe\xff"):
        return raw.decode("utf-16", errors="ignore")
    if len(raw) >= 200 and raw.count(0) > len(raw) * 0.25:
        even_nulls = raw[1::2].count(0)
        enc = "utf-16-le" if even_nulls > raw[0::2].count(0) else "utf-16-be"
        return raw.decode(enc, errors="ignore")

    # 잘린 멀티바이트 문자 때문에 엄격 디코딩이 헛되이 실패하지 않도록 꼬리를 다듬는다
    for trim in range(0, 4):
        chunk = raw[: len(raw) - trim] if trim else raw
        try:
            return chunk.decode("utf-8")
        except UnicodeDecodeError:
            continue

    for enc in ("cp949", "euc-kr"):
        for trim in range(0, 2):
            chunk = raw[: len(raw) - trim] if trim else raw
            try:
                return chunk.decode(enc)
            except UnicodeDecodeError:
                continue

    # 어느 것으로도 깨끗이 안 읽히면 한국어로 가장 잘 읽히는 쪽을 쓴다
    best_text, best_score = "", -1.0
    for enc in ("utf-8", "cp949", "euc-kr"):
        text = raw.decode(enc, errors="ignore")
        likeness = korean_likeness(text)
        score = likeness if likeness is not None else 0.001
        if score > best_score:
            best_score, best_text = score, text
    return best_text


def read_txt_text(fpath: Path, head_chars: int, tail_chars: int) -> str:
    """TXT 앞부분과 뒷부분을 인코딩 추정하여 읽는다"""
    try:
        size = fpath.stat().st_size
    except OSError:
        return ""

    parts: List[str] = []
    try:
        with open(fpath, "rb") as f:
            parts.append(decode_best(f.read(head_chars * 3))[:head_chars])
            if tail_chars > 0 and size > head_chars * 3:
                f.seek(max(0, size - tail_chars * 3))
                parts.append(decode_best(f.read())[-tail_chars:])
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
            body = [n for n in names if not any(k in n.lower() for k in ("cover", "nav", "toc", "titlepage", "index", "contents"))]
            names = body or names
            # 앞쪽 문서는 표지·목차·판권지다. 거기서 표본을 뽑으면 본문을 못 본다.
            # 목차만 읽고 어휘가 빈약하다는 이유로 손상 판정이 나온 사례가 있었다.
            if len(names) > 4:
                mid = len(names) // 3
                names = names[mid:] + names[:mid]

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


def read_document_text(fpath: Path, head_chars: int = TEXT_HEAD_CHARS, tail_chars: int = TEXT_TAIL_CHARS) -> str:
    """확장자에 맞춰 본문 표본을 읽는다"""
    ext = fpath.suffix.lower()
    if ext == ".txt":
        return read_txt_text(fpath, head_chars, tail_chars)
    if ext == ".epub":
        return read_epub_text(fpath, head_chars, tail_chars)
    return ""


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

    def rank(self, text: str, top_k: int = 5, kiwi: Any = None) -> List[Tuple[str, float, float]]:
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
