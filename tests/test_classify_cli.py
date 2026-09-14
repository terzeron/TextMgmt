#!/usr/bin/env python3
"""자동분류 CLI 테스트

CLI 는 운영 표면이라 한 번 터지면 긴 작업이 통째로 날아간다.
`calibrate` 는 서점을 2,000번 조회한 뒤에야 마지막 집계에서 죽는 구조라,
이름 하나만 빠져도 100분을 버린다. 그래서 명령마다 최소 한 번은 실제로 돌린다.
"""

import json
from typing import Any, Dict, List, Tuple

import pytest

from backend.classifier.config import DEFAULT_CONFIG, SERIES_TO_PARENT, merge_config
from backend.classifier.corpus import write_jsonl
from backend.classifier.training import train
from tests.test_classifier import make_docs
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


def test_calibrate_runs_to_the_end_and_writes_the_threshold(model_file, monkeypatch, tmp_path, capsys):
    """서점 조회부터 집계·저장까지 실제로 통과시킨다.

    이 경로가 `Counter` 하나가 없어 마지막 줄에서 NameError 로 죽은 적이 있다.
    """
    import backend.book_classifier as bc

    # 두 서점이 같은 카테고리를 내면 다수결이 성립한다. 세 번째는 응답 없음이다.
    service = FakeBookstoreService([("3_무협", "3_무협", ""), ("3_판타지", "", ""), ("", "", "")])
    monkeypatch.setattr(bc, "BookClassifierService", lambda **kwargs: service)

    _, corpus = model_file
    path, _ = model_file
    out = tmp_path / "calibration.json"
    assert main(["calibrate", "--model", str(path), "--corpus", str(corpus), "--sample", "12", "--delay", "0", "--out", str(out)]) == 0

    saved = json.loads(out.read_text(encoding="utf-8"))
    assert saved["sample"] == 12
    assert 0.0 <= saved["bookstore_override_below"] <= 1.01
    assert service.calls == 12
    assert "서점으로 갈아탈 확신도 상한" in capsys.readouterr().out


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
