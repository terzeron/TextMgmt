"""인코딩 손상 탐지기 테스트

판정 로직은 결정론적이다. 학습도 확률도 없이 문자만 센다.
"""

import json
import zipfile
from pathlib import Path

import pytest

from utils.detect_mojibake import MIN_SYLLABLES_FOR_LIKENESS, MOJIBAKE_LIKENESS_THRESHOLD, VERDICT_CLEAN, VERDICT_CORRUPTED, VERDICT_UNDETERMINED, VERDICT_UNREADABLE, classify_text, inspect_epub_structure, inspect_file, korean_likeness, list_library_files, scan, scan_epub_structure
from utils.detect_mojibake import DEFAULT_LIBRARY_ROOT, default_report_path

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


def test_importing_adds_repo_root_to_sys_path(monkeypatch):
    import runpy
    import sys

    from utils import detect_mojibake

    repo_str = str(detect_mojibake.REPO_ROOT)
    monkeypatch.setattr(sys, "path", [p for p in sys.path if p != repo_str])
    runpy.run_module("utils.detect_mojibake", run_name="test_mod")
    assert repo_str in sys.path


def test_running_as_script_exits_with_main_status(monkeypatch, capsys):
    import runpy
    import sys

    monkeypatch.setattr(sys, "argv", ["detect_mojibake.py", "check"])
    with pytest.raises(SystemExit) as exc:
        runpy.run_module("utils.detect_mojibake", run_name="__main__")
    assert exc.value.code == 1


# ---------------------------------------------------------------------------
# EPUB 구조 검사: epubcheck 는 통과시키지만 뷰어가 렌더링하지 못하는 구조
# ---------------------------------------------------------------------------

XHTML_NS = "http://www.w3.org/1999/xhtml"
CONTAINER_XML = '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>'
OPF_XML = '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest><item id="c1" href="ch%201.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>'
PLAIN_CHAPTER = f'<?xml version="1.0"?>\n<!DOCTYPE html [<!ELEMENT html ANY>]>\n<!-- <fake> -->\n<html xmlns="{XHTML_NS}"><head><title/></head><body><p>본문</p></body></html>'
PREFIXED_CHAPTER = f'<?xml version="1.0"?>\n<html:html xmlns:html="{XHTML_NS}"><html:head><html:title/></html:head><html:body><html:p>본문</html:p></html:body></html:html>'


def make_epub(path, chapter=PLAIN_CHAPTER, *, mimetype_first=True, container=CONTAINER_XML, opf=OPF_XML, chapter_name="OEBPS/ch 1.xhtml"):
    with zipfile.ZipFile(path, "w") as z:
        if mimetype_first:
            z.writestr("mimetype", "application/epub+zip", zipfile.ZIP_STORED)
        if container is not None:
            z.writestr("META-INF/container.xml", container, zipfile.ZIP_DEFLATED)
        if opf is not None:
            z.writestr("OEBPS/content.opf", opf, zipfile.ZIP_DEFLATED)
        if chapter_name:
            z.writestr(chapter_name, chapter, zipfile.ZIP_DEFLATED)
        if not mimetype_first:
            z.writestr("mimetype", "application/epub+zip", zipfile.ZIP_DEFLATED)
    return path


def test_epub_structure_accepts_a_standard_epub(tmp_path):
    """루트 앞의 DOCTYPE 내부 선언과 주석에 속지 않고, 퍼센트 인코딩된 href 도 푼다"""
    res = inspect_epub_structure(str(make_epub(tmp_path / "ok.epub")))
    assert res["issues"] == []


def test_epub_structure_flags_prefixed_root(tmp_path):
    res = inspect_epub_structure(str(make_epub(tmp_path / "p.epub", PREFIXED_CHAPTER)))
    assert res["issues"] == ["prefixed_root"]
    assert res["examples"]["prefixed_root"] == "OEBPS/ch 1.xhtml"


def test_epub_structure_reports_each_issue_once_with_first_example(tmp_path):
    opf = OPF_XML.replace('<itemref idref="c1"/>', '<itemref idref="c1"/><itemref idref="c2"/>').replace("</manifest>", '<item id="c2" href="ch2.xhtml" media-type="application/xhtml+xml"/></manifest>')
    epub = make_epub(tmp_path / "t.epub", PREFIXED_CHAPTER, opf=opf)
    with zipfile.ZipFile(epub, "a") as z:
        z.writestr("OEBPS/ch2.xhtml", PREFIXED_CHAPTER)
    res = inspect_epub_structure(str(epub))
    assert res["issues"] == ["prefixed_root"]
    assert res["examples"]["prefixed_root"] == "OEBPS/ch 1.xhtml"


