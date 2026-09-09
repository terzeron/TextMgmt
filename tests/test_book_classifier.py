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


def test_evaluate_category_decision_resolves_false_conflict_by_title_similarity():
    # yes24는 무관한 추천 도서(유사도 낮음), kyobo는 정확한 제목 일치
    yes24 = {"title": "세네카 오늘을 빼앗기고 있는 당신에게", "mapped": "4_철학윤리"}
    kyobo = {"title": "공포의 산장", "mapped": "2_소설외국"}
    aladin = {"title": "", "mapped": None}

    cat, method, reason = evaluate_category_decision(
        "공포의 산장.epub",
        None,
        "공포의 산장",
        "",
        "공포의 산장",
        yes24,
        aladin,
        kyobo,
    )
    assert cat == "2_소설외국"
    assert method == "single_valid_match_resolved"
    assert "False conflict resolved" in reason


def test_resolve_genre_conflict_new_rules():
    # 1. 소설외국 vs 스릴러 (스릴러 키워드 포함 시 3_스릴러, 없을 시 2_소설외국)
    assert resolve_genre_conflict(["2_소설외국", "3_스릴러"], "셜록홈즈의 모험.txt", "") == "3_스릴러"
    assert resolve_genre_conflict(["2_소설외국", "3_스릴러"], "어린왕자.txt", "") == "2_소설외국"

    # 2. 여성향 vs BLGL
    assert resolve_genre_conflict(["3_여성향", "9_BLGL"], "[BL] 패션.txt", "") == "9_BLGL"
    assert resolve_genre_conflict(["3_여성향", "9_BLGL"], "황태자의 약혼녀.txt", "") == "3_여성향"


def test_inspect_txt_content_cp949_and_header_genre(tmp_path):
    f = tmp_path / "test_cp949.txt"
    content = "=====================\n[판타지] 룬의 아이들 1권\n=====================\n프롤로그..."
    f.write_bytes(content.encode("cp949"))

    res = inspect_txt_content(f)
    assert res.get("header_genre") == "판타지"
    assert "룬의 아이들" in res.get("snippet", "")


def test_classify_5_genres_from_content_all_categories():
    from backend.book_classifier import classify_5_genres_from_content

    # 1. 3_무협
    wuxia_text = "화산파의 장문인은 단전의 진기를 운기조식하여 매화검법의 절기를 펼쳤다. 마교와 천마의 위협에 강호 무림이 진동했다."
    cat, score, _ = classify_5_genres_from_content("무림기", wuxia_text)
    assert cat == "3_무협"

    # 2. 3_판타지
    fantasy_text = "각성한 S급 헌터는 던전 게이트 안에서 보스 몬스터 드래곤을 마주했다. 상태창에 새로운 스킬과 마나가 생성되었다."
    cat, score, _ = classify_5_genres_from_content("나혼자만렙", fantasy_text)
    assert cat == "3_판타지"

    # 3. 3_여성향
    rofan_text = "공작가의 시한부 악녀로 빙의한 영애는 냉혈한 황태자와의 파혼을 결심했다. 무도회에서 남주인공의 눈빛이 마주쳤다."
    cat, score, _ = classify_5_genres_from_content("악녀의파혼", rofan_text)
    assert cat == "3_여성향"

    # 4. 9_BLGL
    bl_text = "우성 알파인 다정공과 오메가버스 세계관의 단정수가 페로몬에 반응하여 각인되었다. 에스퍼와 가이드의 파장이 일치했다."
    cat, score, _ = classify_5_genres_from_content("패션", bl_text)
    assert cat == "9_BLGL"

    # 5. 9_성인
    adult_text = "그녀는 뜨거운 애액을 흘리며 교성을 내질렀고 그의 단단한 자지가 질내로 깊숙이 삽입되어 정액을 사정했다. 유두가 바짝 서 올랐다."
    cat, score, _ = classify_5_genres_from_content("야설모음", adult_text)
    assert cat == "9_성인"

    # 6. 일반 비문학/영문 도서는 None
    non_fiction = "The quick brown fox jumps over the lazy dog. General cooking recipes for breakfast and dinner."
    cat, score, _ = classify_5_genres_from_content("Cookery", non_fiction)
    assert cat is None


