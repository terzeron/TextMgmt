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
        print("[Stat] text: %d / %.4f" % (cls.text_count, cls._divide(cls.text_total_time, cls.text_count)), end=", ")
        print("normal_epub: %d / %.4f" % (cls.normal_epub_count, cls._divide(cls.normal_epub_total_time, cls.normal_epub_count)), end=", ")
        print("zipped_epub: %d / %.4f" % (cls.zipped_epub_count, cls._divide(cls.zipped_epub_total_time, cls.zipped_epub_count)), end=", ")
        print("pdf: %d / %.4f" % (cls.pdf_count, cls._divide(cls.pdf_total_time, cls.pdf_count)), end=", ")
        print("html: %d / %.4f" % (cls.html_count, cls._divide(cls.html_total_time, cls.html_count)), end=", ")
        print("docx: %d / %.4f" % (cls.docx_count, cls._divide(cls.docx_total_time, cls.docx_count)), end=", ")
        print("rtf: %d / %.4f" % (cls.rtf_count, cls._divide(cls.rtf_total_time, cls.rtf_count)), end=", ")
        print("image: %d / %.4f" % (cls.index_count, cls._divide(cls.index_total_time, cls.index_count)), end=", ")
        print("index: %d / %.4f" % (cls.index_count, cls._divide(cls.index_total_time, cls.index_count)))
