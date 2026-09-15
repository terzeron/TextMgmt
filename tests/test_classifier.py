#!/usr/bin/env python3
"""지도학습 기반 자동분류 모듈 테스트"""

import json
import zipfile
from pathlib import Path

import numpy as np
import pytest

from backend.classifier import BookCategoryClassifier
from backend.classifier.config import DEFAULT_CONFIG, SERIES_TO_PARENT, config_hash, enabled_fields, load_config, merge_config, resolve_parent, save_config
from backend.classifier.corpus import _document, absolute_path, attach_publishers, is_trainable, read_jsonl, write_jsonl
from backend.classifier.features import FeatureSpace, field_text, make_vectorizer
from backend.classifier.model import DEFAULT_MODEL_PATH, MODEL_ENV, CategoryModel, Prediction, model_path, softmax_confidence
from backend.classifier.reader import read_epub_publisher
from backend.classifier.training import coverage_curve, evaluate, filter_by_size, format_report, max_coverage_at, train


# ---------------------------------------------------------------------------
# 시험용 데이터
#
# 카테고리마다 어휘를 확실히 갈라 둔다. 선형 모델이 배울 신호가 있어야
# 학습 경로 전체를 실제로 통과시킬 수 있다.
# ---------------------------------------------------------------------------

VOCAB = {"3_무협": "무림맹 장문인 내공 운기조식 화산파 검법 강호 협객", "3_판타지": "드래곤 마나 던전 길드 마법사 레벨업 소환 마왕", "5_수학과학일반": "미분 적분 함수 벡터 행렬 확률 통계 방정식"}


def make_docs(per_category: int = 40) -> list[dict]:
    docs = []
    for cat, words in VOCAB.items():
        tokens = words.split()
        for i in range(per_category):
            # 문서마다 어순을 바꿔 같은 문자열이 반복되지 않게 한다.
            shifted = tokens[i % len(tokens) :] + tokens[: i % len(tokens)]
            docs.append(
                {"id": f"{cat}-{i}", "cat": cat, "text": " ".join(shifted * 3), "title": f"{tokens[i % len(tokens)]} 이야기 {i}", "author": f"작가{i % 5}", "publisher": "테스트출판사", "type": "epub" if i % 2 == 0 else "txt", "name": f"{tokens[i % len(tokens)]}_{i}.epub", "path": f"/mnt/data/text/{cat}/{i}.epub"}
            )
    return docs


@pytest.fixture
def trained():
    config = merge_config(
        DEFAULT_CONFIG,
        {"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": False}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}}, "holdout": 0.25},
    )
    model, report = train(make_docs(), config, accept_parent=SERIES_TO_PARENT)
    return model, report


# ---------------------------------------------------------------------------
# config
# ---------------------------------------------------------------------------


def test_load_config_falls_back_to_defaults_when_file_is_missing_or_broken(tmp_path):
    assert load_config(tmp_path / "없는파일.json") == DEFAULT_CONFIG

    broken = tmp_path / "broken.json"
    broken.write_text("{ 이건 JSON 이 아니다", encoding="utf-8")
    assert load_config(broken) == DEFAULT_CONFIG


def test_merge_config_overrides_only_given_keys():
    merged = merge_config(DEFAULT_CONFIG, {"model": {"C": 4.0}})
    assert merged["model"]["C"] == 4.0
    # 같은 딕셔너리의 다른 값은 남아야 한다
    assert merged["model"]["class_weight"] == DEFAULT_CONFIG["model"]["class_weight"]
    assert merged["fields"] == DEFAULT_CONFIG["fields"]


def test_save_and_load_config_round_trip(tmp_path):
    path = tmp_path / "config.json"
    cfg = merge_config(DEFAULT_CONFIG, {"decision": {"min_confidence": 0.77}})
    save_config(cfg, path)
    assert load_config(path)["decision"]["min_confidence"] == 0.77


def test_config_hash_changes_with_content():
    other = merge_config(DEFAULT_CONFIG, {"model": {"C": 2.0}})
    assert config_hash(DEFAULT_CONFIG) != config_hash(other)
    assert config_hash(DEFAULT_CONFIG) == config_hash(dict(DEFAULT_CONFIG))


