#!/usr/bin/env python3
import pytest
import zipfile
from pathlib import Path
from backend.book_classifier import (
    extract_explicit_genre,
    inspect_epub_metadata,
    inspect_txt_content,
    clean_filename_to_author_title,
    get_effective_filename,
    title_similarity,
    is_single_match_valid,
    score_text_genre,
    resolve_genre_conflict,
    map_category,
    clean_empty_parent_dirs,
    evaluate_category_decision,
    BookClassifierService,
)

def test_extract_explicit_genre():
    assert extract_explicit_genre("[로판] 악녀는 오늘도") == "3_여성향"
    assert extract_explicit_genre("(무협) 광마회귀 1-100") == "3_무협"
    assert extract_explicit_genre("[판타지] 달빛조각사 01권") == "3_판타지"
    assert extract_explicit_genre("[BL] 사랑의 형태") == "9_BLGL"
    assert extract_explicit_genre("[SF] 은하영웅전설") == "3_SF"
    assert extract_explicit_genre("[라노벨] 소드아트온라인") == "3_라이트노벨"
    assert extract_explicit_genre("일반 소설 제목.txt") is None

def test_clean_filename_to_author_title():
    author, title, st = clean_filename_to_author_title("[남희성] 달빛조각사 1-50권 完.txt")
    assert author == "남희성"
    assert "달빛조각사" in title
    assert st == "달빛조각사"
    author2, title2, st2 = clean_filename_to_author_title("화산귀환 001-500화 (비가).txt")
    assert author2 == "비가"
    assert "화산귀환" in st2

def test_get_effective_filename(tmp_path):
    root = tmp_path / "library"
    series_dir = root / "[전민희] 룬의 아이들 윈터러"
    series_dir.mkdir(parents=True)
    vol_file = series_dir / "01권.txt"
    vol_file.write_text("content")
    eff = get_effective_filename(vol_file, root)
    assert "[전민희] 룬의 아이들 윈터러" in eff or "01권.txt" in eff

def test_get_effective_filename_keeps_direct_child_and_inherits_parent_author(tmp_path):
    root = tmp_path / "library"
    root.mkdir()
    direct_file = root / "제목.txt"
    direct_file.write_text("content", encoding="utf-8")
    series_dir = root / "[작가] 시리즈"
    series_dir.mkdir()
    chapter_file = series_dir / "긴 제목.txt"
    chapter_file.write_text("content", encoding="utf-8")

    assert get_effective_filename(direct_file, root) == "제목.txt"
    assert get_effective_filename(chapter_file, root) == "[작가] 긴 제목.txt"

def test_inspect_epub_metadata_extracts_opf_fields(tmp_path):
    epub_path = tmp_path / "metadata.epub"
    opf = """<?xml version="1.0" encoding="utf-8"?>
    <package xmlns:dc="http://purl.org/dc/elements/1.1/">
      <metadata>
        <dc:title>룬의 아이들</dc:title>
        <dc:creator>전민희</dc:creator>
        <dc:subject>판타지</dc:subject>
        <dc:subject>한국소설</dc:subject>
        <dc:description>마법과 모험 이야기</dc:description>
      </metadata>
    </package>
    """
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr("OPS/content.opf", opf)

    meta = inspect_epub_metadata(epub_path)

    assert meta == {
        "title": "룬의 아이들",
        "author": "전민희",
        "subject": "판타지 > 한국소설",
        "description": "마법과 모험 이야기",
    }

def test_inspect_epub_metadata_returns_empty_metadata_without_opf_or_on_error(tmp_path):
    no_opf = tmp_path / "no-opf.epub"
    with zipfile.ZipFile(no_opf, "w") as z:
        z.writestr("mimetype", "application/epub+zip")
    broken = tmp_path / "broken.epub"
    broken.write_text("not a zip", encoding="utf-8")

    empty_meta = {"title": "", "author": "", "subject": "", "description": ""}
    assert inspect_epub_metadata(no_opf) == empty_meta
    assert inspect_epub_metadata(broken) == empty_meta

