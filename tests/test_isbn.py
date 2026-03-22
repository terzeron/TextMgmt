import unittest, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import utils.isbn as isbn
class TestISBN(unittest.TestCase):
    def test_validate(self):
        self.assertTrue(isbn.validate_isbn13('9788994492032'))
    def test_search(self):
        res = isbn.search_in_content('ISBN 9788994492032')
        self.assertIn('9788994492032', res)
