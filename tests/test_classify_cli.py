#!/usr/bin/env python3
"""자동분류 CLI 테스트

CLI 는 운영 표면이라 한 번 터지면 긴 작업이 통째로 날아간다.
`bookstore-policy` 는 서점을 2,000번 조회한 뒤에야 마지막 집계에서 죽는 구조라,
이름 하나만 빠져도 100분을 버린다. 그래서 명령마다 최소 한 번은 실제로 돌린다.
"""

import json
from typing import Any, Dict, List, Tuple

import pytest

from backend.classifier.config import DEFAULT_CONFIG, SERIES_TO_PARENT, merge_config
from backend.classifier.corpus import write_jsonl
from backend.classifier.training import train
from tests.test_classifier import VOCAB, make_docs
from utils.classify_cli import COMMANDS, main

# 학습을 몇 초 안에 끝내는 설정. 어휘가 갈려 있어 판정은 실제로 배운다.
TEST_CONFIG = merge_config(
    DEFAULT_CONFIG,
    {"corpus": {"min_per_category": 5}, "model": {"n_jobs": 1}, "fields": {"body_word": {"min_df": 1, "max_features": 5000}, "body_char": {"enabled": False}, "filename": {"min_df": 1}, "title": {"min_df": 1}, "author": {"min_df": 1}, "publisher": {"min_df": 1}, "file_type": {"min_df": 1}}, "holdout": 0.25},
)


class FakeBookstoreService:
    """서점 조회를 대신한다. 네트워크를 타지 않고 1초 간격도 안 쉰다."""

    def __init__(self, answers: List[Tuple[str, str, str]], delay: float = 0.0):
        self.answers = answers
        self.delay = delay
        self.calls = 0

    def query_bookstores(self, search_title: str, raw_author: str, raw_title: str) -> Tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any]]:
        y, a, k = self.answers[self.calls % len(self.answers)]
        self.calls += 1
        return ({"mapped": y}, {"mapped": a}, {"mapped": k})


@pytest.fixture
def model_file(tmp_path):
    """학습한 모델과 그 학습 데이터를 파일로 남긴다. CLI 는 파일만 받는다."""
    docs = make_docs()
    corpus = tmp_path / "corpus.jsonl"
    write_jsonl(docs, corpus)

    model, _ = train(docs, TEST_CONFIG, accept_parent=SERIES_TO_PARENT)
    path = tmp_path / "model.joblib"
    model.save(path)
    return path, corpus


# ---------------------------------------------------------------------------
# 인자 파서
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("command", sorted(COMMANDS))
def test_every_command_prints_help(command, capsys):
    with pytest.raises(SystemExit) as e:
        main([command, "-h"])
    assert e.value.code == 0
    assert capsys.readouterr().out.strip()


def test_no_command_prints_help_and_fails():
    assert main([]) == 1


# ---------------------------------------------------------------------------
# 명령 실행
# ---------------------------------------------------------------------------


def test_info_prints_training_metadata(model_file, capsys):
    path, _ = model_file
    assert main(["info", "--model", str(path)]) == 0
    out = capsys.readouterr().out
    assert "학습 시각" in out
    assert "학습 문서  120건 / 카테고리 3개" in out
    assert "판정 임계값" in out


def test_evaluate_prints_the_coverage_curve(model_file, capsys):
    path, corpus = model_file
    assert main(["evaluate", "--model", str(path), "--corpus", str(corpus)]) == 0
    assert "판정률" in capsys.readouterr().out


