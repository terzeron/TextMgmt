import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from utils.stat import Stat


def test_divide_normal():
    assert Stat._divide(10, 2) == 5.0


def test_divide_by_zero():
    assert Stat._divide(10, 0) == 0.0


def test_print(capsys):
    # Reset all counters
    Stat.text_count = 1
    Stat.text_total_time = 0.5
    Stat.normal_epub_count = 0
    Stat.normal_epub_total_time = 0.0
    Stat.zipped_epub_count = 0
    Stat.zipped_epub_total_time = 0.0
    Stat.pdf_count = 2
    Stat.pdf_total_time = 1.0
    Stat.html_count = 0
    Stat.html_total_time = 0.0
    Stat.docx_count = 0
    Stat.docx_total_time = 0.0
    Stat.doc_count = 0
    Stat.doc_total_time = 0.0
    Stat.hwp_count = 0
    Stat.hwp_total_time = 0.0
    Stat.rtf_count = 0
    Stat.rtf_total_time = 0.0
    Stat.image_count = 0
    Stat.image_total_time = 0.0
    Stat.index_count = 3
    Stat.index_total_time = 1.5

    Stat.print()
    captured = capsys.readouterr()
    assert "[Stat]" in captured.out
    assert "text:" in captured.out
    assert "pdf:" in captured.out
    assert "total:" in captured.out


def test_counters_are_class_level():
    Stat.text_count = 0
    assert Stat.text_count == 0
    Stat.text_count += 1
    assert Stat.text_count == 1
