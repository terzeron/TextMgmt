import json
import runpy
import sys
from pathlib import Path
from unittest.mock import patch

from utils.cluster_hybrid_genres import (
    extract_head_tail_words,
    calculate_genre_frequencies,
    main,
)


def test_extract_head_tail_words(tmp_path: Path):
    f = tmp_path / "test.txt"
    f.write_text("무협 강호의 소림사 화산파 내공 진기 무림맹 " * 50, encoding="utf-8")
    h, t = extract_head_tail_words(f, 10, 10)
    assert isinstance(h, str)
    assert len(h) > 0
    assert t == ""


def test_calculate_genre_frequencies():
    res = calculate_genre_frequencies("화산귀환", "화산파 매화검법 내공", "소림사 무림맹")
    assert res["valid"] is True
    assert "3_무협" in res["scores"]
    assert res["scores"]["3_무협"] > 0


def test_main_cli_workflow(tmp_path: Path, monkeypatch, capsys):
    src_dir = tmp_path / "source"
    src_dir.mkdir()
    lib_root = tmp_path / "library"
    lib_root.mkdir()
    report_file = tmp_path / "report.json"

    # 1. 확실한 무협 파일 (>=75%, score >=4)
    f_wuxia = src_dir / "정통무협.txt"
    f_wuxia.write_text("소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 " * 20, encoding="utf-8")

    # 2. 하이브리드 무협-판타지 파일 (무협 >=2, 판타지 >=2)
    f_hybrid = src_dir / "무림속던전.txt"
    f_hybrid.write_text("화산파 내공 검법 던전 상태창 마나 스킬 몬스터 레이드 " * 20, encoding="utf-8")

    # 3. 준확실 로맨스 파일 (무협 vs 로맨스는 하이브리드 규칙에 없으므로 준확실_3_여성향으로 분류)
    # 로맨스 3개 + 무협 2개 -> 60.0% 로맨스 (75% 미만)
    f_semi = src_dir / "준확실로맨스.txt"
    f_semi.write_text("로맨스 남주 여주 화산파 소림사 " * 10, encoding="utf-8")

    # 4. 기타 미분류 파일
    f_other = src_dir / "기타문서.txt"
    f_other.write_text("가나다라마바사 아무런 키워드가 없는 일반적인 글입니다. " * 15, encoding="utf-8")

    # CLI 인자 설정 (리포트만 생성)
    test_args = [
        "cluster_hybrid_genres.py",
        "--source-dir", str(src_dir),
        "--library-root", str(lib_root),
        "--report", str(report_file),
    ]
    monkeypatch.setattr(sys, "argv", test_args)

    # 모든 클러스터 분기 및 1000번째 로그 출력을 위해 4종류 파일을 250회 반복 (총 1000개)
    dummy_files = [f_wuxia, f_hybrid, f_semi, f_other] * 250
    with patch("pathlib.Path.rglob", return_value=dummy_files):
        main()

    out, _ = capsys.readouterr()
    assert "클러스터링 분석 완료" in out
    assert "[1000/1000] 분석 진행 중..." in out
    assert report_file.exists()


def test_main_cli_auto_move_and_clean_existing(tmp_path: Path, monkeypatch, capsys):
    src_dir = tmp_path / "source"
    src_dir.mkdir()
    lib_root = tmp_path / "library"
    lib_root.mkdir()
    report_file = tmp_path / "report_move.json"

    # 대상 폴더에 파일 사전 생성 (중복 케이스 테스트)
    dest_cat = lib_root / "3_무협"
    dest_cat.mkdir(parents=True, exist_ok=True)
    existing_dest = dest_cat / "중복무협.txt"
    existing_dest.write_text("기존 파일", encoding="utf-8")

    # 1. 중복 파일 (기존 파일이 있어 clean_existing으로 삭제되어야 함)
    f_dup = src_dir / "중복무협.txt"
    f_dup.write_text("소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 " * 20, encoding="utf-8")

    # 2. 신규 파일 (이동되어야 함)
    f_new = src_dir / "신규무협.txt"
    f_new.write_text("소림사 무당파 화산파 장문인 내공 진기 운기조식 강호 무림맹 " * 20, encoding="utf-8")

    test_args = [
        "cluster_hybrid_genres.py",
        "--source-dir", str(src_dir),
        "--library-root", str(lib_root),
        "--auto-move-pure",
        "--clean-existing",
        "--limit", "2",
        "--report", str(report_file),
    ]
    monkeypatch.setattr(sys, "argv", test_args)

    main()

    out, _ = capsys.readouterr()
    assert "자동 이동 결과" in out
    assert not f_dup.exists()
    assert not f_new.exists()
    assert (dest_cat / "신규무협.txt").exists()


def test_run_as_main(monkeypatch, tmp_path: Path):
    src_dir = tmp_path / "source"
    src_dir.mkdir()
    f = src_dir / "test.txt"
    f.write_text("무협 소림사", encoding="utf-8")
    rep = tmp_path / "rep.json"
    monkeypatch.setattr(sys, "argv", ["cluster_hybrid_genres.py", "--source-dir", str(src_dir), "--report", str(rep), "--limit", "1"])
    runpy.run_module("utils.cluster_hybrid_genres", run_name="__main__")
    assert rep.exists()


def test_sys_path_insert(monkeypatch):
    from utils import cluster_hybrid_genres
    repo_str = str(cluster_hybrid_genres.REPO_ROOT)
    monkeypatch.setattr(sys, "path", [p for p in sys.path if p != repo_str])
    runpy.run_module("utils.cluster_hybrid_genres", run_name="test_mod")



def test_main_cli_move_hybrids_to(tmp_path: Path, monkeypatch, capsys):
    """--move-hybrids-to: 하이브리드 클러스터를 지정 카테고리로 모은다 (신규 이동 + 중복 정리)."""
    src_dir = tmp_path / "source"
    src_dir.mkdir()
    lib_root = tmp_path / "library"
    lib_root.mkdir()
    report_file = tmp_path / "report_hybrid.json"

    hybrid_text = "화산파 내공 검법 던전 상태창 마나 스킬 몬스터 레이드 " * 20

    # 1. 대상 폴더에 같은 이름이 이미 있는 경우 → clean_existing으로 원본 삭제
    dest_cat = lib_root / "3_판타지"
    dest_cat.mkdir(parents=True, exist_ok=True)
    (dest_cat / "중복하이브리드.txt").write_text("기존 파일", encoding="utf-8")
    f_dup = src_dir / "중복하이브리드.txt"
    f_dup.write_text(hybrid_text, encoding="utf-8")

    # 2. 대상 폴더에 없는 경우 → 이동
    f_new = src_dir / "신규하이브리드.txt"
    f_new.write_text(hybrid_text, encoding="utf-8")

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "cluster_hybrid_genres.py",
            "--source-dir", str(src_dir),
            "--library-root", str(lib_root),
            "--move-hybrids-to", "3_판타지",
            "--clean-existing",
            "--limit", "2",
            "--report", str(report_file),
        ],
    )

    main()

    report = json.loads(report_file.read_text(encoding="utf-8"))
    clusters = {r["cluster"] for r in report["results"]} if "results" in report else set()
    assert any(c.startswith("하이브리드_") for c in clusters) or "하이브리드" in capsys.readouterr().out

    assert not f_dup.exists(), "대상에 이미 있는 원본을 정리하지 않았다"
    assert not f_new.exists(), "신규 하이브리드를 이동하지 않았다"
    assert (dest_cat / "신규하이브리드.txt").exists()
