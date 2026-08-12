import logging
import os
import time
from contextlib import contextmanager
from typing import Any

import pymysql
from pymysql.cursors import DictCursor

LOGGER = logging.getLogger(__name__)

# 사용자·콘텐츠 유형별로 보관하는 최근 조회 건수. 초과분은 기록 시점에 정리하므로
# 테이블 크기가 (사용자 수 × 유형 2 × 이 값) 으로 고정되고 별도 purge 가 필요 없다.
MAX_RECENT_VIEWS = 50

CONTENT_TYPES = ("book", "comic")

# 조회 목록 전체를 읽을 때의 방어적 상한. record_view 가 사용자별로 정리하므로 정상
# 상태에서는 도달하지 않는다.
MAX_HISTORY_ROWS = 20000

_CREATE_TABLE_SQL = """
    CREATE TABLE IF NOT EXISTS view_history (
        id BIGINT AUTO_INCREMENT PRIMARY KEY,
        email VARCHAR(320) NOT NULL,
        content_type VARCHAR(10) NOT NULL,
        book_id INT NOT NULL,
        title VARCHAR(512) NOT NULL,
        category VARCHAR(255) NOT NULL DEFAULT '',
        viewed_at BIGINT NOT NULL,
        UNIQUE KEY uniq_view_history (email, content_type, book_id),
        INDEX idx_view_history_recent (email, content_type, viewed_at)
    ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci
"""

# 같은 책을 다시 열면 새 row 를 만들지 않고 조회 시각과 스냅샷만 갱신한다.
# 덕분에 목록에 같은 책이 중복으로 나오지 않는다.
_UPSERT_SQL = """
    INSERT INTO view_history (email, content_type, book_id, title, category, viewed_at)
    VALUES (%s, %s, %s, %s, %s, %s)
    ON DUPLICATE KEY UPDATE
        title = VALUES(title),
        category = VALUES(category),
        viewed_at = VALUES(viewed_at)
"""

# 최신 N 건만 남기고 나머지를 지운다. MySQL 은 IN 서브쿼리에 LIMIT 을 직접 쓸 수 없어
# 파생 테이블로 한 번 감싼다.
_TRIM_SQL = """
    DELETE FROM view_history
    WHERE email = %s AND content_type = %s AND id NOT IN (
        SELECT id FROM (
            SELECT id FROM view_history
            WHERE email = %s AND content_type = %s
            ORDER BY viewed_at DESC, id DESC
            LIMIT %s
        ) AS keep
    )
"""

_SELECT_ALL_SQL = """
    SELECT email, content_type, book_id, title, category, viewed_at
    FROM view_history
    ORDER BY viewed_at DESC, id DESC
    LIMIT %s
"""


class ViewHistoryStore:
    """사용자별 최근 조회 이력을 MySQL 에 보관한다.

    운영은 k8s 멀티 replica 라 pod 로컬 저장은 이력이 파편화되므로 MySQL 전용이다
    (`CategoryMapping` 과 같은 구성).

    제목·카테고리는 조회 시점의 스냅샷으로 저장한다. 책이 삭제되거나 이동해도 이력이
    남고, 조회 때 원본을 join 할 필요가 없다.
    """

    def __init__(self, host: str | None = None, port: int | None = None, database: str | None = None, user: str | None = None, password: str | None = None) -> None:
        self.host = host or os.environ.get("TM_MYSQL_HOST", "localhost")
        self.port = port or int(os.environ.get("TM_MYSQL_PORT", "3306"))
        self.database = database or os.environ.get("TM_MYSQL_DATABASE", "textmanager")
        self.user = user or os.environ.get("TM_MYSQL_USER", "textmanager")
        self.password = password if password is not None else os.environ.get("TM_MYSQL_PASSWORD", "")
        self._init_db()

    @contextmanager
    def _get_connection(self):
        conn = pymysql.connect(host=self.host, port=self.port, database=self.database, user=self.user, password=self.password, charset="utf8mb4", cursorclass=DictCursor, autocommit=False)
        try:
            yield conn
        finally:
            conn.close()

    def _init_db(self) -> None:
        with self._get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute(_CREATE_TABLE_SQL)
            conn.commit()

    def record_view(self, *, email: str, content_type: str, book_id: int, title: str, category: str = "") -> None:
        """조회 1건을 기록하고 사용자·유형별 보관 상한을 유지한다."""
        if content_type not in CONTENT_TYPES:
            raise ValueError(f"unknown content_type: {content_type}")
        now = int(time.time())
        with self._get_connection() as conn:
            try:
                conn.begin()
                with conn.cursor() as cur:
                    cur.execute(_UPSERT_SQL, (email, content_type, book_id, title, category, now))
                    cur.execute(_TRIM_SQL, (email, content_type, email, content_type, MAX_RECENT_VIEWS))
                conn.commit()
            except Exception:
                conn.rollback()
                raise

    def list_recent_views(self, *, limit: int = MAX_RECENT_VIEWS) -> dict[str, Any]:
        """사용자별로 책/만화 최근 조회 목록을 돌려준다.

        SQL 이 이미 최신순으로 정렬해 주므로 여기서는 사용자별 그룹핑과 유형별 상한만
        적용한다. 사용자는 마지막 조회가 최근인 순서로 정렬한다.
        """
        with self._get_connection() as conn:
            with conn.cursor() as cur:
                cur.execute(_SELECT_ALL_SQL, (MAX_HISTORY_ROWS,))
                rows = list(cur.fetchall() or [])
        if len(rows) >= MAX_HISTORY_ROWS:
            LOGGER.warning("조회 이력이 상한(%d)에 도달해 잘렸습니다.", MAX_HISTORY_ROWS)

        users: dict[str, dict[str, Any]] = {}
        for row in rows:
            email = row["email"]
            user = users.setdefault(email, {"email": email, "last_viewed_at": 0, "book": [], "comic": []})
            bucket = user.get(row["content_type"])
            if bucket is None:
                # 알 수 없는 content_type 은 조용히 버리지 않고 남긴다.
                LOGGER.warning("알 수 없는 content_type 이력을 건너뜁니다: %s", row["content_type"])
                continue
            viewed_at = int(row["viewed_at"])
            user["last_viewed_at"] = max(user["last_viewed_at"], viewed_at)
            if len(bucket) < limit:
                bucket.append({"book_id": int(row["book_id"]), "title": row["title"], "category": row["category"], "viewed_at": viewed_at})

        ordered = sorted(users.values(), key=lambda u: u["last_viewed_at"], reverse=True)
        return {"limit": limit, "users": ordered}


def create_view_history_store() -> ViewHistoryStore:
    return ViewHistoryStore()