def test_classify_5_genres_wuxia_fantasy_hybrid_routes_to_fantasy():
    from backend.book_classifier import classify_5_genres_from_content

    # 무협 키워드(화산파, 단전, 내공, 검법)가 다수 있어도 판타지 키워드(던전, 상태창, 스킬, 마나)가 2건 이상 포함되면 3_판타지로 분류
    hybrid_text = (
        "화산파의 후기지수는 단전의 내공을 끌어올려 검법을 펼쳤으나, "
        "갑자기 눈앞에 푸른 상태창과 퀘스트 창이 떠올랐다. "
        "던전 안의 몬스터를 처치하여 레벨업하고 마나를 획득하라는 시스템 메시지였다."
    )
    cat, score, reason = classify_5_genres_from_content("무한레벨업무림", hybrid_text)
    assert cat == "3_판타지"
    assert "hybrid_wuxia_fantasy" in reason


def test_classify_5_genres_pure_wuxia_routes_to_wuxia():
    from backend.book_classifier import classify_5_genres_from_content

    # 판타지 키워드가 없는 순수 무협은 3_무협 유지
    pure_wuxia_text = (
        "소림사의 방주와 무당파의 장문인은 마교의 천마가 강호 무림을 침략한다는 소식을 듣고 "
        "정파와 백도의 무림맹을 소집하여 화산파의 매화검법과 소림의 절기를 결집하기로 했다."
    )
    cat, score, reason = classify_5_genres_from_content("정통무협지", pure_wuxia_text)
    assert cat == "3_무협"
    assert "pure_wuxia" in reason


def test_resolve_genre_conflict_hybrid_prefers_fantasy():
    from backend.book_classifier import resolve_genre_conflict

    # 서점 간 무협 vs 판타지 충돌 시 판타지 어휘가 있으면 3_판타지
    assert resolve_genre_conflict(["3_무협", "3_판타지"], "작품.txt", "마나가 담긴 화산파 검법") == "3_판타지"
    # 판타지 어휘 없이 순수 무협 어휘만 있으면 3_무협
    assert resolve_genre_conflict(["3_무협", "3_판타지"], "작품.txt", "화산파의 매화검법과 내공") == "3_무협"


def test_extract_content_head_tail_words(tmp_path):
    from backend.book_classifier import extract_content_head_tail_words

    f = tmp_path / "hybrid_test.txt"
    head_content = "화산파 제자가 무공을 수련하며 강호를 방랑했다. " * 30 + "\n"
    mid_content = "긴 여행의 중간 내용... " * 100 + "\n"
    tail_content = "결국 그는 이계로 차원이동하여 드래곤과 마법사를 마주하고 마나를 각성했다. " * 30
    f.write_text(head_content + mid_content + tail_content, encoding="utf-8")

    combined = extract_content_head_tail_words(f, 50, 50)
    assert "화산파" in combined
    assert "드래곤" in combined or "마법사" in combined




def test_title_similarity_and_reliable_edge_cases():
    from backend.book_classifier import is_title_match_reliable, title_similarity

    # st_words 빈 문자열 (172)
    assert is_title_match_reliable("!@#$%", "제목") is False
    # OST 키워드 필터링 (176)
    assert is_title_match_reliable("도깨비", "도깨비 OST (CD)") is False
    # 완전 포함 len >= 4 (184)
    assert is_title_match_reliable("달빛조각사", "달빛조각사 완결판") is True
    # ratio >= 0.7 & sim >= 0.5 (191)
    assert is_title_match_reliable("해리포터와 마법사의 돌", "해리포터와 마법사의 돌 특별판") is True
    # 단어 1개 일치 및 sim >= 0.6 (193)
    assert is_title_match_reliable("화산귀환", "화산귀환 1") is True
    # t1 or t2 비어있을 때 (131)
    assert title_similarity("", "abc") == 0.0
    assert title_similarity("a", "b") == 0.0


def test_extract_head_tail_words_exceptions_and_epub(tmp_path):
    import zipfile
    from backend.book_classifier import extract_content_head_tail_words

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
    from backend.book_classifier import calculate_5_genre_scores_and_ratios

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


