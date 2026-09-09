#!/usr/bin/env python3
"""
BookClassifierService - 결정론적(Deterministic) 도서 자동 분류 엔진
1번: 3대 온라인 서점(Yes24, 알라딘, 교보) 교차 검증 및 2/3 다수결 판정
2번: EPUB 메타데이터(dc:subject 등) 및 TXT 상단 본문 키워드 정적 스코어링 판정
"""

import os
import re
import json
import shutil
import logging
import zipfile
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

from backend.bookstore import AbstractBookstore, Yes24Bookstore, AladinBookstore, KyoboBookstore

logger = logging.getLogger(__name__)


STANDARD_CATEGORIES = [
    "0_html", "0_hwp", "0_telegram",
    "1_동양고전", "1_동양고전한문", "1_문헌서지", "1_서양고전", "1_올재", "1_한국고전국역총서",
    "2_동서문화사월드북", "2_문예세계문학선", "2_문학일반서평작법독서", "2_소설Abe전집", "2_소설역사", "2_소설외국",
    "2_소설일본", "2_소설일본게이고", "2_소설일본하루키", "2_소설중국", "2_소설한국", "2_수필서간일기", "2_시",
    "2_열린책들세계문학", "2_을유세계문학전집",
    "3_SF", "3_SF그리폰북스", "3_SF영문", "3_SF직지", "3_SF환상문학전집", "3_그래픽노블", "3_라이트노벨",
    "3_무협", "3_셜록홈즈", "3_스릴러", "3_여성향", "3_장르문학자료", "3_판타지", "3_판타지pdf",
    "4_경영마케팅", "4_경제", "4_누워서읽는법학", "4_법", "4_사회인류", "4_살림지식총서", "4_시공디스커버리",
    "4_심리학뇌과학", "4_역사인물", "4_인문일반논픽션", "4_정치외교군사", "4_종교신화", "4_철학윤리",
    "5_그림으로_읽는", "5_미술예술건축", "5_사진영상", "5_서브컬쳐", "5_수학과학일반", "5_스포츠", "5_영화", "5_음악", "5_이지사이언스",
    "6_재테크", "6_처세술리더십창의성",
    "7_교육일반", "7_국어교육", "7_언어일반", "7_영문일반", "7_영어교육", "7_외국어교육", "7_외국인을위한한국어읽기",
    "7_일문일반", "7_일어교육", "7_중문일반", "7_중어한자교육",
    "8_IT", "8_건강일반", "8_공인중개사", "8_모델링", "8_밀리터리", "8_성", "8_실용의학회계", "8_악보", "8_여행", "8_요리음료",
    "9_BLGL", "9_격언명언", "9_북스캔OCR", "9_성인", "9_어린이육아", "9_역학해몽퍼즐", "9_유머", "9_청소년"
]

def extract_explicit_genre(filename: str) -> Optional[str]:
    """파일명(URL 디코딩 후)에 명시된 장르 태그나 접두어를 추출하여 표준 카테고리로 반환"""
    fname = urllib.parse.unquote(filename)
    tags = [
        (r"\[[^\s\]]*(?:로맨스판타지|로맨스|로판|현대로맨스|현로|로맨틱판타지|동양로맨스)[^\s\]]*\]|\([^\s\)]*(?:로맨스판타지|로맨스|로판|현대로맨스|현로|로맨틱판타지|동양로맨스)[^\s\)]*\)|^(?:동양)?(?:로맨스판타지|로맨스|로판|현대로맨스|현로)[0-9a-zA-Z가-힣\s\(\)\]【】]*[\)\s\]]", "3_여성향"),
        (r"\[[^\s\]]*(?:BL|GL|백합|오메가버스)[^\s\]]*\]|\([^\s\)]*(?:BL|GL|백합|오메가버스)[^\s\)]*\)|^\bBL\b", "9_BLGL"),
        (r"\[[^\s\]]*(?:신무협|정통무협|무협소설|무협|선협)[^\s\]]*\]|\([^\s\)]*(?:신무협|정통무협|무협소설|무협|선협)[^\s\)]*\)|^(?:미완\)|단편\))?무협[0-9a-zA-Z가-힣\s\(\)\]【】]*[\)\s\]]", "3_무협"),
        (r"\[[^\s\]]*(?:퓨전판타지|현대판타지|게임판타지|퓨판|현판|겜판|판타지소설|판타지|레이드물|헌터물)[^\s\]]*\]|\([^\s\)]*(?:퓨전판타지|현대판타지|게임판타지|퓨판|현판|겜판|판타지소설|판타지)[^\s\)]*\)|^(?:현판|퓨판|겜판|판타지)[0-9a-zA-Z가-힣\s\(\)\]【】]*[\)\s\]]", "3_판타지"),
        (r"\[(?:TS|티에스)\]|\((?:TS|티에스)\)|(?:^|[^a-zA-Z0-9가-힣])TS(?:물|소설|됐|은|는|이|가|도|[0-9\s\]\)_]|$)|^TS[가-힣\s]", "3_판타지"),
        (r"\[[^\s\]]*(?:라이트노벨|라노벨)[^\s\]]*\]|\([^\s\)]*(?:라이트노벨|라노벨)[^\s\)]*\)", "3_라이트노벨"),
        (r"\[[^\s\]]*(?:SF소설|과학소설|SF)[^\s\]]*\]|\([^\s\)]*(?:SF소설|과학소설|SF)[^\s\)]*\)|^\bSF\b[\s\)]", "3_SF"),
        (r"\[[^\s\]]*(?:추리소설|미스터리소설|스릴러소설|추리|미스터리|스릴러)[^\s\]]*\]|\([^\s\)]*(?:추리소설|미스터리소설|스릴러소설|추리|미스터리|스릴러)[^\s\)]*\)", "3_스릴러"),
        (r"\[[^\s\]]*(?:그래픽노블|만화|코믹스|웹툰)[^\s\]]*\]|\([^\s\)]*(?:그래픽노블|만화|코믹스|웹툰)[^\s\)]*\)", "3_그래픽노블"),
        (r"\[[^\s\]]*(?:소설|한국소설)[^\s\]]*\]|\([^\s\)]*(?:소설|한국소설)[^\s\)]*\)", "2_소설한국"),
    ]
    for pat, cat in tags:
        if re.search(pat, fname, re.IGNORECASE):
            return cat
    return None


def inspect_epub_metadata(fpath: Path) -> Dict[str, str]:
    """EPUB 파일 내부의 .opf 메타데이터(title, author, subject, description) 추출"""
    meta = {"title": "", "author": "", "subject": "", "description": ""}
    try:
        with zipfile.ZipFile(fpath, "r") as z:
            opf_files = [n for n in z.namelist() if n.endswith(".opf")]
            if not opf_files:
                return meta
            opf_content = z.read(opf_files[0]).decode("utf-8", errors="ignore")
            root = ET.fromstring(opf_content)
            for elem in root.iter():
                tag = elem.tag.split("}")[-1].lower()
                if tag == "title" and elem.text and not meta["title"]:
                    meta["title"] = elem.text.strip()
                elif tag in ["creator", "author"] and elem.text and not meta["author"]:
                    meta["author"] = elem.text.strip()
                elif tag == "subject" and elem.text:
                    if meta["subject"]:
                        meta["subject"] += " > " + elem.text.strip()
                    else:
                        meta["subject"] = elem.text.strip()
                elif tag == "description" and elem.text and not meta["description"]:
                    meta["description"] = elem.text.strip()
    except Exception:
        pass
    return meta


def inspect_txt_content(fpath: Path) -> Dict[str, Any]:
    """TXT 파일 앞부분 줄(최대 40줄)에서 해시태그, 헤더 장르 및 소개글 스니펫 추출 (UTF-8 실패 시 CP949 자동 폴백)"""
    info = {"hashtags": [], "snippet": ""}
    try:
        lines = []
        try:
            with open(fpath, "r", encoding="utf-8") as f:
                lines = [f.readline() for _ in range(40)]
        except UnicodeDecodeError:
            with open(fpath, "r", encoding="cp949", errors="ignore") as f:
                lines = [f.readline() for _ in range(40)]

        raw_text = "".join(lines)
        if not re.search(r"[가-힣]{2,}", raw_text):
            try:
                with open(fpath, "r", encoding="cp949", errors="ignore") as f:
                    lines = [f.readline() for _ in range(40)]
            except Exception:
                pass

        lines = [l.strip() for l in lines]
        snippet = " ".join([l for l in lines if l])[:2000]
        info["snippet"] = snippet
        tags = re.findall(r"#([가-힣a-zA-Z0-9_]{2,15})", snippet)
        info["hashtags"] = tags

        m = re.search(r"(?:장르|분류)\s*[:：]\s*([가-힣a-zA-Z]+)|(?:^|\s)\[([가-힣]{2,6})\]|(?:^|\s)【([가-힣]{2,6})】", snippet)
        if m:
            g = m.group(1) or m.group(2) or m.group(3)
            info["header_genre"] = g
    except Exception:
        pass
    return info


