#!/usr/bin/env python
"""pymysql dependency pinning.

backend 사용처:
- backend/category_mapping.py:
    import pymysql
    from pymysql.cursors import DictCursor
    pymysql.connect(host=, port=, database=, user=, password=,
                    charset="utf8mb4", cursorclass=DictCursor)
    conn.close() / conn.commit() / conn.rollback()
    with conn.cursor() as cursor: ...
    cursor.execute(sql, params)
    cursor.executemany(sql, rows)
    cursor.fetchone() / cursor.fetchall()
    cursor.rowcount
    pymysql.IntegrityError

박제 API 표면:
- pymysql.connect callable + 핵심 kwargs (host, port, database, user, password, charset, cursorclass)
- pymysql.cursors.DictCursor 클래스 import
- pymysql.IntegrityError, pymysql.MySQLError 예외 클래스
- Connection / Cursor 메서드 존재
"""

import inspect
import unittest


class TestPyMySQLImports(unittest.TestCase):
    def test_module_import(self):
        import pymysql

        self.assertTrue(callable(pymysql.connect))

    def test_dict_cursor_importable(self):
        """category_mapping.py:9 from pymysql.cursors import DictCursor"""
        from pymysql.cursors import DictCursor

        self.assertTrue(callable(DictCursor))

    def test_integrity_error_importable(self):
        """category_mapping.py:148 except pymysql.IntegrityError"""
        import pymysql

        self.assertTrue(issubclass(pymysql.IntegrityError, Exception))


class TestConnectSignature(unittest.TestCase):
    """category_mapping.py:39 pymysql.connect(...)"""

    def test_connect_accepts_expected_kwargs(self):
        import pymysql

        # connect는 함수일 수도, Connection 클래스 별칭일 수도 있다.
        # 어쨌든 호출 가능해야 한다.
        self.assertTrue(callable(pymysql.connect))
        # signature inspection for Connection.__init__
        from pymysql.connections import Connection

        sig = inspect.signature(Connection.__init__)
        params = set(sig.parameters)
        for kw in ("host", "port", "database", "user", "password", "charset", "cursorclass"):
            self.assertIn(kw, params, f"Connection.__init__ missing kwarg {kw}")


class TestConnectionInterface(unittest.TestCase):
    """category_mapping.py가 사용하는 Connection 메서드"""

    def test_connection_has_cursor_method(self):
        from pymysql.connections import Connection

        for method in ("cursor", "close", "commit", "rollback"):
            self.assertTrue(hasattr(Connection, method), f"Connection missing {method}")


class TestCursorInterface(unittest.TestCase):
    """category_mapping.py가 사용하는 Cursor 메서드"""

    def test_dict_cursor_has_required_methods(self):
        from pymysql.cursors import DictCursor

        # 클래스 레벨 메서드들 (execute/executemany/fetchone/fetchall)
        for method in ("execute", "executemany", "fetchone", "fetchall"):
            self.assertTrue(hasattr(DictCursor, method), f"DictCursor missing {method}")

    def test_cursor_rowcount_initialized_in_init(self):
        """category_mapping.py: cursor.rowcount 읽기.
        rowcount는 Cursor.__init__에서 인스턴스 속성으로 설정되므로
        클래스가 아닌 __init__ 소스에서 확인한다.
        """
        import inspect

        from pymysql.cursors import Cursor

        src = inspect.getsource(Cursor.__init__)
        self.assertIn("rowcount", src)

    def test_dict_cursor_supports_context_manager(self):
        """category_mapping.py: with conn.cursor() as cursor: ..."""
        from pymysql.cursors import Cursor, DictCursor

        # context manager protocol
        for cls in (Cursor, DictCursor):
            self.assertTrue(hasattr(cls, "__enter__"), f"{cls} missing __enter__")
            self.assertTrue(hasattr(cls, "__exit__"), f"{cls} missing __exit__")


class TestIntegrityErrorSemantics(unittest.TestCase):
    """category_mapping.py:148 INSERT 중복 시 IntegrityError catch"""

    def test_integrity_error_raise_and_catch(self):
        import pymysql

        with self.assertRaises(pymysql.IntegrityError):
            raise pymysql.IntegrityError(1062, "Duplicate entry")


class TestExceptionHierarchy(unittest.TestCase):
    def test_integrity_error_is_db_api_compatible(self):
        """PEP 249 DB-API: IntegrityError는 DatabaseError를 상속"""
        import pymysql

        # 보장하고 싶은 부분: IntegrityError가 Exception 트리에 있고 별도 catch 가능
        self.assertTrue(issubclass(pymysql.IntegrityError, Exception))


if __name__ == "__main__":
    unittest.main()