def test_inspect_txt_content_extracts_snippet_and_hashtags(tmp_path):
    txt_path = tmp_path / "sample.txt"
    txt_path.write_text("#무협 #강호\n" + "문파와 마교의 대결\n" * 45, encoding="utf-8")

    info = inspect_txt_content(txt_path)

    assert info["hashtags"] == ["무협", "강호"]
    assert "문파와 마교" in info["snippet"]
    assert len(info["snippet"]) <= 2000

def test_inspect_txt_content_returns_default_for_unreadable_file(tmp_path):
    assert inspect_txt_content(tmp_path / "missing.txt") == {
        "hashtags": [],
        "snippet": "",
    }

def test_title_similarity_and_single_match():
    sim = title_similarity("달빛조각사", "달빛 조각사 1권")
    assert sim > 0.4
    assert is_single_match_valid("달빛조각사", "달빛조각사 1") is True
    assert is_single_match_valid("전혀다른책", "달빛조각사") is False

def test_title_similarity_and_single_match_edge_cases():
    assert title_similarity("", "달빛조각사") == 0.0
    assert title_similarity("a", "b") == 0.0
    assert is_single_match_valid("", "달빛조각사") is False
    assert is_single_match_valid("알파 베타", "알파 감마 델타 엡실론 제타 에타 세타 베타") is True
    assert is_single_match_valid("무협", "현대 무협 소설") is True

def test_score_text_genre():
    txt_wuxia = "강호의 무림맹과 마교의 대결, 단전의 내공을 끌어올려 검법을 펼쳤다."
    assert score_text_genre(txt_wuxia) == "3_무협"
    txt_fantasy = "마법사가 마나를 모아 스킬을 발동하고 던전의 몬스터를 사냥했다. 상태창을 열었다."
    assert score_text_genre(txt_fantasy) == "3_판타지"
    txt_romance = "공작가의 황태자가 여주에게 다가와 파티 드레스를 칭찬했다. 로판 악역 영애의 삶."
    assert score_text_genre(txt_romance) == "3_여성향"

def test_score_text_genre_returns_none_without_enough_evidence():
    assert score_text_genre("") is None
    assert score_text_genre("조용한 오후에 책을 읽었다.") is None

def test_resolve_genre_conflict():
    cats = ["3_무협", "3_판타지"]
    res = resolve_genre_conflict(cats, "[무협] 절대검제.txt")
    assert res == "3_무협"

@pytest.mark.parametrize(
    "cats,fname,text_sample,expected",
    [
        (["3_무협", "3_판타지"], "던전 헌터.txt", "마나와 스킬로 몬스터를 사냥", "3_판타지"),
        (["3_그래픽노블", "3_라이트노벨"], "이세계.txt", "", "3_라이트노벨"),
        (["3_그래픽노블", "3_라이트노벨"], "이세계.cbz", "", "3_그래픽노블"),
        (["3_그래픽노블", "3_판타지"], "마왕.epub", "", "3_판타지"),
        (["3_그래픽노블", "3_판타지"], "마왕.cbz", "", "3_그래픽노블"),
        (["3_판타지", "3_여성향"], "악녀.txt", "공작가 영애와 황태자", "3_여성향"),
        (["3_판타지", "3_여성향"], "레이드.txt", "던전과 몬스터", "3_판타지"),
        (["3_그래픽노블", "3_여성향"], "로판.epub", "", "3_여성향"),
        (["3_그래픽노블", "3_여성향"], "로판.cbz", "", "3_그래픽노블"),
        (["3_라이트노벨", "3_여성향"], "악역영애.txt", "로판 황태자 영애", "3_여성향"),
        (["3_라이트노벨", "3_여성향"], "동아리.txt", "평범한 학교생활", "3_라이트노벨"),
        (["3_라이트노벨", "3_판타지"], "슬라임.txt", "마왕과 용사와 이세계 전생", "3_라이트노벨"),
        (["3_라이트노벨", "3_판타지"], "헌터.txt", "레이드와 몬스터", "3_판타지"),
        (["2_소설한국", "2_수필서간일기"], "산문집.txt", "일기와 에세이", "2_수필서간일기"),
        (["2_소설한국", "2_수필서간일기"], "단편집.txt", "문학 소설", "2_소설한국"),
        (["2_소설역사", "4_역사인물"], "대하소설.txt", "대하 소설 열전", "2_소설역사"),
        (["2_소설한국", "4_역사인물"], "단편문학.txt", "한국 단편 문학", "2_소설한국"),
        (["6_처세술리더십창의성", "4_경영마케팅"], "성공습관.txt", "인간관계와 시간관리", "6_처세술리더십창의성"),
        (["3_SF", "3_스릴러"], "미분류.txt", "", None),
    ],
)
def test_resolve_genre_conflict_branches(cats, fname, text_sample, expected):
    assert resolve_genre_conflict(cats, fname, text_sample) == expected