def title_similarity(t1: str, t2: str) -> float:
    """두 도서 제목 간의 글자 bi-gram 유사도(Dice coefficient) 계산"""
    s1 = re.sub(r"[^\w가-힣]", "", t1.lower())
    s2 = re.sub(r"[^\w가-힣]", "", t2.lower())
    if not s1 or not s2:
        return 0.0
    if s1 in s2 or s2 in s1:
        return len(min(s1, s2, key=len)) / len(max(s1, s2, key=len))
    b1 = set(s1[i:i+2] for i in range(len(s1)-1))
    b2 = set(s2[i:i+2] for i in range(len(s2)-1))
    if not b1 or not b2:
        return 0.0
    return 2.0 * len(b1 & b2) / (len(b1) + len(b2))


def is_single_match_valid(search_title: str, found_title: str) -> bool:
    """단일 서점 매칭 시 검색된 제목이 원본 검색어와 유효하게 일치하는지 검증 (추천도서 오매칭 방지)"""
    if not found_title or not search_title:
        return False
    sim = title_similarity(search_title, found_title)
    if sim >= 0.35:
        return True

    # 부제 등을 제외한 앞부분 단어 2개 이상 일치 여부 확인
    st_clean = re.sub(r"[^\w가-힣\s]", "", search_title.lower()).strip()
    ft_clean = re.sub(r"[^\w가-힣\s]", "", found_title.lower()).strip()
    st_words = [w for w in st_clean.split() if len(w) >= 2]
    ft_words = [w for w in ft_clean.split() if len(w) >= 2]
    matching = [w for w in st_words if w in ft_words]
    if len(matching) >= 2:
        return True
    if len(st_words) == 1 and len(ft_words) <= 3 and st_words[0] in ft_words:
        return True

    return False


def is_title_match_reliable(search_title: str, found_title: str) -> bool:
    """서점 간 충돌 해소 및 엄격한 단일 매칭 판정을 위한 고신뢰도 제목 일치 검증"""
    if not found_title or not search_title:
        return False
    st_clean = re.sub(r"[^\w가-힣\s]", "", search_title.lower()).strip()
    ft_clean = re.sub(r"[^\w가-힣\s]", "", found_title.lower()).strip()
    st_words = [w for w in st_clean.split() if len(w) >= 2]
    ft_words = [w for w in ft_clean.split() if len(w) >= 2]
    if not st_words:
        return False

    # 음반/OST 오매칭 필터링 (도서 파일인데 음반/사운드트랙으로 매칭되는 경우 방지)
    if any(ost_kw in found_title.upper() for ost_kw in ["O.S.T", "OST", "(CD", "CD-ROM", "음반", "사운드트랙"]):
        return False

    # 1. 공백 제외 완전 일치 또는 통째 포함
    s1 = re.sub(r"\s+", "", st_clean)
    s2 = re.sub(r"\s+", "", ft_clean)
    if s1 == s2:
        return True
    if s1 in s2 and len(s1) >= 4:
        return True

    # 2. search_title의 단어 중 70% 이상이 found_title에 포함되고 bi-gram 유사도 >= 0.5
    matching = [w for w in st_words if any(w in fw or fw in w for fw in ft_words)]
    ratio = len(matching) / len(st_words)
    sim = title_similarity(search_title, found_title)
    if ratio >= 0.7 and sim >= 0.5:
        return True

    return False


def extract_content_head_tail_words(fpath: Path, head_n: int = 500, tail_n: int = 500) -> str:
    """TXT/EPUB 파일에서 처음 N단어와 마지막 N단어 분량의 본문 텍스트를 추출하여 결합 (하이브리드 장르 탐색용)"""
    ext = fpath.suffix.lower()
    head_text = ""
    tail_text = ""

    if ext == ".txt":
        # Head
        for enc in ["utf-8", "cp949"]:
            try:
                with open(fpath, "r", encoding=enc, errors="ignore") as f:
                    lines = [f.readline() for _ in range(250)]
                raw = "".join(lines)
                if re.search(r"[가-힣]{3,}", raw):
                    head_text = raw
                    break
            except Exception:
                continue

        # Tail (seek from end)
        try:
            size = fpath.stat().st_size
            read_bytes = min(size, 40000)
            for enc in ["utf-8", "cp949"]:
                try:
                    with open(fpath, "rb") as f:
                        f.seek(max(0, size - read_bytes))
                        raw_b = f.read()
                    raw_str = raw_b.decode(enc, errors="ignore")
                    lines = raw_str.splitlines()
                    clean_lines = lines[1:] if len(lines) > 1 else lines
                    raw = " ".join(clean_lines)
                    if re.search(r"[가-힣]{3,}", raw):
                        tail_text = raw
                        break
                except Exception:
                    continue
        except Exception:
            pass

    elif ext == ".epub":
        try:
            with zipfile.ZipFile(fpath, "r") as z:
                htmls = [n for n in z.namelist() if n.lower().endswith((".html", ".xhtml", ".htm")) and not any(k in n.lower() for k in ["cover", "nav", "toc", "title"])]
                if not htmls:
                    htmls = [n for n in z.namelist() if n.lower().endswith((".html", ".xhtml", ".htm"))]
                chunks_h = []
                for h in htmls[:2]:
                    raw = z.read(h).decode("utf-8", errors="ignore")
                    clean = re.sub(r"<[^>]+>", " ", raw)
                    if len(clean.strip()) > 50:
                        chunks_h.append(clean)
                head_text = " ".join(chunks_h)

                chunks_t = []
                for h in htmls[-2:]:
                    raw = z.read(h).decode("utf-8", errors="ignore")
                    clean = re.sub(r"<[^>]+>", " ", raw)
                    if len(clean.strip()) > 50:
                        chunks_t.append(clean)
                tail_text = " ".join(chunks_t)
        except Exception:
            pass

    h_words = head_text.split()[:head_n]
    t_words = tail_text.split()[-tail_n:] if tail_text else []
    return (" ".join(h_words) + " " + " ".join(t_words)).strip()


def extract_content_first_1000_words(fpath: Path) -> str:
    """TXT/EPUB 파일에서 처음 500단어와 마지막 500단어를 결합하여 1000단어 반환 (하위 호환)"""
    return extract_content_head_tail_words(fpath, 500, 500)


def clean_disclaimer_and_colophon(text: str) -> str:
    """
    저작권 경고, 무단전재 공지, 판권지(Colophon), 출판사 정보 등
    장르 판정과 무관한 시스템/법적 문구를 텍스트 분석 전에 정제
    """
    if not text:
        return ""
    # 1. 무단전재, 무단복제, 저작권 관련 문장/블록 제거
    cleaned = re.sub(r"무단\s*전재[^\n.]*(?:금합니다|금지|처벌|법적[^\n.]*책임)[^\n.]*", " ", text, flags=re.IGNORECASE)
    cleaned = re.sub(r"이\s*(?:전자)?책은\s*저작권법[^\n.]*(?:보호|금합니다|처벌|금지)[^\n.]*", " ", cleaned, flags=re.IGNORECASE)
    cleaned = re.sub(r"저작권자의\s*(?:서면\s*)?동의\s*없이[^\n.]*(?:금합니다|처벌)[^\n.]*", " ", cleaned, flags=re.IGNORECASE)
    # 2. 판권란 (발행인, 편집인, 디자인, 제작, 마케팅, 등록번호 등) 라인/블록 제거
    cleaned = re.sub(r"(?:발행인|편집인|책임편집|디자인|제작|마케팅|펴낸곳|출판사|등록번호|isbn)\s+[:\w\s,&]+(?=[.\n]|$)", " ", cleaned, flags=re.IGNORECASE)
    # 3. 조아라/문피아 뷰어 경고 문구 제거
    cleaned = re.sub(r"\*경고\*[^\n.]*(?:정상적인 경로의 뷰어가 아닙니다|처벌대상이 되실 수 있으니)[^\n.]*", " ", cleaned, flags=re.IGNORECASE)
    return cleaned


def count_genre_word_patterns(text: str, kws: List[str]) -> int:
    cnt = 0
    for kw in kws:
        if kw == "단전":
            cnt += len(re.findall(r"(?<!무)단전(?:[를이가의로통량석홀]?)(?:$|[^\w가-힣])", text))
        elif kw == "진기":
            cnt += len(re.findall(r"(?<!민)진기(?!한)(?:[를가의로]?)(?:$|[^\w가-힣])", text))
        elif kw == "강수":
            cnt += len(re.findall(r"(?<![가-힣a-zA-Z])강수(?:[를이가의로]?)(?:$|[^\w가-힣])", text))
        elif kw == "마나":
            cnt += len(re.findall(r"(?:^|[^\w가-힣])마나(?:[를이가의로통량석홀]?)(?:$|[^\w가-힣])", text))
        elif kw in ["bl", "gl"]:
            cnt += len(re.findall(r"(?:^|[^a-zA-Z])" + kw + r"(?:$|[^a-zA-Z])", text))
        else:
            cnt += text.count(kw)
    return cnt


