import logging
import time
from contextlib import contextmanager

import pytest

import backend.view_history_store as view_history_store_mod
from backend.view_history_store import MAX_RECENT_VIEWS, ViewHistoryStore, create_view_history_store


@pytest.fixture()
def store(mysql_container):
    """실제 MySQL 컨테이너에 붙은 스토어. 테스트마다 테이블을 비운다."""
    s = ViewHistoryStore()
    with s._get_connection() as conn:
        with conn.cursor() as cur:
            cur.execute("TRUNCATE TABLE view_history")
        conn.commit()
    return s


def _emails(result):
    return [u["email"] for u in result["users"]]


def _titles(result, email, content_type):
    user = next(u for u in result["users"] if u["email"] == email)
    return [item["title"] for item in user[content_type]]


def test_record_and_list_single_view(store):
    store.record_view(email="a@example.com", content_type="book", book_id=1, title="책 하나", category="소설")

    result = store.list_recent_views()

    assert result["limit"] == MAX_RECENT_VIEWS
    assert _emails(result) == ["a@example.com"]
    user = result["users"][0]
    assert user["comic"] == []
    assert len(user["book"]) == 1
    assert user["book"][0]["book_id"] == 1
    assert user["book"][0]["title"] == "책 하나"
    assert user["book"][0]["category"] == "소설"
    assert user["last_viewed_at"] == user["book"][0]["viewed_at"]


def test_reviewing_same_book_does_not_duplicate(store):
    store.record_view(email="a@example.com", content_type="book", book_id=1, title="원래 제목", category="소설")
    store.record_view(email="a@example.com", content_type="book", book_id=1, title="바뀐 제목", category="에세이")

    user = store.list_recent_views()["users"][0]

    # 같은 책은 row 가 하나뿐이고 스냅샷이 최신으로 갱신된다
    assert len(user["book"]) == 1
    assert user["book"][0]["title"] == "바뀐 제목"
    assert user["book"][0]["category"] == "에세이"


def test_reviewing_moves_book_to_front(store):
    for book_id, title in ((1, "첫째"), (2, "둘째"), (3, "셋째")):
        store.record_view(email="a@example.com", content_type="book", book_id=book_id, title=title)
        time.sleep(0.01)

    assert _titles(store.list_recent_views(), "a@example.com", "book") == ["셋째", "둘째", "첫째"]

    # 가장 오래된 책을 다시 열면 맨 앞으로 온다
    time.sleep(1.1)  # viewed_at 은 초 단위라 순서를 확실히 벌린다
    store.record_view(email="a@example.com", content_type="book", book_id=1, title="첫째")

    assert _titles(store.list_recent_views(), "a@example.com", "book")[0] == "첫째"


def test_keeps_only_max_recent_views_per_type(store):
    overflow = MAX_RECENT_VIEWS + 5
    for book_id in range(overflow):
        store.record_view(email="a@example.com", content_type="book", book_id=book_id, title=f"책 {book_id}")

    user = store.list_recent_views()["users"][0]
    assert len(user["book"]) == MAX_RECENT_VIEWS

    # DB 에도 상한만 남아야 한다(초과분은 기록 시점에 정리)
    with store._get_connection() as conn:
        with conn.cursor() as cur:
            cur.execute("SELECT COUNT(*) AS cnt FROM view_history WHERE email = %s AND content_type = %s", ("a@example.com", "book"))
            assert cur.fetchone()["cnt"] == MAX_RECENT_VIEWS


def test_book_and_comic_limits_are_independent(store):
    for book_id in range(MAX_RECENT_VIEWS + 3):
        store.record_view(email="a@example.com", content_type="book", book_id=book_id, title=f"책 {book_id}")
    for book_id in range(MAX_RECENT_VIEWS + 3):
        store.record_view(email="a@example.com", content_type="comic", book_id=book_id, title=f"만화 {book_id}")

    user = store.list_recent_views()["users"][0]

    assert len(user["book"]) == MAX_RECENT_VIEWS
    assert len(user["comic"]) == MAX_RECENT_VIEWS
    # 같은 book_id 라도 유형이 다르면 별개 이력이다
    assert all(item["title"].startswith("책") for item in user["book"])
    assert all(item["title"].startswith("만화") for item in user["comic"])


def test_users_ordered_by_most_recent_activity(store):
    store.record_view(email="old@example.com", content_type="book", book_id=1, title="오래된 조회")
    time.sleep(1.1)
    store.record_view(email="new@example.com", content_type="comic", book_id=2, title="최근 조회")

    assert _emails(store.list_recent_views()) == ["new@example.com", "old@example.com"]


