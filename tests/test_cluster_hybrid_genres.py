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


# ---------------------------------------------------------------------------
# 아래 테스트는 tests/test_book_classifier.py 에서 옮겨 왔다.
# 5대 장르 어휘 스코어링이 백엔드 판정 로직에서 빠져 이 모듈로 옮겨졌기 때문이다.
# ---------------------------------------------------------------------------

def test_extract_content_head_tail_words(tmp_path):
    from utils.cluster_hybrid_genres import extract_content_head_tail_words

    f = tmp_path / "hybrid_test.txt"
    head_content = "화산파 제자가 무공을 수련하며 강호를 방랑했다. " * 30 + "\n"
    mid_content = "긴 여행의 중간 내용... " * 100 + "\n"
    tail_content = "결국 그는 이계로 차원이동하여 드래곤과 마법사를 마주하고 마나를 각성했다. " * 30
    f.write_text(head_content + mid_content + tail_content, encoding="utf-8")

    combined = extract_content_head_tail_words(f, 50, 50)
    assert "화산파" in combined
    assert "드래곤" in combined or "마법사" in combined


def test_extract_head_tail_words_exceptions_and_epub(tmp_path):
    import zipfile
    from utils.cluster_hybrid_genres import extract_content_head_tail_words

    # EPUB 파일 생성 및 head/tail 파싱 (246-249, 254-257)
    epub_path = tmp_path / "sample.epub"
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr("mimetype", "application/epub+zip")
        z.writestr("text/ch1.xhtml", "<html><body><p>" + "화산파 매화검법 수련 시작 " * 20 + "</p></body></html>")
        z.writestr("text/ch2.xhtml", "<html><body><p>" + "단전의 내공을 운기조식하다 " * 20 + "</p></body></html>")
        z.writestr("text/ch3.xhtml", "<html><body><p>" + "드래곤과 마주쳐 마나를 각성 " * 20 + "</p></body></html>")
        z.writestr("text/ch4.xhtml", "<html><body><p>" + "이세계 판타지 모험의 끝 " * 20 + "</p></body></html>")

    res = extract_content_head_tail_words(epub_path, 50, 50)
    assert "화산파" in res
    assert "이세계" in res

    # 손상된 EPUB 파일 예외 (259-260)
    bad_epub = tmp_path / "bad.epub"
    bad_epub.write_bytes(b"not a zip file")
    assert extract_content_head_tail_words(bad_epub) == ""

    # 빈 TXT 파일
    empty_txt = tmp_path / "empty.txt"
    empty_txt.write_bytes(b"")
    assert extract_content_head_tail_words(empty_txt) == ""