@pytest.mark.parametrize(
    "cat_str,raw_title,raw_author,expected",
    [
        ("", "", "", None),
        ("", "평범한 제목", "히가시노 게이고", "2_소설일본게이고"),
        ("", "평범한 제목", "무라카미 하루키", "2_소설일본하루키"),
        ("", "을유세계문학 고전", "", "2_을유세계문학전집"),
        ("", "열린책들 세계문학", "", "2_열린책들세계문학"),
        ("", "문예세계문학선", "", "2_문예세계문학선"),
        ("", "살림지식총서 001", "", "4_살림지식총서"),
        ("", "시공디스커버리 총서", "", "4_시공디스커버리"),
        ("", "올재 클래식스", "", "1_올재"),
        ("", "누워서 읽는 법학", "", "4_누워서읽는법학"),
        ("", "이지사이언스", "", "5_이지사이언스"),
        ("", "그림으로 읽는 과학", "", "5_그림으로_읽는"),
        ("", "손자병법", "", "1_동양고전"),
        ("라이트노벨", "", "", "3_라이트노벨"),
        ("BL/GL", "", "", "9_BLGL"),
        ("그래픽노블", "", "", "3_그래픽노블"),
        ("미스터리", "", "", "3_스릴러"),
        ("과학소설 > SF", "", "", "3_SF"),
        ("판타지소설", "", "", "3_판타지"),
        ("무협소설", "", "", "3_무협"),
        ("로맨스판타지", "", "", "3_여성향"),
        ("에세이", "", "", "2_수필서간일기"),
        ("한국소설", "", "", "2_소설한국"),
        ("일본소설", "", "", "2_소설일본"),
        ("중국소설", "", "", "2_소설중국"),
        ("영미소설", "", "", "2_소설외국"),
        ("역사소설", "", "", "2_소설역사"),
        ("시집", "", "", "2_시"),
        ("글쓰기", "", "", "2_문학일반서평작법독서"),
        ("문헌정보", "", "", "1_문헌서지"),
        ("동양고전", "", "", "1_동양고전"),
        ("서양고전", "", "", "1_서양고전"),
        ("투자", "", "", "6_재테크"),
        ("국내도서 > 경제일반", "", "", "4_경제"),
        ("마케팅", "", "", "4_경영마케팅"),
        ("국내도서 > 경영", "", "", "4_경영마케팅"),
        ("자기계발", "", "", "6_처세술리더십창의성"),
        ("심리학", "", "", "4_심리학뇌과학"),
        ("철학", "", "", "4_철학윤리"),
        ("한국사", "", "", "4_역사인물"),
        ("정치학", "", "", "4_정치외교군사"),
        ("법학", "", "", "4_법"),
        ("사회과학", "", "", "4_사회인류"),
        ("종교", "", "", "4_종교신화"),
        ("인문학", "", "", "4_인문일반논픽션"),
        ("교양과학", "", "", "5_수학과학일반"),
        ("미술", "", "", "5_미술예술건축"),
        ("음악", "", "", "5_음악"),
        ("영화", "", "", "5_영화"),
        ("사진", "", "", "5_사진영상"),
        ("스포츠", "", "", "5_스포츠"),
        ("서브컬쳐", "", "", "5_서브컬쳐"),
        ("교육학", "", "", "7_교육일반"),
        ("국어", "", "", "7_국어교육"),
        ("영어회화", "", "", "7_영어교육"),
        ("토익", "", "", "7_영어교육"),
        ("일본어", "", "", "7_일어교육"),
        ("중국어", "", "", "7_중어한자교육"),
        ("HSK", "", "", "7_중어한자교육"),
        ("프랑스어", "", "", "7_외국어교육"),
        ("언어학", "", "", "7_언어일반"),
        ("영어원서", "", "", "7_영문일반"),
        ("영문원서", "", "", "7_영문일반"),
        ("프로그래밍", "", "", "8_IT"),
        ("건강", "", "", "8_건강일반"),
        ("요리", "", "", "8_요리음료"),
        ("여행", "", "", "8_여행"),
        ("공인중개사", "", "", "8_공인중개사"),
        ("프라모델", "", "", "8_모델링"),
        ("밀리터리", "", "", "8_밀리터리"),
        ("성교육", "", "", "8_성"),
        ("성의학", "", "", "8_성"),
        ("살림", "", "", "8_실용의학회계"),
        ("악보", "", "", "8_악보"),
        ("어린이", "", "", "9_어린이육아"),
        ("청소년문학", "", "", "9_청소년"),
        ("사주", "", "", "9_역학해몽퍼즐"),
        ("유머", "", "", "9_유머"),
        ("명언", "", "", "9_격언명언"),
    ],
)
def test_map_category_rules(cat_str, raw_title, raw_author, expected):
    assert map_category(cat_str, raw_title, raw_author) == expected

