#!/usr/bin/env python3
"""자동분류 모델 내보내기(Rust 추론용) 테스트

작은 합성 모델을 학습해 내보내고, 파일 구조·sha256·shape·전치 레이아웃을 검증한다.
`python tests/test_classifier_export.py` 로 실행하면 저장소 fixture
(`tests/fixtures/classifier_tiny/`)를 다시 쓴다. Rust parity 테스트가 이 fixture 를 읽는다.
"""

import hashlib
import json
import sys
from pathlib import Path
from typing import Any, Dict, List

import numpy as np
import pytest
from sklearn.multiclass import OneVsRestClassifier
from sklearn.svm import LinearSVC

ROOT = Path(__file__).resolve().parent.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from backend.classifier.export import FORMAT_VERSION, export_model, main  # noqa: E402
from backend.classifier.features import FeatureSpace  # noqa: E402
from backend.classifier.model import CategoryModel  # noqa: E402

FIXTURE_DIR = ROOT / "tests" / "fixtures" / "classifier_tiny"

# word/char/char_wb 분석기, ngram (1,1)(1,2)(2,4), weight 1.0/0.3/3.0, max_chars, name 소스를 모두 쓴다.
TINY_FIELDS: Dict[str, Dict[str, Any]] = {
    "body_word": {"source": "text", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 500, "max_chars": 0},
    "body_char": {"source": "text", "weight": 1.0, "analyzer": "char_wb", "ngram": [2, 4], "min_df": 1, "max_features": 400, "max_chars": 30},
    "filename": {"source": "name", "weight": 1.0, "analyzer": "char", "ngram": [1, 3], "min_df": 1, "max_features": 300, "max_chars": 20},
    "title": {"source": "title", "weight": 1.0, "analyzer": "char", "ngram": [2, 4], "min_df": 1, "max_features": 300, "max_chars": 0},
    "author": {"source": "author", "weight": 0.3, "analyzer": "char_wb", "ngram": [2, 4], "min_df": 1, "max_features": 100, "max_chars": 0},
    "publisher": {"source": "publisher", "weight": 3.0, "analyzer": "word", "ngram": [1, 2], "min_df": 1, "max_features": 50, "max_chars": 0},
    "file_type": {"source": "type", "weight": 0.5, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 10, "max_chars": 0},
}

TINY_VOCAB = {
    "3_무협": "무림맹 장문인 내공 運氣調息 화산파 검법 강호 協客",
    "3_판타지": "dragon mana 던전 길드 마법사 level-up 소환 マ王",
    "5_수학과학일반": "미분 적분 function vector 행렬 확률 통계 방정식",
}


def make_tiny_model() -> CategoryModel:
    """클래스 3개, 문서 36개짜리 합성 모델. 실제 도서 자료는 쓰지 않는다."""
    docs: List[Dict[str, Any]] = []
    labels: List[str] = []
    for cat, words in TINY_VOCAB.items():
        tokens = words.split()
        for i in range(12):
            shifted = tokens[i % len(tokens) :] + tokens[: i % len(tokens)]
            docs.append(
                {
                    "text": " ".join(shifted * 2) + " ABC def",
                    "name": f"{tokens[i % len(tokens)]}_{1000 + i}_{i}.epub",
                    "title": f"{tokens[i % len(tokens)]} 이야기 {i}",
                    "author": [f"작가{i % 4}", "Kim"] if i % 3 == 0 else f"작가{i % 4}",
                    "publisher": f"출판사{len(cat)} 북스",
                    "type": "epub" if i % 2 == 0 else "txt",
                }
            )
            labels.append(cat)
    space = FeatureSpace(TINY_FIELDS)
    x = space.fit_transform(docs, dtype=np.float64)
    clf = OneVsRestClassifier(LinearSVC(C=1.0, dual="auto", max_iter=2000)).fit(x, labels)
    config = {"decision": {"min_confidence": None, "target_accuracy": 0.9, "temperature": 0.5}, "fields": TINY_FIELDS}
    meta = {"calibrated_threshold": np.float64(0.4), "trained_at": "2026-01-01T00:00:00", "n_docs": np.int64(len(docs)), "holdout": None}
    return CategoryModel({"feature_space": space, "classifier": clf, "classes": np.asarray(sorted(set(labels))), "config": config, "meta": meta})