def test_enabled_fields_drops_disabled_blocks():
    cfg = merge_config(DEFAULT_CONFIG, {"fields": {"body_char": {"enabled": False}}})
    assert "body_char" not in enabled_fields(cfg)
    assert "body_word" in enabled_fields(cfg)


def test_resolve_parent_folds_series_into_parent_genre():
    assert resolve_parent("2_을유세계문학전집") == "2_소설외국"
    assert resolve_parent("3_판타지") == "3_판타지"
    assert resolve_parent(None) is None
    assert resolve_parent("") == ""


# ---------------------------------------------------------------------------
# corpus
# ---------------------------------------------------------------------------


def test_document_takes_top_level_category_and_filename():
    doc = _document({"_id": "12345", "_source": {"category": "9_북스캔OCR/completed/2024", "summary": "본문", "title": "제목", "author": "저자", "publisher": "출판사", "file_path": "/mnt/data/text/9_북스캔OCR/책.epub", "file_type": "epub"}})
    assert doc["id"] == "12345"
    assert doc["cat"] == "9_북스캔OCR"
    assert doc["name"] == "책.epub"
    assert doc["publisher"] == "출판사"


def test_document_tolerates_missing_fields():
    doc = _document({"_id": 7, "_source": {}})
    assert doc == {"id": "7", "cat": "", "text": "", "title": "", "author": "", "publisher": "", "type": "", "name": "", "path": ""}


@pytest.mark.parametrize(
    "doc,expected",
    [
        ({"cat": "3_무협", "text": "가" * 200}, True),
        ({"cat": "3_무협", "text": "가" * 199}, False),
        ({"cat": "", "text": "가" * 500}, False),
        # 0_* 는 미분류/격리라 레이블이 아니다
        ({"cat": "0_nf", "text": "가" * 500}, False),
        # 책이 아니라 페이지 이미지다
        ({"cat": "9_북스캔OCR", "text": "가" * 500}, False),
        # 카테고리가 아니라 운영용 디렉토리다. 실측에서 789건이 섞여 들어왔다.
        ({"cat": "trash", "text": "가" * 500}, False),
        ({"cat": "_root", "text": "가" * 500}, False),
        ({"cat": ".preview_cache", "text": "가" * 500}, False),
    ],
)
def test_is_trainable(doc, expected):
    assert is_trainable(doc, 200, ["9_북스캔OCR"]) is expected


def test_absolute_path_resolves_paths_relative_to_the_library_root():
    # loader 가 라이브러리 루트 기준 상대 경로로 색인하므로 그대로 열면 못 찾는다
    assert absolute_path({"path": "3_무협/책.epub"}, "/mnt/data/text") == Path("/mnt/data/text/3_무협/책.epub")
    assert absolute_path({"path": "/절대/경로/책.epub"}, "/mnt/data/text") == Path("/절대/경로/책.epub")
    assert absolute_path({"path": ""}, "/mnt/data/text") is None
    assert absolute_path({}, "/mnt/data/text") is None


def test_jsonl_round_trip(tmp_path):
    path = tmp_path / "sub" / "corpus.jsonl"
    docs = make_docs(2)
    assert write_jsonl(docs, path) == len(docs)
    assert read_jsonl(path) == docs


def _make_epub(path: Path, publisher: str) -> Path:
    opf = f'<?xml version="1.0"?><package><metadata><dc:publisher>{publisher}</dc:publisher></metadata></package>'
    with zipfile.ZipFile(path, "w") as z:
        z.writestr("content.opf", opf)
    return path


def test_read_epub_publisher_reads_opf_and_tolerates_broken_files(tmp_path):
    good = _make_epub(tmp_path / "good.epub", "을유문화사")
    assert read_epub_publisher(good) == "을유문화사"

    no_opf = tmp_path / "no_opf.epub"
    with zipfile.ZipFile(no_opf, "w") as z:
        z.writestr("a.txt", "내용")
    assert read_epub_publisher(no_opf) == ""

    broken = tmp_path / "broken.epub"
    broken.write_bytes(b"not a zip")
    assert read_epub_publisher(broken) == ""


