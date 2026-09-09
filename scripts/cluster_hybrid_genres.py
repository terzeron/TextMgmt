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

def extract_head_tail_words(fpath: Path, head_n: int = 500, tail_n: int = 500) -> Tuple[str, str]:
    ext = fpath.suffix.lower()
    head_text = ""
    tail_text = ""

    if ext == ".txt":
        # Head text
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

        # Tail text (seek from end)
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

    elif ext == ".epub":
        try:
            with zipfile.ZipFile(fpath, "r") as z:
                htmls = [
                    n for n in z.namelist()
                    if n.lower().endswith((".html", ".xhtml", ".htm"))
                    and not any(k in n.lower() for k in ["cover", "nav", "toc", "title"])
                ]
                if not htmls:
                    htmls = [n for n in z.namelist() if n.lower().endswith((".html", ".xhtml", ".htm"))]

                # Head: first 2 files
                chunks_h = []
                for h in htmls[:2]:
                    raw = z.read(h).decode("utf-8", errors="ignore")
                    clean = re.sub(r"<[^>]+>", " ", raw)
                    if len(clean.strip()) > 50:
                        chunks_h.append(clean)
                head_text = " ".join(chunks_h)

                # Tail: last 2 files
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
    return " ".join(h_words), " ".join(t_words)


def count_patterns(text: str, kws: List[str]) -> int:
    cnt = 0
    for kw in kws:
        if kw == "마나":
            cnt += len(re.findall(r"(?:^|[^\w가-힣])마나(?:[를이가의로통량석홀]?)(?:$|[^\w가-힣])", text))
        elif kw in ["bl", "gl"]:
            cnt += len(re.findall(r"(?:^|[^a-zA-Z])" + kw + r"(?:$|[^a-zA-Z])", text))
        else:
            cnt += text.count(kw)
    return cnt


