#!/usr/bin/env python
"""striprtf dependency pinning.

utils 사용처:
- utils/loader.py:24,261  RTF → 평문 변환
    from striprtf.striprtf import rtf_to_text
    result = rtf_to_text(doc, errors="ignore")

박제 API:
- striprtf.striprtf.rtf_to_text(rtf_str, errors="ignore") -> str
"""

import unittest


class TestStripRTFDependencyPinning(unittest.TestCase):
    def test_rtf_to_text_importable(self):
        """from striprtf.striprtf import rtf_to_text"""
        from striprtf.striprtf import rtf_to_text

        self.assertTrue(callable(rtf_to_text))

    def test_rtf_to_text_basic_conversion(self):
        from striprtf.striprtf import rtf_to_text

        result = rtf_to_text(r"{\rtf1\ansi Hello World}")
        self.assertIsInstance(result, str)
        self.assertIn("Hello", result)

    def test_rtf_to_text_errors_ignore_kwarg(self):
        """loader.py: rtf_to_text(doc, errors="ignore")"""
        from striprtf.striprtf import rtf_to_text

        result = rtf_to_text(r"{\rtf1\ansi Test content}", errors="ignore")
        self.assertIsInstance(result, str)
        self.assertIn("Test", result)


if __name__ == "__main__":
    unittest.main()