def test_users_are_isolated(store):
    store.record_view(email="a@example.com", content_type="book", book_id=1, title="A 의 책")
    store.record_view(email="b@example.com", content_type="book", book_id=2, title="B 의 책")

    result = store.list_recent_views()

    assert _titles(result, "a@example.com", "book") == ["A 의 책"]
    assert _titles(result, "b@example.com", "book") == ["B 의 책"]


def test_limit_argument_caps_returned_items(store):
    for book_id in range(5):
        store.record_view(email="a@example.com", content_type="book", book_id=book_id, title=f"책 {book_id}")

    result = store.list_recent_views(limit=2)

    assert result["limit"] == 2
    assert len(result["users"][0]["book"]) == 2


def test_snapshot_survives_when_source_book_is_gone(store):
    """책이 삭제돼도 이력은 조회 시점 제목을 그대로 보여준다(원본 join 없음)."""
    store.record_view(email="a@example.com", content_type="book", book_id=999, title="삭제될 책", category="소설")

    user = store.list_recent_views()["users"][0]

    assert user["book"][0]["title"] == "삭제될 책"
    assert user["book"][0]["book_id"] == 999


def test_rejects_unknown_content_type(store):
    with pytest.raises(ValueError, match="content_type"):
        store.record_view(email="a@example.com", content_type="magazine", book_id=1, title="x")


def test_record_view_rolls_back_and_reraises_on_db_error():
    class FailingCursor:
        def __init__(self):
            self.calls = 0

        def execute(self, *_args):
            self.calls += 1
            if self.calls == 2:
                raise RuntimeError("trim failed")

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

    class FakeConn:
        def __init__(self):
            self.cursor_obj = FailingCursor()
            self.began = False
            self.committed = False
            self.rolled_back = False

        def begin(self):
            self.began = True

        def cursor(self):
            return self.cursor_obj

        def commit(self):
            self.committed = True

        def rollback(self):
            self.rolled_back = True

    store = ViewHistoryStore.__new__(ViewHistoryStore)
    conn = FakeConn()

    @contextmanager
    def fake_connection():
        yield conn

    store._get_connection = fake_connection

    with pytest.raises(RuntimeError, match="trim failed"):
        store.record_view(
            email="a@example.com",
            content_type="book",
            book_id=1,
            title="책",
        )

    assert conn.began is True
    assert conn.committed is False
    assert conn.rolled_back is True


def test_list_recent_views_warns_and_skips_unknown_content_type(monkeypatch, caplog):
    class RowsCursor:
        def __init__(self, rows):
            self.rows = rows
            self.executed = []

        def execute(self, sql, params):
            self.executed.append((sql, params))

        def fetchall(self):
            return self.rows

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

    class FakeConn:
        def __init__(self, rows):
            self.cursor_obj = RowsCursor(rows)

        def cursor(self):
            return self.cursor_obj

    rows = [
        {
            "email": "book@example.com",
            "content_type": "magazine",
            "book_id": 1,
            "title": "잡지",
            "category": "",
            "viewed_at": 20,
        },
        {
            "email": "book@example.com",
            "content_type": "book",
            "book_id": 2,
            "title": "책",
            "category": "소설",
            "viewed_at": 10,
        },
    ]
    store = ViewHistoryStore.__new__(ViewHistoryStore)
    conn = FakeConn(rows)

    @contextmanager
    def fake_connection():
        yield conn

    store._get_connection = fake_connection
    monkeypatch.setattr(view_history_store_mod, "MAX_HISTORY_ROWS", len(rows))
    caplog.set_level(logging.WARNING, logger=view_history_store_mod.LOGGER.name)

    result = store.list_recent_views()

    assert conn.cursor_obj.executed[0][1] == (len(rows),)
    assert result == {
        "limit": MAX_RECENT_VIEWS,
        "users": [
            {
                "email": "book@example.com",
                "last_viewed_at": 10,
                "book": [
                    {
                        "book_id": 2,
                        "title": "책",
                        "category": "소설",
                        "viewed_at": 10,
                    }
                ],
                "comic": [],
            }
        ],
    }
    assert "조회 이력이 상한" in caplog.text
    assert "알 수 없는 content_type" in caplog.text


def test_empty_store_returns_no_users(store):
    assert store.list_recent_views() == {"limit": MAX_RECENT_VIEWS, "users": []}


def test_long_title_is_stored(store):
    long_title = "가" * 500
    store.record_view(email="a@example.com", content_type="book", book_id=1, title=long_title)

    assert store.list_recent_views()["users"][0]["book"][0]["title"] == long_title


def test_factory_returns_store(mysql_container):
    assert isinstance(create_view_history_store(), ViewHistoryStore)
