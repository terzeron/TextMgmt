"""인코딩 손상 탐지기 테스트

판정 로직은 결정론적이다. 학습도 확률도 없이 문자만 센다.
"""

import json
from pathlib import Path

import pytest

from utils.detect_mojibake import MIN_SYLLABLES_FOR_LIKENESS, MOJIBAKE_LIKENESS_THRESHOLD, VERDICT_CLEAN, VERDICT_CORRUPTED, VERDICT_UNDETERMINED, VERDICT_UNREADABLE, classify_text, inspect_file, korean_likeness, list_library_files, scan

CLEAN_KOREAN = "소림사 장로가 강호에서 내공을 운기조식하며 화산파 검법을 수련했다. 그는 무림맹의 장문인으로서 제자들을 이끌었다. " * 6

# 정상 한국어를 UTF-8로 저장한 뒤 CP949로 잘못 읽으면 나오는 바로 그 문자열.
# 실제 코퍼스의 손상 파일과 같은 방식으로 만든다.
CORRUPTED = (CLEAN_KOREAN * 4).encode("utf-8").decode("cp949", errors="ignore")

HANJA_CLASSIC = "孟子曰 仁義禮智 信也 天下之達道也 君子之道 費而隱 夫婦之愚 可以與知焉 " * 20
ENGLISH = "The quick brown fox jumps over the lazy dog. " * 20
# 조사와 어미가 거의 없는 목록형 문서. 형태소 기반 판정은 여기서 오탐했다.
KOREAN_WORD_LIST = "천상 지하 만물 유전 성쇠 흥망 강산 세월 인생 무상 일장 춘몽 기고 만장 " * 20


# ---------------------------------------------------------------------------
# korean_likeness
# ---------------------------------------------------------------------------


def test_likeness_is_none_when_too_few_syllables():
    assert korean_likeness("가나다") is None
    assert korean_likeness("") is None
    assert korean_likeness("가" * (MIN_SYLLABLES_FOR_LIKENESS - 1)) is None


def test_likeness_is_computed_at_the_syllable_floor():
    assert korean_likeness("이" * MIN_SYLLABLES_FOR_LIKENESS) == 1.0


def test_likeness_is_high_for_real_korean():
    assert korean_likeness(CLEAN_KOREAN) >= 0.20


def test_likeness_is_near_zero_for_corrupted_text():
    assert korean_likeness(CORRUPTED) < MOJIBAKE_LIKENESS_THRESHOLD


def test_likeness_ignores_non_hangul_characters():
    """한자와 영문은 분모에 들어가지 않는다"""
    assert korean_likeness("이다는을가" * 10) == korean_likeness("이다는을가" * 10 + "孟子曰 " * 50)


def test_likeness_is_deterministic():
    assert korean_likeness(CLEAN_KOREAN) == korean_likeness(CLEAN_KOREAN)


# ---------------------------------------------------------------------------
# classify_text
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("text,expected", [(CLEAN_KOREAN, VERDICT_CLEAN), (CORRUPTED, VERDICT_CORRUPTED), (HANJA_CLASSIC, VERDICT_UNDETERMINED), (ENGLISH, VERDICT_UNDETERMINED), (KOREAN_WORD_LIST, VERDICT_CLEAN), ("", VERDICT_UNREADABLE), ("짧은 글", VERDICT_UNREADABLE)])
def test_classify_text(text, expected):
    assert classify_text(text)[0] == expected


def test_hanja_heavy_document_is_never_called_corrupted():
    """한자가 많은 것 자체는 손상 근거가 아니다"""
    verdict, likeness = classify_text(HANJA_CLASSIC)
    assert verdict == VERDICT_UNDETERMINED
    assert likeness is None


def test_korean_word_list_is_not_flagged():
    """조사·어미가 없는 목록형 문서도 음절은 정상 한국어다"""
    assert classify_text(KOREAN_WORD_LIST)[0] == VERDICT_CLEAN


