#!/usr/bin/env python
"""chardet dependency pinning.

backend 사용처:
- backend/book_manager.py:493
  encoding_metadata = chardet.detect(content.encode())

박제 API:
- chardet.detect(bytes) -> dict {"encoding": str | None, "confidence": float, ...}
"""

import unittest


class TestChardetDetectAPI(unittest.TestCase):
    def test_module_import_and_detect_callable(self):
        import chardet

        self.assertTrue(callable(chardet.detect))

    def test_detect_returns_dict_with_required_keys(self):
        """book_manager.py가 의존하는 dict 형태"""
        import chardet

        result = chardet.detect("abc".encode("utf-8"))
        self.assertIsInstance(result, dict)
        self.assertIn("encoding", result)
        self.assertIn("confidence", result)

    def test_detect_utf8_text(self):
        import chardet

        result = chardet.detect("안녕하세요 hello world".encode("utf-8"))
        self.assertIsNotNone(result["encoding"])
        self.assertIsInstance(result["confidence"], float)
        self.assertGreater(result["confidence"], 0.5)

    def test_detect_ascii_text(self):
        import chardet

        result = chardet.detect(b"plain ascii content")
        self.assertIsInstance(result["encoding"], str)
        self.assertGreater(result["confidence"], 0.0)

    def test_detect_euc_kr_text(self):
        import chardet

        result = chardet.detect("대한민국 서울".encode("euc-kr"))
        self.assertIsNotNone(result["encoding"])
        self.assertGreater(result["confidence"], 0.0)

    def test_detect_empty_bytes_returns_dict(self):
        import chardet

        result = chardet.detect(b"")
        self.assertIsInstance(result, dict)
        self.assertIn("encoding", result)


if __name__ == "__main__":
    unittest.main()