def test_bookstore_policy_runs_to_the_end_and_writes_the_threshold(model_file, monkeypatch, tmp_path, capsys):
    """서점 조회부터 집계·저장까지 실제로 통과시킨다.

    이 경로가 `Counter` 하나가 없어 마지막 줄에서 NameError 로 죽은 적이 있다.
    """
    import backend.book_classifier as bc

    # 두 서점이 같은 카테고리를 내면 다수결이 성립한다. 세 번째는 응답 없음이다.
    service = FakeBookstoreService([("3_무협", "3_무협", ""), ("3_판타지", "", ""), ("", "", "")])
    monkeypatch.setattr(bc, "BookClassifierService", lambda **kwargs: service)

    _, corpus = model_file
    path, _ = model_file
    out = tmp_path / "policy.json"
    # 시험용 모델은 어휘가 뚜렷해 거부되는 문서가 없다. 집계 경로를 보려는 테스트라 전체에서 뽑는다.
    assert main(["bookstore-policy", "--model", str(path), "--corpus", str(corpus), "--sample", "12", "--delay", "0", "--sample-from", "all", "--out", str(out)]) == 0

    saved = json.loads(out.read_text(encoding="utf-8"))
    assert saved["sample"] == 12
    assert 0.0 <= saved["bookstore_override_below"] <= 1.01
    assert service.calls == 12
    assert "서점으로 갈아탈 확신도 상한" in capsys.readouterr().out


def test_reclassify_uses_the_model_threshold_when_none_is_given(model_file, tmp_path, capsys):
    """임계값을 안 주면 모델이 학습 때 고른 값을 쓴다.

    이 기본값이 0.90 으로 박혀 있어 실제 라이브러리에서 40건 전부 판정 거부가 났다.
    같은 파일을 classify 로 돌리면 판정됐다. 확신도 척도는 데이터마다 달라서
    CLI 가 숫자를 정해 두면 안 된다.
    """
    from backend.classifier.model import CategoryModel

    path, _ = model_file
    library = tmp_path / "library"
    source = library / "3_무협"
    source.mkdir(parents=True)
    (source / "무림맹_장문인.txt").write_text(" ".join([VOCAB["3_무협"]] * 3), encoding="utf-8")

    # 임계값을 안 주고 돌린 결과와, 모델이 고른 값을 직접 준 결과가 같아야 한다.
    # 결과 문자열 자체를 못 박으면 시험용 모델의 임계값이 조금만 달라져도 깨진다.
    # 여기서 볼 것은 "CLI 가 숫자를 정해 두지 않는다" 하나다.
    assert main(["reclassify", "3_무협", "--model", str(path), "--library-root", str(library), "--no-es"]) == 0
    default_out = capsys.readouterr().out

    model = CategoryModel.load(path)
    assert main(["reclassify", "3_무협", "--model", str(path), "--library-root", str(library), "--no-es", "--min-confidence", str(model.min_confidence)]) == 0
    assert capsys.readouterr().out == default_out

    # 0.90 이 박혀 있던 시절의 동작과는 달라야 한다 - 그때는 전부 거부됐다.
    assert model.min_confidence != 0.90


def test_reclassify_preview_does_not_move_files(model_file, tmp_path, capsys):
    path, _ = model_file
    library = tmp_path / "library"
    source = library / "3_무협"
    source.mkdir(parents=True)
    book = source / "무림맹_장문인.txt"
    book.write_text("무림맹 장문인 내공 운기조식 화산파 검법 강호 협객", encoding="utf-8")

    assert main(["reclassify", "3_무협", "--model", str(path), "--library-root", str(library), "--no-es", "--min-confidence", "0.0"]) == 0

    # --apply 가 없으면 파일은 그대로 있어야 한다
    assert book.exists()
    assert "미리보기다" in capsys.readouterr().out


def test_bookstore_policy_refuses_to_run_without_refused_documents(model_file, tmp_path):
    """거부 구간에서 뽑으라고 했는데 그런 문서가 없으면 조용히 전체를 쓰지 않고 멈춘다.

    서점을 4시간 조회한 뒤 엉뚱한 표본이었음을 아는 것보다 낫다.
    """
    path, corpus = model_file
    with pytest.raises(SystemExit) as e:
        main(["bookstore-policy", "--model", str(path), "--corpus", str(corpus), "--sample", "5", "--delay", "0", "--out", str(tmp_path / "p.json")])
    assert "거부한 구간" in str(e.value)


