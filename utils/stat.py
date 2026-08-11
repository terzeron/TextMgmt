#!/usr/bin/env python


import threading


class Stat:
    _lock = threading.RLock()
    text_count = 0
    text_total_time = 0.0
    text_reencoded_count = 0
    normal_epub_count = 0
    normal_epub_total_time = 0.0
    zipped_epub_count = 0
    zipped_epub_total_time = 0.0
    pdf_count = 0
    pdf_total_time = 0.0
    pdf_stage_count: dict[str, int] = {}
    pdf_stage_total_time: dict[str, float] = {}
    pdf_stage_outcome_count: dict[tuple[str, str], int] = {}
    html_count = 0
    html_total_time = 0.0
    docx_count = 0
    docx_total_time = 0.0
    doc_count = 0
    doc_total_time = 0.0
    hwp_count = 0
    hwp_total_time = 0.0
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
    def add(cls, name: str, value: int | float) -> None:
        with cls._lock:
            setattr(cls, name, getattr(cls, name) + value)

    @classmethod
    def set_value(cls, name: str, value: int | float) -> None:
        with cls._lock:
            setattr(cls, name, value)

    @classmethod
    def record_pdf_stage(cls, stage: str, elapsed: float, outcome: str) -> None:
        with cls._lock:
            cls.pdf_stage_count[stage] = cls.pdf_stage_count.get(stage, 0) + 1
            cls.pdf_stage_total_time[stage] = cls.pdf_stage_total_time.get(stage, 0.0) + elapsed
            key = (stage, outcome)
            cls.pdf_stage_outcome_count[key] = cls.pdf_stage_outcome_count.get(key, 0) + 1

    @classmethod
    def print(cls) -> None:
        with cls._lock:
            print("[Stat]")
            print("text:    %07d / %03.4f" % (cls.text_count, cls._divide(cls.text_total_time, cls.text_count)))
            if cls.text_reencoded_count:
                print("  └ UTF-8 재인코딩: %d건" % cls.text_reencoded_count)
            print("epub:    %07d / %03.4f" % (cls.normal_epub_count, cls._divide(cls.normal_epub_total_time, cls.normal_epub_count)))
            print("epub(z): %07d / %03.4f" % (cls.zipped_epub_count, cls._divide(cls.zipped_epub_total_time, cls.zipped_epub_count)))
            print("pdf:     %07d / %03.4f" % (cls.pdf_count, cls._divide(cls.pdf_total_time, cls.pdf_count)))
            for stage in sorted(cls.pdf_stage_count):
                count = cls.pdf_stage_count[stage]
                ok = cls.pdf_stage_outcome_count.get((stage, "ok"), 0)
                empty = cls.pdf_stage_outcome_count.get((stage, "empty"), 0)
                damaged = cls.pdf_stage_outcome_count.get((stage, "damaged"), 0)
                timeout = cls.pdf_stage_outcome_count.get((stage, "timeout"), 0)
                error = cls.pdf_stage_outcome_count.get((stage, "error"), 0)
                avg = cls._divide(cls.pdf_stage_total_time.get(stage, 0.0), count)
                print(
                    "  - %-24s %07d / %03.4f / ok=%d empty=%d damaged=%d timeout=%d error=%d"
                    % (stage, count, avg, ok, empty, damaged, timeout, error)
                )
            print("html:    %07d / %03.4f" % (cls.html_count, cls._divide(cls.html_total_time, cls.html_count)))
            print("docx:    %07d / %03.4f" % (cls.docx_count, cls._divide(cls.docx_total_time, cls.docx_count)))
            print("doc:     %07d / %03.4f" % (cls.doc_count, cls._divide(cls.doc_total_time, cls.doc_count)))
            print("hwp:     %07d / %03.4f" % (cls.hwp_count, cls._divide(cls.hwp_total_time, cls.hwp_count)))
            print("rtf:     %07d / %03.4f" % (cls.rtf_count, cls._divide(cls.rtf_total_time, cls.rtf_count)))
            print("image:   %07d / %03.4f" % (cls.image_count, cls._divide(cls.image_total_time, cls.image_count)))
            print("total:   %07d / %03.4f" % (cls.index_count, cls._divide(cls.index_total_time, cls.index_count)))