def _epub_with_media_spine_item(path, media_type, member_name, payload):
    opf = (
        '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><manifest>'
        f'<item id="a" href="{member_name}" media-type="{media_type}"/></manifest><spine><itemref idref="a"/></spine></package>'
    )
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("mimetype", "application/epub+zip", zipfile.ZIP_STORED)
        zf.writestr("META-INF/container.xml", CONTAINER_XML)
        zf.writestr("OEBPS/content.opf", opf)
        zf.writestr(f"OEBPS/{member_name}", payload)
    return path


def test_epub_structure_does_not_misread_binary_spine_item_as_prefixed_root(tmp_path):
    """spine 에 오디오가 들어 있는 EPUB 은 바이너리의 우연한 '<x:y' 를 접두사 루트로 오탐하지 않는다.

    코퍼스의 0_telegram 천승세의 포대령.epub 이 이 형태였다(mp3 헤더에 '<' 와 ':' 가 섞여 있었다).
    """
    epub = _epub_with_media_spine_item(tmp_path / "audio.epub", "audio/mpeg", "Audio/001.mp3", b"\xff\xfb\x90\x00 <ab:cd garbage")
    assert inspect_epub_structure(str(epub))["issues"] == []


@pytest.mark.parametrize(("media_type", "name"), [("application/xhtml+xml", "c.xhtml"), ("text/html", "c.html"), ("image/svg+xml", "c.svg"), ("application/xml", "c.xml")])
def test_epub_structure_still_checks_markup_spine_items(tmp_path, media_type, name):
    epub = _epub_with_media_spine_item(tmp_path / "m.epub", media_type, name, PREFIXED_CHAPTER)
    assert inspect_epub_structure(str(epub))["issues"] == ["prefixed_root"]


def test_epub_structure_falls_back_to_extension_when_media_type_is_missing(tmp_path):
    epub = _epub_with_media_spine_item(tmp_path / "e.epub", "", "c.xhtml", PREFIXED_CHAPTER)
    assert inspect_epub_structure(str(epub))["issues"] == ["prefixed_root"]


def test_epub_structure_flags_non_zip(tmp_path):
    bad = tmp_path / "bad.epub"
    bad.write_bytes(b"not a zip")
    assert inspect_epub_structure(str(bad))["issues"] == ["not_zip"]


def test_epub_structure_flags_truncated_download_as_non_zip(tmp_path):
    """중단된 다운로드: 앞부분은 정상 ZIP 이지만 끝 디렉토리(EOCD)가 없다"""
    epub = make_epub(tmp_path / "t.epub")
    cut = tmp_path / "cut.epub"
    cut.write_bytes(epub.read_bytes()[:-100])
    assert inspect_epub_structure(str(cut))["issues"] == ["not_zip"]


def test_epub_structure_flags_missing_file_as_non_zip(tmp_path):
    assert inspect_epub_structure(str(tmp_path / "none.epub"))["issues"] == ["not_zip"]


def test_epub_structure_flags_mimetype_not_first_or_stored(tmp_path):
    res = inspect_epub_structure(str(make_epub(tmp_path / "m.epub", mimetype_first=False)))
    assert res["issues"] == ["bad_mimetype"]


def test_epub_structure_flags_empty_zip(tmp_path):
    empty = tmp_path / "empty.epub"
    zipfile.ZipFile(empty, "w").close()
    assert inspect_epub_structure(str(empty))["issues"] == ["bad_mimetype", "no_container"]


@pytest.mark.parametrize("container", [None, "<container", "<container><rootfiles/></container>"])
def test_epub_structure_flags_unusable_container(tmp_path, container):
    res = inspect_epub_structure(str(make_epub(tmp_path / "c.epub", container=container)))
    assert res["issues"] == ["no_container"]


def _corrupt_local_header(path, member):
    """ZIP 중앙 디렉토리는 그대로 두고 멤버의 로컬 헤더 시그니처만 망가뜨린다(Bad magic number 를 재현)."""
    with zipfile.ZipFile(path) as zf:
        offset = zf.getinfo(member).header_offset
    data = bytearray(path.read_bytes())
    data[offset : offset + 4] = b"\x00\x00\x00\x00"
    path.write_bytes(bytes(data))


def test_epub_structure_flags_unreadable_opf_not_missing(tmp_path):
    """OPF 가 ZIP 에 있지만 읽히지 않으면 opf_missing 이 아니라 member_unreadable 이다.

    코퍼스의 Lumineers Songbook 이 이 형태였다. '없음'이라고 보고하면 파일을 고칠 수 있다고 오해한다.
    """
    epub = make_epub(tmp_path / "u.epub")
    _corrupt_local_header(epub, "OEBPS/content.opf")
    res = inspect_epub_structure(str(epub))
    assert res["issues"] == ["member_unreadable"]
    assert res["examples"]["member_unreadable"] == "OEBPS/content.opf"