@pytest.fixture(scope="module")
def tiny() -> CategoryModel:
    return make_tiny_model()


@pytest.fixture(scope="module")
def exported(tiny: CategoryModel, tmp_path_factory: pytest.TempPathFactory):
    out = tmp_path_factory.mktemp("export")
    summary = export_model(tiny, out)
    return out, summary


def sha256_of(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_manifest(out: Path) -> Dict[str, Any]:
    return json.loads((out / "model.json").read_text(encoding="utf-8"))


def test_manifest_top_level(tiny: CategoryModel, exported):
    out, summary = exported
    m = read_manifest(out)
    assert m["format_version"] == FORMAT_VERSION == 1
    assert m["classes"] == [str(c) for c in tiny.classes]
    assert m["n_classes"] == 3
    assert m["n_features"] == sum(f["size"] for f in m["fields"])
    assert m["coef_file"] == "coef.f32"
    assert len(m["intercept"]) == 3
    assert m["config"]["decision"]["temperature"] == 0.5
    # numpy 타입은 파이썬 기본형으로 바뀌어 있어야 한다.
    assert m["meta"]["calibrated_threshold"] == 0.4
    assert m["meta"]["n_docs"] == 36
    assert summary["n_features"] == m["n_features"]


def test_fields_order_offsets_and_options(tiny: CategoryModel, exported):
    out, _ = exported
    m = read_manifest(out)
    assert [f["name"] for f in m["fields"]] == list(tiny.feature_space.vectorizers.keys())
    offset = 0
    for f in m["fields"]:
        vec = tiny.feature_space.vectorizers[f["name"]]
        spec = TINY_FIELDS[f["name"]]
        assert f["offset"] == offset
        assert f["size"] == len(vec.vocabulary_)
        offset += f["size"]
        assert f["source"] == spec["source"]
        assert f["weight"] == spec["weight"]
        assert f["max_chars"] == spec["max_chars"]
        assert f["analyzer"] == spec["analyzer"]
        assert f["ngram_range"] == list(spec["ngram"])
        assert f["lowercase"] is True and f["sublinear_tf"] is True and f["norm"] == "l2"
        assert f["token_pattern"] == vec.token_pattern
    analyzers = {f["analyzer"] for f in m["fields"]}
    assert analyzers == {"word", "char", "char_wb"}


def test_vocab_and_idf_files(tiny: CategoryModel, exported):
    out, _ = exported
    m = read_manifest(out)
    for f in m["fields"]:
        vec = tiny.feature_space.vectorizers[f["name"]]
        vocab = json.loads((out / f["vocab_file"]).read_text(encoding="utf-8"))
        assert len(vocab) == f["size"] == len(set(vocab))
        for term, idx in vec.vocabulary_.items():
            assert vocab[idx] == term
        assert f["vocab_sha256"] == sha256_of(out / f["vocab_file"])
        raw = (out / f["idf_file"]).read_bytes()
        assert len(raw) == 4 * f["size"]
        idf = np.frombuffer(raw, dtype="<f4")
        np.testing.assert_allclose(idf, vec.idf_.astype(np.float32), rtol=0, atol=0)
        assert f["idf_sha256"] == hashlib.sha256(raw).hexdigest()


def test_coef_is_transposed_row_major_f32(tiny: CategoryModel, exported):
    out, _ = exported
    m = read_manifest(out)
    raw = (out / "coef.f32").read_bytes()
    assert len(raw) == m["n_features"] * m["n_classes"] * 4
    assert m["coef_sha256"] == hashlib.sha256(raw).hexdigest()
    coef = np.frombuffer(raw, dtype="<f4").reshape(m["n_features"], m["n_classes"])
    for k, est in enumerate(tiny.classifier.estimators_):
        np.testing.assert_array_equal(coef[:, k], est.coef_[0].astype(np.float32))
        assert m["intercept"][k] == float(est.intercept_[0])


def test_decision_matches_after_roundtrip(tiny: CategoryModel, exported):
    """내보낸 파일만으로 sklearn 결정값을 1e-4 안에서 재현한다(전치 레이아웃 검증)."""
    out, _ = exported
    m = read_manifest(out)
    coef = np.fromfile(out / "coef.f32", dtype="<f4").reshape(m["n_features"], m["n_classes"]).astype(np.float64)
    docs = [{"text": "무림맹 장문인 dragon 내공", "name": "무림맹_123456.epub", "title": "무림 이야기", "author": "작가1", "publisher": "출판사5 북스", "type": "epub"}]
    x = tiny.feature_space.transform(docs)
    expected = tiny.scores(docs)
    got = x.toarray().astype(np.float64) @ coef + np.asarray(m["intercept"])
    np.testing.assert_allclose(got, expected, atol=1e-4)


def test_golden_written_and_consistent(tiny: CategoryModel, tmp_path: Path):
    export_model(tiny, tmp_path, golden=12, seed=3)
    g = json.loads((tmp_path / "golden.json").read_text(encoding="utf-8"))
    assert len(g["cases"]) >= 12
    names = {c["label"] for c in g["cases"]}
    assert {"empty", "hangul", "english", "cjk", "whitespace", "digits_name", "nfd", "emoji"} <= names
    for case in g["cases"]:
        exp = tiny.scores([case["doc"]])[0]
        np.testing.assert_allclose(case["scores"], exp, atol=1e-9)
        pred = tiny.predict_one(case["doc"])
        assert case["reason"] == pred.reason
        assert case["category"] == pred.category
        assert [t["category"] for t in case["top"]] == [c for c, _ in pred.ranked]
    # 같은 시드면 같은 문서가 나온다.
    export_model(tiny, tmp_path / "again", golden=12, seed=3)
    g2 = json.loads((tmp_path / "again" / "golden.json").read_text(encoding="utf-8"))
    assert [c["doc"] for c in g["cases"]] == [c["doc"] for c in g2["cases"]]


def test_rejects_unsupported_vectorizer(tiny: CategoryModel, tmp_path: Path):
    vec = tiny.feature_space.vectorizers["title"]
    vec.set_params(strip_accents="unicode")
    try:
        with pytest.raises(ValueError, match="strip_accents"):
            export_model(tiny, tmp_path)
    finally:
        vec.set_params(strip_accents=None)


def test_main_cli(tiny: CategoryModel, tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    import backend.classifier.export as export_mod

    monkeypatch.setattr(export_mod.CategoryModel, "load", classmethod(lambda cls, path=None: tiny))
    out = tmp_path / "cli"
    assert main(["--model", "dummy.joblib", "--out", str(out), "--golden", "5", "--seed", "1"]) == 0
    assert (out / "model.json").exists() and (out / "golden.json").exists()


def test_committed_fixture_is_intact():
    """저장소 fixture 의 sha256 이 model.json 과 맞는지 본다(Rust 로더와 같은 검증)."""
    m = read_manifest(FIXTURE_DIR)
    assert m["format_version"] == 1
    assert sha256_of(FIXTURE_DIR / m["coef_file"]) == m["coef_sha256"]
    for f in m["fields"]:
        assert sha256_of(FIXTURE_DIR / f["vocab_file"]) == f["vocab_sha256"]
        assert sha256_of(FIXTURE_DIR / f["idf_file"]) == f["idf_sha256"]
    assert (FIXTURE_DIR / "golden.json").exists()


if __name__ == "__main__":
    import shutil

    if FIXTURE_DIR.exists():
        shutil.rmtree(FIXTURE_DIR)
    export_model(make_tiny_model(), FIXTURE_DIR, golden=40, seed=11)
    print(f"fixture 를 썼다: {FIXTURE_DIR}")
