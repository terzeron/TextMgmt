#!/usr/bin/env python3
"""
cluster_hybrid_genres.py

본문 처음 500단어와 마지막 500단어, 그리고 제목을 결합 분석하여
도서의 장르별 단어 빈도 비율(무협 %, 판타지 %, 여성향 %, BL %, 성인 %)을 계산하고
클러스터로 그룹화하여 확실한 클러스터는 자동 분류, 하이브리드는 리포팅하는 스크립트.
"""

import os
import sys
import re
import json
import zipfile
from pathlib import Path
from collections import Counter, defaultdict
from typing import Dict, Any, Tuple, Optional, List

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from backend.book_classifier import (
    extract_explicit_genre,
    get_effective_filename,
    BookClassifierService,
    clean_empty_parent_dirs,
)

# ---------------------------------------------------------------------------
# 5대 장르 어휘 스코어링
#
# 백엔드의 자동분류는 지도학습 모델로 바뀌었고, 이 수작업 어휘 스코어러는 거기서
# 빠졌다. 하이브리드 장르 탐색은 "무협 40% / 판타지 35%" 같은 비율 자체를 보는
# 도구라 모델의 단일 판정으로는 대체가 안 된다. 그래서 유일한 사용처인 이 파일로
# 옮겨 왔다. 백엔드 판정 로직과는 더 이상 연결되지 않는다.
# ---------------------------------------------------------------------------


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



def extract_head_tail_words(fpath: Path, head_n: int = 500, tail_n: int = 500) -> Tuple[str, str]:
    """하위 호환을 위한 래퍼: backend.book_classifier의 extract_content_head_tail_words 재사용"""
    return extract_content_head_tail_words(fpath, head_n, tail_n), ""


def calculate_genre_frequencies(title: str, head_text: str, tail_text: str = "") -> Dict[str, Any]:
    """하위 호환을 위한 래퍼: backend.book_classifier의 calculate_5_genre_scores_and_ratios 재사용"""
    combined_text = (head_text + " " + tail_text).strip()
    return calculate_5_genre_scores_and_ratios(title, combined_text)