def test_clean_empty_parent_dirs_removes_empty_dirs_until_stop_dir(tmp_path):
    stop_dir = tmp_path / "library"
    empty_child = stop_dir / "series" / "volume"
    empty_child.mkdir(parents=True)

    clean_empty_parent_dirs(empty_child, stop_dir)

    assert stop_dir.exists()
    assert not (stop_dir / "series").exists()

def test_clean_empty_parent_dirs_stops_when_directory_has_visible_items(tmp_path):
    stop_dir = tmp_path / "library"
    child = stop_dir / "series"
    child.mkdir(parents=True)
    (child / "keep.txt").write_text("content", encoding="utf-8")

    clean_empty_parent_dirs(child, stop_dir)

    assert child.exists()

def test_evaluate_category_decision_majority():
    y = {"mapped": "3_판타지", "title": "테스트"}
    a = {"mapped": "3_판타지", "title": "테스트"}
    k = {"mapped": "3_무협", "title": "테스트"}
    cat, method, reason = evaluate_category_decision(
        "테스트도서.txt", None, "테스트도서", "저자", "테스트도서", y, a, k
    )
    assert cat == "3_판타지"
    assert method == "majority"

def test_evaluate_category_decision_content_metadata(tmp_path):
    txt_file = tmp_path / "정체불명소설.txt"
    content_text = "#무협 " + ("소림사의 장로와 화산파 문도들이 강호의 평화를 위해 내공을 수련했다. 단전과 기경팔맥. " * 5)
    txt_file.write_text(content_text, encoding="utf-8")
    y = {"mapped": None}
    a = {"mapped": None}
    k = {"mapped": None}
    cat, method, reason = evaluate_category_decision(
        "정체불명소설.txt", txt_file, "정체불명소설", "", "정체불명소설", y, a, k
    )
    assert cat == "3_무협"
    assert method == "content_metadata"

def test_evaluate_category_decision_resolves_store_conflict_with_txt_content(tmp_path):
    txt_file = tmp_path / "헌터.txt"
    txt_file.write_text("던전 마나 스킬 몬스터 레이드 상태창", encoding="utf-8")

    cat, method, reason = evaluate_category_decision(
        "헌터.txt",
        txt_file,
        "헌터",
        "",
        "헌터",
        {"mapped": "3_무협"},
        {"mapped": "3_판타지"},
        {"mapped": None},
    )

    assert cat == "3_판타지"
    assert method == "conflict_resolved"
    assert "Conflict resolved" in reason

