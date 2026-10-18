"""정식 표지 유무 판정과 결과 캐시. 미커버 목록이 쓴다.

EPUB은 표지 메타데이터(has_cover)로 판정한다. PDF는 메타데이터가 없어서
첫 쪽의 첫 이미지가 표준 표지 크기인지로 판정한다.
"""

import logging
import sqlite3
import threading
from pathlib import Path
from typing import Any, Callable

LOGGER = logging.getLogger(__name__)

# 네이버 웹툰 480x623, 카카오 웹툰 합성 표지 750x824, 네이버 도전만화 썸네일 202x164.
STANDARD_COVER_SIZES = frozenset({(480, 623), (750, 824), (202, 164)})


def pdf_first_image_size(path: Path, lock: Any) -> tuple[int, int] | None:
    """PDF 첫 쪽의 첫 이미지 픽셀 크기. 이미지가 없으면 None."""
    import pypdfium2
    import pypdfium2.raw as pdfium_raw

    # pdfium은 스레드에 안전하지 않아 썸네일 렌더링과 같은 lock을 쓴다. 파일 객체는 직접 소유한다.
    with lock, open(path, "rb") as fp:
        pdf = pypdfium2.PdfDocument(fp)
        try:
            if len(pdf) == 0:
                return None
            for obj in pdf[0].get_objects(filter=[pdfium_raw.FPDF_PAGEOBJ_IMAGE], max_depth=1):
                return obj.get_px_size()
            return None
        finally:
            pdf.close()


def has_official_cover(path: Path, file_type: str, lock: Any) -> bool:
    """정식 표지가 있으면 True. epub, pdf 말고는 판정하지 않고 True로 둔다."""
    if file_type == "epub":
        from backend.epub_cover import inspect_epub

        return bool(inspect_epub(path)["has_cover"])
    if file_type == "pdf":
        return pdf_first_image_size(path, lock) in STANDARD_COVER_SIZES
    return True


class CoverStatusCache:
    """파일별 판정을 (경로, mtime, 크기)로 묶어 sqlite에 저장한다. 파일이 바뀌면 다시 판정한다."""

    def __init__(self, db_path: Path):
        self.db_path = db_path
        self._init_lock = threading.Lock()
        self._ready = False

    def _connect(self) -> sqlite3.Connection:
        if not self._ready:
            with self._init_lock:
                if not self._ready:
                    self.db_path.parent.mkdir(parents=True, exist_ok=True)
                    with sqlite3.connect(self.db_path, timeout=10) as conn:
                        conn.execute("CREATE TABLE IF NOT EXISTS cover_status (file_path TEXT PRIMARY KEY, mtime_ns INTEGER NOT NULL, size INTEGER NOT NULL, has_cover INTEGER NOT NULL)")
                    self._ready = True
        return sqlite3.connect(self.db_path, timeout=10)

    def get_or_compute(self, path: Path, compute: Callable[[], bool]) -> bool:
        stat = path.stat()
        key = str(path)
        conn = self._connect()
        try:
            row = conn.execute("SELECT mtime_ns, size, has_cover FROM cover_status WHERE file_path = ?", (key,)).fetchone()
            if row and row[0] == stat.st_mtime_ns and row[1] == stat.st_size:
                return bool(row[2])
            covered = bool(compute())
            with conn:
                conn.execute("INSERT OR REPLACE INTO cover_status (file_path, mtime_ns, size, has_cover) VALUES (?, ?, ?, ?)", (key, stat.st_mtime_ns, stat.st_size, int(covered)))
            return covered
        finally:
            conn.close()