def test_bookstore_policy_saves_rows_for_rebanding(model_file, monkeypatch, tmp_path):
    """건별 결과를 남겨야 구간을 다시 나눌 때 서점을 다시 조회하지 않는다.

    표본 2,000건 조회에 4시간 35분이 걸렸다.
    """
    import backend.book_classifier as bc

    service = FakeBookstoreService([("3_무협", "3_무협", "")])
    monkeypatch.setattr(bc, "BookClassifierService", lambda **kwargs: service)

    path, corpus = model_file
    out = tmp_path / "policy.json"
    assert main(["bookstore-policy", "--model", str(path), "--corpus", str(corpus), "--sample", "10", "--delay", "0", "--sample-from", "all", "--out", str(out)]) == 0

    saved = json.loads(out.read_text(encoding="utf-8"))
    assert len(saved["rows"]) == 10
    assert {"confidence", "model_ok", "store_ok", "store_answered"} <= set(saved["rows"][0])


# ---------------------------------------------------------------------------
# 입력 검증
# ---------------------------------------------------------------------------


def test_load_model_stops_with_a_next_step(tmp_path):
    from utils.classify_cli import _load_model

    with pytest.raises(SystemExit) as e:
        _load_model(tmp_path / "없는모델.joblib")
    # 다음에 무엇을 해야 하는지 알려야 한다.
    assert "collect" in str(e.value) and "train" in str(e.value)


def test_load_corpus_stops_when_missing_or_empty(tmp_path):
    from utils.classify_cli import _load_corpus

    with pytest.raises(SystemExit) as e:
        _load_corpus(tmp_path / "없는코퍼스.jsonl")
    assert "collect" in str(e.value)

    empty = tmp_path / "empty.jsonl"
    empty.write_text("", encoding="utf-8")
    with pytest.raises(SystemExit) as e:
        _load_corpus(empty)
    assert "비었다" in str(e.value)


def test_publisher_cache_path_sits_next_to_the_corpus(tmp_path):
    from utils.classify_cli import _publisher_cache_path

    assert _publisher_cache_path(tmp_path / "corpus.jsonl").name == "corpus.jsonl.publisher.json"


def test_es_manager_passes_the_index_name(monkeypatch):
    import backend.es_manager as es_mod
    from utils.classify_cli import _es_manager

    seen = {}
    monkeypatch.setattr(es_mod, "ESManager", lambda index_name="": seen.setdefault("index", index_name))

    _es_manager("books_v2")
    assert seen["index"] == "books_v2"
    seen.clear()
    _es_manager()
    # 인덱스를 안 주면 빈 문자열을 넘겨 ESManager 가 환경변수를 쓰게 한다.
    assert seen["index"] == ""


def test_overrides_only_carries_values_given_on_the_cli():
    import argparse

    from utils.classify_cli import _overrides

    empty = argparse.Namespace()
    assert _overrides(empty) == {}

    args = argparse.Namespace(min_per_category=7, C=2.0, n_jobs=1, max_iter=500, holdout=0.3, class_weight="balanced", body_max_features=100, body_min_df=2, char_ngram=1)
    over = _overrides(args)
    assert over["corpus"]["min_per_category"] == 7
    assert over["model"]["class_weight"] == "balanced"
    assert over["fields"]["body_char"]["enabled"] is True
    assert over["holdout"] == 0.3

    # 현재 동작을 그대로 고정한다: `--class-weight none` 은 덮어쓰기를 만들지 않는다.
    # put() 이 None 값을 건너뛰기 때문이다. 결과적으로 config.json 의 값이 그대로 남는다.
    assert _overrides(argparse.Namespace(class_weight="none")) == {}
    assert _overrides(argparse.Namespace(char_ngram=0))["fields"]["body_char"]["enabled"] is False


# ---------------------------------------------------------------------------
# collect / train / classify
# ---------------------------------------------------------------------------


