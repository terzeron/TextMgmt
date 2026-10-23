#!/usr/bin/env python3
"""
classify_books_client.py

tm_dev(TextMgmt)의 도서 자동 분류 엔진(BookClassifierService / BookManager)을
호출하여 도서 파일들을 결정론적으로 일괄 분류/이동하는 클라이언트 스크립트.

특징:
1. 3대 서점(Yes24, 알라딘, 교보) 크롤링 교차 검증 (2/3 다수결 또는 높은 유사도 단일 매칭)
2. EPUB opf 메타데이터 및 TXT 본문 키워드 정적 분석 (정규식/휴리스틱)
3. 대상지 동일 파일 존재 시 안전한 중복본 정리 (--clean-existing)
4. 비워진 부모 디렉토리 자동 정리 (clean_empty_parent_dirs)
5. 배치 단위 제한 (--limit 1000) 지원
6. tm_dev REST API 모드(--api-url) 및 로컬 엔진 직접 호출 모드(--direct) 지원
"""

import argparse
import json
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

# tm_dev repo root를 sys.path에 추가
REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

try:
    import httpx
except ImportError:
    httpx = None

from backend.book_classifier import (
    BookClassifierService,
    clean_empty_parent_dirs,
    STANDARD_CATEGORIES,
    get_effective_filename,
)

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    datefmt="%Y-%m-%d %H:%M:%S",
)
LOGGER = logging.getLogger("classify_books_client")


def run_api_client(
    api_url: str,
    category: str,
    recursive: bool = True,
    dry_run: bool = False,
    clean_existing: bool = True,
    use_bookstore: bool = True,
    use_content_meta: bool = True,
    delay: float = 1.2,
    token: str | None = None,
) -> int:
    """tm_dev REST API(/categories/auto-classify)를 호출하여 분류를 수행한다."""
    if httpx is None:
        LOGGER.error("httpx 모듈이 설치되어 있지 않습니다. pip install httpx")
        return 1

    endpoint = f"{api_url.rstrip('/')}/categories/auto-classify"
    status_endpoint = f"{api_url.rstrip('/')}/categories/auto-classify-status"
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"

    payload = {
        "category": category,
        "recursive": recursive,
        "dry_run": dry_run,
        "async_mode": True,
        "clean_existing": clean_existing,
        "use_bookstore": use_bookstore,
        "use_content_meta": use_content_meta,
        "delay": delay,
    }

    LOGGER.info("tm_dev API 호출: %s (payload: %s)", endpoint, payload)
    try:
        with httpx.Client(timeout=30.0) as client:
            resp = client.post(endpoint, json=payload, headers=headers)
            if resp.status_code != 200:
                LOGGER.error("API 요청 실패: status=%s, body=%s", resp.status_code, resp.text)
                return 1

            data = resp.json()
            if data.get("status") != "success":
                LOGGER.error("API 오류: %s", data.get("error"))
                return 1

            LOGGER.info("작업 시작됨. 진행 상태 폴링 중...")
            while True:
                time.sleep(2.0)
                st_resp = client.get(status_endpoint, headers=headers)
                if st_resp.status_code != 200:
                    LOGGER.warning("상태 조회 실패: %s", st_resp.status_code)
                    continue

                st_data = st_resp.json().get("result", {})
                status = st_data.get("status")
                processed = st_data.get("processed_count", 0)
                total = st_data.get("total_count", 0)
                moved = st_data.get("moved_count", 0)
                cleaned = st_data.get("duplicate_cleaned_count", 0)
                skipped = st_data.get("skipped_count", 0)
                failed = st_data.get("failed_count", 0)

                LOGGER.info(
                    "진행 중: [%s] %d/%d (이동: %d, 중복정리: %d, 스킵: %d, 실패: %d)",
                    status,
                    processed,
                    total,
                    moved,
                    cleaned,
                    skipped,
                    failed,
                )

                if status in ("done", "idle", "failed"):
                    if status == "failed":
                        LOGGER.error("작업 실패: %s", st_data.get("error"))
                        return 1
                    LOGGER.info("작업 완료!")
                    return 0

    except Exception as e:
        LOGGER.error("API 통신 예외 발생: %s", e)
        return 1