def test_classify_5_genres_from_content_fallback_branches(monkeypatch):
    from backend import book_classifier
    from backend.book_classifier import classify_5_genres_from_content

    # target_cat 없는 경우 모의 테스트 (451-474)
    # 1. BL 폴백
    mock_res_bl = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 3, "3_여성향": 0, "3_무협": 0, "3_판타지": 0, "9_성인": 0}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_bl)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat == "9_BLGL"

    # 2. 로판 폴백
    mock_res_ro = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 0, "3_여성향": 4, "3_무협": 0, "3_판타지": 0, "9_성인": 0}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_ro)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat == "3_여성향"

    # 3. 무협 vs 판타지 하이브리드 폴백
    mock_res_wf = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 0, "3_여성향": 0, "3_무협": 4, "3_판타지": 3, "9_성인": 0}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_wf)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat == "3_판타지"
    assert "hybrid_wuxia_fantasy" in reason

    # 4. 순수 무협 폴백
    mock_res_w = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 0, "3_여성향": 0, "3_무협": 4, "3_판타지": 1, "9_성인": 0}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_w)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat == "3_무협"
    assert "pure_wuxia" in reason

    # 5. 판타지 폴백
    mock_res_f = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 0, "3_여성향": 0, "3_무협": 0, "3_판타지": 4, "9_성인": 0}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_f)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat == "3_판타지"

    # 6. 성인 폴백
    mock_res_a = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 0, "3_여성향": 0, "3_무협": 0, "3_판타지": 0, "9_성인": 6}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_a)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat == "9_성인"

    # 7. 매칭 없음
    mock_res_none = {
        "valid": True, "target_cat": None, "cluster": "기타_복합",
        "scores": {"9_BLGL": 0, "3_여성향": 0, "3_무협": 0, "3_판타지": 0, "9_성인": 0}
    }
    monkeypatch.setattr(book_classifier, "calculate_5_genre_scores_and_ratios", lambda t, x: mock_res_none)
    cat, score, reason = classify_5_genres_from_content("t", "x")
    assert cat is None
    assert reason == "no_genre_matched"


def test_score_text_genre_sf_and_thriller():
    from backend.book_classifier import score_text_genre

    assert score_text_genre("외계인과 안드로이드가 우주선에서 만났다.") == "3_SF"
    assert score_text_genre("형사가 연쇄살인 살인사건 현장을 수사했다.") == "3_스릴러"
    assert score_text_genre("") is None
    assert score_text_genre("아무런 키워드 없음") is None


def test_inspect_txt_encoding_and_read_exceptions(tmp_path):
    from unittest.mock import patch
    from backend.book_classifier import inspect_txt_content, extract_content_head_tail_words

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


def test_title_match_reliable_ratio():
    from backend.book_classifier import is_title_match_reliable

    # 189: search_title 단어의 70% 이상 포함 및 sim >= 0.5
    assert is_title_match_reliable("해리 포터와 비밀의 방", "해리 포터와 비밀의 방 (전2권)") is True
    # 191: ratio < 0.7로 마지막 return False 도달
    assert is_title_match_reliable("해리포터 마법사의 돌", "해리포터 비밀의 방") is False


def test_semi_cluster_wuxia_fantasy_hybrid():
    from backend.book_classifier import calculate_5_genre_scores_and_ratios

    # 408-409: cat == "3_무협" r >= 60.0이고 scores["3_판타지"] >= 2 -> 하이브리드_무협_판타지
    text = "화산파 소림사 무당파 내공 진기 마교 천마 " * 5 + "던전 마나"
    res = calculate_5_genre_scores_and_ratios("무림판타지", text)
    assert res["cluster"] == "하이브리드_무협_판타지"
    assert res["target_cat"] == "3_판타지"


def test_resolve_genre_conflict_novel_branches():
    from backend.book_classifier import resolve_genre_conflict

    # 595-598: 소설한국 vs 스릴러
    assert resolve_genre_conflict(["2_소설한국", "3_스릴러"], "제목", "살인사건과 형사") == "3_스릴러"
    assert resolve_genre_conflict(["2_소설한국", "3_스릴러"], "제목", "키워드 없음") == "2_소설한국"

    # 602-605: 소설외국 vs 판타지
    assert resolve_genre_conflict(["2_소설외국", "3_판타지"], "제목", "드래곤과 마법") == "3_판타지"
    assert resolve_genre_conflict(["2_소설외국", "3_판타지"], "제목", "키워드 없음") == "2_소설외국"

    # 609-612: 소설외국 vs 여성향
    assert resolve_genre_conflict(["2_소설외국", "3_여성향"], "제목", "로맨스와 사랑") == "3_여성향"
    assert resolve_genre_conflict(["2_소설외국", "3_여성향"], "제목", "키워드 없음") == "2_소설외국"


