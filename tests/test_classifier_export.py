#!/usr/bin/env python3
"""자동분류 모델 내보내기(Rust 추론용) 테스트

작은 합성 모델을 학습해 내보내고, 파일 구조·sha256·shape·전치 레이아웃을 검증한다.
`python tests/test_classifier_export.py` 로 실행하면 저장소 fixture
(`tests/fixtures/classifier_tiny/`)를, `... trained` 로 실행하면 학습 결과 픽스처(`trained/`, `trained_95/`)를 다시 쓴다. Rust parity 테스트가 이 fixture 를 읽는다.
"""

import hashlib
import json
import random
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


def _lcg(seed: int):
    """Rust 테스트(`classifier_parity.rs`)와 같은 결정적 난수. 값은 [0, 1)."""
    state = seed & 0xFFFFFFFFFFFFFFFF
    while True:
        state = (state * 6364136223846793005 + 1442695040888963407) & 0xFFFFFFFFFFFFFFFF
        yield (state >> 11) / float(1 << 53)


def build_parity_fixture() -> Dict[str, Any]:
    """Rust 가 Python 과 같은 수치를 내는지 대조할 기대값. `classifier_parity.rs` 가 읽는다."""
    import hashlib as _hashlib  # noqa: PLC0415

    from backend.classifier.calibration import expected_accuracy, fit_expected_accuracy, from_coverage_report  # noqa: PLC0415
    from backend.classifier.config import DEFAULT_CONFIG, config_hash  # noqa: PLC0415
    from backend.classifier.corpus import write_jsonl  # noqa: PLC0415
    from backend.classifier.training import format_report, split_holdout  # noqa: PLC0415

    rng = _lcg(1)
    conf, correct = [], []
    for _ in range(600):
        c = 0.013 + 0.137 * next(rng)
        conf.append(c)
        correct.append(next(rng) < min(0.99, max(0.0, 0.4 + (c - 0.013) * 4.3)))
    iso = fit_expected_accuracy(conf, correct)
    probes = [0.0, 0.02, 0.054, 0.076, 0.1281779488151631, 0.2, 0.5]
    holdout = {
        "n": 47008,
        "curve": [
            {"coverage": 0.5, "precision": 0.974004424778761, "threshold": 0.1281779488151631},
            {"coverage": 0.7, "precision": 0.942136453426531, "threshold": 0.10072196828305498},
            {"coverage": 0.8, "precision": 0.9265010902515556, "threshold": 0.08265990361070291},
            {"coverage": 0.9, "precision": 0.9044602548041695, "threshold": 0.060971974205370344},
            {"coverage": 0.95, "precision": 0.8889759724119399, "threshold": 0.04635731124736481},
            {"coverage": 1.0, "precision": 0.864788972089857, "threshold": 0.019518310797334633},
        ],
        "at_precision_90": {"coverage": 0.917290673927842, "precision": 0.9, "threshold": 0.056374553813004144},
        "at_precision_95": {"coverage": 0.6435287610619469, "precision": 0.9500181812171499, "threshold": 0.10885944805582125},
    }
    curve = from_coverage_report(holdout)
    blake = [{"data": d, "key": k, "hex": _hashlib.blake2b(d.encode("utf-8"), digest_size=8, key=k.encode()).hexdigest()} for d, k in [("", "1"), ("abc", "20260914"), ("한글 키", "7"), ("x" * 300, "20260914"), ("y" * 128, "5"), ("z" * 256, "5")]]
    docs = [{"id": str(i), "cat": "A"} for i in range(500)] + [{"id": str(i), "cat": "B"} for i in range(1000, 1037)] + [{"id": str(i), "cat": "C"} for i in range(2000, 2002)] + [{"id": "solo", "cat": "D"}]
    train_docs, hold_docs = split_holdout(docs, 0.2, 20260914)
    floats = [0.1, 1.0, 1e-5, 1e16, 1e15, 123456789012345680.0, 0.0001, 1e22, 5e-324, 0.3, 2.5e-7, 100.0, 1.5e300, 0.30000000000000004, -1.5, 0.5, 12345.678]
    sample_doc = {"id": "1", "cat": "3_무협", "text": "본문 \"따옴표\" \n줄바꿈 \u0001", "title": "é", "n": 1.5, "t": True, "none": None}
    tmp = FIXTURE_DIR / "_jsonl.tmp"
    write_jsonl([sample_doc], tmp)
    line = tmp.read_text(encoding="utf-8")
    tmp.unlink()
    report = json.loads((FIXTURE_DIR / "trained" / "report.json").read_text(encoding="utf-8"))
    return {
        "isotonic": {"seed": 1, "n": 600, "expected": iso, "probes": probes, "probe_values": [expected_accuracy(iso, p) for p in probes]},
        "coverage": {"holdout": holdout, "expected": curve, "probes": probes, "probe_values": [expected_accuracy(curve, p) for p in probes]},
        "blake2b": blake,
        "split": {"docs": docs, "ratio": 0.2, "seed": 20260914, "train_ids": [d["id"] for d in train_docs], "hold_ids": [d["id"] for d in hold_docs]},
        "float_repr": [{"value": v, "repr": repr(v)} for v in floats],
        "config_hash_default": config_hash(DEFAULT_CONFIG),
        "json_dumps": {"value": sample_doc, "line": line, "sorted": json.dumps({"b": [1, 2.5, {"é": None}], "a": True, "10": 1e-7}, ensure_ascii=False, sort_keys=True)},
        "report_text": format_report(report),
    }


