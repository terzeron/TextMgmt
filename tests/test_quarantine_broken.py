"""손상 문서 격리 도구 테스트

이동은 항상 되돌릴 수 있어야 한다. 판정에 오탐이 섞일 수 있기 때문이다.
"""

import json
from pathlib import Path

import pytest

from utils.quarantine_broken import BROKEN_DIR_NAME, apply_moves, fix_extension, load_corrupted, plan_moves, true_extension, undo_moves, unmangle_utf16


@pytest.fixture
def library(tmp_path):
    root = tmp_path / "text"
    for cat, names in [("3_판타지", ["a.txt", "b.txt"]), ("9_격언명언", ["c.txt"])]:
        d = root / cat
        d.mkdir(parents=True)
        for n in names:
            (d / n).write_text(f"{cat}/{n} 내용", encoding="utf-8")
    return root


def records_for(library, *rels):
    return [{"path": str(library / r), "verdict": "corrupted", "likeness": 0.004, "size": 100} for r in rels]


# ---------------------------------------------------------------------------
# 계획
# ---------------------------------------------------------------------------


def test_plan_keeps_original_category_as_subdirectory(library):
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    assert moves[0]["dst"] == str(library / BROKEN_DIR_NAME / "3_판타지" / "a.txt")


def test_plan_skips_files_already_quarantined(library):
    (library / BROKEN_DIR_NAME / "3_판타지").mkdir(parents=True)
    already = library / BROKEN_DIR_NAME / "3_판타지" / "z.txt"
    already.write_text("x", encoding="utf-8")
    assert plan_moves(records_for(library, f"{BROKEN_DIR_NAME}/3_판타지/z.txt"), library) == []


def test_plan_skips_paths_outside_the_library(library, tmp_path):
    outside = tmp_path / "바깥.txt"
    outside.write_text("x", encoding="utf-8")
    assert plan_moves([{"path": str(outside), "verdict": "corrupted"}], library) == []


def test_plan_avoids_overwriting_on_name_collision(library):
    dst = library / BROKEN_DIR_NAME / "3_판타지"
    dst.mkdir(parents=True)
    (dst / "a.txt").write_text("먼저 있던 것", encoding="utf-8")
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    assert moves[0]["dst"].endswith("a__1.txt")


def test_plan_gives_distinct_destinations_for_same_name_in_one_run(library):
    (library / "9_격언명언" / "a.txt").write_text("다른 카테고리 동명 파일", encoding="utf-8")
    moves = plan_moves(records_for(library, "3_판타지/a.txt", "9_격언명언/a.txt"), library)
    assert len({m["dst"] for m in moves}) == 2


# ---------------------------------------------------------------------------
# 이동과 복구
# ---------------------------------------------------------------------------


def test_move_relocates_file_and_writes_manifest(library, tmp_path):
    manifest = tmp_path / "manifest.json"
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    result = apply_moves(moves, manifest)

    assert not (library / "3_판타지" / "a.txt").exists()
    assert (library / BROKEN_DIR_NAME / "3_판타지" / "a.txt").read_text(encoding="utf-8") == "3_판타지/a.txt 내용"
    assert len(result["moved"]) == 1
    assert manifest.exists()


def test_dry_run_moves_nothing_and_writes_no_manifest(library, tmp_path):
    manifest = tmp_path / "manifest.json"
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    result = apply_moves(moves, manifest, dry_run=True)

    assert (library / "3_판타지" / "a.txt").exists()
    assert not manifest.exists()
    assert len(result["moved"]) == 1


def test_missing_source_is_reported_not_raised(library, tmp_path):
    moves = [{"src": str(library / "3_판타지" / "없음.txt"), "dst": str(library / BROKEN_DIR_NAME / "x.txt"), "size": 0}]
    result = apply_moves(moves, tmp_path / "m.json")
    assert result["moved"] == []
    assert result["failed"][0]["error"] == "원본 없음"


def test_undo_restores_every_moved_file(library, tmp_path):
    manifest = tmp_path / "manifest.json"
    moves = plan_moves(records_for(library, "3_판타지/a.txt", "9_격언명언/c.txt"), library)
    apply_moves(moves, manifest)
    assert not (library / "3_판타지" / "a.txt").exists()

    res = undo_moves(manifest)
    assert len(res["restored"]) == 2
    assert (library / "3_판타지" / "a.txt").read_text(encoding="utf-8") == "3_판타지/a.txt 내용"
    assert (library / "9_격언명언" / "c.txt").exists()