def test_collect_writes_corpus_and_prints_top_categories(tmp_path, monkeypatch, capsys):
    import utils.classify_cli as cli

    docs = make_docs()

    def fake_collect(es, out_path, **kwargs):
        write_jsonl(docs, out_path)
        per = {}
        for d in docs:
            per[d["cat"]] = per.get(d["cat"], 0) + 1
        return {"kept": len(docs), "skipped": 3, "categories": len(per), "per_category": per}

    monkeypatch.setattr("backend.classifier.corpus.collect", fake_collect)
    monkeypatch.setattr(cli, "_es_manager", lambda index=None: object())

    out = tmp_path / "corpus.jsonl"
    assert main(["collect", "--out", str(out)]) == 0

    printed = capsys.readouterr().out
    assert f"수집 {len(docs):,}건" in printed
    assert "상위 10개 카테고리" in printed
    assert out.exists()


def test_collect_with_publisher_backfills_and_rewrites(tmp_path, monkeypatch, capsys):
    import utils.classify_cli as cli

    docs = make_docs()

    def fake_collect(es, out_path, **kwargs):
        write_jsonl(docs, out_path)
        return {"kept": len(docs), "skipped": 0, "categories": 1, "per_category": {"3_무협": len(docs)}}

    def fake_attach(docs_arg, cache_path, library_root, workers=2):
        for d in docs_arg:
            d["publisher"] = "민음사"
        return {"read": 2, "cached": 2, "with_publisher": len(docs_arg)}

    monkeypatch.setattr("backend.classifier.corpus.collect", fake_collect)
    monkeypatch.setattr("backend.classifier.corpus.attach_publishers", fake_attach)
    monkeypatch.setattr(cli, "_es_manager", lambda index=None: object())

    out = tmp_path / "corpus.jsonl"
    assert main(["collect", "--out", str(out), "--with-publisher", "--library-root", str(tmp_path)]) == 0

    assert "publisher: 새로 읽음 2건" in capsys.readouterr().out
    from backend.classifier.corpus import read_jsonl

    assert all(d["publisher"] == "민음사" for d in read_jsonl(out))


def test_train_writes_a_model_and_reports_blocks(tmp_path, monkeypatch, capsys):
    import utils.classify_cli as cli

    corpus = tmp_path / "corpus.jsonl"
    write_jsonl(make_docs(), corpus)
    monkeypatch.setattr(cli, "load_config", lambda path=None: TEST_CONFIG)

    out = tmp_path / "model.joblib"
    assert main(["train", "--corpus", str(corpus), "--out", str(out)]) == 0

    printed = capsys.readouterr().out
    assert "모델 저장" in printed
    assert out.exists()


def test_train_warns_when_corpus_has_no_publisher(tmp_path, monkeypatch, caplog):
    import utils.classify_cli as cli

    corpus = tmp_path / "corpus.jsonl"
    docs = [{**d, "publisher": ""} for d in make_docs()]
    write_jsonl(docs, corpus)
    monkeypatch.setattr(cli, "load_config", lambda path=None: TEST_CONFIG)

    with caplog.at_level("WARNING", logger="classify_cli"):
        main(["train", "--corpus", str(corpus), "--out", str(tmp_path / "m.joblib")])

    # publisher 가 없으면 전집 판정이 나빠진다. 조용히 넘어가면 안 된다.
    assert any("publisher" in r.getMessage() for r in caplog.records)


def test_classify_prints_the_verdict_and_missing_files(model_file, tmp_path, capsys):
    path, _ = model_file
    book = tmp_path / "무림맹_장문인.txt"
    book.write_text(" ".join([VOCAB["3_무협"]] * 3), encoding="utf-8")
    missing = tmp_path / "없는파일.txt"

    assert main(["classify", str(book), str(missing), "--model", str(path), "--no-es", "--min-confidence", "0.0"]) == 0

    printed = capsys.readouterr().out
    assert "특징 출처: 파일" in printed
    assert "판정:" in printed
    # 한 건이 없어도 나머지는 계속 판정한다.
    assert "없는파일.txt: 파일이 없다" in printed


