#!/usr/bin/env python3
"""자동분류 CLI 의 오류 경로와 긴 작업의 진행 표시를 확인한다."""

import itertools
import json
from types import SimpleNamespace

import pytest

import utils.classify_cli as cli
from backend.classifier.corpus import read_jsonl, write_jsonl
from tests.test_classify_cli import FakeBookstoreService, TEST_CONFIG, model_file  # noqa: F401  (fixture 재사용)
from tests.test_classifier import VOCAB, make_docs
from utils.classify_cli import main


# ---- holdout manifest ----


def test_manifest_with_wrong_shape_is_rejected(tmp_path):
    manifest = tmp_path / "holdout.json"
    manifest.write_text(json.dumps({"document_keys": ["a", 3]}), encoding="utf-8")

    with pytest.raises(ValueError, match="잘못된 holdout manifest"):
        cli._load_manifest_holdout(make_docs(), manifest)


def test_manifest_keys_missing_from_corpus_are_warned_but_found_ones_kept(tmp_path, caplog):
    docs = make_docs()
    manifest = tmp_path / "holdout.json"
    manifest.write_text(json.dumps({"document_keys": [str(docs[0]["id"]), "없는-문서"]}), encoding="utf-8")

    with caplog.at_level("WARNING", logger="classify_cli"):
        held = cli._load_manifest_holdout(docs, manifest)

    assert [doc["id"] for doc in held] == [docs[0]["id"]]
    assert "1건을 현재 코퍼스에서 찾지 못했다" in caplog.text


def test_manifest_with_no_matching_documents_is_rejected(tmp_path):
    manifest = tmp_path / "holdout.json"
    manifest.write_text(json.dumps({"document_keys": ["없는-문서"]}), encoding="utf-8")

    with pytest.raises(ValueError, match="찾지 못했다"):
        cli._load_manifest_holdout(make_docs(), manifest)


# ---- collect / train 출력 ----


def test_collect_prints_bottom_ten_categories_when_there_are_many(tmp_path, monkeypatch, capsys):
    per_category = {f"카테고리{n:02d}": n + 1 for n in range(12)}

    def fake_collect(es, out_path, **kwargs):
        write_jsonl(make_docs(), out_path)
        return {"kept": 12, "skipped": 0, "categories": len(per_category), "per_category": per_category}

    monkeypatch.setattr("backend.classifier.corpus.collect", fake_collect)
    monkeypatch.setattr(cli, "_es_manager", lambda index=None: object())

    assert main(["collect", "--out", str(tmp_path / "corpus.jsonl")]) == 0

    printed = capsys.readouterr().out
    assert "... 하위 10개" in printed
    assert "카테고리11" in printed


def test_train_prints_where_the_config_was_saved(tmp_path, monkeypatch, capsys):
    corpus = tmp_path / "corpus.jsonl"
    write_jsonl(make_docs(), corpus)
    monkeypatch.setattr(cli, "load_config", lambda path=None: TEST_CONFIG)
    saved_to = tmp_path / "config.json"
    monkeypatch.setattr(cli, "save_config", lambda cfg: saved_to)

    assert main(["train", "--corpus", str(corpus), "--out", str(tmp_path / "model.joblib"), "--save-config"]) == 0

    assert f"설정 저장: {saved_to}" in capsys.readouterr().out


# ---- bookstore-policy ----


def _run_policy(model_file, monkeypatch, tmp_path, *extra, docs=None):
    import backend.book_classifier as bc

    path, corpus = model_file
    service = FakeBookstoreService([("3_무협", "3_무협", "")])
    monkeypatch.setattr(bc, "BookClassifierService", lambda **kwargs: service)
    if docs is not None:
        monkeypatch.setattr(cli, "_holdout_docs", lambda *a, **k: list(docs))
    out = tmp_path / "policy.json"
    code = main(["bookstore-policy", "--model", str(path), "--corpus", str(corpus), "--delay", "0", "--out", str(out), *extra])
    return code, json.loads(out.read_text(encoding="utf-8"))


def test_bookstore_policy_samples_the_boundary_above_the_refused_range(model_file, monkeypatch, tmp_path, capsys):
    # 시험용 모델은 확신도가 높아 거부 구간이 비어 있다. 경계 배수를 키워 모든 문서가 들어오게 한다.
    monkeypatch.setattr(cli, "BOUNDARY_FACTOR", 1000.0)

    code, saved = _run_policy(model_file, monkeypatch, tmp_path, "--sample", "6", "--sample-from", "boundary")

    assert code == 0
    assert saved["sample_from"] == "boundary"
    assert "거부 구간과 그 위" in capsys.readouterr().out


def test_bookstore_policy_reports_progress_and_skips_empty_bands(model_file, monkeypatch, tmp_path, capsys):
    """같은 문서를 120번 재면 확신도가 모두 같아 첫 구간이 비고, 100건째에 진행 상황이 찍힌다."""
    _, corpus = model_file
    doc = read_jsonl(corpus)[0]
    ticks = itertools.count(0.0, 1.0)
    monkeypatch.setattr(cli, "time", SimpleNamespace(time=lambda: next(ticks)))

    code, saved = _run_policy(model_file, monkeypatch, tmp_path, "--sample", "120", "--sample-from", "all", "--bands", "3", docs=[doc] * 120)

    assert code == 0
    assert saved["sample"] == 120
    printed = capsys.readouterr().out
    assert "  100/120" in printed
    assert all(band["n"] > 0 for band in saved["bands"])


def test_reband_skips_bands_without_rows(tmp_path, capsys):
    rows = [{"confidence": 0.1, "model_ok": True, "store_ok": True, "store_answered": True} for _ in range(4)]
    saved = tmp_path / "policy.json"
    saved.write_text(json.dumps({"rows": rows}), encoding="utf-8")

    assert main(["bookstore-policy", "--reband", str(saved), "--bands", "3"]) == 0

    out = json.loads(saved.read_text(encoding="utf-8"))
    assert [band["n"] for band in out["bands"]] == [4]


# ---- reclassify 진행 표시 ----


def test_reclassify_prints_progress_every_500_files(model_file, tmp_path, monkeypatch, capsys):
    path, _ = model_file
    library = tmp_path / "library"
    source = library / "0_inbox"
    source.mkdir(parents=True)
    book = source / "a.txt"
    book.write_text(" ".join([VOCAB["3_무협"]] * 3), encoding="utf-8")
    monkeypatch.setattr(cli, "_iter_files", lambda root: [book] * 500)

    assert main(["reclassify", "0_inbox", "--model", str(path), "--library-root", str(library), "--no-es"]) == 0

    assert "500/500 처리" in capsys.readouterr().out
