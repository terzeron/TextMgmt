#!/usr/bin/env python3
"""
move_semi_clusters.py

사용자 승인된 준확실 클러스터 도서들을 각 대상 폴더로 안전하게 이동:
- 준확실_3_무협 -> 3_무협 (단, '이윤기의 그리스 로마 신화'는 1_서양고전)
- 준확실_3_판타지 -> 3_판타지
- 준확실_3_여성향 -> 3_여성향
- 준확실_9_BLGL -> 9_BLGL
- 준확실_9_성인 -> 9_성인
"""

import os
import sys
import json
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from backend.book_classifier import clean_empty_parent_dirs

def main():
    library_root = Path("/mnt/data/text")
    source_dir = library_root / "0_telegram"
    report_path = library_root / "hybrid_clustering_report.json"

    with open(report_path, "r", encoding="utf-8") as f:
        data = json.load(f)

    semi_map = {
        "준확실_3_무협": "3_무협",
        "준확실_3_판타지": "3_판타지",
        "준확실_3_여성향": "3_여성향",
        "준확실_9_BLGL": "9_BLGL",
        "준확실_9_성인": "9_성인",
    }

    moved_counts = {cat: 0 for cat in semi_map.values()}
    moved_counts["1_서양고전"] = 0
    cleaned_counts = {cat: 0 for cat in semi_map.values()}
    cleaned_counts["1_서양고전"] = 0

    # 1. 준확실 클러스터 도서 이동
    for cluster_name, default_cat in semi_map.items():
        items = data["clusters"].get(cluster_name, [])
        for it in items:
            fpath = Path(it["file"])
            if not fpath.exists():
                continue

            target_cat = default_cat
            if "이윤기의 그리스 로마 신화" in fpath.name:
                target_cat = "1_서양고전"

            dest_dir = library_root / target_cat
            dest_dir.mkdir(parents=True, exist_ok=True)
            dest_path = dest_dir / fpath.name

            if dest_path.exists():
                fpath.unlink()
                cleaned_counts[target_cat] += 1
            else:
                fpath.rename(dest_path)
                moved_counts[target_cat] += 1

            clean_empty_parent_dirs(fpath.parent, source_dir)

    # 2. 추가: 0_telegram에 남아있는 이윤기의 그리스 로마 신화 나머지 권수도 1_서양고전으로 이동
    for p in source_dir.rglob("*이윤기의 그리스 로마 신화*"):
        if p.is_file():
            target_cat = "1_서양고전"
            dest_dir = library_root / target_cat
            dest_dir.mkdir(parents=True, exist_ok=True)
            dest_path = dest_dir / p.name
            if dest_path.exists():
                p.unlink()
                cleaned_counts[target_cat] += 1
            else:
                p.rename(dest_path)
                moved_counts[target_cat] += 1
            clean_empty_parent_dirs(p.parent, source_dir)

    print("=== 준확실 클러스터 도서 이동 완료 ===")
    total_moved = sum(moved_counts.values())
    total_cleaned = sum(cleaned_counts.values())
    for cat in sorted(moved_counts.keys()):
        m = moved_counts[cat]
        c = cleaned_counts[cat]
        if m > 0 or c > 0:
            print(f"  ▶ {cat}: 신규 이동 {m}권, 중복 정리 {c}권 (합계 {m+c}권)")
    print(f"\n총 처리: 신규 이동 {total_moved}권, 중복 정리 {total_cleaned}권 (합계 {total_moved + total_cleaned}권)")

if __name__ == "__main__":
    main()
