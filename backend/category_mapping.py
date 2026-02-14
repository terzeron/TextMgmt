#!/usr/bin/env python

import os
import logging.config
from pathlib import Path
from typing import Dict, List, Optional
from contextlib import contextmanager

import pymysql
from pymysql.cursors import DictCursor

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class CategoryMapping:
    """카테고리별 키워드 매핑을 관리하는 클래스 (MySQL 기반)"""

    def __init__(
        self,
        host: Optional[str] = None,
        port: Optional[int] = None,
        database: Optional[str] = None,
        user: Optional[str] = None,
        password: Optional[str] = None
    ) -> None:
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

        LOGGER.info("CategoryMapping initialized with MySQL host: %s:%d, database: %s",
                    self.host, self.port, self.database)
        self._init_db()

    @contextmanager
    def _get_connection(self):
        """MySQL 연결을 관리하는 context manager"""
        conn = pymysql.connect(
            host=self.host,
            port=self.port,
            database=self.database,
            user=self.user,
            password=self.password,
            charset='utf8mb4',
            cursorclass=DictCursor
        )
        try:
            yield conn
        finally:
            conn.close()

    def _init_db(self) -> None:
        """데이터베이스 테이블 초기화"""
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    CREATE TABLE IF NOT EXISTS category_keywords (
                        id INT AUTO_INCREMENT PRIMARY KEY,
                        category VARCHAR(255) NOT NULL,
                        keyword VARCHAR(255) NOT NULL,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                        UNIQUE KEY unique_category_keyword (category, keyword),
                        INDEX idx_category (category)
                    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
                """)
                cursor.execute("""
                    CREATE TABLE IF NOT EXISTS hidden_categories (
                        id INT AUTO_INCREMENT PRIMARY KEY,
                        category VARCHAR(255) NOT NULL UNIQUE,
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                        INDEX idx_category (category)
                    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
                """)
                conn.commit()
        LOGGER.debug("Database initialized")

    def get_all_mappings(self) -> Dict[str, List[str]]:
        """모든 카테고리-키워드 매핑 조회

        Returns:
            {category: [keyword1, keyword2, ...], ...} 형태의 딕셔너리
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT category, keyword
                    FROM category_keywords
                    ORDER BY category, keyword
                """)
                rows = cursor.fetchall()

        mappings: Dict[str, List[str]] = {}
        for row in rows:
            category = row["category"]
            keyword = row["keyword"]
            if category not in mappings:
                mappings[category] = []
            mappings[category].append(keyword)

        LOGGER.debug("get_all_mappings: %d categories", len(mappings))
        return mappings

    def get_keywords(self, category: str) -> List[str]:
        """특정 카테고리의 키워드 목록 조회

        Args:
            category: 카테고리명

        Returns:
            키워드 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT keyword
                    FROM category_keywords
                    WHERE category = %s
                    ORDER BY keyword
                """, (category,))
                rows = cursor.fetchall()

        keywords = [row["keyword"] for row in rows]
        LOGGER.debug("get_keywords(%s): %d keywords", category, len(keywords))
        return keywords

    def add_keyword(self, category: str, keyword: str) -> bool:
        """카테고리에 키워드 추가

        Args:
            category: 카테고리명
            keyword: 추가할 키워드

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
                    cursor.execute("""
                        INSERT INTO category_keywords (category, keyword)
                        VALUES (%s, %s)
                    """, (category, keyword))
                    conn.commit()
                    LOGGER.info("add_keyword(%s, %s): success", category, keyword)
                    return True
                except pymysql.IntegrityError:
                    LOGGER.warning("add_keyword(%s, %s): already exists", category, keyword)
                    return False

    def remove_keyword(self, category: str, keyword: str) -> bool:
        """카테고리에서 키워드 삭제

        Args:
            category: 카테고리명
            keyword: 삭제할 키워드

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    DELETE FROM category_keywords
                    WHERE category = %s AND keyword = %s
                """, (category, keyword))
                conn.commit()
                deleted = cursor.rowcount > 0

        LOGGER.info("remove_keyword(%s, %s): %s", category, keyword, "success" if deleted else "not found")
        return deleted

    def set_keywords(self, category: str, keywords: List[str]) -> bool:
        """카테고리의 키워드 목록을 일괄 설정 (기존 키워드 대체)

        Args:
            category: 카테고리명
            keywords: 새 키워드 목록

        Returns:
            성공 여부
        """
        # 빈 문자열 제거 및 중복 제거
        keywords = list(set(k.strip() for k in keywords if k.strip()))

        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    # 기존 키워드 삭제
                    cursor.execute("""
                        DELETE FROM category_keywords
                        WHERE category = %s
                    """, (category,))

                    # 새 키워드 추가
                    for keyword in keywords:
                        cursor.execute("""
                            INSERT INTO category_keywords (category, keyword)
                            VALUES (%s, %s)
                        """, (category, keyword))

                    conn.commit()
                    LOGGER.info("set_keywords(%s): %d keywords set", category, len(keywords))
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("set_keywords(%s) failed: %s", category, e)
                    return False

    def update_all_mappings(self, mappings: Dict[str, List[str]]) -> bool:
        """전체 매핑을 일괄 업데이트

        Args:
            mappings: {category: [keyword1, keyword2, ...], ...} 형태의 딕셔너리

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    # 기존 데이터 전체 삭제
                    cursor.execute("DELETE FROM category_keywords")

                    # 새 데이터 추가
                    for category, keywords in mappings.items():
                        for keyword in keywords:
                            keyword = keyword.strip()
                            if keyword:
                                cursor.execute("""
                                    INSERT IGNORE INTO category_keywords (category, keyword)
                                    VALUES (%s, %s)
                                """, (category, keyword))

                    conn.commit()
                    LOGGER.info("update_all_mappings: %d categories updated", len(mappings))
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("update_all_mappings failed: %s", e)
                    return False

    def delete_category(self, category: str) -> bool:
        """카테고리의 모든 키워드 삭제

        Args:
            category: 카테고리명

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    DELETE FROM category_keywords
                    WHERE category = %s
                """, (category,))
                conn.commit()
                deleted = cursor.rowcount > 0

        LOGGER.info("delete_category(%s): %s", category, "success" if deleted else "not found")
        return deleted

    def get_categories_with_keywords(self) -> List[str]:
        """키워드가 등록된 카테고리 목록 조회

        Returns:
            카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT DISTINCT category
                    FROM category_keywords
                    ORDER BY category
                """)
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("get_categories_with_keywords: %d categories", len(categories))
        return categories

    def search_by_keyword(self, keyword: str) -> List[str]:
        """키워드로 카테고리 검색 (부분 일치)

        Args:
            keyword: 검색할 키워드

        Returns:
            매칭되는 카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT DISTINCT category
                    FROM category_keywords
                    WHERE keyword LIKE %s
                    ORDER BY category
                """, (f"%{keyword}%",))
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("search_by_keyword(%s): %d categories", keyword, len(categories))
        return categories

    def get_hidden_categories(self) -> List[str]:
        """비노출 카테고리 목록 조회

        Returns:
            비노출 설정된 카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT category
                    FROM hidden_categories
                    ORDER BY category
                """)
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("get_hidden_categories: %d categories", len(categories))
        return categories

    def set_hidden(self, category: str, hidden: bool) -> bool:
        """카테고리의 비노출 설정/해제

        Args:
            category: 카테고리명
            hidden: True면 비노출 설정, False면 해제

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                if hidden:
                    try:
                        cursor.execute("""
                            INSERT IGNORE INTO hidden_categories (category)
                            VALUES (%s)
                        """, (category,))
                        conn.commit()
                        LOGGER.info("set_hidden(%s, True): success", category)
                        return True
                    except Exception as e:
                        LOGGER.error("set_hidden(%s, True) failed: %s", category, e)
                        return False
                else:
                    cursor.execute("""
                        DELETE FROM hidden_categories
                        WHERE category = %s
                    """, (category,))
                    conn.commit()
                    LOGGER.info("set_hidden(%s, False): success", category)
                    return True

    def rename_category(self, old_category: str, new_category: str) -> bool:
        """카테고리명을 변경 (category_keywords, hidden_categories 테이블 모두 갱신)

        트랜잭션으로 원자적 처리한다.

        Args:
            old_category: 기존 카테고리명
            new_category: 새 카테고리명

        Returns:
            성공 여부
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                try:
                    cursor.execute("""
                        UPDATE category_keywords
                        SET category = %s
                        WHERE category = %s
                    """, (new_category, old_category))

                    cursor.execute("""
                        UPDATE hidden_categories
                        SET category = %s
                        WHERE category = %s
                    """, (new_category, old_category))

                    conn.commit()
                    LOGGER.info("rename_category(%s -> %s): success", old_category, new_category)
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("rename_category(%s -> %s) failed: %s", old_category, new_category, e)
                    return False

    def is_hidden(self, category: str) -> bool:
        """카테고리의 비노출 여부 확인 (prefix 매칭 포함)

        부모 카테고리가 비노출이면 자식 카테고리도 비노출로 판단합니다.

        Args:
            category: 카테고리명

        Returns:
            비노출 여부
        """
        hidden_categories = self.get_hidden_categories()
        for hidden_cat in hidden_categories:
            if category == hidden_cat or category.startswith(hidden_cat + "/"):
                return True
        return False
