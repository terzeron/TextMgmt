import contextlib
import json
import sys
import unittest
from datetime import datetime, timedelta
from pathlib import Path
import types
import importlib
from unittest import mock


class FakeCursor:
    def __init__(self, rows=None, rowcount=1, execute_side_effect=None, fetchone_rows=None):
        self._rows = rows or []
        self.rowcount = rowcount
        self.executed = []
        self.executed_many = []
        self._execute_side_effect = execute_side_effect
        self._fetchone_rows = fetchone_rows or [{"cnt": 0}]
        self._fetchone_index = 0

    def execute(self, sql, params=None):
        self.executed.append((sql, params))
        if self._execute_side_effect:
            self._execute_side_effect(sql, params)

    def executemany(self, sql, seq):
        self.executed_many.append((sql, list(seq)))

    def fetchall(self):
        return self._rows

    def fetchone(self):
        if self._fetchone_index < len(self._fetchone_rows):
            row = self._fetchone_rows[self._fetchone_index]
            self._fetchone_index += 1
            return row
        return {"cnt": 0}

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False


class FakeConn:
    def __init__(self, cursor):
        self._cursor = cursor
        self.committed = False
        self.rolled_back = False

    def cursor(self):
        return self._cursor

    def commit(self):
        self.committed = True

    def rollback(self):
        self.rolled_back = True

    def close(self):
        return None


def build_cm(fake_cursor):
    fake_pymysql = types.SimpleNamespace(IntegrityError=type("IntegrityError", (Exception,), {}), connect=lambda **kwargs: FakeConn(fake_cursor))
    fake_cursors = types.SimpleNamespace(DictCursor=object)

    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    with mock.patch.dict(sys.modules, {"pymysql": fake_pymysql, "pymysql.cursors": fake_cursors}):
        import backend.category_mapping as cm_mod

        importlib.reload(cm_mod)
        cm = cm_mod.CategoryMapping(host="h", port=1, database="d", user="u", password="p")
        return cm_mod, cm