def test_classify_returns_likeness_for_decided_verdicts():
    for text in (CLEAN_KOREAN, CORRUPTED):
        _verdict, likeness = classify_text(text)
        assert likeness is not None
        assert 0.0 <= likeness <= 1.0


# ---------------------------------------------------------------------------
# 파일 검사
# ---------------------------------------------------------------------------


def test_inspect_file_reports_verdict(tmp_path):
    p = tmp_path / "clean.txt"
    p.write_text(CLEAN_KOREAN * 4, encoding="utf-8")
    res = inspect_file(str(p))
    assert res["verdict"] == VERDICT_CLEAN
    assert res["size"] > 0


def test_inspect_file_detects_corrupted_file(tmp_path):
    p = tmp_path / "broken.txt"
    p.write_text(CORRUPTED, encoding="utf-8")
    assert inspect_file(str(p))["verdict"] == VERDICT_CORRUPTED


def test_inspect_file_on_missing_path(tmp_path):
    assert inspect_file(str(tmp_path / "없음.txt"))["verdict"] == VERDICT_UNREADABLE


def test_list_library_files_skips_tool_caches(tmp_path):
    (tmp_path / "3_무협").mkdir()
    (tmp_path / "3_무협" / "a.txt").write_text("x", encoding="utf-8")
    (tmp_path / "3_무협" / "b.epub").write_bytes(b"x")
    (tmp_path / "3_무협" / "c.pdf").write_bytes(b"x")
    (tmp_path / "__pycache__").mkdir()
    (tmp_path / "__pycache__" / "d.txt").write_text("x", encoding="utf-8")
    (tmp_path / ".ruff_cache").mkdir()
    (tmp_path / ".ruff_cache" / "e.txt").write_text("x", encoding="utf-8")

    names = sorted(p.name for p in list_library_files(tmp_path))
    assert names == ["a.txt", "b.epub"]


def test_scan_counts_verdicts_per_category(tmp_path):
    for cat, text, n in [("9_격언명언", CLEAN_KOREAN * 4, 3), ("7_중문일반", CORRUPTED, 2)]:
        d = tmp_path / cat
        d.mkdir()
        for i in range(n):
            (d / f"{i}.txt").write_text(text, encoding="utf-8")

    report = scan(tmp_path, workers=1)
    assert report["total"] == 5
    assert report["counts"][VERDICT_CLEAN] == 3
    assert report["counts"][VERDICT_CORRUPTED] == 2
    assert len(report["corrupted"]) == 2
    # 리포트는 그대로 JSON 으로 저장할 수 있어야 한다
    json.dumps(report, ensure_ascii=False)


# ---------------------------------------------------------------------------
# 한글 비율 하한
# ---------------------------------------------------------------------------

# 영문 본문에 OCR 잡음으로 한글이 조금 섞인 문서. 음절 수 하한은 넘지만
# 문서의 0.2%를 보고 전체를 판정하면 안 된다.
ENGLISH_WITH_HANGUL_NOISE = ENGLISH * 12 + "".join("뷁뾃쎯퉳깗" for _ in range(12))


def test_english_document_with_hangul_noise_is_undetermined():
    assert korean_likeness(ENGLISH_WITH_HANGUL_NOISE) is not None  # 음절 수 하한은 넘는다
    assert classify_text(ENGLISH_WITH_HANGUL_NOISE)[0] == VERDICT_UNDETERMINED


def test_korean_document_is_still_judged():
    """한글이 본문의 다수인 문서는 그대로 판정한다"""
    assert classify_text(CLEAN_KOREAN)[0] == VERDICT_CLEAN
    assert classify_text(CORRUPTED)[0] == VERDICT_CORRUPTED


# ---------------------------------------------------------------------------
# 리포트 구성
# ---------------------------------------------------------------------------