def test_undo_empties_the_manifest_so_it_is_not_repeated(library, tmp_path):
    manifest = tmp_path / "manifest.json"
    apply_moves(plan_moves(records_for(library, "3_판타지/a.txt"), library), manifest)
    undo_moves(manifest)
    assert json.loads(manifest.read_text(encoding="utf-8"))["moved"] == []
    assert undo_moves(manifest)["restored"] == []


def test_undo_does_not_overwrite_a_file_back_at_the_origin(library, tmp_path):
    manifest = tmp_path / "manifest.json"
    apply_moves(plan_moves(records_for(library, "3_판타지/a.txt"), library), manifest)
    (library / "3_판타지" / "a.txt").write_text("나중에 생긴 다른 파일", encoding="utf-8")

    res = undo_moves(manifest)
    assert res["restored"] == []
    assert res["failed"][0]["error"] == "원위치에 파일이 이미 있다"
    assert (library / "3_판타지" / "a.txt").read_text(encoding="utf-8") == "나중에 생긴 다른 파일"


def test_manifest_accumulates_across_separate_runs(library, tmp_path):
    """나눠 옮겨도 undo 가 전부를 되돌린다"""
    manifest = tmp_path / "manifest.json"
    apply_moves(plan_moves(records_for(library, "3_판타지/a.txt"), library), manifest)
    apply_moves(plan_moves(records_for(library, "9_격언명언/c.txt"), library), manifest)

    assert len(json.loads(manifest.read_text(encoding="utf-8"))["moved"]) == 2
    assert len(undo_moves(manifest)["restored"]) == 2


def test_undo_without_manifest_raises(tmp_path):
    with pytest.raises(FileNotFoundError):
        undo_moves(tmp_path / "없음.json")


# ---------------------------------------------------------------------------
# 리포트 읽기
# ---------------------------------------------------------------------------


def test_load_corrupted_reads_scan_report_shape(tmp_path):
    p = tmp_path / "report.json"
    p.write_text(json.dumps({"corrupted": [{"path": "/a.txt", "verdict": "corrupted"}]}), encoding="utf-8")
    assert len(load_corrupted(p)) == 1


def test_load_corrupted_reads_recheck_list_shape(tmp_path):
    p = tmp_path / "recheck.json"
    p.write_text(json.dumps([{"path": "/a.txt", "verdict": "corrupted"}, {"path": "/b.txt", "verdict": "clean"}, {"path": "/c.txt", "verdict": "undetermined"}]), encoding="utf-8")
    assert [r["path"] for r in load_corrupted(p)] == ["/a.txt"]


# ---------------------------------------------------------------------------
# 확장자 교정
# ---------------------------------------------------------------------------

# OLE 매직 D0 CF 11 E0 A1 B1 1A E1 을 UTF-16LE 로 읽으면 이 네 글자가 된다.
OLE_AS_UTF16_CHARS = "\ucfd0\ue011\ub1a1\ue11a"
OLE_MAGIC = bytes.fromhex("d0cf11e0a1b11ae1")


def binary_pair(body: str = "FileHeader 본문 내용 가나다라마바사"):
    """
    (원래 바이너리, 망가진 상태) 쌍을 만든다.

    원본을 문자열에서 만들어야 UTF-16LE 왕복이 보장된다.
    바이트를 아무렇게나 만들면 서로게이트 범위에 걸려 디코딩 자체가 실패한다.
    """
    as_text = OLE_AS_UTF16_CHARS + body * 20
    return as_text.encode("utf-16-le"), as_text.encode("utf-8")


def test_unmangle_restores_original_bytes():
    원본, 망가진것 = binary_pair()
    assert 원본.startswith(OLE_MAGIC)
    assert unmangle_utf16(망가진것) == 원본


def test_unmangle_returns_none_for_plain_text():
    assert unmangle_utf16("평범한 한국어 텍스트입니다. ".encode("utf-8") * 20) is None


