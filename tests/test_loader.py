import sys
import unittest
import os
from pathlib import Path
from unittest.mock import MagicMock, patch
import importlib


class TestLoader(unittest.TestCase):
    def test_init(self):
        with patch.dict(sys.modules, {
            'ebooklib': MagicMock(),
            'ebooklib.epub': MagicMock(),
            'docx': MagicMock(),
            'striprtf': MagicMock(),
            'striprtf.striprtf': MagicMock(),
            'stripper_rtf': MagicMock(),
            'openpyxl': MagicMock(),
        }):
            os.environ['TM_BOOK_DIR'] = '/tmp'
            os.environ['TM_COMICS_DIR'] = '/tmp'
            sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
            import utils.loader as loader_mod
            importlib.reload(loader_mod)
            l = loader_mod.Loader()
            self.assertIsNotNone(l)