def test_evaluate_category_decision_metadata_branches(tmp_path):
    from unittest.mock import patch
    from backend.book_classifier import evaluate_category_decision

    # 681-684: valid_title_candidates >= 2 다수결 (제목 일치 검증 후 다수결)
    y = {"title": "정확한책", "mapped": "3_판타지"}
    a = {"title": "정확한책", "mapped": "3_판타지"}
    k = {"title": "다른책", "mapped": "3_무협"}
    cat, method, reason = evaluate_category_decision("정확한책", None, "정확한책", "저자", "정확한책", y, a, k)
    assert cat == "3_판타지"

    # 716: EPUB dc:description -> desc_cat
    epub_f = tmp_path / "desc.epub"
    epub_f.write_bytes(b"dummy")
    with patch("backend.book_classifier.inspect_epub_metadata", return_value={"title": "", "subject": "", "description": "재테크와 주식 투자 이야기"}):
        cat_d, m_d, r_d = evaluate_category_decision(
            "책.epub", epub_f, "책", "저자", "책",
            {}, {}, {}
        )
        assert cat_d == "6_재테크"
        assert "dc:description" in r_d

    # 721-723: TXT header [장르]
    txt_f = tmp_path / "header.txt"
    txt_f.write_text("본문", encoding="utf-8")
    with patch("backend.book_classifier.inspect_txt_content", return_value={"snippet": "", "header_genre": "판타지", "hashtags": []}):
        cat_h, m_h, r_h = evaluate_category_decision(
            "책.txt", txt_f, "책", "저자", "책",
            {}, {}, {}
        )
        assert cat_h == "3_판타지"

    # 738-740: 본문 스코어링 폴백 score_text_genre (SF 키워드)
    txt_sf = tmp_path / "sf.txt"
    txt_sf.write_text("우주선과 안드로이드의 행성 탐사", encoding="utf-8")
    cat_sf, m_sf, r_sf = evaluate_category_decision(
        "책.txt", txt_sf, "책", "저자", "책",
        {}, {}, {}
    )
    assert cat_sf == "3_SF"


def test_clean_filename_author_title_edge_patterns():
    from backend.book_classifier import clean_filename_to_author_title

    # 773-774: @author 패턴
    a, t, s = clean_filename_to_author_title("@홍길동 멋진 소설.txt")
    assert a == "홍길동"

    # 779-780: [저자] 뒤쪽 패턴
    a, t, s = clean_filename_to_author_title("멋진 소설 [설봉].txt")
    assert a == "설봉"

    # 791-794: -저자 뒤쪽 대시 패턴
    a, t, s = clean_filename_to_author_title("멋진 소설-이우혁.txt")
    assert a == "이우혁"

    # 805-806: 저자 - 제목 패턴 (끝에 권수 등이 붙어 대시 저자에 안 걸리는 경우)
    a, t, s = clean_filename_to_author_title("김용 - 사조영웅전 1부.txt")
    assert a == "김용"
    assert "사조영웅전" in t

    # 826: 구분자 ' - ' 분리 (저자가 앞에 있고 제목에 ' - '가 포함된 경우)
    a, t, s = clean_filename_to_author_title("[김용] 영웅문 - 사조영웅전 1부.txt")
    assert a == "김용"
    assert s == "영웅문"


