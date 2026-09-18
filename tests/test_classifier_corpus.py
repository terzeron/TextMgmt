#!/usr/bin/env python3
"""backend/classifier/corpus.py 중 ES 를 거치는 경로와 publisher 뒷마감 테스트.

ES 조회와 EPUB 읽기는 통합 테스트에서 안 밟히는 구간이라 여기서 직접 확인한다.
"""

import json
from pathlib import Path
from typing import Any, Dict, List

import pytest

from backend.classifier import corpus as corpus_mod
from backend.classifier.corpus import (
    attach_publishers,
    collect,
    fetch_by_inode,
    fetch_by_path,
    scan_documents,
)


class FakeES:
    """es_manager 흉내. 필요한 속성만 가진다."""

    def __init__(self, get_result: Any = None, search_result: Any = None, raise_on: str = ""):
        self.index_name = "books"
        self.es = self
        self._get_result = get_result
        self._search_result = search_result
        self._raise_on = raise_on
        self.calls: List[Dict[str, Any]] = []

    def get(self, **kwargs):
        self.calls.append({"op": "get", **kwargs})
        if self._raise_on == "get":
            raise RuntimeError("ES 없음")
        return self._get_result

    def search(self, **kwargs):
        self.calls.append({"op": "search", **kwargs})
        if self._raise_on == "search":
            raise RuntimeError("ES 없음")
        return self._search_result


def _hit(doc_id: str, category: str = "2_소설한국", summary: str = "본문", file_path: str = "2_소설한국/a.epub", file_type: str = "epub", publisher: str = ""):
    return {"_id": doc_id, "_source": {"category": category, "summary": summary, "file_path": file_path, "file_type": file_type, "publisher": publisher, "title": "제목", "author": "저자"}}


def test_scan_documents_yields_records(monkeypatch: pytest.MonkeyPatch):
    import elasticsearch.helpers as helpers

    captured: Dict[str, Any] = {}

    def fake_scan(es, **kwargs):
        captured.update(kwargs)
        yield _hit("1")
        yield _hit("2", category="4_역사인물/하위")

    monkeypatch.setattr(helpers, "scan", fake_scan)
    docs = list(scan_documents(FakeES(), batch=7))

    assert [d["id"] for d in docs] == ["1", "2"]
    # 카테고리가 경로면 최상위만 레이블로 쓴다.
    assert docs[1]["cat"] == "4_역사인물"
    assert captured["index"] == "books"
    assert captured["size"] == 7


def test_scan_documents_honors_explicit_index(monkeypatch: pytest.MonkeyPatch):
    import elasticsearch.helpers as helpers

    captured: Dict[str, Any] = {}

    def fake_scan(es, **kwargs):
        captured.update(kwargs)
        return iter(())

    monkeypatch.setattr(helpers, "scan", fake_scan)
    assert list(scan_documents(FakeES(), index="other")) == []
    assert captured["index"] == "other"


def test_fetch_by_inode_found_and_missing():
    es = FakeES(get_result=_hit("42"))
    doc = fetch_by_inode(es, 42)
    assert doc is not None and doc["id"] == "42"
    assert es.calls[0]["id"] == "42"

    # 못 찾으면 None 을 주고 호출부가 파일에서 직접 읽는 경로로 넘어간다.
    assert fetch_by_inode(FakeES(raise_on="get"), 42) is None


def test_fetch_by_inode_uses_explicit_index():
    es = FakeES(get_result=_hit("42"))
    fetch_by_inode(es, 42, index="other")
    assert es.calls[0]["index"] == "other"


def test_fetch_by_path_found_empty_and_error():
    es = FakeES(search_result={"hits": {"hits": [_hit("7")]}})
    doc = fetch_by_path(es, "2_소설한국/a.epub")
    assert doc is not None and doc["id"] == "7"

    assert fetch_by_path(FakeES(search_result={"hits": {"hits": []}}), "x") is None
    # hits 키 자체가 없어도 죽지 않는다.
    assert fetch_by_path(FakeES(search_result={}), "x") is None
    assert fetch_by_path(FakeES(raise_on="search"), "x") is None


def test_fetch_by_path_uses_explicit_index():
    es = FakeES(search_result={"hits": {"hits": []}})
    fetch_by_path(es, "x", index="other")
    assert es.calls[0]["index"] == "other"


