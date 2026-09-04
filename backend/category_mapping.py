#!/usr/bin/env python

import os
import logging.config
from datetime import datetime, timedelta
from pathlib import Path
from contextlib import contextmanager
from typing import Any

import pymysql
from pymysql.cursors import DictCursor

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class CategoryMapping:
    """카테고리별 키워드 매핑을 관리하는 클래스 (MySQL 기반)"""

    # 재적재 락의 heartbeat(updated_at)가 이 시간 이상 갱신되지 않으면 죽은 작업으로 간주하고
    # 강제 해제한다. 총 소요 시간이 아니라 "최근에 살아있다는 신호"로 판정해야, 정상적으로 오래
    # 걸리는 대량 재적재와 죽어서 안 풀리는 락을 구분할 수 있다. 파일 1건 처리 상한(150초)보다
    # 여유 있게 잡는다.
    RELOAD_LOCK_HEARTBEAT_STALE_SECONDS = 5 * 60

    def __init__(self, host: str | None = None, port: int | None = None, database: str | None = None, user: str | None = None, password: str | None = None) -> None:
        """
        Args:
            host: MySQL 호스트. None이면 환경변수 사용
            port: MySQL 포트. None이면 환경변수 사용
            database: 데이터베이스명. None이면 환경변수 사용
            user: 사용자명. None이면 환경변수 사용
            password: 비밀번호. None이면 환경변수 사용
        """
        self.host = host or os.environ.get("TM_MYSQL_HOST", "localhost")
        self.port = port or int(os.environ.get("TM_MYSQL_PORT", "3306"))
        self.database = database or os.environ.get("TM_MYSQL_DATABASE", "textmanager")
        self.user = user or os.environ.get("TM_MYSQL_USER", "tmuser")
        self.password = password or os.environ.get("TM_MYSQL_PASSWORD", "")

        LOGGER.info("CategoryMapping initialized with MySQL host: %s:%d, database: %s", self.host, self.port, self.database)
        self._init_db()

    @contextmanager
    def _get_connection(self):
        """MySQL 연결을 관리하는 context manager"""
        conn = pymysql.connect(host=self.host, port=self.port, database=self.database, user=self.user, password=self.password, charset="utf8mb4", cursorclass=DictCursor)
        try:
            yield conn
        finally:
            conn.close()

    def _init_db(self) -> None:
        """데이터베이스 테이블 초기화"""
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute(
                    "CREATE TABLE IF NOT EXISTS category_keywords (id INT AUTO_INCREMENT PRIMARY KEY, category VARCHAR(255) NOT NULL, keyword VARCHAR(255) NOT NULL, content_type VARCHAR(10) NOT NULL DEFAULT 'book', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, UNIQUE KEY unique_category_keyword (category, keyword, content_type), INDEX idx_category (category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
                )
                cursor.execute(
                    "CREATE TABLE IF NOT EXISTS hidden_categories (id INT AUTO_INCREMENT PRIMARY KEY, category VARCHAR(255) NOT NULL, content_type VARCHAR(10) NOT NULL DEFAULT 'book', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, UNIQUE KEY unique_category_content_type (category, content_type), INDEX idx_category (category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
                )
                cursor.execute(
                    "CREATE TABLE IF NOT EXISTS latest_excluded_categories (id INT AUTO_INCREMENT PRIMARY KEY, category VARCHAR(255) NOT NULL, content_type VARCHAR(10) NOT NULL DEFAULT 'book', created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, UNIQUE KEY unique_latest_excluded_category_content_type (category, content_type), INDEX idx_category (category), INDEX idx_content_type (content_type)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
                )
                cursor.execute("CREATE TABLE IF NOT EXISTS reload_locks (content_type VARCHAR(10) NOT NULL PRIMARY KEY, started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci")
                # 기존 테이블 마이그레이션: content_type 컬럼이 없으면 추가
                self._migrate_add_content_type(cursor)
                # reload_locks를 진행 상황까지 담는 공유 작업 상태 테이블로 확장
                self._migrate_reload_locks(cursor)
                conn.commit()
        LOGGER.debug("Database initialized")

    def _migrate_add_content_type(self, cursor) -> None:
        """기존 테이블에 content_type 컬럼 추가 마이그레이션"""
        table_unique_indexes = {"category_keywords": "unique_category_keyword", "hidden_categories": "unique_category_content_type", "latest_excluded_categories": "unique_latest_excluded_category_content_type"}
        for table in table_unique_indexes:
            cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = %s AND table_name = %s AND column_name = 'content_type'", (self.database, table))
            row = cursor.fetchone()
            if row and row["cnt"] == 0:
                LOGGER.info("Migrating table %s: adding content_type column", table)
                cursor.execute(f"ALTER TABLE {table} ADD COLUMN content_type VARCHAR(10) NOT NULL DEFAULT 'book'")
                if table == "category_keywords":
                    try:
                        cursor.execute(f"ALTER TABLE {table} DROP INDEX unique_category_keyword")
                    except Exception as e:
                        LOGGER.debug("Index unique_category_keyword not found, skipping: %s", e)
                    cursor.execute(f"ALTER TABLE {table} ADD UNIQUE KEY unique_category_keyword (category, keyword, content_type)")
                else:
                    try:
                        cursor.execute(f"ALTER TABLE {table} DROP INDEX category")
                    except Exception as e:
                        LOGGER.debug("Index category not found, skipping: %s", e)
                    cursor.execute(f"ALTER TABLE {table} ADD UNIQUE KEY {table_unique_indexes[table]} (category, content_type)")

    def _migrate_reload_locks(self, cursor) -> None:
        """reload_locks를 진행 상황(heartbeat·카운트)까지 담는 공유 작업 상태 테이블로 확장"""
        new_columns = {
            "category": "ALTER TABLE reload_locks ADD COLUMN category VARCHAR(255) NULL",
            "status": "ALTER TABLE reload_locks ADD COLUMN status VARCHAR(10) NOT NULL DEFAULT 'running'",
            "updated_at": "ALTER TABLE reload_locks ADD COLUMN updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP",
            "indexed_count": "ALTER TABLE reload_locks ADD COLUMN indexed_count INT NOT NULL DEFAULT 0",
            "deleted_count": "ALTER TABLE reload_locks ADD COLUMN deleted_count INT NOT NULL DEFAULT 0",
            "failed_count": "ALTER TABLE reload_locks ADD COLUMN failed_count INT NOT NULL DEFAULT 0",
            "before_count": "ALTER TABLE reload_locks ADD COLUMN before_count INT NOT NULL DEFAULT 0",
            "after_count": "ALTER TABLE reload_locks ADD COLUMN after_count INT NOT NULL DEFAULT 0",
            "error": "ALTER TABLE reload_locks ADD COLUMN error TEXT NULL",
        }
        for column, alter_sql in new_columns.items():
            cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = %s AND table_name = 'reload_locks' AND column_name = %s", (self.database, column))
            row = cursor.fetchone()
            if row and row["cnt"] == 0:
                LOGGER.info("Migrating table reload_locks: adding %s column", column)
                cursor.execute(alter_sql)

    def get_all_mappings(self, content_type: str = "book") -> dict[str, list[str]]:
        """모든 카테고리-키워드 매핑 조회

        Args:
            content_type: 콘텐츠 유형 ('book' 또는 'comic')

        Returns:
            {category: [keyword1, keyword2, ...], ...} 형태의 딕셔너리
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT category, keyword FROM category_keywords WHERE content_type = %s ORDER BY category, keyword", (content_type,))
                rows = cursor.fetchall()

        mappings: dict[str, list[str]] = {}
        for row in rows:
            category = row["category"]
            keyword = row["keyword"]
            if category not in mappings:
                mappings[category] = []
            mappings[category].append(keyword)

        LOGGER.debug("get_all_mappings(%s): %d categories", content_type, len(mappings))
        return mappings

    def get_keywords(self, category: str, content_type: str = "book") -> list[str]:
        """특정 카테고리의 키워드 목록 조회

        Args:
            category: 카테고리명
            content_type: 콘텐츠 유형

        Returns:
            키워드 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT keyword FROM category_keywords WHERE category = %s AND content_type = %s ORDER BY keyword", (category, content_type))
                rows = cursor.fetchall()

        keywords = [row["keyword"] for row in rows]
        LOGGER.debug("get_keywords(%s, %s): %d keywords", category, content_type, len(keywords))
        return keywords

    def add_keyword(self, category: str, keyword: str, content_type: str = "book") -> bool:
        """카테고리에 키워드 추가

        Args:
            category: 카테고리명
            keyword: 추가할 키워드
            content_type: 콘텐츠 유형

        Returns:
            성공 여부
        """
        keyword = keyword.strip()
        if not keyword:
            LOGGER.warning("add_keyword: empty keyword")
            return False

        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    cursor.execute("INSERT INTO category_keywords (category, keyword, content_type) VALUES (%s, %s, %s)", (category, keyword, content_type))
                    conn.commit()
                    LOGGER.info("add_keyword(%s, %s, %s): success", category, keyword, content_type)
                    return True
                except pymysql.IntegrityError:
                    LOGGER.warning("add_keyword(%s, %s, %s): already exists", category, keyword, content_type)
                    return False

    def remove_keyword(self, category: str, keyword: str, content_type: str = "book") -> bool:
        """카테고리에서 키워드 삭제

        Args:
            category: 카테고리명
            keyword: 삭제할 키워드
            content_type: 콘텐츠 유형

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("DELETE FROM category_keywords WHERE category = %s AND keyword = %s AND content_type = %s", (category, keyword, content_type))
                conn.commit()
                deleted = int(cursor.rowcount or 0) > 0

        LOGGER.info("remove_keyword(%s, %s, %s): %s", category, keyword, content_type, "success" if deleted else "not found")
        return deleted

    def set_keywords(self, category: str, keywords: list[str], content_type: str = "book") -> bool:
        """카테고리의 키워드 목록을 일괄 설정 (기존 키워드 대체)

        Args:
            category: 카테고리명
            keywords: 새 키워드 목록
            content_type: 콘텐츠 유형

        Returns:
            성공 여부
        """
        # 빈 문자열 제거 및 중복 제거
        keywords = list(set(k.strip() for k in keywords if k.strip()))

        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    # 기존 키워드 삭제
                    cursor.execute("DELETE FROM category_keywords WHERE category = %s AND content_type = %s", (category, content_type))

                    # 새 키워드 일괄 추가
                    if keywords:
                        cursor.executemany("INSERT INTO category_keywords (category, keyword, content_type) VALUES (%s, %s, %s)", [(category, kw, content_type) for kw in keywords])

                    conn.commit()
                    LOGGER.info("set_keywords(%s, %s): %d keywords set", category, content_type, len(keywords))
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("set_keywords(%s, %s) failed: %s", category, content_type, e)
                    return False

    def update_all_mappings(self, mappings: dict[str, list[str]], content_type: str = "book") -> bool:
        """전체 매핑을 일괄 업데이트

        Args:
            mappings: {category: [keyword1, keyword2, ...], ...} 형태의 딕셔너리
            content_type: 콘텐츠 유형

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    # 해당 content_type의 기존 데이터 삭제
                    cursor.execute("DELETE FROM category_keywords WHERE content_type = %s", (content_type,))

                    # 새 데이터 일괄 추가
                    rows = []
                    for category, keywords in mappings.items():
                        for keyword in keywords:
                            keyword = keyword.strip()
                            if keyword:
                                rows.append((category, keyword, content_type))
                    if rows:
                        cursor.executemany("INSERT IGNORE INTO category_keywords (category, keyword, content_type) VALUES (%s, %s, %s)", rows)

                    conn.commit()
                    LOGGER.info("update_all_mappings(%s): %d categories updated", content_type, len(mappings))
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("update_all_mappings(%s) failed: %s", content_type, e)
                    return False

    def delete_category(self, category: str, content_type: str = "book", prefix: bool = False) -> bool:
        """카테고리의 모든 키워드 삭제

        Args:
            category: 카테고리명
            content_type: 콘텐츠 유형
            prefix: True이면 하위 카테고리(category/*)도 포함하여 삭제

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                if prefix:
                    cursor.execute("DELETE FROM category_keywords WHERE (category = %s OR category LIKE %s) AND content_type = %s", (category, category + "/%", content_type))
                else:
                    cursor.execute("DELETE FROM category_keywords WHERE category = %s AND content_type = %s", (category, content_type))
                conn.commit()
                deleted = int(cursor.rowcount or 0) > 0

        LOGGER.info("delete_category(%s, %s, prefix=%s): %s", category, content_type, prefix, "success" if deleted else "not found")
        return deleted

    def search_by_keyword(self, keyword: str, content_type: str = "book") -> list[str]:
        """키워드로 카테고리 검색 (부분 일치)

        Args:
            keyword: 검색할 키워드
            content_type: 콘텐츠 유형

        Returns:
            매칭되는 카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT DISTINCT category FROM category_keywords WHERE keyword LIKE %s AND content_type = %s ORDER BY category", (f"%{keyword}%", content_type))
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("search_by_keyword(%s, %s): %d categories", keyword, content_type, len(categories))
        return categories

    def get_hidden_categories(self, content_type: str = "book") -> list[str]:
        """비노출 카테고리 목록 조회

        Args:
            content_type: 콘텐츠 유형

        Returns:
            비노출 설정된 카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT category FROM hidden_categories WHERE content_type = %s ORDER BY category", (content_type,))
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("get_hidden_categories(%s): %d categories", content_type, len(categories))
        return categories

    def set_hidden(self, category: str, hidden: bool, content_type: str = "book") -> bool:
        """카테고리의 비노출 설정/해제

        Args:
            category: 카테고리명
            hidden: True면 비노출 설정, False면 해제
            content_type: 콘텐츠 유형

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                if hidden:
                    try:
                        cursor.execute("INSERT IGNORE INTO hidden_categories (category, content_type) VALUES (%s, %s)", (category, content_type))
                        conn.commit()
                        LOGGER.info("set_hidden(%s, True, %s): success", category, content_type)
                        return True
                    except Exception as e:
                        LOGGER.error("set_hidden(%s, True, %s) failed: %s", category, content_type, e)
                        return False
                else:
                    cursor.execute("DELETE FROM hidden_categories WHERE category = %s AND content_type = %s", (category, content_type))
                    conn.commit()
                    LOGGER.info("set_hidden(%s, False, %s): success", category, content_type)
                    return True

    def get_latest_excluded_categories(self, content_type: str = "book") -> list[str]:
        """최신 자료 검색에서 제외할 카테고리 목록 조회"""
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT category FROM latest_excluded_categories WHERE content_type = %s ORDER BY category", (content_type,))
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("get_latest_excluded_categories(%s): %d categories", content_type, len(categories))
        return categories

    def set_latest_excluded(self, category: str, excluded: bool, content_type: str = "book") -> bool:
        """카테고리의 최신 자료 검색 제외 설정/해제"""
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                if excluded:
                    try:
                        cursor.execute("INSERT IGNORE INTO latest_excluded_categories (category, content_type) VALUES (%s, %s)", (category, content_type))
                        conn.commit()
                        LOGGER.info("set_latest_excluded(%s, True, %s): success", category, content_type)
                        return True
                    except Exception as e:
                        LOGGER.error("set_latest_excluded(%s, True, %s) failed: %s", category, content_type, e)
                        return False
                else:
                    cursor.execute("DELETE FROM latest_excluded_categories WHERE category = %s AND content_type = %s", (category, content_type))
                    conn.commit()
                    LOGGER.info("set_latest_excluded(%s, False, %s): success", category, content_type)
                    return True

    def rename_category(self, old_category: str, new_category: str, content_type: str = "book") -> bool:
        """카테고리명을 변경 (category_keywords, hidden/latest 설정 테이블 모두 갱신)

        트랜잭션으로 원자적 처리한다.

        Args:
            old_category: 기존 카테고리명
            new_category: 새 카테고리명
            content_type: 콘텐츠 유형

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    cursor.execute("UPDATE category_keywords SET category = %s WHERE category = %s AND content_type = %s", (new_category, old_category, content_type))
                    cursor.execute("UPDATE hidden_categories SET category = %s WHERE category = %s AND content_type = %s", (new_category, old_category, content_type))
                    cursor.execute("UPDATE latest_excluded_categories SET category = %s WHERE category = %s AND content_type = %s", (new_category, old_category, content_type))
                    conn.commit()
                    LOGGER.info("rename_category(%s -> %s, %s): success", old_category, new_category, content_type)
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("rename_category(%s -> %s, %s) failed: %s", old_category, new_category, content_type, e)
                    return False

    def acquire_reload_lock(self, content_type: str = "book", category: str | None = None) -> tuple[bool, str | None]:
        """카테고리 불일치 재적재 작업 상태를 초기화하고 락을 획득한다. 이미 진행 중이면 (False, 안내 메시지)를 반환.

        완료된 작업도 마지막 결과 조회를 위해 행을 지우지 않고 남겨두므로, INSERT 실패(중복 키)가
        아니라 현재 status/heartbeat를 직접 봐서 획득 가능 여부를 판단한다. 재적재가 pod 재시작 등으로
        heartbeat를 못 남기고 죽었을 경우를 대비해, RELOAD_LOCK_HEARTBEAT_STALE_SECONDS 동안
        updated_at이 갱신되지 않은 'running' 행은 죽은 작업으로 간주하고 강제로 갈아치운다.
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT status, updated_at FROM reload_locks WHERE content_type = %s FOR UPDATE", (content_type,))
                row = cursor.fetchone()
                if row and row["status"] == "running":
                    if (datetime.now() - row["updated_at"]) <= timedelta(seconds=self.RELOAD_LOCK_HEARTBEAT_STALE_SECONDS):
                        conn.commit()
                        return False, "이미 재적재 작업이 진행 중입니다. 완료 후 다시 시도하세요."
                    LOGGER.warning("acquire_reload_lock(%s): heartbeat(%s) 정지된 죽은 락 감지, 강제 해제 후 재획득", content_type, row["updated_at"])
                cursor.execute(
                    "INSERT INTO reload_locks (content_type, category, status, started_at, updated_at, indexed_count, deleted_count, failed_count, before_count, after_count, error) "
                    "VALUES (%s, %s, 'running', NOW(), NOW(), 0, 0, 0, 0, 0, NULL) "
                    "ON DUPLICATE KEY UPDATE category = VALUES(category), status = 'running', started_at = NOW(), updated_at = NOW(), "
                    "indexed_count = 0, deleted_count = 0, failed_count = 0, before_count = 0, after_count = 0, error = NULL",
                    (content_type, category),
                )
                conn.commit()
                return True, None

    def heartbeat_reload_lock(self, content_type: str = "book", **progress_counts: int) -> None:
        """재적재 진행 중 heartbeat(updated_at)와 진행 카운트를 갱신한다."""
        allowed = {"indexed_count", "deleted_count", "failed_count", "before_count", "after_count"}
        unknown = set(progress_counts) - allowed
        if unknown:
            raise ValueError(f"알 수 없는 진행 카운트 필드: {unknown}")
        set_clause = "".join(f", {field} = %s" for field in progress_counts)
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute(f"UPDATE reload_locks SET updated_at = NOW(){set_clause} WHERE content_type = %s", (*progress_counts.values(), content_type))
                conn.commit()

    def complete_reload_lock(self, content_type: str = "book", status: str = "done", error: str | None = None, **final_counts: int) -> None:
        """재적재 작업 완료(성공/실패)를 기록한다. 행은 삭제하지 않고 상태만 남겨, 새로고침 후에도
        마지막 결과를 볼 수 있게 한다. 다음 acquire_reload_lock 호출 시 덮어써진다."""
        allowed = {"indexed_count", "deleted_count", "failed_count", "before_count", "after_count"}
        unknown = set(final_counts) - allowed
        if unknown:
            raise ValueError(f"알 수 없는 진행 카운트 필드: {unknown}")
        set_clause = "".join(f", {field} = %s" for field in final_counts)
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute(f"UPDATE reload_locks SET status = %s, updated_at = NOW(), error = %s{set_clause} WHERE content_type = %s", (status, error, *final_counts.values(), content_type))
                conn.commit()

    def get_reload_status(self, content_type: str = "book") -> dict[str, Any] | None:
        """현재 재적재 작업 상태를 조회한다. 작업 이력이 없으면 None.

        status가 'running'인데 heartbeat가 RELOAD_LOCK_HEARTBEAT_STALE_SECONDS 이상 끊겼으면,
        DB 행을 고치지 않고 조회 결과에서만 'failed'로 보여준다(실제 재획득/정리는
        acquire_reload_lock이 다음 시작 시점에 처리).
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT category, status, started_at, updated_at, indexed_count, deleted_count, failed_count, before_count, after_count, error FROM reload_locks WHERE content_type = %s", (content_type,))
                row = cursor.fetchone()
        if not row:
            return None
        status = row["status"]
        error = row["error"]
        if status == "running" and (datetime.now() - row["updated_at"]) > timedelta(seconds=self.RELOAD_LOCK_HEARTBEAT_STALE_SECONDS):
            status = "failed"
            error = error or "응답 없이 중단된 것으로 보입니다."
        return {
            "category": row["category"],
            "status": status,
            "started_at": row["started_at"].isoformat() if row["started_at"] else None,
            "updated_at": row["updated_at"].isoformat() if row["updated_at"] else None,
            "indexed_count": row["indexed_count"],
            "deleted_count": row["deleted_count"],
            "failed_count": row["failed_count"],
            "before_count": row["before_count"],
            "after_count": row["after_count"],
            "error": error,
        }

    def release_reload_lock(self, content_type: str = "book") -> None:
        """재적재 락 행을 완전히 지운다 (일반 완료 경로에서는 complete_reload_lock을 쓴다)."""
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("DELETE FROM reload_locks WHERE content_type = %s", (content_type,))
                conn.commit()
