#!/usr/bin/env python3
"""
필드별 특징 추출 - 설정대로 TF-IDF 블록을 만들어 가로로 잇는다

필드마다 벡터라이저를 따로 두고, 만들어진 블록에 설정의 `weight` 를 곱한 뒤
하나로 붙인다. 선형 모델이라 블록에 곱한 값이 그 필드의 영향력을 그대로 바꾼다.
가중치를 바꾸려면 `config.json` 만 고치고 다시 학습하면 된다.
"""

import gc
import logging
import re
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple

import numpy as np
from scipy.sparse import csr_matrix, hstack
from sklearn.feature_extraction.text import TfidfVectorizer

logger = logging.getLogger(__name__)

# 파일명의 긴 숫자는 권 번호나 일련번호라 카테고리를 가리키지 않는다.
DIGIT_RUN = re.compile(r"\d{2,}")


def field_text(doc: Dict[str, Any], spec: Dict[str, Any]) -> str:
    """레코드 하나에서 한 필드의 원문을 꺼낸다."""
    raw = doc.get(spec["source"]) or ""
    if isinstance(raw, (list, tuple)):
        raw = " ".join(str(x) for x in raw if x)
    text = str(raw)
    if spec["source"] == "name":
        text = DIGIT_RUN.sub(" ", text)
    limit = int(spec.get("max_chars") or 0)
    return text[:limit] if limit > 0 else text


def make_vectorizer(spec: Dict[str, Any]) -> TfidfVectorizer:
    lo, hi = spec.get("ngram") or [1, 1]
    kwargs: Dict[str, Any] = {
        "sublinear_tf": True,
        "dtype": np.float32,
        "analyzer": spec.get("analyzer", "word"),
        "ngram_range": (int(lo), int(hi)),
        "min_df": int(spec.get("min_df", 1)),
        "max_features": int(spec["max_features"]) if spec.get("max_features") else None,
    }
    if kwargs["analyzer"] == "word":
        # 기본 패턴은 한 글자 토큰을 버린다. 한자 한 글자가 의미를 갖는 경우가 있어 살린다.
        kwargs["token_pattern"] = r"(?u)\b\w+\b"
    return TfidfVectorizer(**kwargs)


def shrink_sources(docs: Sequence[Dict[str, Any]], all_sources: Sequence[str], remaining: Sequence[Tuple[str, Dict[str, Any]]]) -> None:
    """남은 필드가 볼 만큼만 남기고 원문을 제자리에서 줄인다.

    벡터화는 필드를 차례로 돈다. 앞 필드가 본문 전체를 읽고 나면, 뒤 필드는 앞
    1,500자만 보는데도 본문 전체가 메모리에 그대로 남아 있다. 23만 건이면 그게 3GB 고,
    `body_char` 가 그 위에서 8GB 를 더 쓰다가 죽었다.

    아무 필드도 안 보는 원문은 지운다. 보는 필드가 남았으면 그중 가장 긴 요구만큼만
    남긴다. `max_chars` 가 0(전부)인 필드가 남아 있으면 그 원문은 건드리지 않는다.

    `name` 은 건드리지 않는다. `field_text` 가 숫자를 지운 **뒤에** 자르는 필드라,
    먼저 자르면 지워질 숫자가 글자 수를 차지해 결과가 달라진다. 파일명은 어차피 짧다.
    """
    limits: Dict[str, Optional[int]] = {}
    for _name, spec in remaining:
        source = spec["source"]
        want = None if source == "name" else (int(spec.get("max_chars") or 0) or None)
        if source in limits:
            limits[source] = None if limits[source] is None or want is None else max(limits[source], want)
        else:
            limits[source] = want

    drop = [src for src in all_sources if src not in limits]
    cut = {src: n for src, n in limits.items() if n is not None}
    if not drop and not cut:
        return
    for doc in docs:
        for source in drop:
            doc.pop(source, None)
        for source, limit in cut.items():
            value = doc.get(source)
            if isinstance(value, str) and len(value) > limit:
                doc[source] = value[:limit]
    gc.collect()