def test_evaluate_category_decision_trusts_valid_single_match():
    cat, method, reason = evaluate_category_decision(
        "달빛조각사.txt",
        None,
        "달빛조각사",
        "",
        "달빛조각사",
        {"mapped": "3_판타지", "title": "달빛조각사 1권"},
        {"mapped": None},
        {"mapped": None},
    )

    assert cat == "3_판타지"
    assert method == "single_match"
    assert "Single match trusted" in reason

def test_evaluate_category_decision_uses_epub_subject_metadata(tmp_path):
    epub_path = tmp_path / "subject.epub"
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr(
            "OPS/content.opf",
            """
            <package xmlns:dc="http://purl.org/dc/elements/1.1/">
              <metadata>
                <dc:title>마법 학교</dc:title>
                <dc:subject>판타지소설</dc:subject>
              </metadata>
            </package>
            """,
        )

    cat, method, reason = evaluate_category_decision(
        "마법학교.epub",
        epub_path,
        "마법학교",
        "",
        "마법학교",
        {"mapped": None},
        {"mapped": None},
        {"mapped": None},
    )

    assert cat == "3_판타지"
    assert method == "content_metadata"
    assert "EPUB dc:subject" in reason

def test_evaluate_category_decision_scores_epub_text_when_metadata_is_not_mapped(tmp_path):
    epub_path = tmp_path / "score.epub"
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr(
            "OPS/content.opf",
            """
            <package xmlns:dc="http://purl.org/dc/elements/1.1/">
              <metadata>
                <dc:title>강호의 검</dc:title>
                <dc:description>문파 마교 무림 내공 검법 강호</dc:description>
              </metadata>
            </package>
            """,
        )

    cat, method, reason = evaluate_category_decision(
        "강호의검.epub",
        epub_path,
        "강호의검",
        "",
        "강호의검",
        {"mapped": None},
        {"mapped": None},
        {"mapped": None},
    )

    assert cat == "3_무협"
    assert method == "content_metadata"
    assert "EPUB content scored" in reason

@pytest.mark.parametrize(
    "yes24,aladin,kyobo,expected_method",
    [
        ({"mapped": "3_SF"}, {"mapped": "3_스릴러"}, {"mapped": None}, "conflict"),
        ({"mapped": "3_판타지", "title": "전혀 다른 책"}, {"mapped": None}, {"mapped": None}, "single_match"),
        ({"mapped": None}, {"mapped": None}, {"mapped": None}, "not_found"),
    ],
)
def test_evaluate_category_decision_unresolved_outcomes(yes24, aladin, kyobo, expected_method):
    cat, method, _reason = evaluate_category_decision(
        "미분류.txt",
        None,
        "미분류",
        "",
        "미분류",
        yes24,
        aladin,
        kyobo,
    )

    assert cat is None
    assert method == expected_method

def test_book_classifier_service_process_file(tmp_path):
    lib_root = tmp_path / "text"
    src_dir = lib_root / "0_telegram"
    src_dir.mkdir(parents=True)
    fpath = src_dir / "[판타지] 나 혼자 만렙 귀환자 01.txt"
    fpath.write_text("스킬과 던전 마나 시스템 몬스터", encoding="utf-8")
    service = BookClassifierService(
        library_root=lib_root,
        cache_file=tmp_path / "cache.json",
        verbose=False
    )
    res_dry = service.process_file(fpath, src_dir, auto_move=True, dry_run=True, use_bookstore=False)
    assert res_dry["target_category"] == "3_판타지"
    assert res_dry["action"] == "classified"
    assert fpath.exists()
    res_move = service.process_file(fpath, src_dir, auto_move=True, dry_run=False, use_bookstore=False)
    assert res_move["target_category"] == "3_판타지"
    assert res_move["action"] == "moved"
    assert not fpath.exists()
    assert (lib_root / "3_판타지" / fpath.name).exists()