def test_unmangle_tolerates_truncated_tail():
    _원본, 망가진것 = binary_pair()
    assert unmangle_utf16(망가진것[:-1]) is not None


def test_true_extension_identifies_hwp():
    _원본, 망가진것 = binary_pair()
    assert true_extension(망가진것) == ".hwp"


def test_true_extension_is_none_for_text():
    assert true_extension("무림맹의 장문인. ".encode("utf-8") * 40) is None


def test_fix_extension_renames_and_restores(tmp_path):
    원본, 망가진것 = binary_pair()
    p = tmp_path / "경혼기.txt"
    p.write_bytes(망가진것)

    res = fix_extension(p)
    assert res is not None and "error" not in res
    assert not p.exists()
    assert (tmp_path / "경혼기.hwp").read_bytes() == 원본


def test_fix_extension_dry_run_changes_nothing(tmp_path):
    _원본, 망가진것 = binary_pair()
    p = tmp_path / "책.txt"
    p.write_bytes(망가진것)

    assert fix_extension(p, dry_run=True) is not None
    assert p.exists()
    assert not (tmp_path / "책.hwp").exists()


def test_fix_extension_ignores_normal_text_file(tmp_path):
    p = tmp_path / "소설.txt"
    p.write_text("무림맹의 장문인은 제자들을 이끌었다. " * 30, encoding="utf-8")
    assert fix_extension(p) is None
    assert p.exists()


def test_fix_extension_avoids_overwriting_existing_target(tmp_path):
    _원본, 망가진것 = binary_pair()
    p = tmp_path / "책.txt"
    p.write_bytes(망가진것)
    기존 = "먼저 있던 것".encode("utf-8")
    (tmp_path / "책.hwp").write_bytes(기존)

    res = fix_extension(p)
    assert Path(res["restored_to"]).name == "책__1.hwp"
    assert (tmp_path / "책.hwp").read_bytes() == 기존


# ---------------------------------------------------------------------------
# 실패 경로
# ---------------------------------------------------------------------------


def test_fix_extension_reports_unreadable_file(tmp_path, monkeypatch):
    f = tmp_path / "a.txt"
    f.write_text("내용", encoding="utf-8")

    def boom(self, *args, **kwargs):
        raise OSError("읽을 수 없음")

    monkeypatch.setattr(Path, "read_bytes", boom)
    res = fix_extension(f)
    assert res is not None and "읽을 수 없음" in res["error"]


def test_fix_extension_refuses_lossy_roundtrip(tmp_path, monkeypatch):
    original, mangled = binary_pair()
    f = tmp_path / "a.txt"
    f.write_bytes(mangled)

    # 무손실이 아니면 손대지 않는다 — 원본을 잃는 것이 오탐보다 나쁘다.
    monkeypatch.setattr("utils.quarantine_broken.unmangle_utf16", lambda raw: original + b"\x00\x00")
    res = fix_extension(f)
    assert res is not None and res["error"] == "무손실 왕복 실패"
    assert f.read_bytes() == mangled


def test_fix_extension_skips_when_unmangle_returns_none(tmp_path, monkeypatch):
    f = tmp_path / "a.txt"
    f.write_bytes(binary_pair()[1])

    calls = {"n": 0}

    def flaky(raw):
        # true_extension 에서는 성공하고 본 검사에서는 실패하는 경계.
        calls["n"] += 1
        return binary_pair()[0] if calls["n"] == 1 else None

    monkeypatch.setattr("utils.quarantine_broken.unmangle_utf16", flaky)
    assert fix_extension(f) is None


def test_unmangle_gives_up_when_nothing_decodes():
    # 어느 꼬리 길이로도 UTF-8 로 안 읽히면 포기한다.
    assert unmangle_utf16(b"\xff\xfe\xfd\xfc\xfb") is None


def test_true_extension_is_none_when_magic_not_listed(monkeypatch):
    monkeypatch.setattr("utils.quarantine_broken.unmangle_utf16", lambda raw: b"ZZZZ not a known magic")
    assert true_extension(b"whatever") is None


def test_move_reports_os_error(library, tmp_path, monkeypatch):
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)

    def boom(src, dst):
        raise OSError("디스크 가득 참")

    monkeypatch.setattr("utils.quarantine_broken.shutil.move", boom)
    result = apply_moves(moves, tmp_path / "manifest.json")

    assert result["moved"] == []
    assert "디스크 가득 참" in result["failed"][0]["error"]


