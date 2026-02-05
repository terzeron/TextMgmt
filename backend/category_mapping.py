#!/usr/bin/env python

import os
import sqlite3
import logging.config
from pathlib import Path
from typing import Dict, List, Optional
from contextlib import contextmanager

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger(__name__)


class CategoryMapping:
    """카테고리별 키워드 매핑을 관리하는 클래스 (SQLite 기반)"""

    def __init__(self, db_path: Optional[str] = None) -> None:
        """
        Args:
            db_path: SQLite 데이터베이스 파일 경로. None이면 환경변수 또는 기본값 사용
        """
        if db_path:
            self.db_path = db_path
        else:
            # 환경변수에서 경로 가져오기, 없으면 기본값 사용
            default_path = Path(__file__).parent / "category_mapping.db"
            self.db_path = os.environ.get("TM_CATEGORY_MAPPING_DB", str(default_path))

        LOGGER.info("CategoryMapping initialized with db_path: %s", self.db_path)
        self._init_db()

    @contextmanager
    def _get_connection(self):
        """SQLite 연결을 관리하는 context manager"""
        conn = sqlite3.connect(self.db_path)
        conn.row_factory = sqlite3.Row
        try:
            yield conn
        finally:
            conn.close()

    def _init_db(self) -> None:
        """데이터베이스 테이블 초기화"""
        with self._get_connection() as conn:
            cursor = conn.cursor()
            cursor.execute("""
                CREATE TABLE IF NOT EXISTS category_keywords (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    category TEXT NOT NULL,
                    keyword TEXT NOT NULL,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    UNIQUE(category, keyword)
                )
            """)
            cursor.execute("""
                CREATE INDEX IF NOT EXISTS idx_category
                ON category_keywords(category)
            """)
            conn.commit()
        LOGGER.debug("Database initialized")

    def get_all_mappings(self) -> Dict[str, List[str]]:
        """모든 카테고리-키워드 매핑 조회

        Returns:
            {category: [keyword1, keyword2, ...], ...} 형태의 딕셔너리
        """
        with self._get_connection() as conn:
            cursor = conn.cursor()
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
            cursor = conn.cursor()
            cursor.execute("""
                SELECT keyword
                FROM category_keywords
                WHERE category = ?
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
            cursor = conn.cursor()
            try:
                cursor.execute("""
                    INSERT INTO category_keywords (category, keyword)
                    VALUES (?, ?)
                """, (category, keyword))
                conn.commit()
                LOGGER.info("add_keyword(%s, %s): success", category, keyword)
                return True
            except sqlite3.IntegrityError:
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
            cursor = conn.cursor()
            cursor.execute("""
                DELETE FROM category_keywords
                WHERE category = ? AND keyword = ?
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
            cursor = conn.cursor()
            try:
                # 기존 키워드 삭제
                cursor.execute("""
                    DELETE FROM category_keywords
                    WHERE category = ?
                """, (category,))

                # 새 키워드 추가
                for keyword in keywords:
                    cursor.execute("""
                        INSERT INTO category_keywords (category, keyword)
                        VALUES (?, ?)
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
            cursor = conn.cursor()
            try:
                # 기존 데이터 전체 삭제
                cursor.execute("DELETE FROM category_keywords")

                # 새 데이터 추가
                for category, keywords in mappings.items():
                    for keyword in keywords:
                        keyword = keyword.strip()
                        if keyword:
                            cursor.execute("""
                                INSERT OR IGNORE INTO category_keywords (category, keyword)
                                VALUES (?, ?)
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
            cursor = conn.cursor()
            cursor.execute("""
                DELETE FROM category_keywords
                WHERE category = ?
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
            cursor = conn.cursor()
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
            cursor = conn.cursor()
            cursor.execute("""
                SELECT DISTINCT category
                FROM category_keywords
                WHERE keyword LIKE ?
                ORDER BY category
            """, (f"%{keyword}%",))
            rows = cursor.fetchall()

        categories = [row["category"] for row in rows]
        LOGGER.debug("search_by_keyword(%s): %d categories", keyword, len(categories))
        return categories