def test_report_keeps_every_non_clean_file(tmp_path):
    """clean 을 뺀 전부를 목록으로 남긴다. 없으면 나중에 원인을 되짚을 수 없다."""
    d = tmp_path / "3_무협"
    d.mkdir()
    (d / "clean.txt").write_text(CLEAN_KOREAN * 4, encoding="utf-8")
    (d / "broken.txt").write_text(CORRUPTED, encoding="utf-8")
    (d / "english.txt").write_text(ENGLISH * 4, encoding="utf-8")
    (d / "empty.txt").write_text("", encoding="utf-8")

    report = scan(tmp_path, workers=1)
    assert [r["path"] for r in report["corrupted"]] == [str(d / "broken.txt")]
    assert [r["path"] for r in report["undetermined"]] == [str(d / "english.txt")]
    assert [r["path"] for r in report["unreadable"]] == [str(d / "empty.txt")]


def test_unreadable_reason_distinguishes_empty_from_short(tmp_path):
    empty = tmp_path / "empty.txt"
    empty.write_text("", encoding="utf-8")
    short = tmp_path / "short.txt"
    short.write_text("짧은 글", encoding="utf-8")

    assert inspect_file(str(empty))["reason"] == "missing_or_empty"
    assert inspect_file(str(short))["reason"] == "too_short"


def test_clean_file_has_no_reason(tmp_path):
    p = tmp_path / "clean.txt"
    p.write_text(CLEAN_KOREAN * 4, encoding="utf-8")
    assert inspect_file(str(p))["reason"] == ""


# ---------------------------------------------------------------------------
# 요약 출력과 CLI
# ---------------------------------------------------------------------------


def _report(**overrides):
    base = {
        "scanned_at": "2026-09-18T00:00:00+00:00",
        "root": "/mnt/data/text",
        "threshold": MOJIBAKE_LIKENESS_THRESHOLD,
        "total": 4,
        "counts": {VERDICT_CLEAN: 2, VERDICT_CORRUPTED: 1, VERDICT_UNDETERMINED: 1},
        "elapsed_sec": 120.0,
        "per_category": {
            "3_판타지": {VERDICT_CLEAN: 1, VERDICT_CORRUPTED: 1},
            "9_격언명언": {VERDICT_CLEAN: 1},
        },
        "corrupted": [],
        "unreadable": [],
        "undetermined": [],
    }
    base.update(overrides)
    return base


def test_print_summary_lists_categories_with_damage(capsys):
    from utils.detect_mojibake import print_summary

    print_summary(_report())
    out = capsys.readouterr().out

    assert "검사 4건" in out
    assert "손상이 있는 카테고리 1개" in out
    # 손상이 0건인 카테고리는 표에 넣지 않는다. 눈으로 훑을 목록만 남긴다.
    assert "3_판타지" in out
    assert "9_격언명언" not in out


def test_print_summary_without_damage_skips_the_table(capsys):
    from utils.detect_mojibake import print_summary

    print_summary(_report(counts={VERDICT_CLEAN: 4}, per_category={"3_판타지": {VERDICT_CLEAN: 4}}))
    out = capsys.readouterr().out

    assert "손상이 있는 카테고리" not in out


def test_print_summary_survives_zero_total(capsys):
    from utils.detect_mojibake import print_summary

    print_summary(_report(total=0, counts={}, per_category={}))
    # 0으로 나누지 않는다.
    assert "검사 0건" in capsys.readouterr().out


def run_cli(monkeypatch, *argv):
    from utils.detect_mojibake import main

    monkeypatch.setattr("sys.argv", ["detect_mojibake.py", *argv])
    return main()