def test_reclassify_apply_moves_files(model_file, tmp_path, capsys):
    path, _ = model_file
    library = tmp_path / "library"
    source = library / "0_inbox"
    source.mkdir(parents=True)
    book = source / "무림맹_장문인.txt"
    book.write_text(" ".join([VOCAB["3_무협"]] * 3), encoding="utf-8")

    assert main(["reclassify", "0_inbox", "--model", str(path), "--library-root", str(library), "--no-es", "--min-confidence", "0.0", "--apply"]) == 0

    assert not book.exists()
    assert "실제로 파일을 옮긴다" in capsys.readouterr().out


def test_reclassify_skips_when_destination_already_has_the_file(model_file, tmp_path, capsys):
    path, _ = model_file
    library = tmp_path / "library"
    source = library / "0_inbox"
    source.mkdir(parents=True)
    name = "무림맹_장문인.txt"
    (source / name).write_text(" ".join([VOCAB["3_무협"]] * 3), encoding="utf-8")
    dest_dir = library / "3_무협"
    dest_dir.mkdir(parents=True)
    (dest_dir / name).write_text("이미 있는 파일", encoding="utf-8")

    assert main(["reclassify", "0_inbox", "--model", str(path), "--library-root", str(library), "--no-es", "--min-confidence", "0.0", "--apply"]) == 0

    # 덮어쓰지 않는다. 원본도 그대로 둔다.
    assert (source / name).exists()
    assert "옮김 0건" in capsys.readouterr().out


def test_reclassify_stops_when_directory_is_missing(model_file, tmp_path):
    path, _ = model_file
    with pytest.raises(SystemExit) as e:
        main(["reclassify", "없는카테고리", "--model", str(path), "--library-root", str(tmp_path), "--no-es"])
    assert "디렉토리가 없다" in str(e.value)


# ---------------------------------------------------------------------------
# reband
# ---------------------------------------------------------------------------


def _policy_rows():
    rows = []
    for i in range(20):
        conf = i / 20
        rows.append({"confidence": conf, "model_ok": conf > 0.5, "store_ok": True, "store_answered": True})
    return rows


def test_reband_rebuilds_bands_without_querying_bookstores(tmp_path, capsys):
    saved = tmp_path / "policy.json"
    saved.write_text(json.dumps({"rows": _policy_rows(), "sample_from": "refused"}, ensure_ascii=False), encoding="utf-8")

    assert main(["bookstore-policy", "--reband", str(saved), "--bands", "4"]) == 0

    out = json.loads(saved.read_text(encoding="utf-8"))
    assert out["sample_from"] == "refused"
    assert len(out["bands"]) >= 1
    # 경계는 실제로 잰 범위를 넘으면 안 된다.
    assert out["bookstore_override_below"] <= out["measured_max_confidence"]
    assert "서점으로 갈아탈 확신도 상한" in capsys.readouterr().out


def test_reband_writes_to_a_separate_file_when_asked(tmp_path):
    saved = tmp_path / "policy.json"
    saved.write_text(json.dumps({"rows": _policy_rows()}, ensure_ascii=False), encoding="utf-8")
    dest = tmp_path / "reband.json"

    assert main(["bookstore-policy", "--reband", str(saved), "--bands", "3", "--out", str(dest)]) == 0
    assert dest.exists()
    assert json.loads(dest.read_text(encoding="utf-8"))["sample_from"] == "unknown"


def test_reband_stops_when_the_file_is_missing(tmp_path):
    with pytest.raises(SystemExit) as e:
        main(["bookstore-policy", "--reband", str(tmp_path / "없는파일.json")])
    assert "측정 결과 파일이 없다" in str(e.value)


def test_reband_stops_when_there_are_no_rows(tmp_path):
    saved = tmp_path / "policy.json"
    saved.write_text(json.dumps({"bands": []}), encoding="utf-8")
    with pytest.raises(SystemExit) as e:
        main(["bookstore-policy", "--reband", str(saved)])
    # 서점 조회부터 다시 해야 한다는 것을 알려야 한다.
    assert "서점 조회부터" in str(e.value)