def test_move_rewrites_manifest_when_previous_one_is_broken(library, tmp_path):
    manifest = tmp_path / "manifest.json"
    manifest.write_text("이건 JSON 이 아니다", encoding="utf-8")

    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    result = apply_moves(moves, manifest)

    # 옛 매니페스트를 못 읽어도 이번 이동 기록은 남겨야 undo 가 가능하다.
    assert len(result["moved"]) == 1
    assert len(json.loads(manifest.read_text(encoding="utf-8"))["moved"]) == 1


def test_undo_reports_missing_quarantined_file(library, tmp_path):
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    apply_moves(moves, tmp_path / "manifest.json")
    Path(moves[0]["dst"]).unlink()

    res = undo_moves(tmp_path / "manifest.json")
    assert res["restored"] == []
    assert res["failed"][0]["error"] == "격리본 없음"


def test_undo_dry_run_moves_nothing(library, tmp_path):
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    apply_moves(moves, tmp_path / "manifest.json")

    res = undo_moves(tmp_path / "manifest.json", dry_run=True)
    assert len(res["restored"]) == 1
    # 실제로는 안 옮긴다. 매니페스트도 그대로 남는다.
    assert Path(moves[0]["dst"]).exists()
    assert len(json.loads((tmp_path / "manifest.json").read_text(encoding="utf-8"))["moved"]) == 1


def test_undo_reports_os_error(library, tmp_path, monkeypatch):
    moves = plan_moves(records_for(library, "3_판타지/a.txt"), library)
    apply_moves(moves, tmp_path / "manifest.json")

    def boom(src, dst):
        raise OSError("권한 없음")

    monkeypatch.setattr("utils.quarantine_broken.shutil.move", boom)
    res = undo_moves(tmp_path / "manifest.json")
    assert res["restored"] == []
    assert "권한 없음" in res["failed"][0]["error"]


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def report_for(tmp_path, library, *rels):
    p = tmp_path / "report.json"
    p.write_text(json.dumps(records_for(library, *rels), ensure_ascii=False), encoding="utf-8")
    return p


def run_cli(monkeypatch, *argv):
    from utils.quarantine_broken import main

    monkeypatch.setattr("sys.argv", ["quarantine_broken.py", *argv])
    return main()


def test_cli_plan_prints_summary_without_moving(library, tmp_path, monkeypatch, capsys):
    report = report_for(tmp_path, library, "3_판타지/a.txt", "9_격언명언/c.txt")

    assert run_cli(monkeypatch, "plan", "--report", str(report), "--library-root", str(library)) == 0

    out = capsys.readouterr().out
    assert "대상 2건" in out
    assert "3_판타지" in out
    # 계획만 세운다. 파일은 그대로다.
    assert (library / "3_판타지" / "a.txt").exists()


def test_cli_move_then_undo_round_trip(library, tmp_path, monkeypatch, capsys):
    report = report_for(tmp_path, library, "3_판타지/a.txt")
    manifest = tmp_path / "manifest.json"
    src = library / "3_판타지" / "a.txt"

    assert run_cli(monkeypatch, "move", "--report", str(report), "--library-root", str(library), "--manifest", str(manifest)) == 0
    assert not src.exists()
    assert "이동 완료 1건" in capsys.readouterr().out

    assert run_cli(monkeypatch, "undo", "--manifest", str(manifest)) == 0
    assert src.exists()
    assert "원위치 1건" in capsys.readouterr().out


def test_cli_move_dry_run_keeps_files(library, tmp_path, monkeypatch, capsys):
    report = report_for(tmp_path, library, "3_판타지/a.txt")
    manifest = tmp_path / "manifest.json"

    assert run_cli(monkeypatch, "move", "--report", str(report), "--library-root", str(library), "--manifest", str(manifest), "--dry-run") == 0

    assert "이동 예정 1건" in capsys.readouterr().out
    assert (library / "3_판타지" / "a.txt").exists()
    assert not manifest.exists()