def test_attach_publishers_only_fills_missing_values_and_caches(tmp_path):
    # ES 에 들어 있는 모양대로 라이브러리 루트 기준 상대 경로를 준다
    _make_epub(tmp_path / "book.epub", "열린책들")
    cache = tmp_path / "publisher.json"
    docs = [
        {"id": "1", "type": "epub", "path": "book.epub", "publisher": ""},
        # ES 가 이미 값을 갖고 있으면 파일을 안 읽는다
        {"id": "2", "type": "epub", "path": "없는파일.epub", "publisher": "이미있음"},
        # EPUB 이 아니면 dc:publisher 가 없다
        {"id": "3", "type": "txt", "path": "a.txt", "publisher": ""},
    ]
    stats = attach_publishers(docs, cache, tmp_path, workers=1)
    assert stats["read"] == 1
    assert stats["with_publisher"] == 2
    assert docs[0]["publisher"] == "열린책들"
    assert docs[1]["publisher"] == "이미있음"
    assert docs[2]["publisher"] == ""
    assert json.loads(cache.read_text(encoding="utf-8")) == {"1": "열린책들"}

    # 두 번째 호출은 캐시만 쓰고 파일을 다시 안 읽는다
    (tmp_path / "book.epub").unlink()
    again = [{"id": "1", "type": "epub", "path": "book.epub", "publisher": ""}]
    assert attach_publishers(again, cache, tmp_path, workers=1)["read"] == 0
    assert again[0]["publisher"] == "열린책들"


# ---------------------------------------------------------------------------
# features
# ---------------------------------------------------------------------------


def test_field_text_strips_digit_runs_from_filenames_and_honors_max_chars():
    spec = {"source": "name", "max_chars": 0}
    # 두 자리 이상 숫자만 공백으로 바꾼다. 공백을 다시 접지는 않는다.
    assert field_text({"name": "달빛조각사 01권 2024.epub"}, spec) == "달빛조각사  권  .epub"

    assert field_text({"text": "가나다라마"}, {"source": "text", "max_chars": 3}) == "가나다"
    assert field_text({"text": None}, {"source": "text", "max_chars": 0}) == ""
    assert field_text({"subject": ["무협", "판타지"]}, {"source": "subject", "max_chars": 0}) == "무협 판타지"