# ==========================================
# 5대 장르(무협, 판타지, 여성향, BLGL, 성인) 도메인 키워드 상수 정의 (Single Source of Truth)
# ==========================================
WUXIA_CORE = [
    "무협", "무림", "단전", "내공", "진기", "운기조식", "기경팔맥", "주화입마", "환골탈태",
    "화산파", "무당파", "소림사", "개방", "사파", "정파", "마교", "천마", "비급", "검법", "도법", "심법",
    "세가", "사천당가", "남궁세가", "제갈세가", "모용세가", "하북팽가", "소가주", "녹림", "암기", "독공", "독문", "검기"
]
WUXIA_KWS = WUXIA_CORE + [
    "강호", "임독이맥", "종남파", "혈교", "장문인", "소교주", "맹주", "무림맹", "절기", "초식",
    "공자", "가주", "하오문", "점창파", "도기", "호법", "총채주"
]

# 판타지: 현대 헌터물 + 중세 서양 판타지/영지물/제국물 전면 보강
FANTASY_CORE = [
    "판타지", "던전", "몬스터", "헌터", "각성", "레이드", "게이트", "상태창", "마나", "마법진",
    "오크", "고블린", "드래곤", "엘프", "용사", "이세계", "귀환자", "만렙",
    "오러", "소드마스터", "기사단", "마법사", "마탑", "영지", "영주", "마력"
]
FANTASY_KWS = FANTASY_CORE + [
    "길드", "시스템", "퀘스트", "스킬", "스탯", "레벨업", "플레이어", "인벤토리", "아이템", "서클", "아카데미", "골렘", "마물",
    "제국", "왕국", "공국", "백작", "남작", "차원이동", "환생자", "기사"
]

# 여성향/로판: 서양 판타지 영지물과 겹치는 작위(황제, 공작, 영애) 및 시한부는 CORE에서 제외하고
# 결정적 로맨스 관계성 어휘가 최소 1개 이상 존재할 때만 보조 키워드 점수 합산
ROFAN_CORE = [
    "로맨스", "로판", "현로", "남주", "여주", "남주인공", "여주인공",
    "파혼", "시월드", "후회남", "집착남", "계략남", "악녀", "햇살여주", "빙의녀"
]
ROFAN_KWS = ROFAN_CORE + [
    "공작", "공작가", "황제", "황실", "사교계", "무도회", "드레스", "시녀", "집사", "약혼", "키스", "설렘", "영애", "시한부"
]

# BL: 페로몬/강수/각인은 생물학 및 인명 오탐 방지를 위해 CORE에서 제외하고
# 결정적 BL 관계성 어휘가 최소 1개 이상 존재할 때만 보조 키워드 점수 합산
BL_CORE = [
    "미인공", "미남공", "다정공", "광공", "집착공", "연하공", "연상공", "후회공", "미인수", "단정수", "지랄수",
    "임신수", "순진수", "오메가버스", "가이드버스", "보이즈러브", "동성애", "백합"
]
BL_KWS = BL_CORE + ["에스퍼", "가이딩", "히트사이클", "페로몬", "노팅", "각인", "강수", "bl", "gl"]

ADULT_KWS = [
    "야설", "성인소설", "음란", "음탕", "육덕", "최면", "조교", "근친", "스와핑", "섹스", "자위",
    "사정액", "쿠퍼액", "애액", "정액", "자지", "보지에", "보지를", "보지속", "음순", "클리토리스",
    "귀두", "유두", "유륜", "젖가슴", "피스톤", "허리짓", "교성", "오르가즘", "절정에", "펠라치오"
]