def run_direct_classifier(
    library_root: Path,
    category: str,
    recursive: bool = True,
    dry_run: bool = False,
    clean_existing: bool = True,
    use_bookstore: bool = True,
    use_content_meta: bool = True,
    delay: float = 1.2,
    limit: int = 1000,
    offset: int = 0,
    uncached_only: bool = False,
    re_evaluate: bool = False,
) -> int:
    """BookClassifierService를 직접 로컬에서 실행하여 도서 분류를 수행한다."""
    if re_evaluate:
        use_bookstore = False
        LOGGER.info("=== [초고속 재평가 모드 (--re-evaluate)] 외부 서점 딜레이 없이 캐시 및 정적 메타데이터로 즉시 판정 ===")

    source_dir = library_root / category if category != "_root" else library_root
    if not source_dir.is_dir():
        LOGGER.error("디렉토리를 찾을 수 없습니다: %s", source_dir)
        return 1

    LOGGER.info("BookClassifierService 초기화 (root=%s, delay=%.1fs, re_evaluate=%s)", library_root, delay, re_evaluate)
    classifier = BookClassifierService(library_root=library_root, delay=delay)

    exts = {".epub", ".pdf", ".txt", ".html", ".htm"}
    files: list[Path] = []
    skipped_offset = 0
    iterator = source_dir.rglob("*") if recursive else source_dir.iterdir()
    for p in iterator:
        if p.is_file() and p.suffix.lower() in exts:
            if not any(part.startswith(".") for part in p.parts):
                if uncached_only:
                    rel_p = str(p.relative_to(source_dir))
                    eff_name = get_effective_filename(p, source_dir)
                    in_cache = (
                        rel_p in classifier.cache
                        or eff_name in classifier.cache
                        or p.name in classifier.cache
                    )
                    if in_cache:
                        # 이미 캐시에서 조회되었고 미분류로 끝난 항목은 건너뜀
                        cached_entry = classifier.cache.get(rel_p) or classifier.cache.get(eff_name) or classifier.cache.get(p.name)
                        if cached_entry and cached_entry.get("target_category") is None:
                            continue

                if offset and skipped_offset < offset:
                    skipped_offset += 1
                    continue

                files.append(p)
                if limit and len(files) >= limit:
                    break

    total = len(files)
    LOGGER.info("분류 대상 도서 수집 완료: %d권 (limit=%s, offset=%d, uncached_only=%s)", total, limit, offset, uncached_only)

    stats = {
        "processed": 0,
        "classified": 0,
        "moved": 0,
        "duplicate_cleaned": 0,
        "skipped": 0,
        "failed": 0,
    }

    for idx, file_path in enumerate(files, 1):
        stats["processed"] += 1
        rel_path = file_path.relative_to(library_root)
        try:
            target_category, method, reason, details = classifier.classify_file(
                file_path,
                source_dir=source_dir,
                use_bookstore=use_bookstore,
                use_content_meta=use_content_meta,
            )

            if not target_category or target_category == category:
                stats["skipped"] += 1
                LOGGER.debug("[%d/%d] [SKIP] %s -> 사유: %s", idx, total, file_path.name, reason or "동일 카테고리/미분류")
                if idx % 200 == 0:
                    LOGGER.info("[%d/%d 진행 중] 이동: %d, 정리: %d, 스킵: %d, 실패: %d", idx, total, stats["moved"], stats["duplicate_cleaned"], stats["skipped"], stats["failed"])
                if idx % 500 == 0 and not dry_run:
                    classifier.save_cache()
                continue

            target_path = library_root / target_category / file_path.name
            target_rel = target_path.relative_to(library_root)
            stats["classified"] += 1

            if target_path.exists():
                is_same = False
                try:
                    is_same = file_path.samefile(target_path)
                except OSError:
                    is_same = False

                if is_same:
                    stats["skipped"] += 1
                    LOGGER.debug("[%d/%d] [SAME] %s (이미 동일 위치)", idx, total, file_path.name)
                    continue

                if clean_existing:
                    if dry_run:
                        LOGGER.info("[%d/%d] [DRY-RUN CLEAN] %s (대상지 존재 중복본 삭제 예정)", idx, total, file_path.name)
                        stats["duplicate_cleaned"] += 1
                    else:
                        file_path.unlink()
                        clean_empty_parent_dirs(file_path.parent, source_dir)
                        LOGGER.info("[%d/%d] [CLEANED] %s (중복본 삭제 완료)", idx, total, file_path.name)
                        stats["duplicate_cleaned"] += 1
                    continue
                else:
                    stats["failed"] += 1
                    LOGGER.warning("[%d/%d] [CONFLICT] %s -> %s (이미 존재함)", idx, total, file_path.name, target_rel)
                    continue

            # 파일 이동
            if dry_run:
                LOGGER.info("[%d/%d] [DRY-RUN MOVE] %s -> %s (%s, %s)", idx, total, file_path.name, target_rel, method, reason)
                stats["moved"] += 1
            else:
                target_path.parent.mkdir(parents=True, exist_ok=True)
                old_parent = file_path.parent
                file_path.rename(target_path)
                clean_empty_parent_dirs(old_parent, source_dir)
                LOGGER.info("[%d/%d] [MOVED] %s -> %s (%s, %s)", idx, total, file_path.name, target_rel, method, reason)
                stats["moved"] += 1

            if idx % 200 == 0:
                LOGGER.info("[%d/%d 진행 중] 이동: %d, 정리: %d, 스킵: %d, 실패: %d", idx, total, stats["moved"], stats["duplicate_cleaned"], stats["skipped"], stats["failed"])
            if idx % 500 == 0 and not dry_run:
                classifier.save_cache()

        except Exception as e:
            stats["failed"] += 1
            LOGGER.error("[%d/%d] [ERROR] %s: %s", idx, total, file_path.name, e)

    if not dry_run:
        classifier.save_cache()

    LOGGER.info(
        "=== 분류 완료 요약 ==="
        "\n총 처리: %d권\n분류 결정: %d권\n이동: %d권\n중복본 정리: %d권\n스킵: %d권\n실패: %d권",
        stats["processed"],
        stats["classified"],
        stats["moved"],
        stats["duplicate_cleaned"],
        stats["skipped"],
        stats["failed"],
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="tm_dev 기반 결정론적 도서 분류 클라이언트 (1번 서점 3개사 다수결 + 2번 메타데이터 정적 분석)",
    )
    parser.add_argument("--library-root", type=Path, default=Path("/mnt/data/text"), help="라이브러리 루트 디렉토리 (기본값: /mnt/data/text)")
    parser.add_argument("--category", type=str, default="0_telegram", help="분류 대상 소스 카테고리 (기본값: 0_telegram)")
    parser.add_argument("--limit", type=int, default=1000, help="한 번에 처리할 최대 권수 (기본값: 1000)")
    parser.add_argument("--offset", type=int, default=0, help="처리 시작 위치 오프셋 (기본값: 0)")
    parser.add_argument("--uncached-only", action="store_true", help="캐시에 아직 조회되지 않은 신규 도서만 우선 수집")
    parser.add_argument("--re-evaluate", action="store_true", help="이미 캐시된 도서들에 대해 개선된 규칙으로 외부 요청 없이 즉시 초고속 재평가 및 이동")
    parser.add_argument("--delay", type=float, default=1.2, help="서점 크롤링 시 요청 대기 시간(초) (기본값: 1.2)")
    parser.add_argument("--dry-run", action="store_true", help="실제 파일 이동/삭제 없이 시뮬레이션만 수행")
    parser.add_argument("--no-recursive", action="store_true", help="하위 디렉토리 재귀 탐색 비활성화")
    parser.add_argument("--no-clean-existing", action="store_true", help="대상 경로에 이미 파일 존재 시 소스 중복본 삭제 비활성화")
    parser.add_argument("--no-bookstore", action="store_true", help="서점 3개사 검색 비활성화")
    parser.add_argument("--no-content-meta", action="store_true", help="EPUB/TXT 파일 메타데이터 및 본문 키워드 분석 비활성화")
    parser.add_argument("--api-url", type=str, default=None, help="tm_dev 백엔드 서버 URL (지정 시 REST API 호출 모드로 동작)")
    parser.add_argument("--token", type=str, default=None, help="API 호출 시 인증용 Bearer 토큰")

    args = parser.parse_args()

    if args.api_url:
        return run_api_client(
            api_url=args.api_url,
            category=args.category,
            recursive=not args.no_recursive,
            dry_run=args.dry_run,
            clean_existing=not args.no_clean_existing,
            use_bookstore=not args.no_bookstore,
            use_content_meta=not args.no_content_meta,
            delay=args.delay,
            token=args.token,
        )
    else:
        return run_direct_classifier(
            library_root=args.library_root,
            category=args.category,
            recursive=not args.no_recursive,
            dry_run=args.dry_run,
            clean_existing=not args.no_clean_existing,
            use_bookstore=not args.no_bookstore,
            use_content_meta=not args.no_content_meta,
            delay=args.delay,
            limit=args.limit,
            offset=args.offset,
            uncached_only=args.uncached_only,
            re_evaluate=args.re_evaluate,
        )


if __name__ == "__main__":
    sys.exit(main())
