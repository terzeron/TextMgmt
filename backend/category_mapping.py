#!/usr/bin/env python

import os
import json
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

    # reload_locks의 락 단위(lock_key). 카테고리별 재적재는 카테고리명을 그대로 쓰고,
    # 일괄/전체 재적재는 모든 카테고리에 영향을 주므로 이 sentinel을 쓴다.
    BULK_LOCK_KEY = "__all__"
    RELOAD_SOURCE_BULK = "bulk"
    RELOAD_SOURCE_MISMATCH = "mismatch"
    RELOAD_SOURCES = {RELOAD_SOURCE_BULK, RELOAD_SOURCE_MISMATCH}

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
                # 분류 제안 항목을 담는 테이블. 항목을 JSON 상태 파일 하나에 담으면 책 한 권
                # 기록할 때마다 지금까지의 목록 전체를 다시 직렬화해야 해서 비용이 책 수의
                # 제곱으로 늘어난다(실측: 79,589권 카테고리에서 기록만 약 33시간). 행 단위로
                # 쌓으면 이 문제가 원천적으로 사라진다. file_path는 1024자까지라 UNIQUE로
                # 걸면 인덱스 길이 제한에 걸리므로 걸지 않는다 — 중복은 clear로 관리한다.
                # 항목 필드가 앞으로 늘 수 있어(candidates 배열 등) payload 하나에 JSON으로
                # 담아, 항목 스키마가 바뀌어도 테이블을 안 고쳐도 되게 한다.
                # apply_status/apply_error: 파일 이동 상태를 건별로 기록한다. payload JSON
                # 안에 두면 "이동 완료된 행만 지운다"가 DELETE ... WHERE payload->>'...'
                # 같은 JSON 경로 조회가 되어 인덱스를 못 타므로, 컬럼으로 분리해 인덱스를 건다.
                cursor.execute(
                    "CREATE TABLE IF NOT EXISTS classify_proposal_items (id INT AUTO_INCREMENT PRIMARY KEY, content_type VARCHAR(10) NOT NULL DEFAULT 'book', source_category VARCHAR(255) NOT NULL, file_path VARCHAR(1024) NOT NULL, payload JSON NOT NULL, apply_status VARCHAR(20) NOT NULL DEFAULT 'pending', apply_error TEXT NULL, created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP, INDEX idx_content_source (content_type, source_category), INDEX idx_content_file (content_type, file_path(255)), INDEX idx_apply_status (content_type, apply_status)) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci"
                )
                # 기존 테이블 마이그레이션: content_type 컬럼이 없으면 추가
                self._migrate_add_content_type(cursor)
                # 기존 테이블 마이그레이션: apply_status/apply_error 컬럼이 없으면 추가
                self._migrate_add_apply_status(cursor)
                # 기존 테이블 마이그레이션: file_path 인덱스 추가 + 중복 인덱스 제거
                self._migrate_classify_proposal_item_indexes(cursor)
                # reload_locks를 진행 상황까지 담는 공유 작업 상태 테이블로 확장
                self._migrate_reload_locks(cursor)
                # reload_locks를 content_type 단일 락에서 (content_type, lock_key) 복합 락으로 확장
                self._migrate_reload_locks_lock_key(cursor)
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

    def _migrate_add_apply_status(self, cursor) -> None:
        """기존 classify_proposal_items 테이블에 apply_status/apply_error 컬럼과 인덱스를 추가한다.

        _migrate_add_content_type과 같은 방식: 이미 테이블이 만들어진 환경이 있으므로
        information_schema로 컬럼 존재 여부를 먼저 확인하고 없을 때만 ALTER한다.
        """
        cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = %s AND table_name = 'classify_proposal_items' AND column_name = 'apply_status'", (self.database,))
        row = cursor.fetchone()
        if row and row["cnt"] == 0:
            LOGGER.info("Migrating table classify_proposal_items: adding apply_status/apply_error columns")
            cursor.execute("ALTER TABLE classify_proposal_items ADD COLUMN apply_status VARCHAR(20) NOT NULL DEFAULT 'pending'")
            cursor.execute("ALTER TABLE classify_proposal_items ADD COLUMN apply_error TEXT NULL")
            cursor.execute("ALTER TABLE classify_proposal_items ADD INDEX idx_apply_status (content_type, apply_status)")

    def _migrate_classify_proposal_item_indexes(self, cursor) -> None:
        """classify_proposal_items 의 인덱스를 (content_type, file_path) 기준으로 맞춘다.

        승인 작업은 책 한 권마다 `WHERE content_type = %s AND file_path = %s` 로 상태를
        갱신한다. file_path 에 인덱스가 없으면 그 조건을 만족하는 행을 찾으려고 매번
        테이블 전체를 훑는다. 79,589 행을 넣고 실측하면 UPDATE 한 건이 63.2ms (전수 훑기)
        이고, 이 인덱스를 붙이면 0.9ms (한 행 조회) 다. 같은 카테고리를 승인할 때 DB 대기만
        약 2.8 시간에서 2.4 분으로 줄어든다. file_path 는 1024 자라 인덱스 길이 제한에
        걸리므로 앞 255 자만 쓴다 — 실제 경로는 그보다 훨씬 짧아 사실상 완전 일치다.

        idx_content_type (content_type) 은 idx_content_source (content_type,
        source_category) 의 왼쪽 접두사와 같아 조회를 하나도 더 처리하지 못하면서 INSERT
        마다 유지 비용만 든다. 함께 지운다.
        """
        cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.statistics WHERE table_schema = %s AND table_name = 'classify_proposal_items' AND index_name = 'idx_content_file'", (self.database,))
        row = cursor.fetchone()
        if row and row["cnt"] == 0:
            LOGGER.info("Migrating table classify_proposal_items: adding idx_content_file")
            cursor.execute("ALTER TABLE classify_proposal_items ADD INDEX idx_content_file (content_type, file_path(255))")

        cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.statistics WHERE table_schema = %s AND table_name = 'classify_proposal_items' AND index_name = 'idx_content_type'", (self.database,))
        row = cursor.fetchone()
        if row and row["cnt"] > 0:
            LOGGER.info("Migrating table classify_proposal_items: dropping redundant idx_content_type")
            cursor.execute("ALTER TABLE classify_proposal_items DROP INDEX idx_content_type")

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
            "reload_source": "ALTER TABLE reload_locks ADD COLUMN reload_source VARCHAR(20) NOT NULL DEFAULT 'bulk'",
        }
        for column, alter_sql in new_columns.items():
            cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = %s AND table_name = 'reload_locks' AND column_name = %s", (self.database, column))
            row = cursor.fetchone()
            if row and row["cnt"] == 0:
                LOGGER.info("Migrating table reload_locks: adding %s column", column)
                cursor.execute(alter_sql)
                if column == "reload_source":
                    cursor.execute("UPDATE reload_locks SET reload_source = %s WHERE category IS NOT NULL", (self.RELOAD_SOURCE_MISMATCH,))

    def _migrate_reload_locks_lock_key(self, cursor) -> None:
        """reload_locks를 content_type 단일 락에서 (content_type, lock_key) 복합 락으로 확장한다.

        기존 행은 category가 있으면 그 카테고리를, 없으면 BULK_LOCK_KEY를 lock_key로 채워
        그대로 보존한다. 이 이후로는 카테고리별 재적재와 일괄 재적재가 서로 다른 행을 쓰므로
        서로 다른 카테고리끼리는 물론, 카테고리별 작업과 일괄 작업도 (일괄이 전체에 영향을
        주는 경우를 제외하고) 독립적으로 동시 진행될 수 있다.
        """
        cursor.execute("SELECT COUNT(*) AS cnt FROM information_schema.columns WHERE table_schema = %s AND table_name = 'reload_locks' AND column_name = 'lock_key'", (self.database,))
        row = cursor.fetchone()
        if row and row["cnt"] > 0:
            return
        LOGGER.info("Migrating table reload_locks: adding lock_key column and switching primary key to (content_type, lock_key)")
        # DDL의 DEFAULT 절은 바인드 파라미터를 지원하지 않으므로, 상수인 BULK_LOCK_KEY를 직접 삽입한다.
        cursor.execute(f"ALTER TABLE reload_locks ADD COLUMN lock_key VARCHAR(255) NOT NULL DEFAULT '{self.BULK_LOCK_KEY}'")
        cursor.execute("UPDATE reload_locks SET lock_key = COALESCE(category, %s)", (self.BULK_LOCK_KEY,))
        cursor.execute("ALTER TABLE reload_locks DROP PRIMARY KEY, ADD PRIMARY KEY (content_type, lock_key)")

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

    def _normalize_reload_source(self, reload_source: str | None = None, category: str | None = None) -> str:
        if category:
            return self.RELOAD_SOURCE_MISMATCH
        if reload_source in self.RELOAD_SOURCES:
            return reload_source
        return self.RELOAD_SOURCE_BULK

    def acquire_reload_lock(self, content_type: str = "book", category: str | None = None, reload_source: str | None = None) -> tuple[bool, str | None, dict[str, Any] | None]:
        """카테고리 불일치 재적재 작업 상태를 초기화하고 락을 획득한다.

        category가 있으면 그 카테고리 전용 락을 쓰고, 없으면 전체(BULK_LOCK_KEY) 락을 쓴다. 일괄
        재적재는 모든 카테고리에 영향을 주므로, 카테고리별 락을 잡을 때는 같은 카테고리 락뿐 아니라
        전체 락도 진행 중이 아닌지 함께 확인하고, 전체 락을 잡을 때는 이 content_type의 어떤 락이든
        진행 중이면 안 된다. 서로 다른 카테고리끼리는 이 검사에 걸리지 않아 독립적으로 동시 진행된다.

        이미 다른 작업이 진행 중이면 (False, 안내 메시지, 그 작업의 현재 상태)를 반환한다. 완료된
        작업도 마지막 결과 조회를 위해 행을 지우지 않고 남겨두므로, INSERT 실패(중복 키)가 아니라
        현재 status/heartbeat를 직접 봐서 획득 가능 여부를 판단한다. 재적재가 pod 재시작 등으로
        heartbeat를 못 남기고 죽었을 경우를 대비해, RELOAD_LOCK_HEARTBEAT_STALE_SECONDS 동안
        updated_at이 갱신되지 않은 'running' 행은 죽은 작업으로 간주하고 무시한다.
        """
        lock_key = category or self.BULK_LOCK_KEY
        normalized_reload_source = self._normalize_reload_source(reload_source, category)
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                if lock_key == self.BULK_LOCK_KEY:
                    cursor.execute("SELECT lock_key, status, updated_at FROM reload_locks WHERE content_type = %s FOR UPDATE", (content_type,))
                else:
                    cursor.execute("SELECT lock_key, status, updated_at FROM reload_locks WHERE content_type = %s AND lock_key IN (%s, %s) FOR UPDATE", (content_type, lock_key, self.BULK_LOCK_KEY))
                rows = cursor.fetchall()

                blocking_key = None
                for row in rows:
                    if row["status"] != "running":
                        continue
                    if (datetime.now() - row["updated_at"]) <= timedelta(seconds=self.RELOAD_LOCK_HEARTBEAT_STALE_SECONDS):
                        blocking_key = row["lock_key"]
                        break
                    LOGGER.warning("acquire_reload_lock(%s/%s): heartbeat(%s) 정지된 죽은 락(%s) 감지, 무시하고 진행", content_type, lock_key, row["updated_at"], row["lock_key"])

                if blocking_key is not None:
                    conn.commit()
                    blocking_category = None if blocking_key == self.BULK_LOCK_KEY else blocking_key
                    blocking_status = self.get_reload_status(content_type, blocking_category)
                    if blocking_key == lock_key:
                        message = "이미 재적재 작업이 진행 중입니다. 완료 후 다시 시도하세요."
                    else:
                        message = "다른 재적재 작업이 진행 중이라 지금은 실행할 수 없습니다. 완료 후 다시 시도하세요."
                    return False, message, blocking_status

                cursor.execute(
                    "INSERT INTO reload_locks (content_type, lock_key, category, reload_source, status, started_at, updated_at, indexed_count, deleted_count, failed_count, before_count, after_count, error) "
                    "VALUES (%s, %s, %s, %s, 'running', NOW(), NOW(), 0, 0, 0, 0, 0, NULL) "
                    "ON DUPLICATE KEY UPDATE category = VALUES(category), status = 'running', started_at = NOW(), updated_at = NOW(), "
                    "reload_source = VALUES(reload_source), indexed_count = 0, deleted_count = 0, failed_count = 0, before_count = 0, after_count = 0, error = NULL",
                    (content_type, lock_key, category, normalized_reload_source),
                )
                conn.commit()
                return True, None, None

    def heartbeat_reload_lock(self, content_type: str = "book", category: str | None = None, **progress_counts: int) -> None:
        """재적재 진행 중 heartbeat(updated_at)와 진행 카운트를 갱신한다."""
        allowed = {"indexed_count", "deleted_count", "failed_count", "before_count", "after_count"}
        unknown = set(progress_counts) - allowed
        if unknown:
            raise ValueError(f"알 수 없는 진행 카운트 필드: {unknown}")
        lock_key = category or self.BULK_LOCK_KEY
        set_clause = "".join(f", {field} = %s" for field in progress_counts)
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute(f"UPDATE reload_locks SET updated_at = NOW(){set_clause} WHERE content_type = %s AND lock_key = %s", (*progress_counts.values(), content_type, lock_key))
                conn.commit()

    def complete_reload_lock(self, content_type: str = "book", status: str = "done", error: str | None = None, category: str | None = None, **final_counts: int) -> None:
        """재적재 작업 완료(성공/실패)를 기록한다. 행은 삭제하지 않고 상태만 남겨, 새로고침 후에도
        마지막 결과를 볼 수 있게 한다. 다음 acquire_reload_lock 호출 시 덮어써진다."""
        allowed = {"indexed_count", "deleted_count", "failed_count", "before_count", "after_count"}
        unknown = set(final_counts) - allowed
        if unknown:
            raise ValueError(f"알 수 없는 진행 카운트 필드: {unknown}")
        lock_key = category or self.BULK_LOCK_KEY
        set_clause = "".join(f", {field} = %s" for field in final_counts)
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute(f"UPDATE reload_locks SET status = %s, updated_at = NOW(), error = %s{set_clause} WHERE content_type = %s AND lock_key = %s", (status, error, *final_counts.values(), content_type, lock_key))
                conn.commit()

    def get_reload_status(self, content_type: str = "book", category: str | None = None) -> dict[str, Any] | None:
        """재적재 작업 상태를 조회한다. category가 있으면 그 카테고리 전용 락을, 없으면 전체
        (BULK_LOCK_KEY) 락을 조회한다. 작업 이력이 없으면 None.

        status가 'running'인데 heartbeat가 RELOAD_LOCK_HEARTBEAT_STALE_SECONDS 이상 끊겼으면,
        DB 행을 고치지 않고 조회 결과에서만 'failed'로 보여준다(실제 재획득/정리는
        acquire_reload_lock이 다음 시작 시점에 처리).
        """
        lock_key = category or self.BULK_LOCK_KEY
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT category, reload_source, status, started_at, updated_at, indexed_count, deleted_count, failed_count, before_count, after_count, error FROM reload_locks WHERE content_type = %s AND lock_key = %s", (content_type, lock_key))
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
            "reload_source": self._normalize_reload_source(row.get("reload_source"), row["category"]),
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

    def release_reload_lock(self, content_type: str = "book", category: str | None = None) -> None:
        """재적재 락 행을 완전히 지운다 (일반 완료 경로에서는 complete_reload_lock을 쓴다)."""
        lock_key = category or self.BULK_LOCK_KEY
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("DELETE FROM reload_locks WHERE content_type = %s AND lock_key = %s", (content_type, lock_key))
                conn.commit()

    def clear_classify_proposal_items(self, content_type: str = "book") -> None:
        """새 분류 제안을 시작할 때 이 content_type의 기존 제안 항목을 전부 지운다.

        지우지 않으면 이전 제안의 행이 새 제안의 행과 뒤섞여, 관리자가 이미 끝난
        이전 작업의 책까지 새 제안으로 착각하고 승인할 수 있다.
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("DELETE FROM classify_proposal_items WHERE content_type = %s", (content_type,))
                conn.commit()

    def add_classify_proposal_items(self, items: list[dict[str, Any]], source_category: str, content_type: str = "book") -> None:
        """분류된 항목들을 한 번에 적재한다.

        책 한 권마다 INSERT + COMMIT을 하면 커넥션 왕복과 트랜잭션 커밋이 책 수만큼
        생긴다. 호출자가 여러 건을 모아 한 번에 넘기면, 여기서는 executemany 한 번과
        commit 한 번으로 끝나 왕복 비용이 호출 횟수만큼만 생긴다.
        """
        if not items:
            return
        rows = [(content_type, source_category, item.get("file_path"), json.dumps(item, ensure_ascii=False)) for item in items]
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.executemany("INSERT INTO classify_proposal_items (content_type, source_category, file_path, payload) VALUES (%s, %s, %s, %s)", rows)
                conn.commit()

    def get_classify_proposal_items(self, content_type: str = "book") -> list[dict[str, Any]]:
        """제안 항목을 분류가 끝난 순서(id 오름차순) 그대로 돌려준다.

        화면에 보이는 순서가 분류 완료 순서와 같아야 관리자가 진행 상황을
        직관적으로 따라갈 수 있다. apply_status/apply_error는 payload가 아니라
        컬럼에 있으므로 payload를 푼 뒤 덮어써 함께 실어준다.
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("SELECT payload, apply_status, apply_error FROM classify_proposal_items WHERE content_type = %s ORDER BY id ASC", (content_type,))
                rows = cursor.fetchall()
        items = []
        for row in rows:
            item = json.loads(row["payload"])
            item["apply_status"] = row["apply_status"]
            item["apply_error"] = row["apply_error"]
            items.append(item)
        return items

    def update_classify_proposal_item_status(self, file_path: str, apply_status: str, apply_error: str | None = None, content_type: str = "book") -> None:
        """파일 하나를 옮긴 직후 그 행 하나만 바로 기록한다.

        승인 작업을 끝까지 돈 뒤 한꺼번에 기록하면, 도중에 중단됐을 때 무엇이
        옮겨졌는지 알 수 없다. 건별로 바로 기록해야 중단 후 재개가 가능하다.
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("UPDATE classify_proposal_items SET apply_status = %s, apply_error = %s WHERE content_type = %s AND file_path = %s", (apply_status, apply_error, content_type, file_path))
            conn.commit()

    def delete_applied_classify_proposal_items(self, content_type: str = "book") -> int:
        """이동이 끝난(apply_status = 'moved') 행만 지우고, 지운 개수를 돌려준다.

        대기·실패 행은 남겨야 관리자가 재시도하거나 목적지를 고쳐 다시 승인할 수 있다.
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("DELETE FROM classify_proposal_items WHERE content_type = %s AND apply_status = 'moved'", (content_type,))
                deleted_count = cursor.rowcount
            conn.commit()
        return deleted_count