def calculate_genre_frequencies(title: str, head_text: str, tail_text: str) -> Dict[str, Any]:
    """
    제목 + Head 500 + Tail 500단어를 바탕으로 5개 장르의 빈도 및 비율(%) 계산
    """
    combined = (title + " " + head_text + " " + tail_text).lower()
    total_chars = len(combined)
    hangul_chars = len(re.findall(r"[가-힣]", combined))

    if total_chars < 50 or (hangul_chars / total_chars) < 0.20:
        return {
            "valid": False,
            "reason": "not_enough_hangul",
            "scores": {},
            "ratios": {},
            "total_score": 0,
            "cluster": "미분류_비문학영문",
        }

    # 장르별 키워드 정의
    wuxia_core = ["무협", "무림", "단전", "내공", "진기", "운기조식", "기경팔맥", "주화입마", "환골탈태", "화산파", "무당파", "소림사", "개방", "사파", "정파", "마교", "천마", "비급", "검법", "도법", "심법"]
    wuxia_kws = wuxia_core + ["강호", "임독이맥", "종남파", "혈교", "장문인", "소교주", "맹주", "무림맹", "절기", "초식"]

    fantasy_core = ["판타지", "던전", "몬스터", "헌터", "각성", "레이드", "게이트", "상태창", "마나", "마법진", "오크", "고블린", "드래곤", "엘프", "마왕", "용사", "이세계", "귀환자", "만렙"]
    fantasy_kws = fantasy_core + ["길드", "시스템", "퀘스트", "스킬", "스탯", "레벨업", "플레이어", "인벤토리", "아이템", "서클", "전생", "회귀", "아카데미"]

    rofan_core = ["로맨스", "로판", "현로", "영애", "황태자", "황후", "황비", "남주", "여주", "남주인공", "여주인공", "파혼", "시월드", "후회남", "집착남", "계략남", "시한부", "악녀", "햇살여주"]
    rofan_kws = rofan_core + ["공작", "공작가", "황제", "황실", "사교계", "무도회", "드레스", "시녀", "집사", "약혼", "키스", "설렘"]

    bl_core = ["미인공", "미남공", "다정공", "광공", "집착공", "연하공", "연상공", "후회공", "미인수", "단정수", "강수", "지랄수", "임신수", "순진수", "오메가버스", "가이드버스", "에스퍼", "가이딩", "히트사이클", "페로몬", "노팅", "각인", "백합", "보이즈러브", "동성애"]
    bl_kws = bl_core + ["bl", "gl"]

    adult_kws = ["야설", "성인소설", "음란", "음탕", "육덕", "최면", "조교", "근친", "스와핑", "섹스", "자위", "사정액", "쿠퍼액", "애액", "정액", "자지", "보지에", "보지를", "보지속", "음순", "클리토리스", "귀두", "유두", "유륜", "젖가슴", "피스톤", "허리짓", "교성", "오르가즘", "절정에", "펠라치오"]

    w_score = count_patterns(combined, wuxia_kws) if any(kw in combined for kw in wuxia_core) else 0
    f_score = count_patterns(combined, fantasy_kws) if any(kw in combined for kw in fantasy_core) else 0
    r_score = count_patterns(combined, rofan_kws) if any(kw in combined for kw in rofan_core) else 0
    b_score = count_patterns(combined, bl_kws) if (any(kw in combined for kw in bl_core) or bool(re.search(r"(?:^|[^a-zA-Z])(?:bl|gl)(?:$|[^a-zA-Z])", combined))) else 0

    matched_adult = [kw for kw in adult_kws if kw in combined]
    a_score = sum(combined.count(kw) for kw in adult_kws) if (len(matched_adult) >= 3 and sum(combined.count(kw) for kw in adult_kws) >= 5) else 0

    scores = {
        "3_무협": w_score,
        "3_판타지": f_score,
        "3_여성향": r_score,
        "9_BLGL": b_score,
        "9_성인": a_score,
    }
    total_score = sum(scores.values())

    if total_score == 0:
        return {
            "valid": False,
            "reason": "zero_score",
            "scores": scores,
            "ratios": {k: 0.0 for k in scores},
            "total_score": 0,
            "cluster": "미분류_키워드부족",
        }

    ratios = {k: round((v / total_score) * 100, 1) for k, v in scores.items()}

    # 클러스터 결정
    cluster = "기타_복합"
    target_cat = None

    # 1. 단일 압도적 클러스터 (Dominant >= 75% 및 score >= 4)
    for cat, r in ratios.items():
        if r >= 75.0 and scores[cat] >= 4:
            cluster = f"확실한_{cat}"
            target_cat = cat
            break

    # 2. 하이브리드 클러스터
    if not target_cat:
        # 무협 vs 판타지 하이브리드
        if ratios["3_무협"] + ratios["3_판타지"] >= 70.0 and ratios["3_무협"] >= 20.0 and ratios["3_판타지"] >= 20.0 and scores["3_무협"] >= 2 and scores["3_판타지"] >= 2:
            cluster = "하이브리드_무협_판타지"
        # 판타지 vs 로판 하이브리드
        elif ratios["3_판타지"] + ratios["3_여성향"] >= 70.0 and ratios["3_판타지"] >= 20.0 and ratios["3_여성향"] >= 20.0 and scores["3_판타지"] >= 2 and scores["3_여성향"] >= 2:
            cluster = "하이브리드_판타지_로판"
        # 판타지 vs 성인 (떡타지)
        elif ratios["3_판타지"] + ratios["9_성인"] >= 70.0 and ratios["3_판타지"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["3_판타지"] >= 2 and scores["9_성인"] >= 2:
            cluster = "하이브리드_판타지_성인"
        # 로맨스 vs 성인 (고수위 로맨스)
        elif ratios["3_여성향"] + ratios["9_성인"] >= 70.0 and ratios["3_여성향"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["3_여성향"] >= 2 and scores["9_성인"] >= 2:
            cluster = "하이브리드_로맨스_성인"
        # 무협 vs 성인 (무협 야설)
        elif ratios["3_무협"] + ratios["9_성인"] >= 70.0 and ratios["3_무협"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["3_무협"] >= 2 and scores["9_성인"] >= 2:
            cluster = "하이브리드_무협_성인"
        # BL vs 성인 (고수위 BL)
        elif ratios["9_BLGL"] + ratios["9_성인"] >= 70.0 and ratios["9_BLGL"] >= 20.0 and ratios["9_성인"] >= 20.0 and scores["9_BLGL"] >= 2 and scores["9_성인"] >= 2:
            cluster = "하이브리드_BL_성인"
        # BL vs 판타지 (가이드버스/헌터BL)
        elif ratios["9_BLGL"] + ratios["3_판타지"] >= 70.0 and ratios["9_BLGL"] >= 20.0 and ratios["3_판타지"] >= 20.0 and scores["9_BLGL"] >= 2 and scores["3_판타지"] >= 2:
            cluster = "하이브리드_BL_판타지"
        else:
            # 60% 이상이면 준확실
            for cat, r in ratios.items():
                if r >= 60.0 and scores[cat] >= 3:
                    cluster = f"준확실_{cat}"
                    break

    return {
        "valid": True,
        "scores": scores,
        "ratios": ratios,
        "total_score": total_score,
        "cluster": cluster,
        "target_cat": target_cat,
    }

def main():
    import argparse
    parser = argparse.ArgumentParser(description="Head 500 & Tail 500 words hybrid genre clustering")
    parser.add_argument("--source-dir", type=Path, default=Path("/mnt/data/text/0_telegram"))
    parser.add_argument("--library-root", type=Path, default=Path("/mnt/data/text"))
    parser.add_argument("--limit", type=int, default=0, help="0 for all")
    parser.add_argument("--auto-move-pure", action="store_true", help="Move high-confidence (pure >= 75%) clusters automatically")
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
        h_text, t_text = extract_head_tail_words(p, 500, 500)
        res = calculate_genre_frequencies(eff_name, h_text, t_text)
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
        for it in items[:2]:
            r_str = ", ".join([f"{k.split('_')[-1]}:{v}%" for k, v in it['ratios'].items() if v > 0])
            print(f"     - \"{it['filename'][:50]}\" [{r_str}]")

    print("\n-------------------------------------------------------")
    print("### 4. [미분류 및 기타]")
    for c in other_clusters:
        print(f"  ▶ {c}: {len(clusters[c])}권")

    if args.auto_move_pure:
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