def main():
    import argparse
    parser = argparse.ArgumentParser(description="Head 500 & Tail 500 words hybrid genre clustering")
    parser.add_argument("--source-dir", type=Path, default=Path("/mnt/data/text/0_telegram"))
    parser.add_argument("--library-root", type=Path, default=Path("/mnt/data/text"))
    parser.add_argument("--limit", type=int, default=0, help="0 for all")
    parser.add_argument("--auto-move-pure", action="store_true", help="Move high-confidence (pure >= 75%%) clusters automatically")
    parser.add_argument("--move-hybrids-to", type=str, default="", help="Target category to move all hybrid clusters (e.g. 3_판타지)")
    parser.add_argument("--clean-existing", action="store_true", default=True)
    parser.add_argument("--report", type=Path, default=Path("/mnt/data/text/hybrid_clustering_report.json"))
    args = parser.parse_args()

    exts = {".txt", ".epub", ".html", ".htm", ".pdf"}
    files = [
        p for p in args.source_dir.rglob("*")
        if p.is_file() and p.suffix.lower() in exts and not any(part.startswith(".") for part in p.parts)
    ]
    if args.limit > 0:
        files = files[:args.limit]

    total_files = len(files)
    print(f"=== 장르별 단어 빈도 비율 분석 & 클러스터링 시작 (총 {total_files}권) ===", flush=True)

    clusters = defaultdict(list)
    results = []

    moved_count = 0
    cleaned_count = 0

    for idx, p in enumerate(files, 1):
        eff_name = get_effective_filename(p, args.source_dir)
        content_text = extract_content_head_tail_words(p, 500, 500)
        res = calculate_5_genre_scores_and_ratios(eff_name, content_text)
        res["file"] = str(p)
        res["filename"] = eff_name
        
        c = res.get("cluster", "기타")
        clusters[c].append(res)
        results.append(res)

        # 확실한 클러스터 자동 이동
        if args.auto_move_pure and res.get("target_cat"):
            target_cat = res["target_cat"]
            dest_dir = args.library_root / target_cat
            dest_path = dest_dir / p.name
            dest_dir.mkdir(parents=True, exist_ok=True)
            if dest_path.resolve() != p.resolve():
                if dest_path.exists():
                    if args.clean_existing:
                        p.unlink()
                        clean_empty_parent_dirs(p.parent, args.source_dir)
                        cleaned_count += 1
                else:
                    p.rename(dest_path)
                    clean_empty_parent_dirs(p.parent, args.source_dir)
                    moved_count += 1
        elif args.move_hybrids_to and c.startswith("하이브리드_"):
            dest_dir = args.library_root / args.move_hybrids_to
            dest_path = dest_dir / p.name
            dest_dir.mkdir(parents=True, exist_ok=True)
            if dest_path.resolve() != p.resolve():
                if dest_path.exists():
                    if args.clean_existing:
                        p.unlink()
                        clean_empty_parent_dirs(p.parent, args.source_dir)
                        cleaned_count += 1
                else:
                    p.rename(dest_path)
                    clean_empty_parent_dirs(p.parent, args.source_dir)
                    moved_count += 1

        if idx % 1000 == 0:
            print(f"[{idx}/{total_files}] 분석 진행 중... (클러스터 수: {len(clusters)})", flush=True)

    print("\n=======================================================")
    print(f"=== 클러스터링 분석 완료 (총 {total_files}권) ===")
    print("=======================================================\n")

    # 리포트 통계 출력
    pure_clusters = [c for c in sorted(clusters.keys()) if c.startswith("확실한_")]
    hybrid_clusters = [c for c in sorted(clusters.keys()) if c.startswith("하이브리드_")]
    semi_clusters = [c for c in sorted(clusters.keys()) if c.startswith("준확실_")]
    other_clusters = [c for c in sorted(clusters.keys()) if c not in pure_clusters and c not in hybrid_clusters and c not in semi_clusters]

    print("### 1. [가장 확실한 클러스터 (자동 분류 대상, 비율 >= 75%)]")
    total_pure = sum(len(clusters[c]) for c in pure_clusters)
    print(f"총 {total_pure}권 ({total_pure/total_files*100:.1f}%)")
    for c in pure_clusters:
        items = clusters[c]
        print(f"\n  ▶ {c}: {len(items)}권")
        for it in items[:3]:
            r_str = ", ".join([f"{k.split('_')[-1]}:{v}%" for k, v in it['ratios'].items() if v > 0])
            print(f"     - \"{it['filename'][:50]}\" [{r_str}] (score: {it['total_score']})")

    print("\n-------------------------------------------------------")
    print("### 2. [하이브리드 및 복합 클러스터 (사용자 검토 및 선택 필요)]")
    total_hybrid = sum(len(clusters[c]) for c in hybrid_clusters)
    print(f"총 {total_hybrid}권 ({total_hybrid/total_files*100:.1f}%)")
    for c in hybrid_clusters:
        items = clusters[c]
        print(f"\n  ▶ {c}: {len(items)}권")
        for it in items[:5]:
            r_str = ", ".join([f"{k.split('_')[-1]}:{v}%" for k, v in it['ratios'].items() if v > 0])
            print(f"     - \"{it['filename'][:50]}\" [{r_str}] (score: {it['total_score']})")

    print("\n-------------------------------------------------------")
    print("### 3. [준확실 클러스터 (60% ~ 75%)]")
    for c in semi_clusters:
        items = clusters[c]
        print(f"  ▶ {c}: {len(items)}권")
        for it in items[:3]:
            r_str = ", ".join([f"{k.split('_')[-1]}:{v}%" for k, v in it['ratios'].items() if v > 0])
            print(f"     - \"{it['filename'][:50]}\" [{r_str}] (score: {it['total_score']})")

    print("\n-------------------------------------------------------")
    print("### 4. [미분류 및 기타]")
    for c in other_clusters:
        print(f"  ▶ {c}: {len(clusters[c])}권")

    if args.auto_move_pure or args.move_hybrids_to:
        print(f"\n[자동 이동 결과] 신규 이동: {moved_count}권, 중복 정리: {cleaned_count}권")

    # JSON 저장
    with open(args.report, "w", encoding="utf-8") as f:
        summary_data = {
            "total_files": total_files,
            "cluster_counts": {c: len(clusters[c]) for c in clusters},
            "clusters": {
                c: [{
                    "file": it["file"],
                    "filename": it["filename"],
                    "ratios": it["ratios"],
                    "scores": it["scores"],
                    "total_score": it["total_score"]
                } for it in items]
                for c, items in clusters.items()
            }
        }
        json.dump(summary_data, f, ensure_ascii=False, indent=2)
    print(f"\n상세 리포트가 {args.report} 에 저장되었습니다.")

if __name__ == "__main__":
    main()