def test_cli_limit_and_exclude_narrow_the_target(library, tmp_path, monkeypatch, capsys):
    report = report_for(tmp_path, library, "3_판타지/a.txt", "3_판타지/b.txt", "9_격언명언/c.txt")

    assert run_cli(monkeypatch, "plan", "--report", str(report), "--library-root", str(library), "--exclude", "c.txt", "--limit", "1") == 0

    out = capsys.readouterr().out
    # 제외 뒤 2건이 남고, limit 으로 1건만 계획한다.
    assert "대상 1건" in out


def test_cli_move_reports_missing_source(library, tmp_path, monkeypatch, capsys):
    report = report_for(tmp_path, library, "3_판타지/없는파일.txt")
    manifest = tmp_path / "manifest.json"

    assert run_cli(monkeypatch, "move", "--report", str(report), "--library-root", str(library), "--manifest", str(manifest)) == 0
    assert "실패: 없는파일.txt - 원본 없음" in capsys.readouterr().out


def test_cli_undo_reports_failures(library, tmp_path, monkeypatch, capsys):
    report = report_for(tmp_path, library, "3_판타지/a.txt")
    manifest = tmp_path / "manifest.json"
    run_cli(monkeypatch, "move", "--report", str(report), "--library-root", str(library), "--manifest", str(manifest))
    capsys.readouterr()

    # 격리본을 지우면 되돌릴 수 없다.
    moved = json.loads(manifest.read_text(encoding="utf-8"))["moved"]
    Path(moved[0]["dst"]).unlink()

    assert run_cli(monkeypatch, "undo", "--manifest", str(manifest)) == 0
    assert "실패: a.txt - 격리본 없음" in capsys.readouterr().out


def test_cli_fix_ext_restores_and_reports(tmp_path, monkeypatch, capsys):
    original, mangled = binary_pair()
    target = tmp_path / "망가진문서.txt"
    target.write_bytes(mangled)
    report = tmp_path / "report.json"
    report.write_text(json.dumps([{"path": str(target), "verdict": "corrupted"}], ensure_ascii=False), encoding="utf-8")

    assert run_cli(monkeypatch, "fix-ext", "--report", str(report)) == 0

    out = capsys.readouterr().out
    assert "교정 완료 1건" in out
    assert (tmp_path / "망가진문서.hwp").read_bytes() == original
    assert not target.exists()


def test_cli_fix_ext_dry_run_and_skip_report(tmp_path, monkeypatch, capsys):
    _, mangled = binary_pair()
    target = tmp_path / "망가진문서.txt"
    target.write_bytes(mangled)
    plain = tmp_path / "정상문서.txt"
    plain.write_text("보통 한국어 문장입니다", encoding="utf-8")
    missing = tmp_path / "없는파일.txt"
    report = tmp_path / "report.json"
    report.write_text(json.dumps([{"path": str(target)}, {"path": str(plain)}, {"path": str(missing)}], ensure_ascii=False), encoding="utf-8")

    assert run_cli(monkeypatch, "fix-ext", "--report", str(report), "--dry-run") == 0

    out = capsys.readouterr().out
    assert "교정 예정 1건" in out
    # 대상이 아닌 파일은 조용히 넘어가고, 못 읽은 파일만 건너뜀으로 알린다.
    assert "건너뜀: 없는파일.txt" in out
    assert target.exists()


def test_summarize_groups_by_original_category(capsys):
    from utils.quarantine_broken import summarize

    summarize(
        [
            {"dst": f"/text/{BROKEN_DIR_NAME}/3_판타지/a.txt", "size": 1024 * 1024},
            {"dst": f"/text/{BROKEN_DIR_NAME}/3_판타지/b.txt", "size": None},
            {"dst": f"/text/{BROKEN_DIR_NAME}/9_격언명언/c.txt", "size": 0},
        ]
    )
    out = capsys.readouterr().out
    assert "대상 3건, 1.0MB" in out
    # size 가 None 이어도 합계에서 죽지 않는다.
    assert "3_판타지" in out and "9_격언명언" in out


def test_summarize_labels_files_without_a_category(capsys):
    # 격리 루트 바로 아래 놓인 경로의 방어 갈래.
    from utils.quarantine_broken import summarize

    summarize([{"dst": f"/text/{BROKEN_DIR_NAME}", "size": 0}])
    assert "(최상위)" in capsys.readouterr().out