def build_book_parity_fixture() -> Dict[str, Any]:
    """`backend/book_classifier.py` 의 순수 함수 출력을 모아 둔다. Rust `book_classifier_parity.rs` 가 같은 입력으로 대조한다."""
    import re as _re  # noqa: PLC0415

    from backend import book_classifier as bc  # noqa: PLC0415
    from backend.classifier.bookstore_policy import map_bookstore_candidates, match_keyword  # noqa: PLC0415

    rng = random.Random(20261024)
    src = (ROOT / "backend" / "book_classifier.py").read_text(encoding="utf-8")
    body = src[src.index("def map_category") : src.index("def get_effective_filename")]
    keywords = sorted(set(_re.findall(r'"([^"\n]{1,12})"', body)))
    keywords = [k for k in keywords if not k.startswith(("2_", "3_", "4_", "5_", "6_", "7_", "8_", "9_", "1_")) and k not in ("", " ")]
    pool_title = ["달빛조각사", "해리포터와 마법사의 돌", "화산귀환", "손자병법", "을유세계문학 고전", "열린책들 세계문학", "올재 클래식스", "그림으로 읽는 과학", "평범한 제목", "OST 모음", "도깨비 OST (CD)", "무협 입문", "Harry Potter", "a b", "이지사이언스 3", "살림지식총서 001"]
    pool_author = ["", "히가시노 게이고", "무라카미하루키", "남희성", "김용", "Kim"]

    map_cases = []
    for _ in range(700):
        parts = [rng.choice(keywords) for _ in range(rng.choice([0, 1, 1, 2, 3]))]
        cat = rng.choice([" > ", "/", " ", ">"]).join(parts)
        title = rng.choice(["", "", "", *pool_title])
        author = rng.choice(["", "", *pool_author])
        map_cases.append({"cat": cat, "title": title, "author": author, "expected": bc.map_category(cat, title, author)})

    frags = ["[남희성]", "@작가", " 1-300권 완", "(완결)", "#나무", "_", "(z-lib.org)", "신.화.급", "[로판]", "(무협)", "01권", "2024", "(비가)", " - ", "-이우혁", "[판타지]", "TXT ", "【완】", "vol.3", "특외", "완결", "(19)", "[1~3부]", "외전", "…", "%5B로맨스%5D", "[BL]", "(TS)", "SF ", "[설봉]"]
    words = ["달빛조각사", "화산귀환", "사조영웅전", "룬의 아이들", "Harry Potter", "마법사", "무림", "나혼자만렙", "회귀", "악녀"]
    exts = [".txt", ".epub", ".pdf", ""]
    names = []
    for _ in range(900):
        parts = [rng.choice(words)] + [rng.choice(frags + words) for _ in range(rng.choice([0, 1, 2, 3]))]
        rng.shuffle(parts)
        names.append("".join(p if i == 0 else rng.choice(["", " "]) + p for i, p in enumerate(parts)) + rng.choice(exts))
    name_cases = [{"name": n, "clean": list(bc.clean_filename_to_author_title(n)), "genre": bc.extract_explicit_genre(n)} for n in names]

    title_cases = []
    for _ in range(500):
        a, b = rng.choice(pool_title), rng.choice(pool_title)
        if rng.random() < 0.4:
            b = a + rng.choice(["", " 1", " 완결판", " (전2권)", " 특별판"])
        if rng.random() < 0.2:
            a = " ".join(reversed(a.split()))
        title_cases.append({"a": a, "b": b, "sim": bc.title_similarity(a, b), "single": bc.is_single_match_valid(a, b), "reliable": bc.is_title_match_reliable(a, b)})

    eff = []
    for parent, name in [("lib", "01권.txt"), ("lib/[전민희] 룬", "01권.txt"), ("lib/작품", "긴 제목입니다.txt"), ("lib/[작가] 시리즈", "긴 제목.txt"), ("lib/@작가 시리즈", "긴 제목.txt"), ("lib/시리즈", "[저자] 제목.txt"), ("lib/시리즈", "vol.3.epub"), ("lib/시리즈", "v2.epub"), ("lib/시리즈", "ab.txt"), ("lib/시리즈", "3~5권.txt"), ("lib/a/b", "제목 있음.txt")]:
        eff.append({"path": f"{parent}/{name}", "target": "lib", "expected": bc.get_effective_filename(Path(f"{parent}/{name}"), Path("lib"))})

    kw = {"영어": ["7_영어교육"], "영어전문교육": ["7_영어교육"], "교육": ["7_교육일반"], "클래식": ["1_서양고전", "5_음악"], "한문": ["1_동양고전한문"]}
    kw_cases = []
    for _ in range(150):
        cat = " > ".join(rng.choice(keywords + ["영어", "교육", "클래식", "한문", "영어전문교육"]) for _ in range(rng.choice([0, 1, 2])))
        title = rng.choice(["", *pool_title])
        author = rng.choice(["", *pool_author])
        kw_cases.append({"cat": cat, "title": title, "author": author, "match": match_keyword(cat, kw), "candidates": map_bookstore_candidates(cat, title, author, kw)})
    return {"keywords": kw, "map_category": map_cases, "filenames": name_cases, "titles": title_cases, "effective": eff, "keyword_cases": kw_cases}