def calculate_5_genre_scores_and_ratios(title: str, text: str) -> Dict[str, Any]:
    """
    도서 제목과 텍스트(Head 500 + Tail 500)에서 5대 장르(무협, 판타지, 여성향, BL, 성인)의
    빈도 점수, 비율(%), 및 클러스터를 산출하는 공통 핵심 분석 함수.
    """
    cleaned_text = clean_disclaimer_and_colophon(text)
    combined = (title + " " + cleaned_text).lower()
    total_chars = len(combined)
    hangul_chars = len(re.findall(r"[가-힣]", combined))

    if total_chars < 15 or (hangul_chars / total_chars) < 0.20:
        return {
            "valid": False,
            "reason": "not_enough_hangul",
            "scores": {},
            "ratios": {},
            "total_score": 0,
            "cluster": "미분류_비문학영문",
            "target_cat": None,
        }

    w_score = count_genre_word_patterns(combined, WUXIA_KWS) if any(kw in combined for kw in WUXIA_CORE) else 0
    f_score = count_genre_word_patterns(combined, FANTASY_KWS) if any(kw in combined for kw in FANTASY_CORE) else 0

    # 여성향: 결정적 로맨스 관계성 어휘(ROFAN_CORE)가 존재할 때만 유효 인정
    has_rofan_core = any(kw in combined for kw in ROFAN_CORE)
    r_score = count_genre_word_patterns(combined, ROFAN_KWS) if has_rofan_core else 0

    # BL: 결정적 BL 어휘(BL_CORE)나 독립 단어 bl/gl이 존재할 때만 유효 인정 (단독 페로몬/강수 오탐 방지)
    has_bl_core = any(kw in combined for kw in BL_CORE) or bool(re.search(r"(?:^|[^a-zA-Z])(?:bl|gl)(?:$|[^a-zA-Z])", combined))
    b_score = count_genre_word_patterns(combined, BL_KWS) if has_bl_core else 0

    matched_adult = [kw for kw in ADULT_KWS if kw in combined]
    a_score = sum(combined.count(kw) for kw in ADULT_KWS) if (len(matched_adult) >= 3 and sum(combined.count(kw) for kw in ADULT_KWS) >= 5) else 0

    scores = {
        "3_무협": w_score,
        "3_판타지": f_score,
        "3_여성향": r_score,
        "9_BLGL": b_score,
        "9_성인": a_score,
    }

    # 파일명 명시적 장르(extract_explicit_genre) 최우선 보호 및 가중치 적용
    explicit_cat = extract_explicit_genre(title)
    if explicit_cat and explicit_cat in scores:
        scores[explicit_cat] += 5
        if explicit_cat == "3_무협":
            # 파일명에 무협이 명시된 경우 우연한 노이즈로 인한 BL/여성향 이탈 원천 차단
            scores["9_BLGL"] = 0
            scores["3_여성향"] = 0
        elif explicit_cat == "3_판타지":
            scores["9_BLGL"] = 0
            scores["3_여성향"] = 0

    total_score = sum(scores.values())

    if total_score == 0:
        return {
            "valid": False,
            "reason": "zero_score",
            "scores": scores,
            "ratios": {k: 0.0 for k in scores},
            "total_score": 0,
            "cluster": "미분류_키워드부족",
            "target_cat": None,
        }

    ratios = {k: round((v / total_score) * 100, 1) for k, v in scores.items()}

    # 클러스터 및 타겟 카테고리 결정
    cluster = "기타_복합"
    target_cat = None

    # 1. 단일 압도적 클러스터 (Dominant >= 75% 및 score >= 5)
    for cat, r in ratios.items():
        if r >= 75.0 and scores[cat] >= 5:
            # 3_무협의 경우, 판타지 키워드가 2건 이상 검출되면 순수무협이 아닌 하이브리드로 판단하여 3_판타지로 분류
            if cat == "3_무협" and scores["3_판타지"] >= 2:
                cluster = "하이브리드_무협_판타지"
                target_cat = "3_판타지"
            else:
                cluster = f"확실한_{cat}"
                target_cat = cat
            break

    # 2. 하이브리드 클러스터
    if not target_cat:
        # 무협 vs 판타지 하이브리드 (3_무협은 순수무협만 유지, 하이브리드는 3_판타지로 자동 분류 지정)
        if (ratios["3_무협"] + ratios["3_판타지"] >= 70.0 and ratios["3_무협"] >= 15.0 and ratios["3_판타지"] >= 15.0 and scores["3_판타지"] >= 2) or (scores["3_무협"] >= 2 and scores["3_판타지"] >= 2 and (scores["3_무협"] + scores["3_판타지"]) >= 5):
            cluster = "하이브리드_무협_판타지"
            target_cat = "3_판타지"
        elif ratios["3_판타지"] + ratios["3_여성향"] >= 70.0 and ratios["3_판타지"] >= 20.0 and ratios["3_여성향"] >= 20.0 and scores["3_판타지"] >= 2 and scores["3_여성향"] >= 2 and (scores["3_판타지"] + scores["3_여성향"]) >= 5:
            cluster = "하이브리드_판타지_로판"
        elif ratios["3_판타지"] + ratios["9_성인"] >= 70.0 and ratios["3_판타지"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["3_판타지"] >= 2 and scores["9_성인"] >= 2 and (scores["3_판타지"] + scores["9_성인"]) >= 5:
            cluster = "하이브리드_판타지_성인"
        elif ratios["3_여성향"] + ratios["9_성인"] >= 70.0 and ratios["3_여성향"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["3_여성향"] >= 2 and scores["9_성인"] >= 2 and (scores["3_여성향"] + scores["9_성인"]) >= 5:
            cluster = "하이브리드_로맨스_성인"
        elif ratios["3_무협"] + ratios["9_성인"] >= 70.0 and ratios["3_무협"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["3_무협"] >= 2 and scores["9_성인"] >= 2 and (scores["3_무협"] + scores["9_성인"]) >= 5:
            cluster = "하이브리드_무협_성인"
        elif ratios["9_BLGL"] + ratios["9_성인"] >= 70.0 and ratios["9_BLGL"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["9_BLGL"] >= 2 and scores["9_성인"] >= 2 and (scores["9_BLGL"] + scores["9_성인"]) >= 5:
            cluster = "하이브리드_BL_성인"
        elif ratios["9_BLGL"] + ratios["3_판타지"] >= 70.0 and ratios["9_BLGL"] >= 20.0 and ratios["3_판타지"] >= 20.0 and scores["9_BLGL"] >= 2 and scores["3_판타지"] >= 2 and (scores["9_BLGL"] + scores["3_판타지"]) >= 5:
            cluster = "하이브리드_BL_판타지"
        else:
            # 60% 이상 및 점수 4점 이상이면 준확실 (노이즈 3점 이하는 미분류/보류)
            for cat, r in ratios.items():
                if r >= 60.0 and scores[cat] >= 4:
                    cluster = f"준확실_{cat}"
                    target_cat = cat
                    break

    return {
        "valid": True,
        "scores": scores,
        "ratios": ratios,
        "total_score": total_score,
        "cluster": cluster,
        "target_cat": target_cat,
    }


def classify_5_genres_from_content(title: str, text: str) -> Tuple[Optional[str], int, str]:
    """
    도서 제목과 본문 처음/끝 500단어(총 1000단어)를 정밀 분석하여
    3_무협, 3_판타지, 3_여성향, 9_BLGL, 9_성인 5대 장르로 결정론적 분류.
    - 무협-판타지 하이브리드(판타지 어휘 유의미 포함)는 3_판타지로 분류
    - 3_무협은 판타지 요소가 없는 순수 무협만 분류
    """
    res = calculate_5_genre_scores_and_ratios(title, text)
    if not res.get("valid"):
        return None, 0, res.get("reason", "empty_or_invalid")

    scores = res["scores"]
    target_cat = res.get("target_cat")
    cluster = res.get("cluster", "")

    # 1. 클러스터에서 명확히 결정된 카테고리가 있으면 우선 반환
    if target_cat:
        score = scores.get(target_cat, 0)
        reason = f"cluster={cluster}"
        if cluster == "하이브리드_무협_판타지":
            reason += f":hybrid_wuxia_fantasy(w={scores.get('3_무협', 0)},f={scores.get('3_판타지', 0)}) -> 3_판타지"
        elif cluster == "확실한_3_무협":
            reason += f":pure_wuxia(w={scores.get('3_무협', 0)},f={scores.get('3_판타지', 0)})"
        return target_cat, score, reason

    # 2. 중재 우선순위 폴백
    bl_score = scores.get("9_BLGL", 0)
    rofan_score = scores.get("3_여성향", 0)
    wuxia_score = scores.get("3_무협", 0)
    fantasy_score = scores.get("3_판타지", 0)
    adult_score = scores.get("9_성인", 0)

    if bl_score >= 3 and bl_score >= adult_score * 0.5:
        return "9_BLGL", bl_score, f"bl_score={bl_score}"
    if rofan_score >= 4 and rofan_score >= adult_score * 0.4:
        return "3_여성향", rofan_score, f"rofan_score={rofan_score}"

    # 무협 vs 판타지 하이브리드 판정 규칙:
    # 3_무협은 순수 무협만 남기며, 판타지 어휘가 2건 이상 포함된 하이브리드는 3_판타지로 분류
    if wuxia_score >= 4 and fantasy_score >= 2:
        return "3_판타지", fantasy_score, f"hybrid_wuxia_fantasy(w={wuxia_score},f={fantasy_score}) -> 3_판타지"
    if wuxia_score >= 4 and fantasy_score < 2:
        return "3_무협", wuxia_score, f"pure_wuxia(w={wuxia_score},f={fantasy_score})"
    if fantasy_score >= 4:
        return "3_판타지", fantasy_score, f"fantasy_score={fantasy_score}"

    if adult_score >= 5:
        return "9_성인", adult_score, f"adult_score={adult_score}"

    return None, 0, "no_genre_matched"


def score_text_genre(text: str) -> Optional[str]:
    """기존 호환용 본문 키워드 스코어링"""
    cleaned_text = clean_disclaimer_and_colophon(text)
    cat, score, _ = classify_5_genres_from_content("", cleaned_text)
    if cat:
        return cat
    if not cleaned_text:
        return None
    t = cleaned_text.lower()
    scores = {
        "3_SF": sum(1 for kw in ["우주선", "안드로이드", "인공지능", "사이보그", "외계인", "행성", "타임머신", "디스토피아"] if kw in t),
        "3_스릴러": sum(1 for kw in ["살인사건", "연쇄살인", "형사", "수사관", "시체", "밀실", "트릭", "용의자", "알리바이", "탐정"] if kw in t),
    }
    sorted_scores = sorted(scores.items(), key=lambda x: x[1], reverse=True)
    best_cat, best_score = sorted_scores[0]
    second_score = sorted_scores[1][1]
    if best_score >= 2 and best_score > second_score:
        return best_cat
    return None


def resolve_genre_conflict(cats: List[str], fname: str, text_sample: str = "") -> Optional[str]:
    """서점 간 사소한 장르 차이(특히 3_* 계열) 충돌 시 파일명과 본문 키워드로 보조 판단"""
    valid_cats = set(cats)
    fname_lower = fname.lower()
    text_lower = (fname + " " + text_sample).lower()

    # 1. 무협 vs 판타지 (3_무협은 순수무협만 유지, 하이브리드는 3_판타지 분류)
    if "3_무협" in valid_cats and "3_판타지" in valid_cats:
        wuxia_kws = ["문파", "화산", "마교", "소교주", "맹주", "무림", "장문인", "절기", "검법", "도법", "내공", "심법", "소림", "무당", "개방", "사파", "정파", "강호", "천마"]
        fantasy_kws = ["던전", "헌터", "각성", "마나", "마법", "길드", "몬스터", "레이드", "시스템", "퀘스트", "플레이어", "용사", "드래곤", "아카데미", "스킬", "스탯", "이세계", "재벌"]
        w_score = sum(1 for kw in wuxia_kws if kw in text_lower)
        f_score = sum(1 for kw in fantasy_kws if kw in text_lower)
        # 판타지 키워드가 1회 이상 검출되면 하이브리드로 판단하여 3_판타지 분류
        if f_score >= 1:
            return "3_판타지"
        if w_score >= 1:
            return "3_무협"
        return "3_무협" if "무협" in fname_lower else "3_판타지"

    # 2. 그래픽노블 vs 라이트노벨
    if "3_그래픽노블" in valid_cats and "3_라이트노벨" in valid_cats:
        ext = Path(fname).suffix.lower()
        if ext in [".txt", ".epub"]:
            return "3_라이트노벨"
        return "3_그래픽노블"

    # 3. 그래픽노블 vs 판타지
    if "3_그래픽노블" in valid_cats and "3_판타지" in valid_cats:
        ext = Path(fname).suffix.lower()
        if ext in [".txt", ".epub"]:
            return "3_판타지"
        return "3_그래픽노블"

    # 4. 판타지 vs 여성향 (로맨스판타지)
    if "3_판타지" in valid_cats and "3_여성향" in valid_cats:
        rofan_kws = ["영애", "공작", "황태자", "남주", "여주", "시월드", "파혼", "후회남", "집착", "황후", "악녀", "빙의", "로판", "로맨스"]
        if any(kw in text_lower for kw in rofan_kws):
            return "3_여성향"
        return "3_판타지"

    # 5. 그래픽노블 vs 여성향
    if "3_그래픽노블" in valid_cats and "3_여성향" in valid_cats:
        ext = Path(fname).suffix.lower()
        if ext in [".txt", ".epub"]:
            return "3_여성향"
        return "3_그래픽노블"

    # 6. 라이트노벨 vs 여성향 (TL, 여성향 라노벨, 로맨스)
    if "3_라이트노벨" in valid_cats and "3_여성향" in valid_cats:
        rofan_kws = ["영애", "공작", "황태자", "남주", "여주", "시월드", "파혼", "후회남", "집착", "황후", "악녀", "빙의", "로판", "로맨스", "하숙생", "선생님", "선배"]
        if any(kw in text_lower for kw in rofan_kws):
            return "3_여성향"
        return "3_라이트노벨"

    # 7. 라이트노벨 vs 판타지
    if "3_라이트노벨" in valid_cats and "3_판타지" in valid_cats:
        ln_kws = ["라노벨", "라이트노벨", "이세계", "마왕", "용사", "슬라임", "전생", "히로인"]
        if any(kw in text_lower for kw in ln_kws):
            return "3_라이트노벨"
        return "3_판타지"

    # 8. 소설한국 vs 수필서간일기
    if "2_소설한국" in valid_cats and "2_수필서간일기" in valid_cats:
        if any(kw in text_lower for kw in ["에세이", "일기", "산문"]):
            return "2_수필서간일기"
        return "2_소설한국"

    # 9. 소설역사 vs 역사인물 / 경제
    if "2_소설역사" in valid_cats and any(c in valid_cats for c in ["4_역사인물", "4_경제"]):
        if any(kw in text_lower for kw in ["소설", "대하", "야사", "열전"]):
            return "2_소설역사"

    # 10. 소설한국 vs 역사인물
    if "2_소설한국" in valid_cats and "4_역사인물" in valid_cats:
        if any(kw in text_lower for kw in ["소설", "이야기", "단편", "전집", "문학"]):
            return "2_소설한국"

    # 11. 처세술 vs 경영/마케팅/경제
    if "6_처세술리더십창의성" in valid_cats and any(c in valid_cats for c in ["4_경영마케팅", "4_경제"]):
        if any(kw in text_lower for kw in ["성공", "처세", "습관", "인간관계", "대화법", "시간관리"]):
            return "6_처세술리더십창의성"

    # 12. 소설외국 vs 스릴러
    if "2_소설외국" in valid_cats and "3_스릴러" in valid_cats:
        thriller_kws = ["추리", "미스터리", "스릴러", "살인", "탐정", "형사", "사건", "범인", "시체", "밀실", "수사", "경감", "셜록"]
        if any(kw in text_lower or kw in fname_lower for kw in thriller_kws):
            return "3_스릴러"
        return "2_소설외국"

    # 13. 여성향 vs BLGL
    if "3_여성향" in valid_cats and "9_BLGL" in valid_cats:
        bl_kws = ["bl", "백합", "오메가버스", "알파오메가", "공수", "광공", "다정공", "미인수", "임신수", "비엘"]
        rofan_kws = ["로맨스", "로판", "여주", "남주", "황태자", "공작", "영애", "시월드"]
        if any(kw in text_lower or kw in fname_lower for kw in bl_kws):
            return "9_BLGL"
        if any(kw in text_lower or kw in fname_lower for kw in rofan_kws):
            return "3_여성향"

    # 14. 소설한국 vs 스릴러
    if "2_소설한국" in valid_cats and "3_스릴러" in valid_cats:
        thriller_kws = ["추리", "미스터리", "스릴러", "살인", "탐정", "형사", "사건", "범인", "수사"]
        if any(kw in text_lower or kw in fname_lower for kw in thriller_kws):
            return "3_스릴러"
        return "2_소설한국"

    # 15. 소설외국 vs 판타지
    if "2_소설외국" in valid_cats and "3_판타지" in valid_cats:
        fantasy_kws = ["판타지", "마법", "드래곤", "엘프", "던전", "마왕", "용사", "이세계"]
        if any(kw in text_lower or kw in fname_lower for kw in fantasy_kws):
            return "3_판타지"
        return "2_소설외국"

    # 16. 소설외국 vs 여성향
    if "2_소설외국" in valid_cats and "3_여성향" in valid_cats:
        ro_kws = ["로맨스", "로판", "사랑", "연애", "신부", "귀부인"]
        if any(kw in text_lower or kw in fname_lower for kw in ro_kws):
            return "3_여성향"
        return "2_소설외국"

    return None


def evaluate_category_decision(
    fname: str,
    fpath: Optional[Path],
    raw_title: str,
    raw_author: str,
    search_title: str,
    y_entry: Dict[str, Any],
    a_entry: Dict[str, Any],
    k_entry: Dict[str, Any],
    trust_single_match: bool = True
) -> Tuple[Optional[str], str, str]:
    """
    파일명, 3개 서점 결과, 파일 내부 메타데이터/텍스트를 결합하여 최종 카테고리 결정
    """
    # 1. 파일명 명시적 장르
    explicit_genre = extract_explicit_genre(fname)

    y_map = y_entry.get("mapped") if y_entry else None
    a_map = a_entry.get("mapped") if a_entry else None
    k_map = k_entry.get("mapped") if k_entry else None
    valid_maps = [m for m in [y_map, a_map, k_map] if m]
    from collections import Counter
    counts = Counter(valid_maps)

    # Priority 1: 파일명 명시적 장르가 존재하는 경우
    # 서점 다수결(>=2)이 명시적 장르와 정면 배치되지 않는 한 파일명의 명시적 장르를 최우선 채택
    if explicit_genre:
        opposing_majority = any(cnt >= 2 and cat != explicit_genre for cat, cnt in counts.items())
        if not opposing_majority:
            return explicit_genre, "explicit_genre", f"Explicit genre in filename -> {explicit_genre}"

    # Priority 2: 3개 서점 2/3 이상 다수결
    for cat, count in counts.items():
        if count >= 2:
            return cat, "majority", f"Majority vote ({count}/3) -> {cat}"

    # 본문 및 메타데이터 샘플 추출
    text_sample = ""
    epub_meta: Dict[str, str] = {}
    txt_info: Dict[str, Any] = {}
    if fpath and fpath.exists():
        ext = fpath.suffix.lower()
        if ext == ".epub":
            epub_meta = inspect_epub_metadata(fpath)
            text_sample = f"{epub_meta.get('title', '')} {epub_meta.get('subject', '')} {epub_meta.get('description', '')}"
        elif ext == ".txt":
            txt_info = inspect_txt_content(fpath)
            text_sample = f"{' '.join(txt_info.get('hashtags', []))} {txt_info.get('snippet', '')}"

    # Priority 2.5: 서점 간 충돌 중 유효 제목 매칭 필터링 (False Conflict 해소)
    # 한 서점은 정확한 책을 찾았으나 다른 서점이 엉뚱한 추천도서를 반환하여 발생한 거짓 충돌 해소
    if len(valid_maps) >= 2:
        valid_title_candidates = []
        for store_entry in [y_entry, a_entry, k_entry]:
            if not store_entry or not store_entry.get("mapped"):
                continue
            f_title = store_entry.get("title", "")
            if is_title_match_reliable(search_title, f_title):
                valid_title_candidates.append((store_entry.get("mapped"), f_title))

        if len(valid_title_candidates) == 1:
            m_cat, m_title = valid_title_candidates[0]
            return m_cat, "single_valid_match_resolved", f"False conflict resolved by title similarity ({m_cat}) for '{m_title[:30]}'"

    # Priority 3: 서점 간 사소한 장르 충돌 해결
    if len(valid_maps) >= 2:
        resolved = resolve_genre_conflict(valid_maps, fname, text_sample)
        if resolved:
            return resolved, "conflict_resolved", f"Conflict resolved by keywords -> {resolved}"

    # Priority 4: 단일 서점 매칭 신뢰
    if len(valid_maps) == 1 and trust_single_match:
        single_cat = valid_maps[0]
        found_title = ""
        for store_entry in [y_entry, a_entry, k_entry]:
            if store_entry and store_entry.get("mapped") == single_cat:
                found_title = store_entry.get("title", "")
                break
        if is_single_match_valid(search_title, found_title):
            return single_cat, "single_match", f"Single match trusted ({single_cat}) for '{found_title[:30]}'"

    # Priority 5: 파일 내부 메타데이터 및 처음 1000단어 본문 정밀 장르 분석 활용
    if fpath and fpath.exists():
        ext = fpath.suffix.lower()
        if ext == ".epub" and epub_meta:
            subj = epub_meta.get("subject", "")
            if subj:
                meta_cat = map_category(subj, raw_title, raw_author)
                if meta_cat:
                    return meta_cat, "content_metadata", f"EPUB dc:subject -> {meta_cat}"
            desc = epub_meta.get("description", "")
            if desc:
                desc_cat = map_category(desc, raw_title, raw_author)
                if desc_cat:
                    return desc_cat, "content_metadata", f"EPUB dc:description -> {desc_cat}"

        elif ext == ".txt" and txt_info:
            hg = txt_info.get("header_genre")
            if hg:
                hg_cat = extract_explicit_genre(f"[{hg}]") or map_category(hg, raw_title, raw_author)
                if hg_cat:
                    return hg_cat, "content_metadata", f"TXT header [{hg}] -> {hg_cat}"
            for tag in txt_info.get("hashtags", []):
                tag_cat = map_category(tag, raw_title, raw_author) or extract_explicit_genre(f"[{tag}]")
                if tag_cat:
                    return tag_cat, "content_metadata", f"TXT #{tag} -> {tag_cat}"

        # 본문 처음 1000단어 5대 장르(3_무협, 3_판타지, 3_여성향, 9_BLGL, 9_성인) 정밀 스코어링
        first_1000 = extract_content_first_1000_words(fpath)
        scoring_text = (first_1000 + " " + text_sample).strip()
        g5_cat, g5_score, g5_reason = classify_5_genres_from_content(fname, scoring_text)
        if g5_cat:
            prefix = "EPUB content scored" if ext == ".epub" else ("TXT content scored" if ext == ".txt" else "Content scored")
            return g5_cat, "content_metadata", f"{prefix} -> {g5_cat} ({g5_reason})"

        # 기존 일반 장르(3_SF, 3_스릴러 등) 스코어링 폴백
        sc_cat = score_text_genre(text_sample or first_1000)
        if sc_cat:
            return sc_cat, "content_metadata", f"Content scored -> {sc_cat}"

    # 최종 미해결 상태
    if len(valid_maps) >= 2:
        return None, "conflict", f"Conflict: Y:{y_map} vs A:{a_map} vs K:{k_map}"
    elif len(valid_maps) == 1:
        return None, "single_match", f"Single match (untrusted): {valid_maps[0]}"
    else:
        return None, "not_found", "Not found"


def clean_filename_to_author_title(filename: str) -> Tuple[str, str, str]:
    # URL 인코딩 해제 (예: %5B로맨스판타지%5D -> [로맨스판타지])
    filename = urllib.parse.unquote(filename)
    stem = os.path.splitext(filename)[0]
    stem = re.sub(r"\s*\([^)]*z-lib[^)]*\)", "", stem, flags=re.I)
    stem = re.sub(r"\s*\([^)]*1lib[^)]*\)", "", stem, flags=re.I)

    # 방통위/검열 우회용 문자 사이 점 제거: 예: 신.화.급 -> 신화급
    stem = re.sub(r"(?<=[가-힣a-zA-Z0-9])\.(?=[가-힣a-zA-Z0-9])", "", stem)
    stem = stem.replace("_", " ").strip()

    # 해시태그 및 특수 기호 접두어 제거 (#나무, #공금, ★공금, (019), (19) 등)
    stem = re.sub(r"^[#★■◆●▲\s]*(?:나무|공금|숲|텍본|완결|스캔|TXT|txt|EPUB|epub)[\s]*", "", stem)
    stem = re.sub(r"^#\S+\s*", "", stem)
    stem = re.sub(r"^[\s\(\[【]*(?:019|19|19금|성인)[\s\)\]】]*", "", stem)
    stem = re.sub(r"^[\s\(\[【]*(?:完|완|완결|외전|텍본|스캔|TXT|txt)[\s\)\]】]*", "", stem)
    stem = re.sub(r"^[\s\(\[【]*(?:현대\s*판타지|판타지|무협|로판|로맨스|현판|퓨판|SF)[\s\)\]】]*", "", stem)

    # 1. @author 패턴 처리
    author_from_at = ""
    at_match = re.search(r"@([^\s\(\)\[\]\-]+)", stem)
    if at_match:
        author_from_at = at_match.group(1).strip()
        stem = re.sub(r"@[^\s\(\)\[\]\-]+(?:\([^\)]*\))?", "", stem).strip()

    # 2. 뒤쪽에 붙은 저자 패턴 (예: 제목 1-300 완[]설봉, 제목 [저자], 제목-저자, 제목 (저자))
    end_author_match = re.search(r"\[(?:\s*|저자:?|글:?)\]?\s*([가-힣a-zA-Z]{2,10})\s*\]?\s*$", stem)
    if end_author_match and not author_from_at:
        author_from_at = end_author_match.group(1).strip()
        stem = re.sub(r"\[(?:\s*|저자:?|글:?)\]?\s*[가-힣a-zA-Z]{2,10}\s*\]?\s*$", "", stem).strip()

    end_paren_match = re.search(r"\((?:\s*|저자:?|글:?)?\s*([가-힣a-zA-Z]{2,10})\s*\)\s*$", stem)
    if end_paren_match and not author_from_at:
        cand = end_paren_match.group(1).strip()
        if not re.search(r"(완결|완|개정판|외전|단편|1부|2부|판타지|무협|로판|스릴러)", cand):
            author_from_at = cand
            stem = re.sub(r"\((?:\s*|저자:?|글:?)?\s*[가-힣a-zA-Z]{2,10}\s*\)\s*$", "", stem).strip()

    end_dash_author = re.search(r"[-－]\s*([가-힣a-zA-Z]{2,4})\s*$", stem)
    if end_dash_author and not author_from_at:
        cand = end_dash_author.group(1).strip()
        if not re.search(r"(완결|완|개정판|외전|단편|1부|2부|판타지|무협|로판|스릴러|텍본)", cand):
            author_from_at = cand
            stem = re.sub(r"[-－]\s*[가-힣a-zA-Z]{2,4}\s*$", "", stem).strip()

    # 3. [저자] 제목 패턴
    m = re.search(r"^\[(?P<author>[^\]]+)\]\s*(?P<title>.+)$", stem)
    if m:
        raw_author = m.group("author").strip()
        raw_title = m.group("title").strip()
    else:
        # 저자 - 제목 패턴 확인
        dash_m = re.match(r"^([가-힣]{2,4})\s*[-－]\s*(.+)$", stem)
        if dash_m and not author_from_at:
            raw_author = dash_m.group(1).strip()
            raw_title = dash_m.group(2).strip()
        else:
            raw_author = author_from_at
            raw_title = stem.strip()

    # 저자 정제
    author = raw_author
    author = re.sub(r"\s*(저|역|지음|옮김|펴냄|엮음|글|그림|원작)\b.*$", "", author)
    author = re.sub(r"\s*외\s*\d*명?$", "", author)
    author = author.strip()

    # 제목 정제
    search_title = raw_title
    search_title = re.sub(r"^\[[^\]]*\]\s*", "", search_title)

    for sep in [" - ", "－", " : "]:
        if sep in search_title:
            search_title = search_title.split(sep, 1)[0].strip()

    # 접두/접미 완결 표기 및 권수 정제
    search_title = re.sub(r"^[#\s]+", "", search_title)
    search_title = re.sub(r"^[\s\(\[【]*(?:完|완|완결|외전|텍본|스캔|TXT|txt)[\s\)\]】]*", "", search_title)
    search_title = re.sub(r"\(개정판\)", "", search_title)
    search_title = re.sub(r"\([^\)]*완[^\)]*\)", "", search_title)
    search_title = re.sub(r"\([^\)]*로판[^\)]*\)", "", search_title)
    search_title = re.sub(r"\(19\)", "", search_title)
    search_title = re.sub(r"\[[0-9~]+부\]", "", search_title)
    search_title = re.sub(r"\s+\d+\s+(?:완|完|외전|특외|에필|후기|미완|완결|완외포|완외|붙음).*", "", search_title)
    search_title = re.sub(r"\s+\d+(?:화|권|부|완|完|외|외전|특외|에필|후기|미완|완결|완외포)\b.*", "", search_title)
    search_title = re.sub(r"\s+\d{1,4}\s+\d{1,4}.*", "", search_title)
    search_title = re.sub(r"[\s,]+(?:1[-~]\d+|\d+[-~]\d+.*|\d+[-~]\d+권.*|\d+화.*|\d+권.*|\d+부.*|\b完\b.*|\b완\b.*|\b외전\b.*|\b특외\b.*|\b에필\b.*|\b후기\b.*|\b완외포\b.*|\b완외\b.*|\b완결\b.*|\b단권\b.*|\b미완\b.*)$", "", search_title)
    search_title = re.sub(r"\s*\b(?:issue|vol|v)\.?\s*\d+\b.*", "", search_title, flags=re.I)
    search_title = re.sub(r"\s*\b\d{1,3}\b$", "", search_title)  # 끝의 1, 2, 01, 02 등
    search_title = re.sub(r"\s*\([^)]*\)", "", search_title).strip()
    search_title = re.sub(r"\s*\[[^\]]*\]", "", search_title).strip()
    search_title = re.sub(r"^\s*[-~=,.]+\s*", "", search_title).strip()
    search_title = re.sub(r"\s*[-~=,.]+\s*$", "", search_title).strip()

    return author, raw_title, search_title


def map_category(cat_str: str, raw_title: str, raw_author: str) -> Optional[str]:
    if not cat_str and not raw_title and not raw_author:
        return None

    cat = cat_str or ""
    parts = [p.strip() for p in cat.split(">")] if cat else []
    leaf_cat = " > ".join(parts[1:]) if len(parts) > 1 else cat

    # 1. 저자/시리즈 고유 규칙
    if "히가시노 게이고" in raw_author or "히가시노게이고" in raw_author:
        return "2_소설일본게이고"
    if "무라카미 하루키" in raw_author or "무라카미하루키" in raw_author:
        return "2_소설일본하루키"
    if "을유세계문학" in raw_title or "을유세계문학" in cat:
        return "2_을유세계문학전집"
    if "열린책들 세계문학" in raw_title or "열린책들세계문학" in raw_title or "열린책들" in raw_title:
        return "2_열린책들세계문학"
    if "문예세계문학선" in raw_title:
        return "2_문예세계문학선"
    if "살림지식총서" in raw_title:
        return "4_살림지식총서"
    if "시공디스커버리" in raw_title:
        return "4_시공디스커버리"
    if "올재 클래식스" in raw_title or "(올재" in raw_title or "올재" in raw_title:
        return "1_올재"
    if "누워서 읽는 법학" in raw_title or "누워서읽는법학" in raw_title:
        return "4_누워서읽는법학"
    if "이지사이언스" in raw_title:
        return "5_이지사이언스"
    if "그림으로 읽는" in raw_title or "그림으로읽는" in raw_title:
        return "5_그림으로_읽는"
    if any(k in raw_title for k in ["손자병법", "육도, 삼략", "육도삼략", "한비자", "장자", "채근담", "논어", "맹자"]):
        return "1_동양고전"

    # 2. 카테고리 매핑 규칙
    if any(k in cat for k in ["라이트노벨", "라이트 노벨", "라노벨"]):
        return "3_라이트노벨"
    if any(k in cat for k in ["BL", "GL", "야오이", "백합", "BL/GL", "퀴어"]):
        return "9_BLGL"
    if any(k in cat for k in ["그래픽노블", "그래픽 노블", "만화/코믹", "코믹스", "웹툰", "만화"]):
        return "3_그래픽노블"

    if any(k in cat for k in ["추리", "미스터리", "스릴러", "서스펜스", "공포소설", "호러", "하드보일드"]):
        return "3_스릴러"
    if any(k in cat for k in ["과학소설", "사이버펑크"]) or ("SF" in cat and "SF" in leaf_cat):
        return "3_SF"
    if any(k in cat for k in ["판타지소설", "판타지", "현대판타지", "게임소설", "퓨전판타지", "현대", "퓨전"]):
        if "SF" not in cat and "로맨스" not in cat and "로맨틱판타지" not in cat:
            return "3_판타지"
    if any(k in cat for k in ["무협", "신무협", "무협소설"]):
        return "3_무협"
    if any(k in cat for k in ["로맨스", "로판", "로맨스판타지", "로맨틱판타지", "사랑소설", "연애소설", "칙릿"]):
        return "3_여성향"

    if any(k in cat for k in ["에세이", "수필", "일기", "서간", "산문집"]):
        return "2_수필서간일기"
    if any(k in cat for k in ["한국소설", "한국단편", "한국장편", "한국 문학", "한국문학"]):
        return "2_소설한국"
    if any(k in cat for k in ["일본소설", "일본문학", "일본장편", "일본단편"]):
        return "2_소설일본"
    if any(k in cat for k in ["중국소설", "중국문학", "대만소설"]):
        return "2_소설중국"
    if any(k in cat for k in [
        "영미소설", "영국소설", "미국소설", "프랑스소설", "독일소설", "러시아소설",
        "유럽소설", "스페인소설", "남미소설", "외국소설", "세계의 소설", "각국소설", "북유럽소설", "고전문학"
    ]):
        return "2_소설외국"
    if any(k in cat for k in ["역사소설", "대하역사소설", "대하소설"]):
        return "2_소설역사"
    if any(k in cat for k in ["시집", "한국시", "외국시", "현대시", "희곡", "시/희곡"]):
        if "소설" not in cat and "에세이" not in cat:
            return "2_시"
    if any(k in cat for k in ["독서 에세이", "글쓰기", "작법", "문학비평", "문학이론", "서평", "책읽기", "문학의 이해", "독서법"]):
        return "2_문학일반서평작법독서"
    if any(k in cat for k in ["문헌정보", "서지학", "기록관리", "문헌학"]):
        return "1_문헌서지"
    if any(k in cat for k in ["동양고전", "사서삼경", "제자백가", "한문학", "한문"]):
        return "1_동양고전"
    if any(k in cat for k in ["서양고전", "그리스로마", "그리스 로마"]):
        return "1_서양고전"

    if any(k in cat for k in ["투자", "재테크", "주식", "증권", "부동산", "경매", "가상화폐", "비트코인", "자산관리", "금융상품", "펀드", "연금", "환테크", "청약"]):
        return "6_재테크"
    if any(k in leaf_cat for k in ["경제학", "경제일반", "경제전망", "경제사", "각국 경제", "금융/화폐", "국제경제", "화폐", "거시경제", "미시경제", "행동경제학", "경제"]):
        return "4_경제"
    if any(k in cat for k in ["경제학", "경제사", "각국 경제", "거시경제", "미시경제", "행동경제학"]):
        return "4_경제"
    if any(k in cat for k in ["마케팅", "브랜딩", "광고", "창업", "스타트업", "경영전략", "경영일반", "비즈니스", "조직관리", "e-비즈니스", "기획", "세일즈", "CEO"]):
        return "4_경영마케팅"
    if any(k in leaf_cat for k in ["경영"]):
        return "4_경영마케팅"

    if any(k in cat for k in ["자기계발", "성공/처세", "처세술", "인간관계", "화술", "대화법", "시간관리", "습관", "동기부여", "창의성", "성공학", "마인드셋", "리더십"]):
        return "6_처세술리더십창의성"

    if any(k in cat for k in ["심리", "심리학", "정신분석", "뇌과학", "상담심리", "임상심리", "인지심리"]):
        return "4_심리학뇌과학"
    if any(k in cat for k in ["철학", "서양철학", "동양철학", "윤리학", "사상", "현대철학"]):
        return "4_철학윤리"
    if any(k in cat for k in ["한국사", "동양사", "서양사", "세계사", "역사인물", "역사학", "평전", "전기", "조선사", "고려사", "근현대사", "역사", "조선시대"]):
        return "4_역사인물"
    if any(k in cat for k in ["정치", "외교", "군사", "국제정치", "안보", "통일", "국방", "정치학"]):
        return "4_정치외교군사"
    if any(k in cat for k in ["법학", "법률", "헌법", "형법", "민법", "소송", "행정/법률"]):
        return "4_법"
    if any(k in cat for k in ["사회학", "사회문제", "언론", "미디어", "인류학", "문화인류", "여성학", "페미니즘", "노동", "사회과학"]):
        return "4_사회인류"
    if any(k in cat for k in ["종교", "기독교", "불교", "가톨릭", "천주교", "이슬람", "성경", "신앙", "신화"]):
        return "4_종교신화"
    if any(k in cat for k in ["인문학", "교양인문", "인문일반", "인문", "논픽션"]):
        return "4_인문일반논픽션"

    if any(k in cat for k in ["수학", "물리학", "화학", "생물학", "생명과학", "지구과학", "천문학", "자연과학", "교양과학", "기초과학", "과학"]):
        return "5_수학과학일반"

    # 구체적인 복합 카테고리를 먼저 판정한다. 아래의 broad keyword(회화/국어/교육/의학/영어)
    # 보다 늦게 검사하면 substring match 때문에 다른 분야로 오분류된다.
    if any(k in cat for k in ["영어원서", "영문원서", "영한대역"]):
        return "7_영문일반"
    if any(k in cat for k in ["영어회화", "토익", "토플", "수능영어", "영단어"]):
        return "7_영어교육"
    if any(k in cat for k in ["중국어", "HSK", "한자"]):
        return "7_중어한자교육"
    if any(k in cat for k in ["성교육", "성의학"]):
        return "8_성"

    if any(k in cat for k in ["미술", "예술", "건축", "디자인", "공예", "조각", "회화", "도예"]):
        return "5_미술예술건축"
    if any(k in cat for k in ["음악", "클래식음악", "대중음악", "작곡", "악기", "가요", "뮤지컬"]):
        return "5_음악"
    if any(k in cat for k in ["영화", "시나리오", "각본", "영화사", "드라마 대본"]):
        return "5_영화"
    if any(k in cat for k in ["사진", "영상제작", "카메라", "영상편집"]):
        return "5_사진영상"
    if any(k in cat for k in ["스포츠", "골프", "축구", "야구", "마라톤", "헬스", "피트니스", "수영", "등산", "운동"]):
        return "5_스포츠"
    if any(k in cat for k in ["서브컬쳐", "캐릭터 드로잉", "일러스트", "만화작법"]):
        return "5_서브컬쳐"

    if any(k in cat for k in ["교육학", "학습법", "공부법", "독서지도", "교육"]):
        return "7_교육일반"
    if any(k in cat for k in ["국어", "한국어", "맞춤법", "어휘"]):
        return "7_국어교육"
    if any(k in cat for k in ["영어", "영문법"]):
        return "7_영어교육"
    if any(k in cat for k in ["일본어", "일어", "JLPT"]):
        return "7_일어교육"
    if any(k in cat for k in ["외국어", "프랑스어", "독일어", "스페인어", "러시아어"]):
        return "7_외국어교육"
    if any(k in cat for k in ["언어학", "번역", "통역"]):
        return "7_언어일반"

    if any(k in cat for k in ["컴퓨터", "IT", "프로그래밍", "소프트웨어", "코딩", "인공지능", "AI", "빅데이터", "네트워크", "모바일/태블릿", "웹개발", "데이터베이스"]):
        return "8_IT"

    if any(k in cat for k in ["건강", "의학", "질병", "치료", "한의학", "다이어트", "영양", "수면", "면역", "당뇨", "치매"]):
        return "8_건강일반"
    if any(k in cat for k in ["요리", "베이킹", "레시피", "와인", "커피", "음료", "디저트", "주류"]):
        return "8_요리음료"
    if any(k in cat for k in ["여행", "국내여행", "해외여행", "여행에세이", "가이드북"]):
        return "8_여행"
    if any(k in cat for k in ["공인중개사"]):
        return "8_공인중개사"
    if any(k in cat for k in ["프라모델", "모형", "모델링"]):
        return "8_모델링"
    if any(k in cat for k in ["밀리터리", "무기", "군사무기"]):
        return "8_밀리터리"
    if any(k in cat for k in ["살림", "인테리어", "수예", "뜨개질", "원예", "반려동물", "가정살림", "가사"]):
        return "8_실용의학회계"
    if any(k in cat for k in ["악보", "스코어", "Songbook"]):
        return "8_악보"

    if any(k in cat for k in ["어린이", "유아", "그림책", "동화책", "동화", "육아", "자녀교육", "부모"]):
        return "9_어린이육아"
    if any(k in cat for k in ["청소년", "청소년문학", "청소년교양", "청소년 인문"]):
        return "9_청소년"
    if any(k in cat for k in ["사주", "타로", "점성술", "풍수", "해몽", "퍼즐", "스도쿠", "바둑"]):
        return "9_역학해몽퍼즐"
    if any(k in cat for k in ["유머", "개그", "만담"]):
        return "9_유머"
    if any(k in cat for k in ["명언", "격언", "아포리즘"]):
        return "9_격언명언"

    if "경제" in cat:
        return "4_경제"


def get_effective_filename(fpath: Path, target_dir: Path) -> str:
    """
    서브디렉토리 내 파일명이 단순 번호나 권수(예: 01.txt, 1권.epub)인 경우
    부모 디렉토리명(작품명)을 결합하여 고유하고 온전한 도서 파일명을 생성.
    또한 부모 디렉토리에 [저자]나 @저자가 명시되어 있고 파일명에 없으면 부모의 저자 정보를 상속 결합.
    """
    fname = fpath.name
    if fpath.parent == target_dir:
        return fname
    parent_name = fpath.parent.name
    stem = fpath.stem.strip()
    is_short = re.match(r"^(\d+|[0-9~]+권|[0-9~]+화|[0-9~]+부|\bvol\.?\s*\d+|\bv\d+)$", stem, re.IGNORECASE) or len(stem) <= 3
    if is_short:
        return f"{parent_name} {fname}"

    parent_m = re.search(r"(\[[^\]]+\]|@[^\s]+)", parent_name)
    file_m = re.search(r"(\[[^\]]+\]|@[^\s]+)", fname)
    if parent_m and not file_m:
        author_tag = parent_m.group(1)
        return f"{author_tag} {fname}"

    return fname


def clean_empty_parent_dirs(parent_dir: Path, stop_dir: Path):
    """파일 이동/삭제 후 비어 있는 상위 서브디렉토리를 안전하게 재귀 청소"""
    curr = parent_dir
    while curr != stop_dir and curr.exists() and curr.is_dir():
        items = [p for p in curr.iterdir() if not p.name.startswith(".")]
        if not items:
            try:
                curr.rmdir()
                curr = curr.parent
            except Exception:
                break
        else:
            break



class BookClassifierService:
    """
    도서 자동 분류 통합 서비스 클래스
    - 3대 서점(Yes24, 알라딘, 교보) 크롤링 & 다수결 (1번)
    - EPUB 메타데이터 & TXT 본문 분석 (2번)
    - 캐시 영구화 및 중복 정리/안전 이동
    """

    def __init__(
        self,
        library_root: Path | str = "/mnt/data/text",
        cache_file: Optional[Path | str] = None,
        delay: float = 1.2,
        verbose: bool = False
    ):
        self.library_root = Path(library_root)
        self.delay = delay
        self.verbose = verbose
        if cache_file:
            self.cache_file = Path(cache_file)
        else:
            self.cache_file = self.library_root / "classification_cache.json"

        self.cache: Dict[str, Dict[str, Any]] = {}
        self.title_cache: Dict[str, Dict[str, Any]] = {}
        self.load_cache()

        self.yes24 = Yes24Bookstore(verbose=verbose)
        self.aladin = AladinBookstore(verbose=verbose)
        self.kyobo = KyoboBookstore(verbose=verbose)

    def load_cache(self) -> None:
        if self.cache_file.exists():
            try:
                with open(self.cache_file, "r", encoding="utf-8") as f:
                    self.cache = json.load(f)
                logger.info(f"Loaded {len(self.cache)} cached entries from {self.cache_file}")
            except Exception as e:
                logger.warning(f"Failed to load cache: {e}")
                self.cache = {}

        self.build_title_index()

    def save_cache(self) -> None:
        try:
            temp_path = self.cache_file.with_suffix(".tmp")
            with open(temp_path, "w", encoding="utf-8") as f:
                json.dump(self.cache, f, ensure_ascii=False, indent=2)
            temp_path.replace(self.cache_file)
        except Exception as e:
            logger.error(f"Failed to save cache: {e}")

    def build_title_index(self) -> None:
        self.title_cache = {}
        for k, v in self.cache.items():
            st = v.get("search_title")
            if st and (v.get("yes24", {}).get("cat") or v.get("aladin", {}).get("cat") or v.get("kyobo", {}).get("cat")):
                if st not in self.title_cache or v.get("status") in ["moved", "already_exists_cleaned", "matched"]:
                    self.title_cache[st] = v

    def query_bookstores(self, search_title: str, raw_author: str, raw_title: str) -> Tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any]]:
        import time

        def _do_query(store: AbstractBookstore) -> Dict[str, Any]:
            res = {"title": "", "author": "", "cat": "", "mapped": None, "url": ""}
            try:
                results, _, _ = store.search(title=search_title, author=raw_author)
                if results and len(results) > 0:
                    found_title, found_author, cat_str, detail_url, _, _ = results[0]
                    res["title"] = found_title
                    res["author"] = found_author
                    res["cat"] = cat_str
                    res["mapped"] = map_category(cat_str, raw_title, raw_author)
                    res["url"] = detail_url
            except Exception as e:
                logger.debug("Store query failed: %s", e)
            return res

        y_entry = _do_query(self.yes24)
        time.sleep(self.delay)
        a_entry = _do_query(self.aladin)
        time.sleep(self.delay)
        k_entry = _do_query(self.kyobo)
        time.sleep(self.delay)

        return y_entry, a_entry, k_entry

    def classify_file(
        self,
        fpath: Path,
        source_dir: Path,
        trust_single_match: bool = True,
        use_bookstore: bool = True,
        use_content_meta: bool = True,
        cache_only: bool = False,
    ) -> Tuple[Optional[str], str, str, Dict[str, Any]]:
        fname = fpath.name
        effective_fname = get_effective_filename(fpath, source_dir)
        raw_author, raw_title, search_title = clean_filename_to_author_title(effective_fname)

        try:
            rel_path = str(fpath.relative_to(source_dir))
        except ValueError:
            rel_path = fname

        cache_key = rel_path if rel_path in self.cache else (effective_fname if effective_fname in self.cache else (fname if fname in self.cache else rel_path))
        entry = self.cache.get(cache_key)

        if not entry and search_title in self.title_cache:
            ref_entry = self.title_cache[search_title]
            ref_author = ref_entry.get("author", "")
            if not (raw_author and ref_author and raw_author != ref_author and raw_author not in ref_author and ref_author not in raw_author):
                entry = {
                    "filename": effective_fname,
                    "rel_path": rel_path,
                    "author": raw_author or ref_author,
                    "title": raw_title,
                    "search_title": search_title,
                    "yes24": dict(ref_entry.get("yes24", {})),
                    "aladin": dict(ref_entry.get("aladin", {})),
                    "kyobo": dict(ref_entry.get("kyobo", {})),
                    "status": "pending",
                    "target_category": None
                }
                self.cache[cache_key] = entry

        if not entry:
            if cache_only:
                return None, "cache_only_skip", "Not in cache", {}

            y_entry, a_entry, k_entry = ({"title": "", "author": "", "cat": "", "mapped": None, "url": ""},) * 3
            if use_bookstore:
                y_entry, a_entry, k_entry = self.query_bookstores(search_title, raw_author, raw_title)

            entry = {
                "filename": effective_fname,
                "rel_path": rel_path,
                "author": raw_author,
                "title": raw_title,
                "search_title": search_title,
                "yes24": y_entry,
                "aladin": a_entry,
                "kyobo": k_entry,
                "status": "pending",
                "target_category": None
            }
            self.cache[cache_key] = entry

        target_cat, method, reason = evaluate_category_decision(
            effective_fname,
            fpath if use_content_meta else None,
            raw_title,
            raw_author,
            search_title,
            entry.get("yes24", {}),
            entry.get("aladin", {}),
            entry.get("kyobo", {}),
            trust_single_match=trust_single_match
        )

        entry["target_category"] = target_cat
        entry["status"] = method if target_cat else ("conflict" if "conflict" in method else "not_found")
        return target_cat, method, reason, entry

    def process_file(
        self,
        fpath: Path,
        source_dir: Path,
        auto_move: bool = True,
        clean_existing: bool = True,
        dry_run: bool = False,
        trust_single_match: bool = True,
        use_bookstore: bool = True,
        use_content_meta: bool = True,
        cache_only: bool = False,
    ) -> Dict[str, Any]:
        target_cat, method, reason, entry = self.classify_file(
            fpath,
            source_dir,
            trust_single_match=trust_single_match,
            use_bookstore=use_bookstore,
            use_content_meta=use_content_meta,
            cache_only=cache_only
        )

        result = {
            "file": str(fpath),
            "target_category": target_cat,
            "method": method,
            "reason": reason,
            "action": "none"
        }

        if not target_cat:
            return result

        dest_dir = self.library_root / target_cat
        dest_path = dest_dir / fpath.name

        if dest_path.exists():
            if clean_existing:
                if not dry_run:
                    fpath.unlink()
                    clean_empty_parent_dirs(fpath.parent, source_dir)
                    if entry:
                        entry["status"] = "already_exists_cleaned"
                result["action"] = "cleaned_duplicate"
            else:
                result["action"] = "skipped_already_exists"
        else:
            if auto_move and not dry_run:
                dest_dir.mkdir(parents=True, exist_ok=True)
                shutil.move(str(fpath), str(dest_path))
                clean_empty_parent_dirs(fpath.parent, source_dir)
                if entry:
                    entry["status"] = "moved"
            result["action"] = "moved" if (auto_move and not dry_run) else "classified"

        return result