def test_make_vectorizer_keeps_single_character_word_tokens():
    vec = make_vectorizer({"analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100})
    # 기본 token_pattern 은 한 글자를 버린다. 한자 한 글자가 뜻을 갖는 문서가 있어 살려 둔다.
    assert "天" in vec.fit(["天 地 人 무협"]).vocabulary_


def test_feature_space_applies_per_field_weight():
    fields = {"body_word": {"source": "text", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0}}
    docs = [{"text": "무협 강호"}, {"text": "마법 던전"}]

    plain = FeatureSpace(fields).fit_transform(docs)
    heavy_fields = {"body_word": dict(fields["body_word"], weight=3.0)}
    heavy = FeatureSpace(heavy_fields).fit_transform(docs)

    assert heavy.shape == plain.shape
    assert np.allclose(heavy.toarray(), plain.toarray() * 3.0)


def test_feature_space_skips_fields_without_any_vocabulary():
    fields = {"body_word": {"source": "text", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0}, "publisher": {"source": "publisher", "weight": 3.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0}}
    space = FeatureSpace(fields)
    space.fit_transform([{"text": "무협", "publisher": ""}, {"text": "마법", "publisher": ""}])
    assert "publisher" not in space.vectorizers
    assert "body_word" in space.describe()


def test_feature_space_raises_when_no_field_yields_features():
    fields = {"publisher": {"source": "publisher", "weight": 1.0, "analyzer": "word", "ngram": [1, 1], "min_df": 1, "max_features": 100, "max_chars": 0}}
    with pytest.raises(ValueError):
        FeatureSpace(fields).fit_transform([{"publisher": ""}, {"publisher": ""}])


def test_feature_space_transform_requires_fitted_vectorizers():
    with pytest.raises(ValueError):
        FeatureSpace({}).transform([{"text": "무협"}])


# ---------------------------------------------------------------------------
# model
# ---------------------------------------------------------------------------


def test_softmax_confidence_temperature_flattens_probabilities():
    scores = np.array([[3.0, 1.0, 0.0]])
    sharp = softmax_confidence(scores, 0.5)[0]
    flat = softmax_confidence(scores, 5.0)[0]
    assert np.isclose(sharp.sum(), 1.0)
    assert sharp[0] > flat[0]


def test_train_learns_the_three_categories(trained):
    model, report = trained
    assert set(model.classes) == set(VOCAB)
    assert report["top1_accuracy"] > 0.9
    assert model.meta["documents"] == 120
    assert model.meta["config_hash"]
    assert "publisher" not in model.meta["blocks"] or model.meta["blocks"]["publisher"]["weight"] == 3.0


def test_predict_returns_ranked_candidates_and_refuses_when_unsure(trained):
    model, _ = trained
    doc = {"text": VOCAB["3_무협"], "title": "무림 이야기", "author": "작가1", "publisher": "테스트출판사", "type": "epub", "name": "무림맹_1.epub"}

    confident = model.predict_one(doc, min_confidence=0.0)
    assert confident.category == "3_무협"
    assert [c for c, _ in confident.ranked][0] == "3_무협"
    assert len(confident.ranked) == 3

    refused = model.predict_one(doc, min_confidence=1.0)
    assert refused.category is None
    assert "판정하지 않음" in refused.reason


def test_predict_on_empty_input_returns_empty_list(trained):
    model, _ = trained
    assert model.predict([]) == []


def test_model_save_and_load_round_trip(tmp_path, trained):
    model, _ = trained
    path = model.save(tmp_path / "model.joblib")
    assert path.exists()

    loaded = CategoryModel.load(path)
    assert loaded is not None
    assert set(loaded.classes) == set(model.classes)
    assert loaded.meta["config_hash"] == model.meta["config_hash"]


def test_model_load_returns_none_for_missing_or_unusable_file(tmp_path):
    assert CategoryModel.load(tmp_path / "없는파일.joblib") is None

    garbage = tmp_path / "garbage.joblib"
    garbage.write_bytes(b"not a joblib payload")
    assert CategoryModel.load(garbage) is None

    import joblib

    wrong = tmp_path / "wrong.joblib"
    joblib.dump({"classifier": None}, wrong)
    assert CategoryModel.load(wrong) is None


# ---------------------------------------------------------------------------
# training
# ---------------------------------------------------------------------------


def test_filter_by_size_drops_small_categories():
    docs = [{"cat": "3_무협"}] * 10 + [{"cat": "9_격언명언"}] * 2
    kept, dropped = filter_by_size(docs, 5)
    assert dropped == ["9_격언명언"]
    assert len(kept) == 10


def test_filter_by_size_keeps_at_least_two_per_category_for_stratification():
    docs = [{"cat": "3_무협"}] * 3 + [{"cat": "3_판타지"}]
    kept, dropped = filter_by_size(docs, 1)
    # 층화 분할이 성립하려면 최소 2건이 있어야 한다. 1을 줘도 2로 올린다.
    assert dropped == ["3_판타지"]
    assert len(kept) == 3


def test_coverage_curve_precision_rises_as_coverage_falls():
    # 확신도가 높은 쪽이 맞고 낮은 쪽이 틀리게 둔다
    correct = np.array([True, True, True, False, False])
    confidence = np.array([0.99, 0.95, 0.9, 0.5, 0.4])
    curve = coverage_curve(correct, confidence, points=(0.5, 1.0))
    assert curve[0]["precision"] == 1.0
    assert curve[1]["precision"] == 0.6


def test_max_coverage_at_finds_the_widest_band_meeting_the_target():
    correct = np.array([True, True, True, True, False])
    confidence = np.array([0.99, 0.95, 0.9, 0.85, 0.4])
    best = max_coverage_at(correct, confidence, 0.9)
    assert best["coverage"] == pytest.approx(0.8)
    assert best["precision"] == 1.0


def test_max_coverage_at_returns_zero_when_target_is_unreachable():
    correct = np.array([False, False])
    assert max_coverage_at(correct, np.array([0.9, 0.8]), 0.9)["coverage"] == 0.0


def test_evaluate_accepts_parent_genre_as_correct(trained):
    model, _ = trained
    docs = [{"cat": "3_무협", "text": VOCAB["3_무협"], "title": "", "author": "", "publisher": "", "type": "epub", "name": "무림맹_1.epub"}]

    strict = evaluate(model, docs)
    # 하위 카테고리를 상위 장르로 접으면 같은 예측이 정답이 된다
    folded = evaluate(model, docs, accept_parent={"3_무협": "3_무협"})
    assert strict["n"] == folded["n"] == 1


def test_train_raises_when_every_category_is_too_small():
    config = merge_config(DEFAULT_CONFIG, {"corpus": {"min_per_category": 1000}})
    with pytest.raises(ValueError):
        train(make_docs(3), config)


def test_format_report_shows_curve_and_both_targets(trained):
    _, report = trained
    text = format_report(report)
    assert "홀드아웃" in text
    assert "정답률 90% 유지 최대 판정률" in text
    assert "정답률 95% 유지 최대 판정률" in text


# ---------------------------------------------------------------------------
# 공개 API
# ---------------------------------------------------------------------------


class FakeES:
    """ES 대역. inode 로 찾고, 없으면 경로로 찾는다."""

    index_name = "test-index"

    def __init__(self, by_inode=None, by_path=None):
        self._by_inode = by_inode or {}
        self._by_path = by_path or {}
        self.es = self

    def get(self, index, id, _source=None):
        if id not in self._by_inode:
            raise KeyError(id)
        return {"_id": id, "_source": self._by_inode[id]}

    def search(self, index, query, size=1, _source=None):
        path = query["term"]["file_path"]
        if path not in self._by_path:
            return {"hits": {"hits": []}}
        return {"hits": {"hits": [{"_id": "0", "_source": self._by_path[path]}]}}


def test_classifier_without_a_model_refuses_to_answer(tmp_path):
    clf = BookCategoryClassifier(model=None)
    assert not clf
    pred = clf.classify_path(tmp_path / "책.epub")
    assert pred.category is None
    assert "모델 파일이 없어" in pred.reason
    assert clf.classify_documents([{}, {}]) == [pred, pred] or all(p.category is None for p in clf.classify_documents([{}, {}]))


def test_build_document_prefers_elasticsearch_by_inode(tmp_path, trained):
    model, _ = trained
    f = tmp_path / "책.epub"
    _make_epub(f, "무시될출판사")
    inode = str(f.stat().st_ino)
    source = {"category": "3_무협", "summary": VOCAB["3_무협"], "title": "무림", "author": "작가", "publisher": "을유문화사", "file_path": str(f), "file_type": "epub"}

    clf = BookCategoryClassifier(model=model, es_manager=FakeES(by_inode={inode: source}))
    doc = clf.build_document(f)
    assert doc["id"] == inode
    assert doc["text"] == VOCAB["3_무협"]
    # ES 에 publisher 가 있으면 EPUB 을 다시 열지 않는다
    assert doc["publisher"] == "을유문화사"


def test_build_document_falls_back_to_path_lookup_then_to_the_file(tmp_path, trained):
    model, _ = trained
    f = tmp_path / "무림맹_1.epub"
    _make_epub(f, "파일에서읽은출판사")
    source = {"category": "3_무협", "summary": VOCAB["3_무협"], "title": "", "author": "", "publisher": "", "file_path": str(f), "file_type": "epub"}

    # inode 로는 못 찾고 경로로 찾는다. publisher 는 ES 가 비워 두었으니 파일에서 읽는다.
    clf = BookCategoryClassifier(model=model, es_manager=FakeES(by_path={str(f): source}))
    doc = clf.build_document(f)
    assert doc["text"] == VOCAB["3_무협"]
    assert doc["publisher"] == "파일에서읽은출판사"


def test_build_document_reads_the_file_when_elasticsearch_is_absent(tmp_path, trained):
    model, _ = trained
    f = tmp_path / "무림맹_1.txt"
    f.write_text(VOCAB["3_무협"] * 10, encoding="utf-8")

    clf = BookCategoryClassifier(model=model, es_manager=None)
    doc = clf.build_document(f)
    assert doc["id"] == ""
    assert "무림맹" in doc["text"]
    assert doc["type"] == "txt"
    # TXT 에는 dc:publisher 가 없다
    assert doc["publisher"] == ""


def test_classify_path_uses_the_model(tmp_path, trained):
    model, _ = trained
    f = tmp_path / "무림맹_1.txt"
    f.write_text(" ".join([VOCAB["3_무협"]] * 10), encoding="utf-8")

    clf = BookCategoryClassifier(model=model, es_manager=None, min_confidence=0.0)
    pred = clf.classify_path(f)
    assert pred.category == "3_무협"
    assert isinstance(pred, Prediction)


def test_min_confidence_precedence(trained):
    model, _ = trained
    assert BookCategoryClassifier(model=model, min_confidence=0.55).min_confidence == 0.55
    assert BookCategoryClassifier(model=model).min_confidence == model.min_confidence
    # 모델이 없으면 판정하지 않는다
    assert BookCategoryClassifier(model=None).min_confidence == 1.0


def test_threshold_comes_from_the_holdout_when_config_leaves_it_open(trained):
    """확신도 척도는 데이터마다 달라 고정값을 못 쓴다. 학습이 홀드아웃에서 고른다."""
    model, report = trained
    assert model.config["decision"]["min_confidence"] is None
    assert model.min_confidence == report["at_precision_90"]["threshold"]
    assert model.meta["calibrated_target"] == 0.90


def test_explicit_threshold_wins_over_the_calibrated_one(trained):
    model, _ = trained
    model.config = merge_config(model.config, {"decision": {"min_confidence": 0.77}})
    assert model.min_confidence == 0.77


def test_uncalibrated_model_refuses_everything(trained):
    """홀드아웃 기록이 없으면 오분류보다 무판정을 고른다."""
    model, _ = trained
    model.meta.pop("calibrated_threshold")
    assert model.min_confidence == 1.0


def test_target_accuracy_95_picks_the_stricter_threshold():
    config = merge_config(
        DEFAULT_CONFIG,
        {
            "corpus": {"min_per_category": 5},
            "model": {"n_jobs": 1},
            "fields": {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": False}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}},
            "holdout": 0.25,
            "decision": {"target_accuracy": 0.95},
        },
    )
    model, report = train(make_docs(), config, accept_parent=SERIES_TO_PARENT)
    assert model.meta["calibrated_threshold"] == report["at_precision_95"]["threshold"]
    assert model.meta["calibrated_target"] == 0.95


def test_indexed_path_is_relative_to_the_library_root(tmp_path, trained):
    model, _ = trained
    root = tmp_path / "text"
    (root / "3_무협").mkdir(parents=True)
    f = root / "3_무협" / "책.epub"
    f.write_bytes(b"x")

    clf = BookCategoryClassifier(model=model, library_root=root)
    # ES 는 라이브러리 루트 기준 상대 경로를 저장한다. 절대 경로로 질의하면 하나도 안 맞는다.
    assert clf._indexed_path(f) == "3_무협/책.epub"

    outside = tmp_path / "다른곳" / "책.epub"
    outside.parent.mkdir(parents=True)
    outside.write_bytes(b"x")
    assert clf._indexed_path(outside) == str(outside)


def test_build_document_finds_documents_by_relative_path(tmp_path, trained):
    model, _ = trained
    root = tmp_path / "text"
    (root / "3_무협").mkdir(parents=True)
    f = root / "3_무협" / "무림맹_1.epub"
    _make_epub(f, "출판사")
    source = {"category": "3_무협", "summary": VOCAB["3_무협"], "title": "", "author": "", "publisher": "을유문화사", "file_path": "3_무협/무림맹_1.epub", "file_type": "epub"}

    clf = BookCategoryClassifier(model=model, es_manager=FakeES(by_path={"3_무협/무림맹_1.epub": source}), library_root=root)
    doc = clf.build_document(f)
    assert doc["text"] == VOCAB["3_무협"]
    assert doc["publisher"] == "을유문화사"


def test_model_path_follows_the_environment_variable(tmp_path, monkeypatch):
    monkeypatch.delenv(MODEL_ENV, raising=False)
    assert model_path() == DEFAULT_MODEL_PATH

    # pod 는 /books 로, 학습 머신은 /mnt/data/text 로 같은 파일을 본다. 복사가 없다.
    monkeypatch.setenv(MODEL_ENV, "/books/.classifier/model.joblib")
    assert model_path() == Path("/books/.classifier/model.joblib")


def test_save_and_load_use_the_environment_path(tmp_path, monkeypatch, trained):
    model, _ = trained
    dest = tmp_path / "어딘가" / "model.joblib"
    monkeypatch.setenv(MODEL_ENV, str(dest))

    # 경로를 안 줘도 환경변수가 가리키는 곳에 쓰고 거기서 읽는다
    assert model.save() == dest
    assert dest.exists()
    assert CategoryModel.load() is not None