class TestCategoryMapping(unittest.TestCase):
    def test_init(self):
        cursor = FakeCursor(fetchone_rows=[{"cnt": 0}, {"cnt": 0}, {"cnt": 0}])
        cm_mod, cm = build_cm(cursor)
        assert cm is not None

    def test_init_creates_latest_excluded_categories_table(self):
        cursor = FakeCursor(fetchone_rows=[{"cnt": 1}, {"cnt": 1}, {"cnt": 1}])
        cm_mod, cm = build_cm(cursor)
        assert cm is not None
        assert any("CREATE TABLE IF NOT EXISTS latest_excluded_categories" in str(sql) for sql, _ in cursor.executed)

    def test_get_all_and_keywords(self):
        cursor = FakeCursor(rows=[{"category": "c1", "keyword": "k1"}, {"category": "c1", "keyword": "k2"}])
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        mappings = cm.get_all_mappings()
        assert mappings == {"c1": ["k1", "k2"]}
        keywords = cm.get_keywords("c1")
        assert keywords == ["k1", "k2"]

    def test_add_and_remove_keyword(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.add_keyword("cat", " kw ") is True
        assert cm.remove_keyword("cat", "kw") is True

    def test_add_keyword_empty(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        assert cm.add_keyword("cat", "  ") is False

    def test_add_keyword_duplicate(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)

        def raise_integrity(sql, params):
            raise cm_mod.pymysql.IntegrityError("dup")

        cursor_fail = FakeCursor(execute_side_effect=raise_integrity)

        @contextlib.contextmanager
        def _conn_fail():
            yield FakeConn(cursor_fail)

        cm._get_connection = _conn_fail
        assert cm.add_keyword("cat", "kw") is False

    def test_set_keywords_success_and_fail(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        assert cm.set_keywords("cat", ["a", "a", " ", "b"]) is True
        assert conn.committed is True

        def raise_error(sql, params):
            raise Exception("fail")

        cursor_fail = FakeCursor(execute_side_effect=raise_error)
        cm_mod, cm = build_cm(FakeCursor())
        conn_fail = FakeConn(cursor_fail)

        @contextlib.contextmanager
        def _conn_fail():
            yield conn_fail

        cm._get_connection = _conn_fail
        assert cm.set_keywords("cat", ["a"]) is False
        assert conn_fail.rolled_back is True

    def test_update_all_mappings(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        assert cm.update_all_mappings({"c1": ["k1", "k2"]}) is True
        assert conn.committed is True

    def test_delete_and_hidden(self):
        cursor = FakeCursor(rowcount=1)
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.delete_category("cat") is True
        assert cm.delete_category("cat", prefix=True) is True

        cursor_hidden = FakeCursor(rows=[{"category": "c1"}])
        cm_mod, cm = build_cm(cursor_hidden)

        @contextlib.contextmanager
        def _conn_hidden():
            yield FakeConn(cursor_hidden)

        cm._get_connection = _conn_hidden
        assert cm.get_hidden_categories() == ["c1"]

    def test_get_and_set_latest_excluded_categories(self):
        cursor_latest = FakeCursor(rows=[{"category": "c1"}])
        cm_mod, cm = build_cm(FakeCursor(fetchone_rows=[{"cnt": 1}, {"cnt": 1}, {"cnt": 1}]))

        @contextlib.contextmanager
        def _conn_latest():
            yield FakeConn(cursor_latest)

        cm._get_connection = _conn_latest
        assert cm.get_latest_excluded_categories() == ["c1"]

        cursor_set = FakeCursor()
        conn = FakeConn(cursor_set)

        @contextlib.contextmanager
        def _conn_set():
            yield conn

        cm._get_connection = _conn_set
        assert cm.set_latest_excluded("cat", True) is True
        assert cm.set_latest_excluded("cat", False) is True
        assert conn.committed is True
        assert any("INSERT IGNORE INTO latest_excluded_categories" in str(sql) for sql, _ in cursor_set.executed)
        assert any("DELETE FROM latest_excluded_categories" in str(sql) for sql, _ in cursor_set.executed)

    def test_categories_search_and_rename(self):
        cursor = FakeCursor(rows=[{"category": "a"}, {"category": "b"}])
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.search_by_keyword("key") == ["a", "b"]

        cursor_rename = FakeCursor()
        cm_mod, cm = build_cm(cursor_rename)
        conn = FakeConn(cursor_rename)

        @contextlib.contextmanager
        def _conn_rename():
            yield conn

        cm._get_connection = _conn_rename
        assert cm.rename_category("old", "new") is True
        assert any("UPDATE latest_excluded_categories SET category" in str(sql) for sql, _ in cursor_rename.executed)

        def raise_error(sql, params):
            raise Exception("fail")

        cursor_fail = FakeCursor(execute_side_effect=raise_error)
        cm_mod, cm = build_cm(FakeCursor())
        conn_fail = FakeConn(cursor_fail)

        @contextlib.contextmanager
        def _conn_fail():
            yield conn_fail

        cm._get_connection = _conn_fail
        assert cm.rename_category("old", "new") is False

    def test_set_hidden(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        assert cm.set_hidden("cat", True) is True
        assert cm.set_hidden("cat", False) is True

        def raise_error(sql, params):
            raise Exception("fail")

        cursor_fail = FakeCursor(execute_side_effect=raise_error)
        cm_mod, cm = build_cm(FakeCursor())
        conn_fail = FakeConn(cursor_fail)

        @contextlib.contextmanager
        def _conn_fail():
            yield conn_fail

        cm._get_connection = _conn_fail
        assert cm.set_hidden("cat", True) is False

    def test_set_latest_excluded_failure(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)

        def raise_error(sql, params):
            raise Exception("fail")

        cursor_fail = FakeCursor(execute_side_effect=raise_error)
        conn_fail = FakeConn(cursor_fail)

        @contextlib.contextmanager
        def _conn_fail():
            yield conn_fail

        cm._get_connection = _conn_fail
        assert cm.set_latest_excluded("cat", True) is False

    def test_migrate_drop_index_exception_category_keywords(self):
        """Lines 72-73: exception when dropping unique_category_keyword index"""
        call_count = {"n": 0}

        def side_effect(sql, params=None):
            call_count["n"] += 1
            if "DROP INDEX unique_category_keyword" in str(sql):
                raise Exception("index not found")

        cursor = FakeCursor(fetchone_rows=[{"cnt": 0}, {"cnt": 0}], execute_side_effect=side_effect)
        cm_mod, cm = build_cm(cursor)
        # If we get here without error, the exception was caught
        assert cm is not None

    def test_migrate_drop_index_exception_hidden_categories(self):
        """Lines 78-79: exception when dropping category index on hidden_categories"""
        call_count = {"n": 0}

        def side_effect(sql, params=None):
            call_count["n"] += 1
            if "DROP INDEX category" in str(sql):
                raise Exception("index not found")

        cursor = FakeCursor(fetchone_rows=[{"cnt": 0}, {"cnt": 0}], execute_side_effect=side_effect)
        cm_mod, cm = build_cm(cursor)
        assert cm is not None

    def test_update_all_mappings_exception(self):
        """Lines 234-237: update_all_mappings rollback on exception"""
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)

        def raise_error(sql, params=None):
            if "DELETE" in str(sql):
                raise Exception("db error")

        cursor_fail = FakeCursor(execute_side_effect=raise_error)
        conn_fail = FakeConn(cursor_fail)

        @contextlib.contextmanager
        def _conn_fail():
            yield conn_fail

        cm._get_connection = _conn_fail
        assert cm.update_all_mappings({"c1": ["k1"]}) is False
        assert conn_fail.rolled_back is True

    def test_get_all_mappings_empty_and_remove_keyword_not_found(self):
        cursor = FakeCursor(rows=[], rowcount=0)
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.get_all_mappings() == {}
        assert cm.remove_keyword("cat", "kw") is False

    def test_delete_category_not_found(self):
        cursor = FakeCursor(rowcount=0)
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.delete_category("missing") is False

    def test_set_hidden_false_always_succeeds(self):
        cursor = FakeCursor(rowcount=0)
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        assert cm.set_hidden("cat", False) is True
        assert conn.committed is True

    def test_migrate_skips_when_content_type_already_exists(self):
        cursor = FakeCursor(fetchone_rows=[{"cnt": 1}] * 15)
        cm_mod, cm = build_cm(cursor)
        alter_queries = [sql for sql, _ in cursor.executed if "ALTER TABLE" in str(sql)]
        assert cm is not None
        assert alter_queries == []

    def test_migrate_reload_locks_adds_reload_source(self):
        cursor = FakeCursor(fetchone_rows=[{"cnt": 1}] * 13 + [{"cnt": 0}, {"cnt": 1}])
        cm_mod, cm = build_cm(cursor)
        executed_sql = [sql for sql, _ in cursor.executed]
        assert any("ADD COLUMN reload_source" in s for s in executed_sql)
        assert any("SET reload_source = %s WHERE category IS NOT NULL" in s for s in executed_sql)
        assert cm is not None

    def test_migrate_reload_locks_adds_lock_key_and_switches_primary_key(self):
        cursor = FakeCursor(fetchone_rows=[{"cnt": 1}] * 14 + [{"cnt": 0}])
        cm_mod, cm = build_cm(cursor)
        executed_sql = [sql for sql, _ in cursor.executed]
        assert any("ADD COLUMN lock_key" in s for s in executed_sql)
        assert any(s.startswith("UPDATE reload_locks SET lock_key") for s in executed_sql)
        assert any("DROP PRIMARY KEY" in s and "ADD PRIMARY KEY (content_type, lock_key)" in s for s in executed_sql)
        assert cm is not None

    def test_acquire_reload_lock_success(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor(rows=[])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book")
        assert acquired is True
        assert error is None
        assert blocking is None
        assert any("INSERT INTO reload_locks" in str(sql) for sql, _ in cursor.executed)

    def test_acquire_reload_lock_stores_reload_source(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor(rows=[])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book", reload_source="mismatch")
        assert acquired is True
        assert error is None
        assert blocking is None
        sql, params = next((sql, params) for sql, params in cursor.executed if "INSERT INTO reload_locks" in str(sql))
        assert "reload_source" in sql
        assert "reload_source = VALUES(reload_source)" in sql
        assert params == ("book", "__all__", None, "mismatch")

    def test_acquire_reload_lock_defaults_unknown_reload_source_to_bulk(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor(rows=[])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book", reload_source="unknown")
        assert acquired is True
        assert error is None
        assert blocking is None
        _sql, params = next((sql, params) for sql, params in cursor.executed if "INSERT INTO reload_locks" in str(sql))
        assert params == ("book", "__all__", None, "bulk")

    def test_acquire_reload_lock_ignores_finished_lock_rows(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor(rows=[{"lock_key": "__all__", "status": "done", "updated_at": datetime.now()}])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book")
        assert acquired is True
        assert error is None
        assert blocking is None
        assert any("INSERT INTO reload_locks" in str(sql) for sql, _ in cursor.executed)

    def test_acquire_reload_lock_already_in_progress(self):
        cm_mod, cm = build_cm(FakeCursor())
        now = datetime.now()
        cursor = FakeCursor(rows=[{"lock_key": "__all__", "status": "running", "updated_at": now}], fetchone_rows=[{"category": None, "status": "running", "started_at": now, "updated_at": now, "indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0, "error": None}])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book")
        assert acquired is False
        assert error is not None
        assert "진행 중" in error
        assert blocking["status"] == "running"
        assert not any("INSERT INTO reload_locks" in str(sql) for sql, _ in cursor.executed)

    def test_acquire_reload_lock_replaces_stale_lock(self):
        cm_mod, cm = build_cm(FakeCursor())
        old_updated_at = datetime.now() - timedelta(seconds=cm.RELOAD_LOCK_HEARTBEAT_STALE_SECONDS + 60)
        cursor = FakeCursor(rows=[{"lock_key": "__all__", "status": "running", "updated_at": old_updated_at}])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book")
        assert acquired is True
        assert error is None
        assert blocking is None
        executed_sql = [sql for sql, _ in cursor.executed]
        assert sum("INSERT INTO reload_locks" in s for s in executed_sql) == 1

    def test_acquire_reload_lock_different_categories_are_independent(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor(rows=[])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book", category="1_fiction")
        assert acquired is True
        assert error is None
        assert blocking is None
        select_sql = next(sql for sql, _ in cursor.executed if sql.strip().upper().startswith("SELECT"))
        assert "IN (%s, %s)" in select_sql

    def test_acquire_reload_lock_category_blocked_by_running_bulk_lock(self):
        cm_mod, cm = build_cm(FakeCursor())
        now = datetime.now()
        cursor = FakeCursor(rows=[{"lock_key": "__all__", "status": "running", "updated_at": now}], fetchone_rows=[{"category": None, "status": "running", "started_at": now, "updated_at": now, "indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0, "error": None}])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book", category="1_fiction")
        assert acquired is False
        assert "다른 재적재 작업" in error
        assert blocking["category"] is None
        assert not any("INSERT INTO reload_locks" in str(sql) for sql, _ in cursor.executed)

    def test_acquire_reload_lock_bulk_blocked_by_running_category_lock(self):
        cm_mod, cm = build_cm(FakeCursor())
        now = datetime.now()
        cursor = FakeCursor(rows=[{"lock_key": "1_fiction", "status": "running", "updated_at": now}], fetchone_rows=[{"category": "1_fiction", "status": "running", "started_at": now, "updated_at": now, "indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0, "error": None}])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        acquired, error, blocking = cm.acquire_reload_lock("book")
        assert acquired is False
        assert "다른 재적재 작업" in error
        assert blocking["category"] == "1_fiction"
        assert not any("INSERT INTO reload_locks" in str(sql) for sql, _ in cursor.executed)

    def test_release_reload_lock(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        cm.release_reload_lock("book")
        assert any("DELETE FROM reload_locks" in str(sql) for sql, _ in cursor.executed)

    def test_heartbeat_reload_lock_touches_updated_at_only(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor()

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        cm.heartbeat_reload_lock("book")
        sql, params = cursor.executed[-1]
        assert "updated_at = NOW()" in sql
        assert "indexed_count" not in sql
        assert params == ("book", "__all__")

    def test_heartbeat_reload_lock_scopes_to_category(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor()

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        cm.heartbeat_reload_lock("book", category="1_fiction")
        sql, params = cursor.executed[-1]
        assert params == ("book", "1_fiction")

    def test_heartbeat_reload_lock_updates_counts(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor()

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        cm.heartbeat_reload_lock("book", indexed_count=3, deleted_count=1)
        sql, params = cursor.executed[-1]
        assert "indexed_count = %s" in sql
        assert "deleted_count = %s" in sql
        assert params == (3, 1, "book", "__all__")

    def test_heartbeat_reload_lock_rejects_unknown_field(self):
        cm_mod, cm = build_cm(FakeCursor())
        with self.assertRaises(ValueError):
            cm.heartbeat_reload_lock("book", bogus_count=1)

    def test_complete_reload_lock_records_final_status(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor()

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        cm.complete_reload_lock("book", "done", None, indexed_count=5, deleted_count=2, failed_count=0, before_count=7, after_count=0)
        sql, params = cursor.executed[-1]
        assert "status = %s" in sql
        assert params[0] == "done"
        assert params[-2] == "book"
        assert params[-1] == "__all__"

    def test_complete_reload_lock_scopes_to_category(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor()

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        cm.complete_reload_lock("book", "done", None, category="1_fiction", indexed_count=5)
        sql, params = cursor.executed[-1]
        assert params[-2] == "book"
        assert params[-1] == "1_fiction"

    def test_complete_reload_lock_rejects_unknown_field(self):
        cm_mod, cm = build_cm(FakeCursor())
        with self.assertRaises(ValueError):
            cm.complete_reload_lock("book", "done", None, bogus_count=1)

    def test_get_reload_status_none_when_no_row(self):
        cm_mod, cm = build_cm(FakeCursor())
        cursor = FakeCursor(fetchone_rows=[None])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.get_reload_status("book") is None

    def test_get_reload_status_returns_running_row(self):
        cm_mod, cm = build_cm(FakeCursor())
        now = datetime.now()
        row = {"category": "A", "reload_source": "mismatch", "status": "running", "started_at": now, "updated_at": now, "indexed_count": 1, "deleted_count": 0, "failed_count": 0, "before_count": 2, "after_count": 0, "error": None}
        cursor = FakeCursor(fetchone_rows=[row])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        status = cm.get_reload_status("book")
        assert status["status"] == "running"
        assert status["category"] == "A"
        assert status["reload_source"] == "mismatch"
        assert status["indexed_count"] == 1

    def test_get_reload_status_reports_stale_running_as_failed(self):
        cm_mod, cm = build_cm(FakeCursor())
        old_updated_at = datetime.now() - timedelta(seconds=cm.RELOAD_LOCK_HEARTBEAT_STALE_SECONDS + 60)
        row = {"category": "A", "status": "running", "started_at": old_updated_at, "updated_at": old_updated_at, "indexed_count": 0, "deleted_count": 0, "failed_count": 0, "before_count": 0, "after_count": 0, "error": None}
        cursor = FakeCursor(fetchone_rows=[row])

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        status = cm.get_reload_status("book")
        assert status["status"] == "failed"
        assert status["error"]


class _ClassifyItemsFakeCursor:
    """classify_proposal_items에 대해 INSERT(executemany)/DELETE/SELECT를 흉내내는
    인메모리 저장소. 공용 FakeCursor는 고정된 rows만 돌려줘, "여러 번 나눠 넣어도
    get이 전체를 순서대로 돌려준다"는 시나리오(테일 플러시 검증 포함)를 표현할 수
    없어서 별도로 둔다.
    """

    def __init__(self, store: list[tuple]):
        self.store = store
        self.rowcount = 0
        self._result: list[dict] = []

    def execute(self, sql, params=None):
        if sql.startswith("DELETE FROM classify_proposal_items"):
            (content_type,) = params
            self.store[:] = [row for row in self.store if row[0] != content_type]
        elif sql.startswith("SELECT payload, apply_status, apply_error FROM classify_proposal_items"):
            (content_type,) = params
            self._result = [{"payload": row[3], "apply_status": "pending", "apply_error": None} for row in self.store if row[0] == content_type]
        else:
            raise AssertionError(f"unexpected SQL: {sql}")

    def executemany(self, sql, seq):
        self.store.extend(seq)

    def fetchall(self):
        return self._result

    def fetchone(self):
        return {"cnt": 0}

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False


class _ClassifyItemsFakeConn:
    def __init__(self, cursor):
        self._cursor = cursor
        self.committed = False

    def cursor(self):
        return self._cursor

    def commit(self):
        self.committed = True

    def rollback(self):
        pass

    def close(self):
        return None


class TestClassifyProposalItems(unittest.TestCase):
    """제안 항목을 JSON 상태 파일 대신 담는 classify_proposal_items 테이블 메서드."""

    def test_init_creates_classify_proposal_items_table(self):
        cursor = FakeCursor(fetchone_rows=[{"cnt": 0}] * 14)
        cm_mod, cm = build_cm(cursor)
        assert cm is not None
        assert any("CREATE TABLE IF NOT EXISTS classify_proposal_items" in str(sql) for sql, _ in cursor.executed)

    def test_add_then_get_returns_items_in_insertion_order(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        items = [{"file_path": "a.epub", "target_category": "3_SF"}, {"file_path": "b.epub", "target_category": "3_SF"}]
        cm.add_classify_proposal_items(items, "0_inbox")

        assert conn.committed is True
        assert len(cursor.executed_many) == 1
        sql, rows = cursor.executed_many[0]
        assert "INSERT INTO classify_proposal_items" in sql
        assert rows == [
            ("book", "0_inbox", "a.epub", json.dumps(items[0], ensure_ascii=False)),
            ("book", "0_inbox", "b.epub", json.dumps(items[1], ensure_ascii=False)),
        ]

        cursor_get = FakeCursor(rows=[{"payload": json.dumps(items[0], ensure_ascii=False), "apply_status": "pending", "apply_error": None}, {"payload": json.dumps(items[1], ensure_ascii=False), "apply_status": "pending", "apply_error": None}])

        @contextlib.contextmanager
        def _conn_get():
            yield FakeConn(cursor_get)

        cm._get_connection = _conn_get
        fetched = cm.get_classify_proposal_items()
        assert [i["file_path"] for i in fetched] == ["a.epub", "b.epub"]
        assert any("ORDER BY id ASC" in str(sql) for sql, _ in cursor_get.executed)

    def test_add_empty_items_is_noop(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        cm.add_classify_proposal_items([], "0_inbox")
        assert cursor.executed_many == []
        assert conn.committed is False

    def test_clear_scopes_delete_to_given_content_type(self):
        cursor = FakeCursor()
        cm_mod, cm = build_cm(cursor)
        conn = FakeConn(cursor)

        @contextlib.contextmanager
        def _conn():
            yield conn

        cm._get_connection = _conn
        cm.clear_classify_proposal_items(content_type="comic")

        assert conn.committed is True
        sql, params = cursor.executed[-1]
        assert "DELETE FROM classify_proposal_items WHERE content_type = %s" in sql
        assert params == ("comic",)

    def test_clear_only_removes_matching_content_type(self):
        """다른 content_type의 행은 clear 뒤에도 남아 있어야 한다."""
        cm_mod, cm = build_cm(FakeCursor())
        store = [("book", "0_inbox", "a.epub", "{}"), ("comic", "0_inbox", "b.epub", "{}")]
        cursor = _ClassifyItemsFakeCursor(store)

        @contextlib.contextmanager
        def _conn():
            yield _ClassifyItemsFakeConn(cursor)

        cm._get_connection = _conn
        cm.clear_classify_proposal_items(content_type="book")

        assert [row[0] for row in store] == ["comic"]

    def test_add_across_multiple_calls_then_get_returns_all_in_order_no_tail_dropped(self):
        """100건 단위로 나눠 add를 여러 번 불러도(100+100+50), get은 250건 전체를
        순서대로 돌려준다. 마지막 잔여분(꼬리)이 누락되면 관리자가 목록 끝의 책들을
        못 보고 승인하게 되므로, 이 테일 플러시를 DB 계층에서 직접 확인한다.
        """
        cm_mod, cm = build_cm(FakeCursor())
        store: list[tuple] = []
        cursor = _ClassifyItemsFakeCursor(store)

        @contextlib.contextmanager
        def _conn():
            yield _ClassifyItemsFakeConn(cursor)

        cm._get_connection = _conn
        cm.add_classify_proposal_items([{"file_path": f"{i}.epub"} for i in range(0, 100)], "0_inbox")
        cm.add_classify_proposal_items([{"file_path": f"{i}.epub"} for i in range(100, 200)], "0_inbox")
        cm.add_classify_proposal_items([{"file_path": f"{i}.epub"} for i in range(200, 250)], "0_inbox")

        items = cm.get_classify_proposal_items()
        assert len(items) == 250
        assert [item["file_path"] for item in items] == [f"{i}.epub" for i in range(250)]
