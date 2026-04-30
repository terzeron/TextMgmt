import contextlib
import sys
import unittest
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
        cursor = FakeCursor(fetchone_rows=[{"cnt": 0}, {"cnt": 0}])
        cm_mod, cm = build_cm(cursor)
        assert cm is not None

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
        assert cm.is_hidden("c1/sub") is True

    def test_categories_search_and_rename(self):
        cursor = FakeCursor(rows=[{"category": "a"}, {"category": "b"}])
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.get_categories_with_keywords() == ["a", "b"]
        assert cm.search_by_keyword("key") == ["a", "b"]

        cursor_rename = FakeCursor()
        cm_mod, cm = build_cm(cursor_rename)
        conn = FakeConn(cursor_rename)

        @contextlib.contextmanager
        def _conn_rename():
            yield conn

        cm._get_connection = _conn_rename
        assert cm.rename_category("old", "new") is True

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

    def test_is_hidden_returns_false(self):
        """Line 387: is_hidden returns False when category is not hidden"""
        cursor = FakeCursor(rows=[{"category": "other_cat"}])
        cm_mod, cm = build_cm(cursor)

        @contextlib.contextmanager
        def _conn():
            yield FakeConn(cursor)

        cm._get_connection = _conn
        assert cm.is_hidden("not_hidden_cat") is False

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
        cursor = FakeCursor(fetchone_rows=[{"cnt": 1}, {"cnt": 1}])
        cm_mod, cm = build_cm(cursor)
        alter_queries = [sql for sql, _ in cursor.executed if "ALTER TABLE" in str(sql)]
        assert cm is not None
        assert alter_queries == []