def test_collect_writes_only_trainable_rows(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    import elasticsearch.helpers as helpers

    def fake_scan(es, **kwargs):
        yield _hit("1", category="2_소설한국", summary="본문" * 20)
        yield _hit("2", category="2_소설한국", summary="본문" * 20)
        # 레이블 꼴이 아니면 운영용 디렉토리라 학습에서 뺀다.
        yield _hit("3", category="trash", summary="본문" * 20)
        # 본문이 짧으면 표본 수만 늘리고 품질을 떨어뜨린다.
        yield _hit("4", category="2_소설한국", summary="짧음")
        # 제외 접두사.
        yield _hit("5", category="9_북스캔OCR", summary="본문" * 20)

    monkeypatch.setattr(helpers, "scan", fake_scan)
    out = tmp_path / "corpus.jsonl"
    stats = collect(FakeES(), out, min_chars=10, excluded_prefixes=["9_"])

    assert stats == {"kept": 2, "skipped": 3, "categories": 1, "per_category": {"2_소설한국": 2}}
    assert len(out.read_text(encoding="utf-8").strip().splitlines()) == 2


def test_publisher_of_swallows_reader_errors(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    def boom(path):
        raise RuntimeError("EPUB 깨짐")

    monkeypatch.setattr("backend.classifier.reader.read_epub_publisher", boom)
    # 한 권이 깨져도 전체 뒷마감이 멈추면 안 된다.
    assert corpus_mod._publisher_of(tmp_path / "x.epub") == ""


def _epub_docs():
    return [
        {"id": "1", "type": "epub", "publisher": "", "path": "2_소설외국/a.epub"},
        {"id": "2", "type": "epub", "publisher": "을유문화사", "path": "2_소설외국/b.epub"},
        {"id": "3", "type": "pdf", "publisher": "", "path": "2_소설외국/c.pdf"},
    ]


def test_attach_publishers_reads_only_missing_epubs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(corpus_mod, "_publisher_of", lambda p: "민음사")
    docs = _epub_docs()
    cache_path = tmp_path / "cache" / "pub.json"

    stats = attach_publishers(docs, cache_path, tmp_path, workers=1)

    # ES 에 이미 값이 있는 문서와 EPUB 이 아닌 문서는 읽지 않는다.
    assert stats["read"] == 1
    assert stats["with_publisher"] == 2
    assert docs[0]["publisher"] == "민음사"
    assert json.loads(cache_path.read_text(encoding="utf-8")) == {"1": "민음사"}


def test_attach_publishers_uses_cache_and_skips_disk(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    cache_path = tmp_path / "pub.json"
    cache_path.write_text(json.dumps({"1": "창비"}), encoding="utf-8")

    def should_not_run(path):
        raise AssertionError("캐시에 있으면 디스크를 읽으면 안 된다")

    monkeypatch.setattr(corpus_mod, "_publisher_of", should_not_run)
    docs = _epub_docs()
    stats = attach_publishers(docs, cache_path, tmp_path)

    assert stats["read"] == 0
    assert docs[0]["publisher"] == "창비"


def test_attach_publishers_rebuilds_broken_cache(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    cache_path = tmp_path / "pub.json"
    cache_path.write_text("이건 JSON 이 아니다", encoding="utf-8")
    monkeypatch.setattr(corpus_mod, "_publisher_of", lambda p: "문학동네")

    docs = _epub_docs()
    stats = attach_publishers(docs, cache_path, tmp_path, workers=1)

    # 캐시가 깨졌으면 새로 만든다. 뒷마감 자체는 계속한다.
    assert stats["read"] == 1
    assert docs[0]["publisher"] == "문학동네"


def test_attach_publishers_skips_documents_without_path(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setattr(corpus_mod, "_publisher_of", lambda p: "민음사")
    docs = [{"id": "1", "type": "epub", "publisher": "", "path": ""}]

    stats = attach_publishers(docs, tmp_path / "pub.json", tmp_path)

    assert stats["read"] == 0
    assert docs[0]["publisher"] == ""


def test_attach_publishers_logs_progress(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture):
    monkeypatch.setattr(corpus_mod, "_publisher_of", lambda p: "민음사")
    docs = [{"id": str(i), "type": "epub", "publisher": "", "path": f"2_소설외국/{i}.epub"} for i in range(4)]

    with caplog.at_level("INFO", logger="backend.classifier.corpus"):
        stats = attach_publishers(docs, tmp_path / "pub.json", tmp_path, workers=1, progress_every=2)

    assert stats["read"] == 4
    # 오래 도는 작업이라 진행 상황을 남긴다.
    messages = [r.getMessage() for r in caplog.records]
    assert any("publisher 2/4" in m for m in messages)
    assert any("publisher 4/4" in m for m in messages)