def test_book_classifier_service_methods_and_cache(tmp_path):
    import json
    from unittest.mock import patch, MagicMock
    from backend.book_classifier import BookClassifierService, clean_empty_parent_dirs

    # 1070-1071: clean_empty_parent_dirs 예외 처리
    sub = tmp_path / "a" / "b"
    sub.mkdir(parents=True)
    with patch("pathlib.Path.rmdir", side_effect=OSError("Busy")):
        clean_empty_parent_dirs(sub, tmp_path)

    # 1098: cache_file default
    svc_default = BookClassifierService(library_root=tmp_path)
    assert svc_default.cache_file == tmp_path / "classification_cache.json"

    # 1110-1116: load_cache 손상 파일 예외 처리
    corrupted_cache = tmp_path / "corrupted_cache.json"
    corrupted_cache.write_text("{bad json", encoding="utf-8")
    svc_corrupt = BookClassifierService(library_root=tmp_path, cache_file=corrupted_cache)
    assert svc_corrupt.cache == {}

    # 1113: 유효한 캐시 파일 로드 로그
    valid_cache = tmp_path / "valid_cache.json"
    valid_cache.write_text(json.dumps({"test_item": {"search_title": "삼국지"}}), encoding="utf-8")
    svc_valid = BookClassifierService(library_root=tmp_path, cache_file=valid_cache)
    assert len(svc_valid.cache) == 1

    # 1121-1127: save_cache
    cache_path = tmp_path / "test_cache.json"
    svc = BookClassifierService(library_root=tmp_path, cache_file=cache_path, delay=0.0)
    svc.cache = {
        "existing_key": {
            "search_title": "삼국지",
            "author": "나관중",
            "title": "삼국지",
            "status": "moved",
            "yes24": {"cat": "역사소설", "mapped": "2_소설한국", "title": "삼국지"}
        }
    }
    svc.save_cache()
    assert cache_path.exists()

    # 1132-1135: build_title_index
    svc.build_title_index()
    assert "삼국지" in svc.title_cache

    # save_cache 예외 처리 (1127)
    with patch("pathlib.Path.replace", side_effect=OSError("Permission Denied")):
        svc.save_cache()

    # 1138-1162: query_bookstores & _do_query 예외
    with patch.object(svc.yes24, "search", return_value=([("삼국지", "나관중", "소설", "url", "", "")], "", "")), \
         patch.object(svc.aladin, "search", side_effect=Exception("Timeout")), \
         patch.object(svc.kyobo, "search", return_value=([], "", "")):
        y_res, a_res, k_res = svc.query_bookstores("삼국지", "나관중", "삼국지")
        assert y_res["title"] == "삼국지"
        assert a_res["cat"] == ""
        assert k_res["cat"] == ""

    # 1179-1180: rel_path ValueError (fpath가 source_dir 하위가 아닐 때)
    f_outside = tmp_path / "outside.txt"
    f_outside.write_text("내용", encoding="utf-8")
    other_src = Path("/some/nonexistent/source_dir")
    svc.classify_file(f_outside, other_src, use_bookstore=False)

    # 1186-1201: title_cache hit
    f_samguk = tmp_path / "나관중 - 삼국지 1권.txt"
    f_samguk.write_text("유비 관우 장비 도원결의", encoding="utf-8")
    cat, meth, reas, entry = svc.classify_file(f_samguk, tmp_path, use_bookstore=False)
    assert entry is not None

    # 1205: cache_only
    f_uncached = tmp_path / "미캐시책.txt"
    f_uncached.write_text("내용", encoding="utf-8")
    cat_co, m_co, r_co, _ = svc.classify_file(f_uncached, tmp_path, cache_only=True)
    assert m_co == "cache_only_skip"

    # 1209: use_bookstore=True (query_bookstores 호출)
    with patch.object(svc, "query_bookstores", return_value=({}, {}, {})):
        svc.classify_file(f_uncached, tmp_path, use_bookstore=True)

    # 1271: process_file - not target_cat
    with patch.object(svc, "classify_file", return_value=(None, "not_found", "reason", {})):
        res_none = svc.process_file(f_uncached, tmp_path)
        assert res_none["action"] == "none"

    # 1277-1285 & 1282: dest_path.exists() 분기 (entry 포함)
    dest_dir = tmp_path / "2_소설한국"
    dest_dir.mkdir(parents=True, exist_ok=True)
    existing_in_dest = dest_dir / f_samguk.name
    existing_in_dest.write_text("이미 있음", encoding="utf-8")

    # clean_existing = False -> skipped_already_exists (1285)
    with patch.object(svc, "classify_file", return_value=("2_소설한국", "matched", "reason", {})):
        res_skip = svc.process_file(f_samguk, tmp_path, clean_existing=False)
        assert res_skip["action"] == "skipped_already_exists"

    # clean_existing = True -> cleaned_duplicate (1282, 1283)
    entry_dict = {"status": "pending"}
    with patch.object(svc, "classify_file", return_value=("2_소설한국", "matched", "reason", entry_dict)):
        res_clean = svc.process_file(f_samguk, tmp_path, clean_existing=True)
        assert res_clean["action"] == "cleaned_duplicate"
        assert entry_dict["status"] == "already_exists_cleaned"
        assert not f_samguk.exists()


def test_map_category_and_effective_filename_branches(tmp_path):
    from backend.book_classifier import map_category, get_effective_filename

    # 936: cat_str="거시경제 > 총론" -> leaf_cat="총론" (933 패스, 936 매칭)
    assert map_category("거시경제 > 총론", "제목", "저자") == "4_경제"
    # 994: 영어, 영문법 (969의 영어회화 등이 없는 카테고리)
    assert map_category("기타어학 > 기초 영문법", "제목", "저자") == "7_영어교육"
    # 1034: cat_str="지역경제 > 총론" -> 933/936 패스 후 1034 경제 폴백 매칭
    assert map_category("지역경제 > 총론", "제목", "저자") == "4_경제"

    # 1058: file_m 있고 parent_m 없거나 둘 다 있는 경우
    parent_dir = tmp_path / "작품"
    parent_dir.mkdir(exist_ok=True)
    f = parent_dir / "[저자] 제목 1권.txt"
    f.touch()
    assert get_effective_filename(f, tmp_path) == "[저자] 제목 1권.txt"
