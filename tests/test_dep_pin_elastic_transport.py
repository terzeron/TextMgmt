#!/usr/bin/env python
"""elastic_transport dependency pinning.

backend 사용처:
- backend/es_manager.py:12
  from elastic_transport import SerializationError, ConnectionError, ConnectionTimeout
- bulk()/info() 호출 시 (SerializationError, ConnectionError, ConnectionTimeout)로 catch
"""

import unittest


class TestElasticTransportExceptions(unittest.TestCase):
    def test_three_exception_classes_importable(self):
        from elastic_transport import ConnectionError, ConnectionTimeout, SerializationError

        for cls in (SerializationError, ConnectionError, ConnectionTimeout):
            self.assertTrue(issubclass(cls, Exception), f"{cls} is not Exception subclass")

    def test_serialization_error_raise_and_catch(self):
        from elastic_transport import SerializationError

        with self.assertRaises(SerializationError):
            raise SerializationError("test")

    def test_connection_error_raise_and_catch(self):
        from elastic_transport import ConnectionError as ETConnectionError

        with self.assertRaises(ETConnectionError):
            raise ETConnectionError("test")

    def test_connection_timeout_raise_and_catch(self):
        from elastic_transport import ConnectionTimeout

        with self.assertRaises(ConnectionTimeout):
            raise ConnectionTimeout("test")

    def test_tuple_catch_pattern(self):
        """es_manager.py: except (SerializationError, ConnectionError, ConnectionTimeout) as e"""
        from elastic_transport import ConnectionError as ETConnectionError
        from elastic_transport import ConnectionTimeout, SerializationError

        caught = []
        for exc in (SerializationError("a"), ETConnectionError("b"), ConnectionTimeout("c")):
            try:
                raise exc
            except (SerializationError, ETConnectionError, ConnectionTimeout) as e:
                caught.append(type(e).__name__)
        self.assertEqual(len(caught), 3)


if __name__ == "__main__":
    unittest.main()