def test_epub_structure_flags_unreadable_container_not_missing(tmp_path):
    epub = make_epub(tmp_path / "u.epub")
    _corrupt_local_header(epub, "META-INF/container.xml")
    res = inspect_epub_structure(str(epub))
    assert res["issues"] == ["member_unreadable"]
    assert res["examples"]["member_unreadable"] == "META-INF/container.xml"


def test_epub_structure_flags_unreadable_chapter_instead_of_crashing(tmp_path):
    """본문 멤버의 헤더가 망가져도(Bad magic number / Overlapped entries) 스캔이 죽지 않고 분류한다.

    9_BLGL 의 EPUB 이 zipfile.BadZipFile 로 디렉토리 전체 스캔을 중단시켰다.
    """
    epub = make_epub(tmp_path / "c.epub")
    _corrupt_local_header(epub, "OEBPS/ch 1.xhtml")
    res = inspect_epub_structure(str(epub))
    assert res["issues"] == ["member_unreadable"]
    assert res["examples"]["member_unreadable"] == "OEBPS/ch 1.xhtml"


def test_epub_structure_records_unexpected_errors_instead_of_aborting(tmp_path, monkeypatch):
    """예상 못 한 예외가 나도 그 파일만 scan_error 로 기록하고 다음 파일로 간다."""
    epub = make_epub(tmp_path / "e.epub")

    def boom(head):
        raise RuntimeError("예상 못 한 오류")

    monkeypatch.setattr("utils.detect_mojibake._root_tag_name", boom)
    res = inspect_epub_structure(str(epub))
    assert res["issues"] == ["scan_error"]
    assert "RuntimeError" in res["examples"]["scan_error"]


def test_scan_epub_structure_survives_a_file_that_breaks_the_inspector(tmp_path):
    make_epub(tmp_path / "a_ok.epub")
    bad = make_epub(tmp_path / "b_bad.epub")
    _corrupt_local_header(bad, "OEBPS/ch 1.xhtml")
    make_epub(tmp_path / "c_ok.epub", PREFIXED_CHAPTER)
    report = scan_epub_structure(tmp_path, workers=1)
    assert report["total"] == 3
    assert report["counts"]["member_unreadable"] == 1
    assert report["counts"]["prefixed_root"] == 1


def test_epub_structure_flags_missing_opf(tmp_path):
    res = inspect_epub_structure(str(make_epub(tmp_path / "o.epub", opf=None)))
    assert res["issues"] == ["opf_missing"]
    assert res["examples"]["opf_missing"] == "OEBPS/content.opf"


def test_epub_structure_flags_unparsable_opf(tmp_path):
    res = inspect_epub_structure(str(make_epub(tmp_path / "o.epub", opf="<package")))
    assert res["issues"] == ["opf_unparsable"]


def test_epub_structure_flags_spine_member_missing(tmp_path):
    res = inspect_epub_structure(str(make_epub(tmp_path / "s.epub", chapter_name=None)))
    assert res["issues"] == ["spine_member_missing"]
    assert res["examples"]["spine_member_missing"] == "OEBPS/ch 1.xhtml"


def test_epub_structure_ignores_spine_items_without_manifest_entry(tmp_path):
    opf = OPF_XML.replace('<itemref idref="c1"/>', '<itemref idref="ghost"/>')
    assert inspect_epub_structure(str(make_epub(tmp_path / "g.epub", opf=opf)))["issues"] == []


def test_epub_structure_flags_oversized_xml_member_as_unreadable(tmp_path):
    big_opf = OPF_XML + " " * (5 * 1024 * 1024)
    assert inspect_epub_structure(str(make_epub(tmp_path / "b.epub", opf=big_opf)))["issues"] == ["member_unreadable"]


def test_epub_structure_chapter_without_root_tag_is_not_flagged(tmp_path):
    assert inspect_epub_structure(str(make_epub(tmp_path / "r.epub", chapter="plain text")))["issues"] == []


def test_scan_epub_structure_counts_only_epubs_and_respects_limit(tmp_path):
    make_epub(tmp_path / "a.epub")
    make_epub(tmp_path / "b.epub", PREFIXED_CHAPTER)
    (tmp_path / "c.txt").write_text("본문", encoding="utf-8")
    report = scan_epub_structure(tmp_path, workers=1)
    assert report["total"] == 2
    assert report["flagged_count"] == 1
    assert report["counts"]["prefixed_root"] == 1
    assert [Path(r["path"]).name for r in report["flagged"]] == ["b.epub"]
    assert scan_epub_structure(tmp_path, workers=1, limit=1)["total"] == 1


