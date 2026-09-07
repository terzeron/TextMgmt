#!/usr/bin/env python3
import pytest
from pathlib import Path
from backend.book_classifier import (
    extract_explicit_genre,
    clean_filename_to_author_title,
    get_effective_filename,
    title_similarity,
    is_single_match_valid,
    score_text_genre,
    resolve_genre_conflict,
    map_category,
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

def test_title_similarity_and_single_match():
    sim = title_similarity("달빛조각사", "달빛 조각사 1권")
    assert sim > 0.4
    assert is_single_match_valid("달빛조각사", "달빛조각사 1") is True
    assert is_single_match_valid("전혀다른책", "달빛조각사") is False

def test_score_text_genre():
    txt_wuxia = "강호의 무림맹과 마교의 대결, 단전의 내공을 끌어올려 검법을 펼쳤다."
    assert score_text_genre(txt_wuxia) == "3_무협"
    txt_fantasy = "마법사가 마나를 모아 스킬을 발동하고 던전의 몬스터를 사냥했다. 상태창을 열었다."
    assert score_text_genre(txt_fantasy) == "3_판타지"
    txt_romance = "공작가의 황태자가 여주에게 다가와 파티 드레스를 칭찬했다. 로판 악역 영애의 삶."
    assert score_text_genre(txt_romance) == "3_여성향"

def test_resolve_genre_conflict():
    cats = ["3_무협", "3_판타지"]
    res = resolve_genre_conflict(cats, "[무협] 절대검제.txt")
    assert res == "3_무협"

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
