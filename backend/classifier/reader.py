#!/usr/bin/env python3
"""
파일에서 본문과 메타데이터를 읽는다 - ES 에 없을 때의 대비책

ES 가 문서 대부분을 갖고 있으므로 이 경로는 자주 안 쓴다. 새로 넣은 파일이나
아직 인덱싱되지 않은 파일을 판정할 때만 쓴다.

인코딩 판별은 네 번 고쳐 잡은 로직이라 그대로 옮겨 왔다. 다시 쓰지 말 것:
  - "한글이 하나라도 나오면 그 인코딩 채택" 은 정상 CP949 소설 1,856건을 오판했다
  - 한글 비율 하한이 없으면 영문 OCR 문서를 한글 잡음 60자로 판정한다
  - UTF-16 을 빼면 97건을 못 읽는다
  - EPUB 은 첫 문서가 목차인 경우가 있어 건너뛰어야 한다
"""

import logging
import re
import zipfile
from pathlib import Path
from typing import List, Optional

logger = logging.getLogger(__name__)

COMMON_HANGUL_SYLLABLES = frozenset("이다는을가지에그고하의아은어도로리나를한었기서들게니있라자사시만으해마보했수대것")

# 이 수보다 음절이 적으면 비율이 흔들려 판정하지 않는다.
MIN_SYLLABLES_FOR_LIKENESS = 50


def korean_likeness(text: str) -> Optional[float]:
    """이 글이 정상 한국어로 읽히는 정도. 잴 음절이 모자라면 None."""
    syllables = [ch for ch in text if "가" <= ch <= "힣"]
    if len(syllables) < MIN_SYLLABLES_FOR_LIKENESS:
        return None
    hits = sum(1 for ch in syllables if ch in COMMON_HANGUL_SYLLABLES)
    return hits / len(syllables)


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

# EPUB 의 OPF 에서 읽을 메타데이터. publisher 는 전집 판정의 결정적 증거다.
_DC_PUBLISHER = re.compile(r"<dc:publisher[^>]*>(.*?)</dc:publisher>", re.IGNORECASE | re.DOTALL)
_TAG = re.compile(r"<[^>]+>")


def read_epub_publisher(fpath: Path) -> str:
    """EPUB 의 dc:publisher 를 읽는다. 본문은 안 읽고 OPF 하나만 푼다."""
    try:
        with zipfile.ZipFile(fpath) as zf:
            opf = next((n for n in zf.namelist() if n.lower().endswith(".opf")), None)
            if not opf:
                return ""
            xml = zf.read(opf).decode("utf-8", "replace")
    except Exception as e:
        logger.debug("OPF 를 읽지 못했다 (%s): %s", fpath, e)
        return ""
    values = [_TAG.sub("", m.group(1)).strip() for m in _DC_PUBLISHER.finditer(xml)]
    return " ".join(v for v in values if v)