def test_cli_check_prints_one_json_line_per_file(tmp_path, monkeypatch, capsys):
    clean = tmp_path / "clean.txt"
    clean.write_text(CLEAN_KOREAN, encoding="utf-8")
    broken = tmp_path / "broken.txt"
    broken.write_text(CORRUPTED, encoding="utf-8")

    assert run_cli(monkeypatch, "check", "--file", str(clean), "--file", str(broken)) == 0

    lines = [json.loads(ln) for ln in capsys.readouterr().out.strip().splitlines()]
    assert [r["verdict"] for r in lines] == [VERDICT_CLEAN, VERDICT_CORRUPTED]


def test_cli_check_without_file_is_an_error(monkeypatch):
    # 대상을 안 주면 전수 검사로 오해할 수 있다. 조용히 넘어가지 않는다.
    assert run_cli(monkeypatch, "check") == 1


def test_cli_scan_writes_report_and_summary(tmp_path, monkeypatch, capsys):
    root = tmp_path / "text"
    (root / "3_판타지").mkdir(parents=True)
    (root / "3_판타지" / "clean.txt").write_text(CLEAN_KOREAN, encoding="utf-8")
    (root / "3_판타지" / "broken.txt").write_text(CORRUPTED, encoding="utf-8")
    report_path = tmp_path / "out" / "report.json"

    assert run_cli(monkeypatch, "scan", "--root", str(root), "--workers", "1", "--report", str(report_path)) == 0

    report = json.loads(report_path.read_text(encoding="utf-8"))
    assert report["total"] == 2
    assert report["counts"][VERDICT_CORRUPTED] == 1
    assert [Path(r["path"]).name for r in report["corrupted"]] == ["broken.txt"]
    assert "리포트:" in capsys.readouterr().out


def test_scan_respects_limit_and_logs_progress(tmp_path, caplog):
    root = tmp_path / "text"
    (root / "3_판타지").mkdir(parents=True)
    for i in range(3):
        (root / "3_판타지" / f"{i}.txt").write_text(CLEAN_KOREAN, encoding="utf-8")

    with caplog.at_level("INFO", logger="detect_mojibake"):
        report = scan(root, workers=1, limit=2, progress_every=1)

    assert report["total"] == 2
    assert any("1/2" in r.getMessage() for r in caplog.records)


def test_scan_groups_files_outside_the_default_root_as_other(tmp_path):
    root = tmp_path / "text"
    (root / "3_판타지").mkdir(parents=True)
    (root / "3_판타지" / "a.txt").write_text(CLEAN_KOREAN, encoding="utf-8")

    report = scan(root, workers=1)
    # DEFAULT_LIBRARY_ROOT 밖이면 카테고리를 못 읽어 (기타)로 모은다.
    assert list(report["per_category"]) == ["(기타)"]


def test_classify_text_is_undetermined_when_hangul_is_too_sparse_to_measure():
    # 한글 비율(10%)은 넘지만 음절 수가 모자라 비율을 못 재는 구간.
    # 표지·판권지만 뽑힌 표본이 여기 해당한다.
    text = "가나다라마바사아자차" * 4 + "x" * 300
    assert len(text) >= 200
    hangul = sum(1 for ch in text if "가" <= ch <= "힣")
    assert hangul / len(text) >= 0.10
    assert hangul < MIN_SYLLABLES_FOR_LIKENESS

    verdict, likeness = classify_text(text)
    assert verdict == VERDICT_UNDETERMINED
    assert likeness is None


def test_list_library_files_skips_hidden_files_and_tool_caches(tmp_path):
    root = tmp_path / "text"
    (root / "3_판타지").mkdir(parents=True)
    (root / "3_판타지" / "a.txt").write_text("x", encoding="utf-8")
    (root / "3_판타지" / ".숨김.txt").write_text("x", encoding="utf-8")
    (root / "3_판타지" / "b.pdf").write_text("x", encoding="utf-8")
    cache = root / ".preview_cache"
    cache.mkdir()
    (cache / "c.txt").write_text("x", encoding="utf-8")

    names = [p.name for p in list_library_files(root)]
    assert names == ["a.txt"]