def test_cli_epub_structure_file_prints_one_json_line(tmp_path, monkeypatch, capsys):
    epub = make_epub(tmp_path / "p.epub", PREFIXED_CHAPTER)
    assert run_cli(monkeypatch, "epub-structure", "--file", str(epub)) == 0
    assert json.loads(capsys.readouterr().out)["issues"] == ["prefixed_root"]


def test_cli_epub_structure_scan_writes_report_and_summary(tmp_path, monkeypatch, capsys):
    make_epub(tmp_path / "p.epub", PREFIXED_CHAPTER)
    report = tmp_path / "out" / "r.json"
    assert run_cli(monkeypatch, "epub-structure", "--root", str(tmp_path), "--workers", "1", "--report", str(report)) == 0
    out = capsys.readouterr().out
    assert "prefixed_root" in out and "문제 1건" in out
    assert json.loads(report.read_text(encoding="utf-8"))["flagged_count"] == 1


# ---------------------------------------------------------------------------
# 리포트 파일명: 대상 디렉토리마다 달라야 다른 디렉토리 스캔이 이전 리포트를 덮어쓰지 않는다
# ---------------------------------------------------------------------------


def test_default_report_path_differs_per_directory(monkeypatch, tmp_path):
    monkeypatch.setattr("utils.detect_mojibake.REPO_ROOT", tmp_path)
    novel = default_report_path("epub_structure", DEFAULT_LIBRARY_ROOT / "2_소설외국")
    classics = default_report_path("epub_structure", DEFAULT_LIBRARY_ROOT / "2_소설한국")
    assert novel == tmp_path / "tmp" / "epub_structure_report_2_소설외국.json"
    assert novel != classics


def test_default_report_path_differs_per_kind(monkeypatch, tmp_path):
    monkeypatch.setattr("utils.detect_mojibake.REPO_ROOT", tmp_path)
    root = DEFAULT_LIBRARY_ROOT / "2_소설외국"
    assert default_report_path("mojibake", root).name == "mojibake_report_2_소설외국.json"
    assert default_report_path("mojibake", root) != default_report_path("epub_structure", root)


def test_default_report_path_for_library_root_and_nested_and_outside(monkeypatch, tmp_path):
    monkeypatch.setattr("utils.detect_mojibake.REPO_ROOT", tmp_path)
    assert default_report_path("mojibake", DEFAULT_LIBRARY_ROOT).name == "mojibake_report_all.json"
    assert default_report_path("mojibake", DEFAULT_LIBRARY_ROOT / "2_소설외국" / "completed").name == "mojibake_report_2_소설외국__completed.json"
    assert default_report_path("mojibake", tmp_path / "my dir").name == "mojibake_report_my_dir.json"
    assert default_report_path("mojibake", Path("/")).name == "mojibake_report_all.json"


def test_default_report_path_normalizes_dot_segments_and_trailing_slash(monkeypatch, tmp_path):
    monkeypatch.setattr("utils.detect_mojibake.REPO_ROOT", tmp_path)
    assert default_report_path("mojibake", Path(f"{DEFAULT_LIBRARY_ROOT}/2_소설외국/../2_소설한국/")).name == "mojibake_report_2_소설한국.json"


def test_cli_without_report_writes_a_per_directory_file(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr("utils.detect_mojibake.REPO_ROOT", tmp_path)
    scanned = tmp_path / "library_a"
    scanned.mkdir()
    (scanned / "a.txt").write_text(CLEAN_KOREAN, encoding="utf-8")
    other = tmp_path / "library_b"
    other.mkdir()
    make_epub(other / "b.epub")

    assert run_cli(monkeypatch, "scan", "--root", str(scanned), "--workers", "1") == 0
    assert run_cli(monkeypatch, "epub-structure", "--root", str(other), "--workers", "1") == 0
    assert run_cli(monkeypatch, "epub-structure", "--root", str(scanned), "--workers", "1") == 0

    names = sorted(p.name for p in (tmp_path / "tmp").iterdir())
    assert names == ["epub_structure_report_library_a.json", "epub_structure_report_library_b.json", "mojibake_report_library_a.json"]
    assert json.loads((tmp_path / "tmp" / "epub_structure_report_library_b.json").read_text(encoding="utf-8"))["root"] == str(other)
    assert "epub_structure_report_library_b.json" in capsys.readouterr().out

