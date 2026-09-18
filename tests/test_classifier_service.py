#!/usr/bin/env python3
"""BookCategoryClassifier 의 대비책 경로 테스트.

ES 가 문서 대부분을 갖고 있어 평소엔 안 밟히는 갈래(ES 미적재, stat 실패, 본문 빈 문서,
파일 읽기 실패, 모델 없음)를 직접 확인한다.
"""

from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

import pytest

from backend.classifier import BookCategoryClassifier
from backend.classifier.model import Prediction


class FakeModel:
    min_confidence = 0.3

    def __init__(self):
        self.predict_one_calls: List[Dict[str, Any]] = []
        self.predict_calls: List[Sequence[Dict[str, Any]]] = []
        self.last_floor: Optional[float] = None

    def predict_one(self, doc, min_confidence=None):
        self.predict_one_calls.append(doc)
        self.last_floor = min_confidence
        return Prediction("2_소설한국", 0.9, [], "판정함")

    def predict(self, docs, min_confidence=None):
        self.predict_calls.append(docs)
        self.last_floor = min_confidence
        return [Prediction("2_소설한국", 0.9, [], "판정함") for _ in docs]


def _classifier(tmp_path: Path, es_manager: Any = None, model: Any = None) -> BookCategoryClassifier:
    return BookCategoryClassifier(model=model or FakeModel(), es_manager=es_manager, library_root=tmp_path)


def test_build_document_falls_back_to_file_when_no_es(tmp_path: Path):
    f = tmp_path / "2_소설한국" / "어떤책.txt"
    f.parent.mkdir(parents=True)
    f.write_text("그는 아무 말도 하지 않았다 " * 10, encoding="utf-8")

    doc = _classifier(tmp_path).build_document(f)

    assert doc["name"] == "어떤책.txt"
    assert doc["type"] == "txt"
    assert "아무 말도" in doc["text"]
    # EPUB 이 아니면 OPF 를 풀지 않는다.
    assert doc["publisher"] == ""


def test_build_document_fills_name_when_es_record_has_none(tmp_path: Path):
    class ES:
        index_name = "books"

        class es:
            @staticmethod
            def get(**kwargs):
                # file_path 가 비면 name 도 빈다. 파일명으로 메워야 특징이 살아난다.
                return {"_id": "1", "_source": {"category": "2_소설한국", "summary": "본문" * 20, "file_path": "", "file_type": "txt", "publisher": "민음사"}}

    f = tmp_path / "이름없는레코드.txt"
    f.write_text("내용", encoding="utf-8")

    doc = _classifier(tmp_path, es_manager=ES()).build_document(f)

    assert doc["name"] == "이름없는레코드.txt"
    assert doc["publisher"] == "민음사"


def test_from_elasticsearch_survives_stat_failure(tmp_path: Path):
    """파일이 사라져 inode 를 못 읽어도 경로로 다시 찾는다."""
    searched: Dict[str, Any] = {}

    class ES:
        index_name = "books"

        class es:
            @staticmethod
            def get(**kwargs):
                raise AssertionError("inode 가 없으면 get 을 부르면 안 된다")

            @staticmethod
            def search(**kwargs):
                searched.update(kwargs)
                return {"hits": {"hits": [{"_id": "9", "_source": {"category": "2_소설한국", "summary": "본문" * 20, "file_path": "2_소설한국/a.txt", "file_type": "txt"}}]}}

    missing = tmp_path / "2_소설한국" / "없는파일.txt"
    doc = _classifier(tmp_path, es_manager=ES()).build_document(missing)

    assert doc["id"] == "9"
    assert searched["query"]["term"]["file_path"] == "2_소설한국/없는파일.txt"


def test_from_elasticsearch_reads_file_when_indexed_text_is_empty(tmp_path: Path):
    """인덱싱은 됐지만 본문이 비었으면 ES 가 도움이 안 된다 — 파일에서 읽는다."""

    class ES:
        index_name = "books"

        class es:
            @staticmethod
            def get(**kwargs):
                return {"_id": "1", "_source": {"category": "2_소설한국", "summary": "", "file_path": "2_소설한국/a.txt", "file_type": "txt"}}

    f = tmp_path / "2_소설한국" / "a.txt"
    f.parent.mkdir(parents=True)
    f.write_text("그는 아무 말도 하지 않았다 " * 10, encoding="utf-8")

    doc = _classifier(tmp_path, es_manager=ES()).build_document(f)

    assert "아무 말도" in doc["text"]


def test_indexed_path_keeps_absolute_path_outside_library(tmp_path: Path):
    outside = tmp_path.parent / "밖에있는파일.txt"
    clf = _classifier(tmp_path)
    assert clf._indexed_path(outside) == str(outside)


def test_read_text_swallows_reader_errors(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    def boom(*args, **kwargs):
        raise RuntimeError("파서가 죽음")

    monkeypatch.setattr("backend.classifier.reader.read_document_text", boom)
    f = tmp_path / "a.txt"
    f.write_text("내용", encoding="utf-8")
    # 한 권이 읽히지 않아도 판정 자체는 계속해야 한다.
    assert BookCategoryClassifier._read_text(f) == ""


def test_read_publisher_skips_non_epub_and_swallows_errors(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    assert BookCategoryClassifier._read_publisher(tmp_path / "a.pdf") == ""

    def boom(*args, **kwargs):
        raise RuntimeError("OPF 가 깨짐")

    monkeypatch.setattr("backend.classifier.reader.read_epub_publisher", boom)
    assert BookCategoryClassifier._read_publisher(tmp_path / "a.epub") == ""


def test_classify_without_model_returns_no_verdict(tmp_path: Path):
    clf = BookCategoryClassifier(model=None, library_root=tmp_path)
    assert bool(clf) is False

    f = tmp_path / "a.txt"
    f.write_text("내용", encoding="utf-8")

    for pred in (clf.classify_path(f), clf.classify_document({"name": "a.txt"})):
        assert pred.category is None
        assert pred.reason == "모델 파일이 없어 판정하지 않음"

    preds = clf.classify_documents([{"name": "a.txt"}, {"name": "b.txt"}])
    assert len(preds) == 2
    assert all(p.category is None for p in preds)


def test_classify_documents_passes_confidence_floor(tmp_path: Path):
    model = FakeModel()
    clf = _classifier(tmp_path, model=model)

    clf.classify_documents([{"name": "a.txt"}])
    # 호출자가 안 주면 모델 기본값을 쓴다.
    assert model.last_floor == pytest.approx(0.3)

    clf.classify_documents([{"name": "a.txt"}], min_confidence=0.75)
    assert model.last_floor == pytest.approx(0.75)

    clf.classify_document({"name": "a.txt"}, min_confidence=0.5)
    assert model.last_floor == pytest.approx(0.5)


def test_min_confidence_falls_back_to_config_without_model(tmp_path: Path):
    clf = BookCategoryClassifier(model=None, library_root=tmp_path)
    # 모델이 없으면 판정 자체를 안 하므로 어떤 값이든 실제로는 안 쓰인다.
    assert isinstance(clf.min_confidence, float)

    explicit = BookCategoryClassifier(model=None, min_confidence=0.42, library_root=tmp_path)
    assert explicit.min_confidence == pytest.approx(0.42)