class FeatureSpace:
    """필드별 벡터라이저 묶음. 학습 때 fit 하고, 판정 때는 transform 만 한다."""

    def __init__(self, fields: Dict[str, Dict[str, Any]]):
        self.fields = fields
        self.vectorizers: Dict[str, TfidfVectorizer] = {}
        self.block_sizes: Dict[str, int] = {}

    @property
    def names(self) -> List[str]:
        return list(self.fields.keys())

    def fit_transform(self, docs: Sequence[Dict[str, Any]], spill_dir: Path | str | None = None, dtype: Any = None, after_blocks: Optional[Callable[[], None]] = None) -> csr_matrix:
        """필드마다 TF-IDF 블록을 만들어 하나로 잇는다.

        `spill_dir` 을 주면 블록을 만드는 즉시 디스크에 쓰고 메모리에서 지운다.
        블록 7개를 다 들고 있다가 `hstack` 으로 한 번 더 복사하면 같은 자료가 잠깐
        두 벌이 된다. 23만 건 실측에서 그 순간이 피크였고, 여유 10GB 기계에서
        `systemd-oomd` 가 학습을 죽였다. 스필 경로는 한 번에 블록 하나만 들고 있다.

        `dtype` 을 주면 마지막에 그 자료형으로 바로 합친다. liblinear 은 float64 만
        받는데, float32 로 합친 뒤 `astype` 하면 5GB 짜리 사본이 하나 더 생긴다.

        `after_blocks` 는 블록을 다 만든 뒤, 합치기 직전에 한 번 불린다. 합치기는
        결과 행렬 한 벌을 통째로 잡는 단계라 여기가 피크다. 그때 원문은 이미 쓸모가
        없으니, 호출한 쪽이 이 자리에서 놓아주면 피크가 원문 크기만큼 내려간다.
        """
        if spill_dir is None:
            blocks = []
            for name, spec in self.fields.items():
                block = self._fit_block(name, spec, docs)
                if block is None:
                    continue
                blocks.append(block)
            if not blocks:
                raise ValueError("어느 필드에서도 특징을 못 뽑았다")
            out = hstack(blocks, format="csr")
            return out.astype(dtype) if dtype is not None and out.dtype != dtype else out

        spill = Path(spill_dir)
        spill.mkdir(parents=True, exist_ok=True)
        parts: List[Tuple[str, int]] = []
        ordered = list(self.fields.items())
        for i, (name, spec) in enumerate(ordered):
            block = self._fit_block(name, spec, docs)
            # 이 필드를 마쳤으니, 뒤에 남은 필드가 안 보는 원문은 지금 잘라낸다.
            # body_word 가 본문 전체를 다 쓰고 나면 남은 필드는 앞 1,500자만 본다.
            # 23만 건에서 그 차이가 2GB 다. 다음 필드가 그 2GB 위에서 작업하면 죽는다.
            shrink_sources(docs, [spec["source"] for _n, spec in ordered], ordered[i + 1 :])
            if block is None:
                continue
            np.save(spill / f"{name}.data.npy", block.data)
            np.save(spill / f"{name}.indices.npy", block.indices)
            np.save(spill / f"{name}.indptr.npy", block.indptr)
            parts.append((name, block.shape[1]))
            del block
            gc.collect()
        if not parts:
            raise ValueError("어느 필드에서도 특징을 못 뽑았다")
        n_rows = len(docs)
        if after_blocks is not None:
            after_blocks()
        return _assemble_spilled(spill, parts, n_rows, dtype or np.float32)

    def _fit_block(self, name: str, spec: Dict[str, Any], docs: Sequence[Dict[str, Any]]) -> Optional[csr_matrix]:
        """필드 하나를 벡터화한다. 어휘가 안 남으면 None."""
        texts = [field_text(d, spec) for d in docs]
        vec = make_vectorizer(spec)
        try:
            block = vec.fit_transform(texts)
        except ValueError as e:
            # 어휘가 하나도 안 남는 필드는 조용히 버린다. 전부 빈 문자열일 때 생긴다.
            logger.warning("필드 '%s' 에서 특징을 못 뽑아 건너뛴다: %s", name, e)
            return None
        finally:
            del texts
        self.vectorizers[name] = vec
        self.block_sizes[name] = block.shape[1]
        logger.info("필드 '%s': 특징 %d개, nnz %d", name, block.shape[1], block.nnz)
        return self._scaled(block, spec)

    def transform(self, docs: Sequence[Dict[str, Any]]) -> csr_matrix:
        blocks = []
        for name, vec in self.vectorizers.items():
            spec = self.fields[name]
            texts = [field_text(d, spec) for d in docs]
            blocks.append(self._scaled(vec.transform(texts), spec))
        if not blocks:
            raise ValueError("학습된 벡터라이저가 없다")
        return hstack(blocks, format="csr")

    @staticmethod
    def _scaled(block: csr_matrix, spec: Dict[str, Any]) -> csr_matrix:
        w = float(spec.get("weight", 1.0))
        if w != 1.0:
            # `block * w` 는 사본을 하나 더 만든다. 제자리에서 곱한다.
            block.data *= w
        return block

    def describe(self) -> Dict[str, Dict[str, Any]]:
        return {name: {"features": self.block_sizes.get(name, 0), "weight": self.fields[name].get("weight", 1.0)} for name in self.vectorizers}


