#!/usr/bin/env python
"""기존 ES 인덱스에 category.nori 서브필드 추가 및 기존 문서 재처리 스크립트

사용법:
    # 서브필드 추가 + 백그라운드 update_by_query (기본 500 docs/sec)
    python scripts/update_category_nori.py

    # throttling 조절 (초당 처리 문서 수)
    python scripts/update_category_nori.py --rps 200

    # 진행 상태 확인 (task_id 필요)
    python scripts/update_category_nori.py --status <task_id>

    # 실행 중인 작업 취소
    python scripts/update_category_nori.py --cancel <task_id>
"""

import argparse
import os
import sys
import time

from elasticsearch import Elasticsearch


def get_es_client():
    url = os.environ["TM_ES_URL"]
    user = os.environ["TM_ES_USER"]
    password = os.environ["TM_ES_PASSWORD"]
    return Elasticsearch(
        hosts=[url],
        basic_auth=(user, password),
        request_timeout=120,
        retry_on_timeout=True,
    )


def add_subfield(es, index_name):
    """category.nori 서브필드 추가 (이미 있으면 스킵)"""
    mapping = es.indices.get_mapping(index=index_name)
    cat_props = mapping[index_name]["mappings"]["properties"].get("category", {})

    if "fields" in cat_props and "nori" in cat_props["fields"]:
        print(f"  [스킵] category.nori 서브필드가 이미 존재합니다.")
        return True

    print(f"  category.nori 서브필드 추가 중...")
    es.indices.put_mapping(
        index=index_name,
        properties={
            "category": {
                "type": "keyword",
                "fields": {"nori": {"type": "text", "analyzer": "nori_analyzer"}},
            }
        },
    )
    print(f"  서브필드 추가 완료.")
    return True


def run_update_by_query(es, index_name, rps):
    """백그라운드 _update_by_query 실행"""
    count = es.count(index=index_name)["count"]
    print(f"  총 문서 수: {count:,}")
    if count == 0:
        print(f"  문서가 없으므로 update_by_query 불필요.")
        return None

    estimated_sec = count / rps
    print(f"  예상 소요 시간: {estimated_sec:.0f}초 ({estimated_sec / 60:.1f}분) @ {rps} docs/sec")

    result = es.update_by_query(
        index=index_name,
        body={"query": {"match_all": {}}},
        wait_for_completion=False,
        requests_per_second=rps,
        conflicts="proceed",
    )
    task_id = result["task"]
    print(f"  백그라운드 작업 시작: task_id={task_id}")
    print(f"  진행 확인: python scripts/update_category_nori.py --status {task_id}")
    print(f"  작업 취소: python scripts/update_category_nori.py --cancel {task_id}")
    return task_id


def check_status(es, task_id):
    """작업 진행 상태 확인"""
    result = es.tasks.get(task_id=task_id)
    task = result["task"]
    status = task["status"]
    completed = result.get("completed", False)

    total = status.get("total", 0)
    updated = status.get("updated", 0)
    pct = (updated / total * 100) if total > 0 else 0
    running_ns = task.get("running_time_in_nanos", 0)
    running_sec = running_ns / 1e9

    print(f"  완료 여부: {'완료' if completed else '진행 중'}")
    print(f"  진행률: {updated:,} / {total:,} ({pct:.1f}%)")
    print(f"  경과 시간: {running_sec:.0f}초 ({running_sec / 60:.1f}분)")
    if updated > 0 and not completed:
        remaining = (total - updated) / (updated / running_sec) if running_sec > 0 else 0
        print(f"  예상 잔여: {remaining:.0f}초 ({remaining / 60:.1f}분)")

    if completed and "error" in result:
        print(f"  오류: {result['error']}")

    return completed


def cancel_task(es, task_id):
    """작업 취소"""
    es.tasks.cancel(task_id=task_id)
    print(f"  작업 취소 요청 완료: {task_id}")


def main():
    parser = argparse.ArgumentParser(description="ES category.nori 서브필드 마이그레이션")
    parser.add_argument("--rps", type=int, default=500, help="초당 처리 문서 수 (기본: 500)")
    parser.add_argument("--status", type=str, help="작업 진행 상태 확인 (task_id)")
    parser.add_argument("--cancel", type=str, help="작업 취소 (task_id)")
    parser.add_argument("--wait", action="store_true", help="완료될 때까지 대기하며 진행 표시")
    args = parser.parse_args()

    required_envs = ["TM_ES_URL", "TM_ES_USER", "TM_ES_PASSWORD", "TM_ES_BOOK_INDEX"]
    missing = [e for e in required_envs if e not in os.environ]
    if missing:
        print(f"필수 환경변수 미설정: {', '.join(missing)}")
        sys.exit(1)

    es = get_es_client()
    print(f"ES 연결: {os.environ['TM_ES_URL']}")

    if args.status:
        print(f"\n=== 작업 상태: {args.status} ===")
        check_status(es, args.status)
        return

    if args.cancel:
        print(f"\n=== 작업 취소: {args.cancel} ===")
        cancel_task(es, args.cancel)
        return

    # 대상 인덱스 목록
    indices = [os.environ["TM_ES_BOOK_INDEX"]]
    comics_index = os.environ.get("TM_ES_COMICS_INDEX")
    if comics_index:
        indices.append(comics_index)

    task_ids = []
    for index_name in indices:
        print(f"\n=== 인덱스: {index_name} ===")
        if not es.indices.exists(index=index_name):
            print(f"  인덱스가 존재하지 않습니다. 스킵.")
            continue

        add_subfield(es, index_name)
        task_id = run_update_by_query(es, index_name, args.rps)
        if task_id:
            task_ids.append(task_id)

    if args.wait and task_ids:
        print(f"\n=== 완료 대기 중 ===")
        for task_id in task_ids:
            while True:
                completed = check_status(es, task_id)
                if completed:
                    break
                time.sleep(5)

    print("\n완료.")


if __name__ == "__main__":
    main()
