#!/usr/bin/env python3
"""
지도학습 기반 도서 자동분류

`BookCategoryClassifier` 하나가 공개 API 다. 파일 경로를 주면 카테고리를 돌려준다.

특징은 ES 에서 가져온다. ES 는 inode 를 _id 로 써서 모든 문서를 인덱싱해 두었고,
본문 앞부분·제목·저자·확장자가 들어 있다. /mnt/data 는 회전 디스크라 파일을 직접
읽으면 느리다. ES 에 없을 때만 파일을 읽는다.

    clf = BookCategoryClassifier(es_manager=es)
    result = clf.classify_path(Path("/mnt/data/text/0_nf/어떤책.epub"))
    if result.category:
        ...
"""

import logging
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

from backend.classifier.config import load_config
from backend.classifier.model import CategoryModel, Prediction, get_default_model, reset_default_model

logger = logging.getLogger(__name__)

__all__ = ["BookCategoryClassifier", "CategoryModel", "Prediction", "reset_default_model"]

# 본문을 파일에서 직접 읽을 때 몇 글자까지 볼 것인가. ES summary 상한과 맞춘다.
FALLBACK_TEXT_CHARS = 4096


class BookCategoryClassifier:
    """모델 하나를 들고 파일이나 레코드를 판정한다."""

    def __init__(self, model: Optional[CategoryModel] = None, es_manager: Any = None, min_confidence: Optional[float] = None, library_root: Optional[Path | str] = None):
        self.model = model if model is not None else get_default_model()
        self.es_manager = es_manager
        self._min_confidence = min_confidence
        # ES 의 file_path 는 이 디렉토리 기준 상대 경로다. 절대 경로로 질의하면 안 맞는다.
        self.library_root = Path(library_root) if library_root else Path(load_config()["corpus"]["library_root"])

    def __bool__(self) -> bool:
        return self.model is not None

    @property
    def min_confidence(self) -> float:
        if self._min_confidence is not None:
            return float(self._min_confidence)
        if self.model is not None:
            return self.model.min_confidence
        # 모델이 없으면 판정 자체를 안 한다. 설정값이 있어도 쓸 곳이 없다.
        configured = load_config()["decision"].get("min_confidence")
        return float(configured) if configured is not None else 1.0

    # ------------------------------------------------------------------
    # 특징 수집
    # ------------------------------------------------------------------

    def build_document(self, fpath: Path) -> Dict[str, Any]:
        """판정에 쓸 레코드를 만든다. ES 를 먼저 보고, 없으면 파일에서 읽는다."""
        doc = self._from_elasticsearch(fpath)
        if doc is None:
            doc = {"id": "", "cat": "", "text": "", "title": "", "author": "", "type": fpath.suffix.lstrip(".").lower(), "name": fpath.name, "path": str(fpath)}
            doc["text"] = self._read_text(fpath)
        # publisher 는 ES 가 갖고 있으면 그대로 쓴다. 없을 때만 EPUB 의 OPF 를 푼다.
        if not doc.get("publisher"):
            doc["publisher"] = self._read_publisher(fpath)
        if not doc.get("name"):
            doc["name"] = fpath.name
        return doc

    def _from_elasticsearch(self, fpath: Path) -> Optional[Dict[str, Any]]:
        if self.es_manager is None:
            return None
        from backend.classifier.corpus import fetch_by_inode, fetch_by_path

        try:
            inode = fpath.stat().st_ino
        except OSError:
            inode = None
        doc = fetch_by_inode(self.es_manager, inode) if inode is not None else None
        if doc is None:
            doc = fetch_by_path(self.es_manager, self._indexed_path(fpath))
        if doc is not None and not doc.get("text"):
            # 인덱싱은 됐지만 본문이 비었으면 ES 가 도움이 안 된다.
            doc["text"] = self._read_text(fpath)
        return doc

    def _indexed_path(self, fpath: Path) -> str:
        """ES 에 들어 있는 모양의 경로. loader 가 라이브러리 루트 기준 상대 경로로 넣는다."""
        try:
            return str(fpath.resolve().relative_to(self.library_root.resolve()))
        except ValueError:
            # 라이브러리 밖의 파일이다. 있는 그대로 질의한다.
            return str(fpath)

    @staticmethod
    def _read_text(fpath: Path) -> str:
        from backend.classifier.reader import read_document_text

        try:
            # ES summary 는 본문 앞부분만 담는다. 학습과 판정의 입력을 맞추려면
            # 여기서도 뒷부분을 붙이지 않아야 한다.
            return read_document_text(fpath, head_chars=FALLBACK_TEXT_CHARS, tail_chars=0)
        except Exception as e:
            logger.debug("본문을 읽지 못했다 (%s): %s", fpath, e)
            return ""

    @staticmethod
    def _read_publisher(fpath: Path) -> str:
        from backend.classifier.reader import read_epub_publisher

        if fpath.suffix.lower() != ".epub":
            return ""
        try:
            return read_epub_publisher(fpath)
        except Exception as e:
            logger.debug("publisher 를 읽지 못했다 (%s): %s", fpath, e)
            return ""

    # ------------------------------------------------------------------
    # 판정
    # ------------------------------------------------------------------

    def classify_path(self, fpath: Path, min_confidence: Optional[float] = None) -> Prediction:
        if self.model is None:
            return Prediction(None, 0.0, [], "모델 파일이 없어 판정하지 않음")
        return self.classify_document(self.build_document(fpath), min_confidence=min_confidence)

    def classify_document(self, doc: Dict[str, Any], min_confidence: Optional[float] = None) -> Prediction:
        if self.model is None:
            return Prediction(None, 0.0, [], "모델 파일이 없어 판정하지 않음")
        floor = self.min_confidence if min_confidence is None else float(min_confidence)
        return self.model.predict_one(doc, min_confidence=floor)

    def classify_documents(self, docs: Sequence[Dict[str, Any]], min_confidence: Optional[float] = None) -> List[Prediction]:
        if self.model is None:
            return [Prediction(None, 0.0, [], "모델 파일이 없어 판정하지 않음") for _ in docs]
        floor = self.min_confidence if min_confidence is None else float(min_confidence)
        return self.model.predict(docs, min_confidence=floor)