def _assemble_spilled(spill: Path, parts: List[Tuple[str, int]], n_rows: int, dtype: Any) -> csr_matrix:
    """디스크에 흩어 둔 블록을 CSR 하나로 잇는다. 한 번에 한 블록만 읽는다.

    `hstack` 은 블록 전부와 결과를 동시에 들고 있어야 한다. 여기서는 결과 배열만
    미리 잡아 두고, 블록은 `mmap` 으로 필요한 만큼만 읽어 그 자리에 옮겨 적는다.
    블록 쪽 상주 메모리는 페이지 캐시라 부족하면 커널이 알아서 회수한다.

    행마다 블록들의 열을 차례로 이어 붙이는 것이라, 한 행 안에서 원본도 목적지도
    연속이다. 그래서 행 단위 복사 하나로 끝난다.
    """
    n_cols = 0
    layout: List[Tuple[str, int, np.ndarray]] = []
    total_nnz = 0
    counts = np.zeros(n_rows, dtype=np.int64)
    for name, cols in parts:
        indptr = np.load(spill / f"{name}.indptr.npy")
        counts += np.diff(indptr)
        total_nnz += int(indptr[-1])
        layout.append((name, n_cols, indptr))
        n_cols += cols

    # 열 번호와 누적 위치가 int32 안에 들어가는지 본다. 넘으면 조용히 음수가 된다.
    idx_dtype = np.int32 if total_nnz < np.iinfo(np.int32).max and n_cols < np.iinfo(np.int32).max else np.int64
    indptr_out = np.zeros(n_rows + 1, dtype=idx_dtype)
    np.cumsum(counts, out=indptr_out[1:])
    del counts

    # 결과도 디스크에 만든다. 이게 스필의 두 번째 이유다.
    #
    # liblinear 은 fit 할 때 행렬을 자기 자료구조로 한 벌 더 옮겨 담는다. 비영요소당
    # 16바이트다(2.5만 건 실측: nnz 4,570만, 행렬 0.5GB, fit 이 0.7GB 추가).
    # 23만 건이면 nnz 4.58억이라 사본만 7.3GB다. 행렬까지 익명 메모리로 들고 있으면
    # 12.8GB 가 되어 여유 10GB 기계에서 죽는다.
    #
    # 행렬을 파일로 두면 그 5.5GB 는 페이지 캐시가 된다. 모자라면 커널이 회수하고
    # 필요할 때 다시 읽는다. 느려질 뿐 죽지 않는다. 남는 익명 메모리는 liblinear
    # 사본뿐이다. 값은 그대로라 모델도 그대로다.
    data = np.lib.format.open_memmap(spill / "X.data.npy", mode="w+", dtype=dtype, shape=(total_nnz,))
    indices = np.lib.format.open_memmap(spill / "X.indices.npy", mode="w+", dtype=idx_dtype, shape=(total_nnz,))
    cursor = indptr_out[:-1].astype(np.int64)

    for name, offset, bp in layout:
        bd = np.load(spill / f"{name}.data.npy", mmap_mode="r")
        bi = np.load(spill / f"{name}.indices.npy", mmap_mode="r")
        for row in range(n_rows):
            lo, hi = int(bp[row]), int(bp[row + 1])
            if lo == hi:
                continue
            at = int(cursor[row])
            data[at : at + hi - lo] = bd[lo:hi]
            indices[at : at + hi - lo] = bi[lo:hi]
            indices[at : at + hi - lo] += offset
            cursor[row] = at + hi - lo
        del bd, bi
        gc.collect()
        (spill / f"{name}.data.npy").unlink(missing_ok=True)
        (spill / f"{name}.indices.npy").unlink(missing_ok=True)
        (spill / f"{name}.indptr.npy").unlink(missing_ok=True)
        logger.info("블록 '%s' 합침 (열 %d~%d)", name, offset, offset + dict(parts)[name])

    data.flush()
    indices.flush()
    # 쓰기용 매핑을 읽기 전용으로 다시 연다. fit 은 읽기만 하고, 읽기 전용이면
    # 커널이 페이지를 그냥 버릴 수 있어 회수가 싸진다.
    del data, indices
    gc.collect()
    out = csr_matrix(
        (
            np.load(spill / "X.data.npy", mmap_mode="r"),
            np.load(spill / "X.indices.npy", mmap_mode="r"),
            indptr_out,
        ),
        shape=(n_rows, n_cols),
    )
    logger.info("행렬을 %s 에 두고 mmap 으로 넘긴다", spill)
    return out
