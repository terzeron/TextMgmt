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
                        content_type VARCHAR(10) NOT NULL DEFAULT 'book',
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                        UNIQUE KEY unique_category_keyword (category, keyword, content_type),
                        INDEX idx_category (category),
                        INDEX idx_content_type (content_type)
                    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
                """)
                cursor.execute("""
                    CREATE TABLE IF NOT EXISTS hidden_categories (
                        id INT AUTO_INCREMENT PRIMARY KEY,
                        category VARCHAR(255) NOT NULL,
                        content_type VARCHAR(10) NOT NULL DEFAULT 'book',
                        created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                        UNIQUE KEY unique_category_content_type (category, content_type),
                        INDEX idx_category (category),
                        INDEX idx_content_type (content_type)
                    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
                """)
                # 기존 테이블 마이그레이션: content_type 컬럼이 없으면 추가
                self._migrate_add_content_type(cursor)
                conn.commit()
        LOGGER.debug("Database initialized")

    def _migrate_add_content_type(self, cursor) -> None:
        """기존 테이블에 content_type 컬럼 추가 마이그레이션"""
        for table in ("category_keywords", "hidden_categories"):
            cursor.execute(f"""
                SELECT COUNT(*) AS cnt FROM information_schema.columns
                WHERE table_schema = %s AND table_name = %s AND column_name = 'content_type'
            """, (self.database, table))
            row = cursor.fetchone()
            if row and row["cnt"] == 0:
                LOGGER.info("Migrating table %s: adding content_type column", table)
                cursor.execute(f"""
                    ALTER TABLE {table}
                    ADD COLUMN content_type VARCHAR(10) NOT NULL DEFAULT 'book'
                """)
                if table == "category_keywords":
                    try:
                        cursor.execute(f"ALTER TABLE {table} DROP INDEX unique_category_keyword")
                    except Exception:
                        pass
                    cursor.execute(f"""
                        ALTER TABLE {table}
                        ADD UNIQUE KEY unique_category_keyword (category, keyword, content_type)
                    """)
                elif table == "hidden_categories":
                    try:
                        cursor.execute(f"ALTER TABLE {table} DROP INDEX category")
                    except Exception:
                        pass
                    cursor.execute(f"""
                        ALTER TABLE {table}
                        ADD UNIQUE KEY unique_category_content_type (category, content_type)
                    """)

    def get_all_mappings(self, content_type: str = "book") -> Dict[str, List[str]]:
        """모든 카테고리-키워드 매핑 조회

        Args:
            content_type: 콘텐츠 유형 ('book' 또는 'comic')

        Returns:
            {category: [keyword1, keyword2, ...], ...} 형태의 딕셔너리
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT category, keyword
                    FROM category_keywords
                    WHERE content_type = %s
                    ORDER BY category, keyword
                """, (content_type,))
                rows = cursor.fetchall()

        mappings: Dict[str, List[str]] = {}
        for row in rows:
            category = row["category"]
            keyword = row["keyword"]
            if category not in mappings:
                mappings[category] = []
            mappings[category].append(keyword)

        LOGGER.debug("get_all_mappings(%s): %d categories", content_type, len(mappings))
        return mappings

    def get_keywords(self, category: str, content_type: str = "book") -> List[str]:
        """특정 카테고리의 키워드 목록 조회

        Args:
            category: 카테고리명
            content_type: 콘텐츠 유형

        Returns:
            키워드 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT keyword
                    FROM category_keywords
                    WHERE category = %s AND content_type = %s
                    ORDER BY keyword
                """, (category, content_type))
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
                    cursor.execute("""
                        INSERT INTO category_keywords (category, keyword, content_type)
                        VALUES (%s, %s, %s)
                    """, (category, keyword, content_type))
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
                cursor.execute("""
                    DELETE FROM category_keywords
                    WHERE category = %s AND keyword = %s AND content_type = %s
                """, (category, keyword, content_type))
                conn.commit()
                deleted = cursor.rowcount > 0

        LOGGER.info("remove_keyword(%s, %s, %s): %s", category, keyword, content_type, "success" if deleted else "not found")
        return deleted

    def set_keywords(self, category: str, keywords: List[str], content_type: str = "book") -> bool:
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
                    cursor.execute("""
                        DELETE FROM category_keywords
                        WHERE category = %s AND content_type = %s
                    """, (category, content_type))

                    # 새 키워드 추가
                    for keyword in keywords:
                        cursor.execute("""
                            INSERT INTO category_keywords (category, keyword, content_type)
                            VALUES (%s, %s, %s)
                        """, (category, keyword, content_type))

                    conn.commit()
                    LOGGER.info("set_keywords(%s, %s): %d keywords set", category, content_type, len(keywords))
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("set_keywords(%s, %s) failed: %s", category, content_type, e)
                    return False

    def update_all_mappings(self, mappings: Dict[str, List[str]], content_type: str = "book") -> bool:
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

                    # 새 데이터 추가
                    for category, keywords in mappings.items():
                        for keyword in keywords:
                            keyword = keyword.strip()
                            if keyword:
                                cursor.execute("""
                                    INSERT IGNORE INTO category_keywords (category, keyword, content_type)
                                    VALUES (%s, %s, %s)
                                """, (category, keyword, content_type))

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
                    cursor.execute("""
                        DELETE FROM category_keywords
                        WHERE (category = %s OR category LIKE %s) AND content_type = %s
                    """, (category, category + "/%", content_type))
                else:
                    cursor.execute("""
                        DELETE FROM category_keywords
                        WHERE category = %s AND content_type = %s
                    """, (category, content_type))
                conn.commit()
                deleted = cursor.rowcount > 0

        LOGGER.info("delete_category(%s, %s, prefix=%s): %s", category, content_type, prefix, "success" if deleted else "not found")
        return deleted

    def get_categories_with_keywords(self, content_type: str = "book") -> List[str]:
        """키워드가 등록된 카테고리 목록 조회

        Args:
            content_type: 콘텐츠 유형

        Returns:
            카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT DISTINCT category
                    FROM category_keywords
                    WHERE content_type = %s
                    ORDER BY category
                """, (content_type,))
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("get_categories_with_keywords(%s): %d categories", content_type, len(categories))
        return categories

    def search_by_keyword(self, keyword: str, content_type: str = "book") -> List[str]:
        """키워드로 카테고리 검색 (부분 일치)

        Args:
            keyword: 검색할 키워드
            content_type: 콘텐츠 유형

        Returns:
            매칭되는 카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT DISTINCT category
                    FROM category_keywords
                    WHERE keyword LIKE %s AND content_type = %s
                    ORDER BY category
                """, (f"%{keyword}%", content_type))
                rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("search_by_keyword(%s, %s): %d categories", keyword, content_type, len(categories))
        return categories

    def get_hidden_categories(self, content_type: str = "book") -> List[str]:
        """비노출 카테고리 목록 조회

        Args:
            content_type: 콘텐츠 유형

        Returns:
            비노출 설정된 카테고리 목록
        """
        with self._get_connection() as conn:
            with conn.cursor() as cursor:
                cursor.execute("""
                    SELECT category
                    FROM hidden_categories
                    WHERE content_type = %s
                    ORDER BY category
                """, (content_type,))
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
                        cursor.execute("""
                            INSERT IGNORE INTO hidden_categories (category, content_type)
                            VALUES (%s, %s)
                        """, (category, content_type))
                        conn.commit()
                        LOGGER.info("set_hidden(%s, True, %s): success", category, content_type)
                        return True
                    except Exception as e:
                        LOGGER.error("set_hidden(%s, True, %s) failed: %s", category, content_type, e)
                        return False
                else:
                    cursor.execute("""
                        DELETE FROM hidden_categories
                        WHERE category = %s AND content_type = %s
                    """, (category, content_type))
                    conn.commit()
                    LOGGER.info("set_hidden(%s, False, %s): success", category, content_type)
                    return True

    def rename_category(self, old_category: str, new_category: str, content_type: str = "book") -> bool:
        """카테고리명을 변경 (category_keywords, hidden_categories 테이블 모두 갱신)

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
                    cursor.execute("""
                        UPDATE category_keywords
                        SET category = %s
                        WHERE category = %s AND content_type = %s
                    """, (new_category, old_category, content_type))

                    cursor.execute("""
                        UPDATE hidden_categories
                        SET category = %s
                        WHERE category = %s AND content_type = %s
                    """, (new_category, old_category, content_type))

                    conn.commit()
                    LOGGER.info("rename_category(%s -> %s, %s): success", old_category, new_category, content_type)
                    return True
                except Exception as e:
                    conn.rollback()
                    LOGGER.error("rename_category(%s -> %s, %s) failed: %s", old_category, new_category, content_type, e)
                    return False

    def is_hidden(self, category: str, content_type: str = "book") -> bool:
        """카테고리의 비노출 여부 확인 (prefix 매칭 포함)

        부모 카테고리가 비노출이면 자식 카테고리도 비노출로 판단합니다.

        Args:
            category: 카테고리명
            content_type: 콘텐츠 유형

        Returns:
            비노출 여부
        """
        hidden_categories = self.get_hidden_categories(content_type=content_type)
        for hidden_cat in hidden_categories:
            if category == hidden_cat or category.startswith(hidden_cat + "/"):
                return True
        return False