def test_book_parity_fixture_is_current():
    """book_classifier 순수 함수의 Python 출력이 저장소 기대값과 같은지 본다(Rust 가 이 파일로 대조한다)."""
    committed = json.loads((FIXTURE_DIR / "book_classifier_parity.json").read_text(encoding="utf-8"))
    assert committed == json.loads(json.dumps(build_book_parity_fixture()))


def test_parity_fixture_is_current():
    """저장소의 parity.json 이 지금 Python 구현이 내는 값과 같은지 본다(Rust 는 이 파일을 기대값으로 쓴다)."""
    committed = json.loads((FIXTURE_DIR / "parity.json").read_text(encoding="utf-8"))
    assert committed == json.loads(json.dumps(build_parity_fixture()))


def write_trained_fixtures() -> None:
    """`tests/test_classifier.py` 의 `trained` 픽스처와 같은 학습 결과를 내보낸다.

    Rust 테스트(`tests/rust/backend/classifier.rs`)는 학습하지 않으므로, 같은 문서·설정으로 Python 이 학습한
    모델(`trained/`)과 목표 정답률 95% 판(`trained_95/`)을 읽는다. 홀드아웃 리포트는 각 디렉토리의 `report.json` 이다.
    """
    sys.path.insert(0, str(ROOT / "tests"))
    from test_classifier import make_docs  # noqa: PLC0415

    from backend.classifier.config import DEFAULT_CONFIG, SERIES_TO_PARENT, merge_config  # noqa: PLC0415
    from backend.classifier.training import train  # noqa: PLC0415

    small = {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": False}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}}
    base = {"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": small, "holdout": 0.25}
    for name, extra in (("trained", {}), ("trained_95", {"decision": {"target_accuracy": 0.95}})):
        model, report = train(make_docs(40), merge_config(DEFAULT_CONFIG, merge_config(base, extra)), accept_parent=SERIES_TO_PARENT)
        out = FIXTURE_DIR / name
        export_model(model, out)
        _write_json(out / "report.json", report)


def _write_json(path: Path, obj: Any) -> None:
    path.write_text(json.dumps(obj, ensure_ascii=False, indent=1, allow_nan=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    import shutil

    if sys.argv[1:] == ["book_parity"]:
        _write_json(FIXTURE_DIR / "book_classifier_parity.json", build_book_parity_fixture())
    elif sys.argv[1:] == ["parity"]:
        # 기대값 파일만 다시 쓴다. `trained/report.json` 이 먼저 있어야 한다.
        _write_json(FIXTURE_DIR / "parity.json", build_parity_fixture())
    elif sys.argv[1:] == ["trained"]:
        # 학습 결과 픽스처만 다시 쓴다. 기본 합성 모델은 건드리지 않는다.
        write_trained_fixtures()
    else:
        for stale in FIXTURE_DIR.glob("*"):
            if stale.is_file():
                stale.unlink()
        export_model(make_tiny_model(), FIXTURE_DIR, golden=40, seed=11)
    print(f"fixture 를 썼다: {FIXTURE_DIR}")


@pytest.mark.parametrize(
    ("field", "attribute", "value", "message"),
    [
        ("title", "analyzer", "invalid", "analyzer"),
        ("title", "binary", True, "binary=True"),
        ("title", "use_idf", False, "use_idf=False"),
        ("title", "norm", None, "norm"),
        ("body_word", "token_pattern", r"(?u)\w+", "token_pattern"),
        ("title", "strip_accents", "unicode", "strip_accents"),
        ("title", "stop_words", "english", "stop_words"),
        ("title", "tokenizer", lambda value: value.split(), "tokenizer"),
        ("title", "preprocessor", lambda value: value, "preprocessor"),
    ],
)
def test_check_vectorizer_rejects_each_unsupported_option(tiny, field, attribute, value, message):
    from backend.classifier.export import _check_vectorizer

    vec = tiny.feature_space.vectorizers[field]
    old_value = getattr(vec, attribute)
    setattr(vec, attribute, value)
    try:
        with pytest.raises(ValueError, match=message):
            _check_vectorizer("title", vec)
    finally:
        setattr(vec, attribute, old_value)


def test_export_helpers_cover_json_vocab_and_digest_edges(tmp_path):
    from datetime import datetime
    from types import SimpleNamespace
    from backend.classifier.export import _jsonable, _ordered_vocab, _sha256_file, _write_json

    assert _jsonable(np.array([np.float64(np.inf), np.int64(3)])) == [None, 3]
    assert _jsonable(Path("model.bin")) == "model.bin"
    assert _jsonable(datetime(2026, 1, 2, 3, 4)) == "2026-01-02T03:04:00"
    assert _jsonable(object())

    with pytest.raises(ValueError, match="짝 없는 surrogate"):
        _write_json(tmp_path / "invalid.json", "\ud800")
    empty = tmp_path / "empty"
    empty.write_bytes(b"")
    assert _sha256_file(empty) == hashlib.sha256(b"").hexdigest()

    assert _ordered_vocab("x", SimpleNamespace(vocabulary_={"b": 1, "a": 0})) == ["a", "b"]
    for vocabulary in ({"a": 1}, {"a": 0, "b": 0}):
        with pytest.raises(ValueError, match="어휘 번호"):
            _ordered_vocab("x", SimpleNamespace(vocabulary_=vocabulary))


def test_export_rejects_bad_estimators_and_intercepts(tiny, tmp_path):
    from types import SimpleNamespace
    from backend.classifier.export import _write_coef

    original = tiny.classifier
    try:
        tiny.classifier = SimpleNamespace()
        with pytest.raises(ValueError, match="추정기를 하나씩"):
            _write_coef(tiny, tmp_path, 1)

        tiny.classifier = SimpleNamespace(estimators_=[SimpleNamespace(coef_=np.zeros((2, 1))) for _ in tiny.classes])
        with pytest.raises(ValueError, match="coef_ shape"):
            _write_coef(tiny, tmp_path, 1)

        tiny.classifier = SimpleNamespace(estimators_=[SimpleNamespace(coef_=np.array([[np.inf]]), intercept_=[0.0]) for _ in tiny.classes])
        with pytest.raises(ValueError, match="계수에 NaN/Inf"):
            _write_coef(tiny, tmp_path, 1)

        tiny.classifier = SimpleNamespace(estimators_=[SimpleNamespace(coef_=np.zeros((1, 1)), intercept_=[np.inf]) for _ in tiny.classes])
        with pytest.raises(ValueError, match="intercept"):
            _write_coef(tiny, tmp_path, 1)

        tiny.classifier = SimpleNamespace(estimators_=[SimpleNamespace(coef_=np.zeros((1, 1))) for _ in tiny.classes], classes_=["wrong"] * len(tiny.classes))
        with pytest.raises(ValueError, match="순서가 다르다"):
            _write_coef(tiny, tmp_path, 1)
    finally:
        tiny.classifier = original


def test_export_rejects_invalid_idf_and_empty_feature_space(tiny, tmp_path):
    original = tiny.feature_space.vectorizers
    try:
        vec = next(iter(original.values()))
        old_idf = vec._tfidf.idf_.copy()
        vec._tfidf.idf_ = np.full(len(vec.vocabulary_), np.inf)
        with pytest.raises(ValueError, match="idf 길이 또는 값"):
            export_model(tiny, tmp_path / "bad-idf")
        vec._tfidf.idf_ = old_idf

        tiny.feature_space.vectorizers = {}
        with pytest.raises(ValueError, match="벡터라이저가 하나도 없다"):
            export_model(tiny, tmp_path / "empty")
    finally:
        tiny.feature_space.vectorizers = original


def test_main_returns_error_when_model_cannot_be_loaded(monkeypatch, capsys):
    import backend.classifier.export as export_mod

    monkeypatch.setattr(export_mod.CategoryModel, "load", classmethod(lambda cls, path=None: None))
    assert main(["--model", "missing.joblib", "--out", "unused"]) == 1
    assert "모델을 읽지 못했다" in capsys.readouterr().err


def test_synthetic_document_skips_empty_pools_and_builds_filename():
    import random
    from backend.classifier.export import _synthetic_doc

    generated = _synthetic_doc(random.Random(4), {"unused": [], "name": ["책"], "text": ["내용"]})

    assert generated["name"].startswith("책_") and generated["name"].endswith(".epub")
    assert "text" in generated and "unused" not in generated


def test_module_entrypoint_parses_help(monkeypatch, capsys):
    import runpy

    monkeypatch.setattr(sys, "argv", ["backend.classifier.export", "--help"])
    with pytest.raises(SystemExit) as exited:
        runpy.run_module("backend.classifier.export", run_name="__main__")

    assert exited.value.code == 0
    assert "--model" in capsys.readouterr().out