def test_calculate_5_genre_scores_and_ratios_all_hybrid_clusters():
    from utils.cluster_hybrid_genres import calculate_5_genre_scores_and_ratios

    # 378-379: 단일 압도적 무협인데 판타지 >= 2 -> 하이브리드_무협_판타지
    text_w_f = "소림사 화산파 무당파 내공 진기 마교 천마 " * 10 + " 던전 마나"
    res1 = calculate_5_genre_scores_and_ratios("무림속", text_w_f)
    assert res1["cluster"] == "하이브리드_무협_판타지"
    assert res1["target_cat"] == "3_판타지"

    # 391: 판타지 + 여성향
    text_f_r = "던전 몬스터 헌터 각성 " * 5 + " 영애 황태자 남주 여주 " * 5
    res2 = calculate_5_genre_scores_and_ratios("판타지로맨스", text_f_r)
    assert res2["cluster"] == "하이브리드_판타지_로판"

    # 393: 판타지 + 성인
    text_f_a = "던전 몬스터 헌터 각성 " * 5 + " 섹스 자위 정액 클리토리스 음란 " * 5
    res3 = calculate_5_genre_scores_and_ratios("판타지성인", text_f_a)
    assert res3["cluster"] == "하이브리드_판타지_성인"

    # 395: 로맨스 + 성인
    text_r_a = "로맨스 남주 여주 황태자 " * 5 + " 섹스 자위 정액 클리토리스 음란 " * 5
    res4 = calculate_5_genre_scores_and_ratios("로맨스성인", text_r_a)
    assert res4["cluster"] == "하이브리드_로맨스_성인"

    # 397: 무협 + 성인
    text_w_a = "소림사 화산파 내공 진기 " * 5 + " 섹스 자위 정액 클리토리스 음란 " * 5
    res5 = calculate_5_genre_scores_and_ratios("무협성인", text_w_a)
    assert res5["cluster"] == "하이브리드_무협_성인"

    # 399: BL + 성인
    text_b_a = "미인공 미남공 다정공 미인수 단정수 " * 5 + " 섹스 자위 정액 클리토리스 음란 " * 5
    res6 = calculate_5_genre_scores_and_ratios("BL성인", text_b_a)
    assert res6["cluster"] == "하이브리드_BL_성인"

    # 401: BL + 판타지
    text_b_f = "미인공 미남공 다정공 미인수 단정수 " * 5 + " 던전 몬스터 헌터 각성 " * 5
    res7 = calculate_5_genre_scores_and_ratios("BL판타지", text_b_f)
    assert res7["cluster"] == "하이브리드_BL_판타지"

    # 405-413: 60% 준확실 (로맨스 3개 + 무협 2개)
    text_semi = "로맨스 남주 여주 화산파 소림사 " * 10
    res8 = calculate_5_genre_scores_and_ratios("준확실", text_semi)
    assert res8["cluster"] == "준확실_3_여성향"

    # 한글 부족 (335)
    res_en = calculate_5_genre_scores_and_ratios("English Book", "This is an english book without any hangul text.")
    assert res_en["cluster"] == "미분류_비문학영문"

    # 점수 0 (363)
    res_zero = calculate_5_genre_scores_and_ratios("일반소설", "아침에 일어나서 밥을 먹고 학교에 갔다. 평범한 하루였다.")
    assert res_zero["cluster"] == "미분류_키워드부족"


def test_inspect_txt_encoding_and_read_exceptions(tmp_path):
    from unittest.mock import patch
    from backend.book_classifier import inspect_txt_content
    from utils.cluster_hybrid_genres import extract_content_head_tail_words

    # 106-107: 한글 없는 영문 파일 -> cp949로 재시도하여 lines 읽기 (107 커버)
    eng_f = tmp_path / "english.txt"
    eng_f.write_text("Hello World without hangul keywords for test", encoding="utf-8")
    inspect_txt_content(eng_f)

    # 108-109: utf-8 성공 후 cp949 재시도 중 open 예외 -> pass
    orig_open = open
    calls = 0
    def mock_fail_second(*args, **kwargs):
        nonlocal calls
        calls += 1
        if calls == 2:
            raise OSError("fail")
        return orig_open(*args, **kwargs)
    with patch("builtins.open", mock_fail_second):
        inspect_txt_content(eng_f)

    # 212-213 & 231-234: read 중 예외 발생 시 continue
    sample_tail = tmp_path / "tail_err.txt"
    sample_tail.write_text("가나다라마바사 " * 50, encoding="utf-8")
    class MockFile:
        def __enter__(self):
            return self
        def __exit__(self, *args):
            pass
        def seek(self, *args):
            pass
        def read(self, *args):
            raise OSError("Read failed")
        def readline(self):
            raise OSError("Readline failed")

    with patch("builtins.open", return_value=MockFile()):
        extract_content_head_tail_words(sample_tail)


def test_semi_cluster_wuxia_fantasy_hybrid():
    from utils.cluster_hybrid_genres import calculate_5_genre_scores_and_ratios

    # 408-409: cat == "3_무협" r >= 60.0이고 scores["3_판타지"] >= 2 -> 하이브리드_무협_판타지
    text = "화산파 소림사 무당파 내공 진기 마교 천마 " * 5 + "던전 마나"
    res = calculate_5_genre_scores_and_ratios("무림판타지", text)
    assert res["cluster"] == "하이브리드_무협_판타지"
    assert res["target_cat"] == "3_판타지"
