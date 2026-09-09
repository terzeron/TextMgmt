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
    extract_content_head_tail_words,
    calculate_5_genre_scores_and_ratios,
)


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
