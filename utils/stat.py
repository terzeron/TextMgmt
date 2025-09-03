#!/usr/bin/env python


class Stat:
    text_count = 0
    text_total_time = 0.0
    normal_epub_count = 0
    normal_epub_total_time = 0.0
    zipped_epub_count = 0
    zipped_epub_total_time = 0.0
    pdf_count = 0
    pdf_total_time = 0.0
    html_count = 0
    html_total_time = 0.0
    docx_count = 0
    docx_total_time = 0.0
    rtf_count = 0
    rtf_total_time = 0.0
    image_count = 0
    image_total_time = 0.0
    index_count = 0
    index_total_time = 0.0

    @staticmethod
    def _divide(a, b) -> float:
        return a / b if b != 0 else 0.0

    @classmethod
    def print(cls) -> None:
        print("[Stat]")
        print("text:     %07d / %03.4f" % (cls.text_count, cls._divide(cls.text_total_time, cls.text_count)))
        print("epub:     %07d / %03.4f" % (cls.normal_epub_count, cls._divide(cls.normal_epub_total_time, cls.normal_epub_count)))
        print("epub(z):  %07d / %03.4f" % (cls.zipped_epub_count, cls._divide(cls.zipped_epub_total_time, cls.zipped_epub_count)))
        print("pdf:      %07d / %03.4f" % (cls.pdf_count, cls._divide(cls.pdf_total_time, cls.pdf_count)))
        print("html:     %07d / %03.4f" % (cls.html_count, cls._divide(cls.html_total_time, cls.html_count)))
        print("docx:     %07d / %03.4f" % (cls.docx_count, cls._divide(cls.docx_total_time, cls.docx_count)))
        print("rtf:      %07d / %03.4f" % (cls.rtf_count, cls._divide(cls.rtf_total_time, cls.rtf_count)))
        print("image:    %07d / %03.4f" % (cls.image_count, cls._divide(cls.image_total_time, cls.image_count)))
        print("indexing: %07d / %03.4f" % (cls.index_count, cls._divide(cls.index_total_time, cls.index_count)))
