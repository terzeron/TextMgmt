#!/usr/bin/env python3
import pytest
import zipfile
from pathlib import Path
from backend.classifier import BookCategoryClassifier
from backend.classifier.model import Prediction
from backend.book_classifier import (
    extract_explicit_genre,
    inspect_epub_metadata,
    inspect_txt_content,
    clean_filename_to_author_title,
    get_effective_filename,
    title_similarity,
    is_single_match_valid,
    map_category,
    clean_empty_parent_dirs,
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

# --- 악성 EPUB 방어 (CWE-611, CWE-776, CWE-409) ---


def test_inspect_epub_metadata_does_not_expand_entity_bomb(tmp_path):
    """중첩 엔티티(billion laughs)를 확장하지 않아야 한다."""
    decls = ['<!ENTITY a0 "AAAAAAAAAA">'] + [
        '<!ENTITY a%d "%s">' % (i, "".join("&a%d;" % (i - 1) for _ in range(10))) for i in range(1, 9)
    ]
    bomb = '<?xml version="1.0"?><!DOCTYPE p [%s]><package><title>&a8;</title></package>' % "".join(decls)

    epub_path = tmp_path / "bomb.epub"
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr("content.opf", bomb)

    # 539 바이트가 확장되면 약 100GB 다. 빈 meta 로 끝나야 한다.
    assert inspect_epub_metadata(epub_path) == {"title": "", "author": "", "subject": "", "description": ""}


def test_inspect_epub_metadata_does_not_resolve_external_entity(tmp_path):
    """외부 엔티티(file://)로 로컬 파일을 읽어오지 않아야 한다."""
    secret = tmp_path / "canary.txt"
    secret.write_text("XXE_CANARY_VALUE", encoding="utf-8")
    xxe = '<?xml version="1.0"?><!DOCTYPE p [<!ENTITY x SYSTEM "file://%s">]><package><title>&x;</title></package>' % secret

    epub_path = tmp_path / "xxe.epub"
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr("content.opf", xxe)

    assert "XXE_CANARY_VALUE" not in str(inspect_epub_metadata(epub_path))


def test_inspect_epub_metadata_skips_oversized_opf(tmp_path):
    """압축을 풀면 거대한 OPF 는 읽기 전에 건너뛴다."""
    from backend.book_classifier import MAX_OPF_BYTES

    epub_path = tmp_path / "big.epub"
    with zipfile.ZipFile(epub_path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("content.opf", "<package>" + "x" * (MAX_OPF_BYTES + 1) + "</package>")

    assert epub_path.stat().st_size < 100 * 1024  # 압축본은 작다
    assert inspect_epub_metadata(epub_path) == {"title": "", "author": "", "subject": "", "description": ""}


def test_inspect_epub_metadata_reads_opf_with_comments(tmp_path):
    """주석과 처리명령이 섞여도 메타데이터를 읽어야 한다(lxml iter 회귀 방지)."""
    opf = (
        '<?xml version="1.0" encoding="utf-8"?>'
        '<package xmlns:dc="http://purl.org/dc/elements/1.1/">'
        "<!-- 주석 --><?pi 처리명령?>"
        "<metadata><dc:title>주석있음</dc:title><dc:creator>작가</dc:creator></metadata>"
        "</package>"
    )
    epub_path = tmp_path / "comment.epub"
    with zipfile.ZipFile(epub_path, "w") as z:
        z.writestr("content.opf", opf)

    meta = inspect_epub_metadata(epub_path)
    assert meta["title"] == "주석있음"
    assert meta["author"] == "작가"


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

def test_clean_empty_parent_dirs_keeps_dir_holding_only_hidden_files(tmp_path):
    """rmdir은 숨김 파일까지 봐야 성공한다. 보이는 항목만 세면 코드와 실제 동작이 어긋난다."""
    stop_dir = tmp_path / "library"
    child = stop_dir / "series"
    child.mkdir(parents=True)
    (child / ".nomedia").write_text("", encoding="utf-8")

    clean_empty_parent_dirs(child, stop_dir)

    assert child.exists()


def test_clean_empty_parent_dirs_never_climbs_past_stop_dir(tmp_path):
    """stop_dir를 정규화 없이 비교하면 못 알아보고 조상까지 거슬러 올라간다."""
    stop_dir = tmp_path / "library"
    child = stop_dir / "series"
    child.mkdir(parents=True)
    # 같은 디렉토리를 '..'를 낀 다른 표기로 넘긴다.
    unnormalized_stop = stop_dir / "series" / ".."

    clean_empty_parent_dirs(child, unnormalized_stop)

    assert stop_dir.exists()
    assert tmp_path.exists()


def test_clean_empty_parent_dirs_accepts_str_stop_dir(tmp_path):
    """호출자가 문자열을 넘기면 Path와 절대 같아지지 않아 stop_dir를 지나친다."""
    stop_dir = tmp_path / "library"
    child = stop_dir / "series"
    child.mkdir(parents=True)

    clean_empty_parent_dirs(child, str(stop_dir))

    assert stop_dir.exists()
    assert not child.exists()











class StubClassifier:
    """항상 같은 카테고리를 확신 있게 내놓는 모델 대역."""

    def __init__(self, category="3_판타지", confidence=0.97):
        self.prediction = Prediction(category, confidence, [(category, confidence)], f"모델 확신도 {confidence:.3f} -> {category}")

    def __bool__(self):
        return True

    def classify_path(self, fpath, min_confidence=None):
        return self.prediction

    def classify_document(self, doc, min_confidence=None):
        return self.prediction


def test_book_classifier_service_process_file(tmp_path):
    lib_root = tmp_path / "text"
    src_dir = lib_root / "0_telegram"
    src_dir.mkdir(parents=True)
    fpath = src_dir / "[판타지] 나 혼자 만렙 귀환자 01.txt"
    fpath.write_text("스킬과 던전 마나 시스템 몬스터", encoding="utf-8")
    service = BookClassifierService(
        library_root=lib_root,
        cache_file=tmp_path / "cache.json",
        verbose=False,
        classifier=StubClassifier(),
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





def test_inspect_txt_content_cp949_and_header_genre(tmp_path):
    f = tmp_path / "test_cp949.txt"
    content = "=====================\n[판타지] 룬의 아이들 1권\n=====================\n프롤로그..."
    f.write_bytes(content.encode("cp949"))

    res = inspect_txt_content(f)
    assert res.get("header_genre") == "판타지"
    assert "룬의 아이들" in res.get("snippet", "")














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












def test_title_match_reliable_ratio():
    from backend.book_classifier import is_title_match_reliable

    # 189: search_title 단어의 70% 이상 포함 및 sim >= 0.5
    assert is_title_match_reliable("해리 포터와 비밀의 방", "해리 포터와 비밀의 방 (전2권)") is True
    # 191: ratio < 0.7로 마지막 return False 도달
    assert is_title_match_reliable("해리포터 마법사의 돌", "해리포터 비밀의 방") is False








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

    parent_dir = tmp_path / "작품"
    parent_dir.mkdir(exist_ok=True)
    f = parent_dir / "[저자] 제목 1권.txt"
    f.touch()
    assert get_effective_filename(f, tmp_path) == "[저자] 제목 1권.txt"








# ---------------------------------------------------------------------------
# 판정 결합 — 모델이 먼저, 못 하면 서점
# ---------------------------------------------------------------------------


def _service(tmp_path, classifier):
    return BookClassifierService(library_root=tmp_path, cache_file=tmp_path / "cache.json", classifier=classifier)


class RefusingClassifier:
    """확신이 모자라 판정을 거부하는 모델 대역."""

    def __bool__(self):
        return True

    def classify_path(self, fpath, min_confidence=None):
        return Prediction(None, 0.42, [("3_판타지", 0.42)], "확신도 0.420 < 임계값 0.90 이라 판정하지 않음")

    def classify_document(self, doc, min_confidence=None):
        return self.classify_path(None)


def _entry(**stores):
    base = {"search_title": "달빛조각사", "yes24": {}, "aladin": {}, "kyobo": {}}
    base.update(stores)
    return base


def test_decide_prefers_the_model_over_bookstores(tmp_path):
    service = _service(tmp_path, StubClassifier("3_판타지"))
    entry = _entry(yes24={"mapped": "2_소설외국", "title": "달빛조각사"}, aladin={"mapped": "2_소설외국", "title": "달빛조각사"})
    cat, method, reason, *_ = service._decide(None, "달빛조각사.txt", entry)
    assert (cat, method) == ("3_판타지", "model")
    assert "모델 확신도" in reason


def test_decide_falls_back_to_bookstore_majority_when_the_model_refuses(tmp_path):
    service = _service(tmp_path, RefusingClassifier())
    entry = _entry(yes24={"mapped": "2_소설외국", "title": "달빛조각사"}, aladin={"mapped": "2_소설외국", "title": "달빛조각사"}, kyobo={"mapped": "3_판타지", "title": "달빛조각사"})
    cat, method, reason, *_ = service._decide(None, "달빛조각사.txt", entry)
    assert (cat, method) == ("2_소설외국", "bookstore_majority")
    # 모델이 왜 못 했는지도 근거에 남아야 한다
    assert "확신도" in reason


def test_decide_takes_a_single_bookstore_only_when_trusted(tmp_path):
    service = _service(tmp_path, RefusingClassifier())
    entry = _entry(yes24={"mapped": "3_무협", "title": "달빛조각사"})

    cat, method, _, *_ = service._decide(None, "달빛조각사.txt", entry, trust_single_match=True)
    assert (cat, method) == ("3_무협", "bookstore_single")

    cat, method, _, *_ = service._decide(None, "달빛조각사.txt", entry, trust_single_match=False)
    assert cat is None
    assert method == "conflict"


def test_decide_reports_conflict_when_two_bookstores_disagree(tmp_path):
    service = _service(tmp_path, RefusingClassifier())
    entry = _entry(yes24={"mapped": "3_무협", "title": "달빛조각사"}, aladin={"mapped": "3_판타지", "title": "달빛조각사"})
    cat, method, reason, *_ = service._decide(None, "달빛조각사.txt", entry, trust_single_match=True)
    assert cat is None
    assert method == "conflict"
    assert "갈림" in reason


def test_decide_ignores_bookstore_hits_whose_title_does_not_match(tmp_path):
    service = _service(tmp_path, RefusingClassifier())
    entry = _entry(yes24={"mapped": "3_무협", "title": "전혀 다른 책 제목"}, aladin={"mapped": "3_무협", "title": "또 다른 책"})
    cat, method, _, *_ = service._decide(None, "달빛조각사.txt", entry)
    assert cat is None
    assert method == "not_found"


def test_decide_by_bookstore_records_top_two_votes_on_conflict(tmp_path):
    """갈리면(target=None) 버리던 득표를 entry에 남겨야 화면이 후보 2개를 보여줄 수 있다."""
    service = _service(tmp_path, RefusingClassifier())
    entry = _entry(yes24={"mapped": "3_무협", "title": "달빛조각사"}, aladin={"mapped": "3_판타지", "title": "달빛조각사"})

    cat, method, _ = service._decide_by_bookstore(entry)

    assert cat is None
    assert method == "conflict"
    assert entry["bookstore_candidates"] == [("3_무협", 1), ("3_판타지", 1)]


def test_decide_by_bookstore_records_empty_candidates_when_not_found(tmp_path):
    """득표가 아예 없어도 키는 있어야 호출자가 .get() 없이 읽다가 깨지지 않는다."""
    service = _service(tmp_path, RefusingClassifier())
    entry = _entry()

    cat, method, _ = service._decide_by_bookstore(entry)

    assert cat is None
    assert method == "not_found"
    assert entry["bookstore_candidates"] == []


def test_decide_survives_a_model_that_raises(tmp_path):
    class BrokenClassifier:
        def __bool__(self):
            return True

        def classify_path(self, fpath, min_confidence=None):
            raise RuntimeError("모델 폭발")

        def classify_document(self, doc, min_confidence=None):
            raise RuntimeError("모델 폭발")

    service = _service(tmp_path, BrokenClassifier())
    entry = _entry(yes24={"mapped": "3_무협", "title": "달빛조각사"}, aladin={"mapped": "3_무협", "title": "달빛조각사"})
    cat, method, reason, *_ = service._decide(None, "달빛조각사.txt", entry)
    # 모델이 터져도 서점 경로로 답한다
    assert (cat, method) == ("3_무협", "bookstore_majority")
    assert "모델 판정 실패" in reason


def test_decide_returns_nothing_when_there_is_no_model_and_no_bookstore(tmp_path):
    service = _service(tmp_path, BookCategoryClassifier(model=None))
    cat, method, reason, *_ = service._decide(None, "달빛조각사.txt", _entry())
    assert cat is None
    assert method == "not_found"
    assert "모델 파일이 없어" in reason


def test_classify_file_exposes_model_confidence(tmp_path, monkeypatch):
    """등급을 매기려면 모델 점수가 밖으로 나와야 한다."""
    from backend.book_classifier import BookClassifierService

    service = BookClassifierService.__new__(BookClassifierService)
    service.cache = {}
    service.title_cache = {}
    service.library_root = tmp_path
    monkeypatch.setattr(service, "query_bookstores", lambda *a, **k: ({}, {}, {}))
    monkeypatch.setattr(service, "_decide_by_model", lambda *a, **k: ("3_SF", "모델 판정", 0.87))
    monkeypatch.setattr(service, "_decide_by_bookstore", lambda *a, **k: (None, "not_found", "서점에서 못 찾음"))

    class Policy:
        override_below = 0.5

        def prefers_bookstore(self, confidence):
            return confidence < self.override_below

    service.bookstore_policy = Policy()
    target = tmp_path / "[저자] 제목.epub"
    target.write_text("x")

    _cat, _method, _reason, entry = service.classify_file(target, tmp_path)

    assert entry["confidence"] == 0.87
    assert entry["model_category"] == "3_SF"


# ---------------------------------------------------------------------------
# 확신도 사각지대 — 모델이 답은 했지만 '확실'이라 부를 만큼은 아닌 구간
# ---------------------------------------------------------------------------


def test_decide_takes_bookstore_majority_when_the_model_is_not_confident_enough(tmp_path):
    """확신도가 신뢰 기준 미만이면 서점 다수결이 이긴다.

    예전에는 이 구간에서 서점을 세지도 않고 모델 답을 돌려줬다. 그런데 화면은
    그 답을 '확실'로 인정하지 않아 목적지가 빈 채로 나갔다. 서점 3곳이 한목소리로
    말해도 반영되지 않았다.
    """
    service = _service(tmp_path, StubClassifier("2_수필서간일기", confidence=0.076))
    entry = _entry(
        yes24={"mapped": "2_소설외국", "title": "달빛조각사"},
        aladin={"mapped": "2_소설외국", "title": "달빛조각사"},
        kyobo={"mapped": "2_소설외국", "title": "달빛조각사"},
    )

    cat, method, reason, model_cat, confidence = service._decide(None, "달빛조각사.txt", entry)

    assert (cat, method) == ("2_소설외국", "bookstore_majority")
    assert entry["bookstore_candidates"] == [("2_소설외국", 3)]
    # 모델 답도 근거에 남아야 관리자가 왜 갈렸는지 안다
    assert model_cat == "2_수필서간일기"
    assert confidence == 0.076


def test_decide_records_bookstore_votes_even_when_they_lose(tmp_path):
    """다수결이 안 서도 득표는 남긴다. 화면이 후보로 보여줄 수 있어야 한다."""
    service = _service(tmp_path, StubClassifier("2_수필서간일기", confidence=0.076))
    entry = _entry(yes24={"mapped": "2_소설외국", "title": "달빛조각사"})

    cat, method, _reason, *_ = service._decide(None, "달빛조각사.txt", entry)

    # 한 곳뿐이라 다수결이 아니다. 모델 답이 그대로 남는다.
    assert (cat, method) == ("2_수필서간일기", "model")
    assert entry["bookstore_candidates"] == [("2_소설외국", 1)]


def test_decide_keeps_a_confident_model_without_consulting_bookstores(tmp_path):
    """신뢰 기준을 넘긴 모델은 예전대로 먼저다. 서점을 세는 비용도 안 낸다."""
    service = _service(tmp_path, StubClassifier("3_판타지", confidence=0.97))
    entry = _entry(
        yes24={"mapped": "2_소설외국", "title": "달빛조각사"},
        aladin={"mapped": "2_소설외국", "title": "달빛조각사"},
    )

    cat, method, _reason, *_ = service._decide(None, "달빛조각사.txt", entry)

    assert (cat, method) == ("3_판타지", "model")
    assert "bookstore_candidates" not in entry
