#!/usr/bin/env python


import sys
import os
import re
import getopt
import logging.config
import shutil
import subprocess
import tempfile
import warnings
import zlib
import zipfile
from datetime import datetime
from pathlib import Path
from itertools import islice
from typing import Dict, Any, List, Set, Tuple, Optional, Iterable

import ebooklib
from ebooklib import epub
import pypdf
from docx import Document
from striprtf.striprtf import rtf_to_text
from bs4 import BeautifulSoup

from backend.es_manager import ESManager
from utils.stat import Stat
from utils.isbn import extract as extract_isbn

logging.config.fileConfig(Path(__file__).parent.parent / "logging.conf", disable_existing_loggers=False)
LOGGER = logging.getLogger()

# ebooklib 내부에서 XHTML을 HTML 파서로 읽을 때 발생하는 경고 억제
warnings.filterwarnings("ignore", message=".*XML.*HTML.*")

if "TM_BOOK_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_BOOK_DIR is not set.")
    sys.exit(-1)

if "TM_COMICS_DIR" not in os.environ:
    LOGGER.error("The environment variable TM_COMICS_DIR is not set.")
    sys.exit(-1)


class Loader:
    TEXT_SIZE = 4096
    path_prefix = Path(os.environ["TM_BOOK_DIR"])
    comics_path_prefix = Path(os.environ["TM_COMICS_DIR"])

    @staticmethod
    def get_path_prefix(file_path: Path) -> Path:
        """파일 경로에 해당하는 path_prefix를 반환"""
        if file_path.is_relative_to(Loader.comics_path_prefix):
            return Loader.comics_path_prefix
        return Loader.path_prefix

    @staticmethod
    def read_from_text(file_path: Path) -> Tuple[str, int, int, str]:
        """TXT 파일 읽기. 반환: (summary, line_count, page_count, raw_content)"""
        Stat.text_count += 1
        start_time = datetime.now()

        line_count = 0
        data = ""
        raw_content = ""
        try:
            with file_path.open("r", encoding="utf-8") as infile:
                # 한 번에 읽어서 처리
                raw_content = infile.read()
                line_count = raw_content.count("\n") + 1 if raw_content else 0
                data = raw_content[: Loader.TEXT_SIZE]
                data = data.replace("\ufeff", "")
                data = re.sub(r"[^\w\sㄱ-힣]", " ", data)
        except UnicodeDecodeError as e:
            LOGGER.error(f"can't read unicode text from file '{file_path}', {e}")
            data = ""

        end_time = datetime.now()
        Stat.text_total_time += (end_time - start_time).total_seconds()

        return data, line_count, 0, raw_content

    @staticmethod
    def read_from_epub_with_extracting_zip(file_path: Path) -> Tuple[str, int]:
        """메모리에서 직접 zip 내용을 읽어 처리 (임시 디렉토리 사용 안 함)"""
        result = ""
        total_text = ""
        try:
            with zipfile.ZipFile(file_path, "r") as zf:
                # 1. container.xml에서 rootfile 찾기
                container_content = zf.read("META-INF/container.xml").decode("utf-8", errors="ignore")
                m = re.search(r'<rootfile[^>]*full-path="(?P<root_file>[^"]+)"', container_content)
                if not m:
                    return "", 0
                root_file = m.group("root_file")
                root_dir = "/".join(root_file.split("/")[:-1])

                # 2. OPF 파일에서 chapter 파일 목록 찾기
                opf_content = zf.read(root_file).decode("utf-8", errors="ignore")
                matches = re.findall(r'<(?:opf:)?item\s[^>]*href="(?P<chapter_file>[^"]*\.x?html)"[^>]*media-type="application/xhtml\+xml"', opf_content)

                # 3. 각 chapter 파일 읽기
                for chapter_file in matches:
                    chapter_path = f"{root_dir}/{chapter_file}" if root_dir else chapter_file
                    try:
                        content = zf.read(chapter_path).decode("utf-8", errors="ignore")
                        with warnings.catch_warnings():
                            warnings.filterwarnings("ignore", message=".*XML.*HTML.*")
                            soup = BeautifulSoup(content, "lxml")
                        text = soup.get_text()
                        total_text += text
                        if len(result) < Loader.TEXT_SIZE:
                            result += text
                    except Exception:
                        continue
        except zipfile.BadZipFile as e:
            LOGGER.error(file_path)
            LOGGER.error(e)

        line_count = total_text.count("\n") + 1 if total_text else 0
        return result[: Loader.TEXT_SIZE], line_count

    @staticmethod
    def read_from_epub(file_path: Path) -> Tuple[str, int, int]:
        Stat.normal_epub_count += 1
        start_time = datetime.now()

        result = ""
        total_text = ""
        line_count = 0
        try:
            book = epub.read_epub(file_path)
            titles = book.get_metadata("DC", "title")
            if titles:
                for title in titles:
                    if title[0]:
                        result += " " + title[0]
            creators = book.get_metadata("DC", "creator")
            if creators:
                for creator in creators:
                    if creator[0]:
                        result += " " + creator[0]

            for doc in book.get_items_of_type(ebooklib.ITEM_DOCUMENT):
                with warnings.catch_warnings():
                    warnings.filterwarnings("ignore", message=".*XML.*HTML.*")
                    soup = BeautifulSoup(doc.get_body_content(), "lxml")
                text = soup.get_text()
                total_text += text
                if len(result) < Loader.TEXT_SIZE:
                    result += text

            line_count = total_text.count("\n") + 1 if total_text else 0

            end_time = datetime.now()
            Stat.normal_epub_total_time += (end_time - start_time).total_seconds()
        except Exception as e:
            LOGGER.error(file_path)
            LOGGER.error(e)

            Stat.normal_epub_count -= 1
            Stat.zipped_epub_count += 1
            start_time = datetime.now()

            try:
                result, line_count = Loader.read_from_epub_with_extracting_zip(file_path)
            except Exception as e2:
                LOGGER.error(file_path)
                LOGGER.error(e2)

            end_time = datetime.now()
            Stat.zipped_epub_total_time += (end_time - start_time).total_seconds()

        result = re.sub(r"[^\w\sㄱ-힣]", " ", result)

        return result[: Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_pdf(file_path: Path) -> Tuple[str, int, int]:
        Stat.pdf_count += 1
        start_time = datetime.now()

        result = ""
        page_count = 0
        with file_path.open("rb") as infile:
            try:
                reader = pypdf.PdfReader(infile)
                page_count = len(reader.pages)
                for page in reader.pages:
                    text = page.extract_text()
                    if len(result) < Loader.TEXT_SIZE:
                        result += text
                    else:
                        break
            except Exception as e:
                LOGGER.error(file_path)
                LOGGER.error(e)
        result = re.sub(r"[^\w\sㄱ-힣]", " ", result)

        end_time = datetime.now()
        Stat.pdf_total_time += (end_time - start_time).total_seconds()

        return result[: Loader.TEXT_SIZE], 0, page_count

    @staticmethod
    def read_from_html(file_path: Path) -> Tuple[str, int, int]:
        Stat.html_count += 1
        start_time = datetime.now()

        content = ""
        line_count = 0
        with file_path.open("r") as infile:
            content = infile.read()
            line_count = content.count("\n") + 1

        # XMLParsedAsHTMLWarning 억제하고 lxml 파서 사용
        with warnings.catch_warnings():
            warnings.filterwarnings("ignore", message=".*XML.*HTML.*")
            soup = BeautifulSoup(content, "lxml")
        result = soup.get_text()
        result = re.sub(r"[^\w\sㄱ-힣]", " ", result)

        end_time = datetime.now()
        Stat.html_total_time += (end_time - start_time).total_seconds()

        return result[: Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_docx(file_path: Path) -> Tuple[str, int, int]:
        Stat.docx_count += 1
        start_time = datetime.now()

        result = ""
        total_text = ""
        doc = Document(str(file_path))
        for paragraph in doc.paragraphs:
            text = paragraph.text
            total_text += text + "\n"
            if len(result) < Loader.TEXT_SIZE:
                result += text
        result = re.sub(r"[^\w\sㄱ-힣]", " ", result)
        line_count = total_text.count("\n") if total_text else 0

        end_time = datetime.now()
        Stat.docx_total_time += (end_time - start_time).total_seconds()

        return result[: Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_rtf(file_path: Path) -> Tuple[str, int, int]:
        Stat.rtf_count += 1
        start_time = datetime.now()

        result = ""
        line_count = 0
        try:
            with file_path.open("rb") as infile:
                raw_data = infile.read()
                doc = raw_data.decode("utf-8")
                result = rtf_to_text(doc, errors="ignore")
                line_count = result.count("\n") + 1 if result else 0
        except Exception as e:
            LOGGER.error(file_path)
            LOGGER.error(e)
        result = re.sub(r"[^\w\sㄱ-힣]", " ", result)

        end_time = datetime.now()
        Stat.rtf_total_time += (end_time - start_time).total_seconds()

        return result[: Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def _find_libreoffice() -> str:
        """LibreOffice 실행 파일 경로를 반환"""
        for cmd in ["libreoffice", "soffice"]:
            path = shutil.which(cmd)
            if path:
                return path
        mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice"
        if Path(mac_path).exists():
            return mac_path
        return "libreoffice"

    @staticmethod
    def _convert_with_libreoffice(file_path: Path, output_format: str) -> str:
        """LibreOffice를 사용하여 파일을 변환하고 결과 텍스트를 반환"""
        lo_bin = Loader._find_libreoffice()
        with tempfile.TemporaryDirectory() as tmpdir:
            proc = subprocess.run([lo_bin, "--headless", "--convert-to", output_format, "--outdir", tmpdir, str(file_path)], capture_output=True, timeout=60)
            ext = output_format.split(":")[0]
            out_file = Path(tmpdir) / (file_path.stem + "." + ext)
            if out_file.exists():
                return out_file.read_text(encoding="utf-8", errors="replace")
            # stem 불일치 시 글로브로 검색
            out_files = list(Path(tmpdir).glob(f"*.{ext}"))
            if out_files:
                return out_files[0].read_text(encoding="utf-8", errors="replace")
            # 변환 결과 없음 — 진단 로그
            all_files = list(Path(tmpdir).iterdir())
            LOGGER.error("LibreOffice produced no output: file='%s', format='%s', returncode=%d, stderr=%s, tmpdir_files=%s", file_path, output_format, proc.returncode, proc.stderr.decode("utf-8", errors="replace")[:500], [f.name for f in all_files])
        return ""

    @staticmethod
    def read_from_doc(file_path: Path) -> Tuple[str, int, int]:
        Stat.doc_count += 1
        start_time = datetime.now()
        result = ""
        line_count = 0
        try:
            raw_text = Loader._convert_with_libreoffice(file_path, "txt:Text")
            line_count = raw_text.count("\n") + 1 if raw_text else 0
            result = raw_text[: Loader.TEXT_SIZE]
            result = re.sub(r"[^\w\sㄱ-힣]", " ", result)
        except Exception as e:
            LOGGER.error(f"can't read doc file '{file_path}': {e}")
        end_time = datetime.now()
        Stat.doc_total_time += (end_time - start_time).total_seconds()
        return result[: Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_hwp(file_path: Path) -> Tuple[str, int, int]:
        Stat.hwp_count += 1
        start_time = datetime.now()
        result = ""
        line_count = 0
        try:
            raw_text = Loader._convert_with_libreoffice(file_path, "txt:Text")
            if not raw_text.strip():
                # LibreOffice 변환 실패 — 네이티브 HWP3 파서로 fallback
                from utils.hwp3_parser import extract_text_from_hwp3

                raw_text = extract_text_from_hwp3(file_path)
            line_count = raw_text.count("\n") + 1 if raw_text else 0
            result = raw_text[: Loader.TEXT_SIZE]
            result = re.sub(r"[^\w\sㄱ-힣]", " ", result)
        except Exception as e:
            LOGGER.error(f"can't read hwp file '{file_path}': {e}")
        end_time = datetime.now()
        Stat.hwp_total_time += (end_time - start_time).total_seconds()
        return result[: Loader.TEXT_SIZE], line_count, 0

    @staticmethod
    def read_from_image(_file_path: Path) -> Tuple[str, int, int]:
        Stat.image_count += 1
        # 이미지는 line_count, page_count 해당 없음
        return "", 0, 0

    @staticmethod
    def _find_xref_offset(xref_data: bytes, obj_num: int) -> Optional[int]:
        """traditional xref 테이블에서 특정 object의 파일 내 offset을 찾는다."""
        xref_text = xref_data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
        lines = xref_text.split(b"\n")

        i = 0
        while i < len(lines) and lines[i].strip() != b"xref":
            i += 1
        i += 1  # 'xref' 건너뛰기

        while i < len(lines):
            line = lines[i].strip()
            if not line:
                i += 1
                continue
            if line.startswith(b"trailer"):
                break
            parts = line.split()
            if len(parts) == 2 and parts[0].isdigit() and parts[1].isdigit():
                sub_start = int(parts[0])
                sub_count = int(parts[1])
                if sub_start <= obj_num < sub_start + sub_count:
                    entry_idx = i + 1 + (obj_num - sub_start)
                    if entry_idx < len(lines):
                        entry_parts = lines[entry_idx].strip().split()
                        if len(entry_parts) >= 3 and entry_parts[2] == b"n":
                            return int(entry_parts[0])
                    return None
                i += 1 + sub_count
                continue
            i += 1
        return None

    @staticmethod
    def _apply_png_predictor(data: bytes, columns: int) -> bytes:
        """PNG predictor 해제 (xref stream 디코딩용). Sub/Up 필터 지원."""
        stride = 1 + columns  # filter byte + row data
        rows = len(data) // stride
        result = bytearray()
        prev_row = bytes(columns)
        for i in range(rows):
            offset = i * stride
            fb = data[offset]
            row = bytearray(data[offset + 1 : offset + 1 + columns])
            if fb == 1:  # Sub
                for j in range(1, columns):
                    row[j] = (row[j] + row[j - 1]) & 0xFF
            elif fb == 2:  # Up
                for j in range(columns):
                    row[j] = (row[j] + prev_row[j]) & 0xFF
            result.extend(row)
            prev_row = bytes(row)
        return bytes(result)

    @staticmethod
    def _xref_stream_find_entry(data: bytes, w: List[int], index_ranges: List[int], obj_num: int) -> Optional[Tuple[int, int, int]]:
        """xref stream에서 특정 object의 entry를 찾는다.
        반환: (type, field2, field3) 또는 None.
        type 0: free, type 1: (1, file_offset, gen), type 2: (2, obj_stream_num, index)"""
        w1, w2, w3 = w
        entry_size = w1 + w2 + w3
        if entry_size == 0:
            return None
        pos = 0
        for i in range(0, len(index_ranges) - 1, 2):
            start_obj = index_ranges[i]
            count = index_ranges[i + 1]
            for j in range(count):
                if pos + entry_size > len(data):
                    return None
                if start_obj + j == obj_num:
                    entry = data[pos : pos + entry_size]
                    type_val = int.from_bytes(entry[:w1], "big") if w1 > 0 else 1
                    field2 = int.from_bytes(entry[w1 : w1 + w2], "big") if w2 > 0 else 0
                    field3 = int.from_bytes(entry[w1 + w2 : w1 + w2 + w3], "big") if w3 > 0 else 0
                    return (type_val, field2, field3)
                pos += entry_size
        return None

    @staticmethod
    def _read_from_obj_stream(f, stream_offset: int, target_obj_num: int) -> Optional[bytes]:
        """Object stream에서 특정 object의 딕셔너리 데이터를 읽는다."""
        f.seek(stream_offset)
        header = f.read(4096)

        first_match = re.search(rb"/First\s+(\d+)", header)
        length_match = re.search(rb"/Length\s+(\d+)", header)
        if not first_match or not length_match:
            return None
        if re.search(rb"/Length\s+\d+\s+\d+\s+R", header):
            return None

        first_offset = int(first_match.group(1))
        stream_length = int(length_match.group(1))

        stream_start = re.search(rb"stream\r?\n", header)
        if not stream_start:
            return None

        f.seek(stream_offset + stream_start.end())
        raw = f.read(stream_length)

        filter_match = re.search(rb"/Filter\s*/(\w+)", header)
        if filter_match:
            if filter_match.group(1) != b"FlateDecode":
                return None
            try:
                raw = zlib.decompress(raw)
            except zlib.error:
                return None

        # 헤더 파싱: "obj_num offset obj_num offset ..."
        header_text = raw[:first_offset].decode("ascii", errors="replace")
        tokens = header_text.split()

        target_offset = None
        next_offset = len(raw)
        for i in range(0, len(tokens) - 1, 2):
            if int(tokens[i]) == target_obj_num:
                target_offset = int(tokens[i + 1]) + first_offset
                if i + 3 < len(tokens):
                    next_offset = int(tokens[i + 3]) + first_offset
                break

        if target_offset is None:
            return None
        return raw[target_offset:next_offset]

    @staticmethod
    def _parse_one_xref_stream(f, xref_offset: int) -> Optional[tuple]:
        """단일 xref stream을 파싱하여 (decompressed, w, index_ranges, prev_offset)를 반환.
        실패 시 None."""
        f.seek(xref_offset)
        stream_obj_data = f.read(4096)

        w_match = re.search(rb"/W\s*\[\s*(\d+)\s+(\d+)\s+(\d+)\s*\]", stream_obj_data)
        if not w_match:
            return None
        w = [int(w_match.group(i)) for i in (1, 2, 3)]

        size_match = re.search(rb"/Size\s+(\d+)", stream_obj_data)
        if not size_match:
            return None
        xref_size = int(size_match.group(1))

        index_match = re.search(rb"/Index\s*\[([^\]]+)\]", stream_obj_data)
        index_ranges = [int(x) for x in index_match.group(1).split()] if index_match else [0, xref_size]

        # /Length가 indirect reference이면 포기
        if re.search(rb"/Length\s+\d+\s+\d+\s+R", stream_obj_data):
            return None
        length_match = re.search(rb"/Length\s+(\d+)", stream_obj_data)
        if not length_match:
            return None
        stream_length = int(length_match.group(1))

        stream_start = re.search(rb"stream\r?\n", stream_obj_data)
        if not stream_start:
            return None

        f.seek(xref_offset + stream_start.end())
        raw_stream = f.read(stream_length)

        # 필터 처리
        filter_match = re.search(rb"/Filter\s*/(\w+)", stream_obj_data)
        if filter_match:
            if filter_match.group(1) != b"FlateDecode":
                return None
            try:
                decompressed = zlib.decompress(raw_stream)
            except zlib.error:
                return None
        else:
            decompressed = raw_stream

        # PNG predictor 처리
        parms_match = re.search(rb"/DecodeParms\s*<<([^>]*)>>", stream_obj_data)
        if parms_match:
            pred_match = re.search(rb"/Predictor\s+(\d+)", parms_match.group(1))
            if pred_match and int(pred_match.group(1)) >= 10:
                cols_match = re.search(rb"/Columns\s+(\d+)", parms_match.group(1))
                columns = int(cols_match.group(1)) if cols_match else sum(w)
                decompressed = Loader._apply_png_predictor(decompressed, columns)

        prev_match = re.search(rb"/Prev\s+(\d+)", stream_obj_data)
        prev_offset = int(prev_match.group(1)) if prev_match else None

        return decompressed, w, index_ranges, prev_offset

    @staticmethod
    def _fast_pdf_page_count(file_path: Path) -> Optional[int]:
        """PDF xref/trailer에서 페이지 수만 경량 추출 (전체 파싱 없이).

        trailer → /Root → /Pages → /Count 경로를 따라 필요한 객체만 읽는다.
        traditional xref table과 xref stream을 모두 지원하며,
        /Prev 체인을 따라 이전 xref 섹션도 탐색한다.
        실패 시 None 반환 (pypdf fallback용).
        """
        try:
            with file_path.open("rb") as f:
                f.seek(0, 2)
                file_size = f.tell()

                # 1. 파일 끝에서 startxref 찾기
                tail_size = min(4096, file_size)
                f.seek(-tail_size, 2)
                tail = f.read()

                m = re.search(rb"startxref\s+(\d+)", tail)
                if not m:
                    return None

                # 2. xref 체인을 따라가며 lookup 함수들 수집
                # lookups: [('traditional', fn) | ('stream', fn)] newest→oldest
                root_obj_num = None
                lookups: List[Tuple[str, Any]] = []
                xref_offset: Optional[int] = int(m.group(1))
                max_depth = 10

                while xref_offset is not None and max_depth > 0:
                    max_depth -= 1
                    f.seek(xref_offset)
                    xref_header = f.read(256)

                    if xref_header.lstrip().startswith(b"xref"):
                        # === Traditional xref table ===
                        header_match = re.search(rb"xref\s+\d+\s+(\d+)", xref_header)
                        entry_count = int(header_match.group(1)) if header_match else 5000
                        xref_read_size = min(entry_count * 20 + 4096, file_size - xref_offset)
                        f.seek(xref_offset)
                        xref_data = f.read(xref_read_size)

                        if root_obj_num is None:
                            root_match = re.search(rb"/Root\s+(\d+)\s+\d+\s+R", xref_data)
                            if root_match:
                                root_obj_num = int(root_match.group(1))

                        lookups.append(("traditional", lambda obj_num, xd=xref_data: Loader._find_xref_offset(xd, obj_num)))

                        prev_match = re.search(rb"/Prev\s+(\d+)", xref_data)
                        xref_offset = int(prev_match.group(1)) if prev_match else None
                    else:
                        # === Xref stream ===
                        f.seek(xref_offset)
                        peek = f.read(512)

                        if root_obj_num is None:
                            root_match = re.search(rb"/Root\s+(\d+)\s+\d+\s+R", peek)
                            if root_match:
                                root_obj_num = int(root_match.group(1))

                        parsed = Loader._parse_one_xref_stream(f, xref_offset)
                        if parsed is None:
                            break
                        decompressed, w, index_ranges, prev_offset = parsed
                        lookups.append(("stream", lambda obj_num, d=decompressed, ww=list(w), ir=list(index_ranges): Loader._xref_stream_find_entry(d, ww, ir, obj_num)))
                        xref_offset = prev_offset

                if root_obj_num is None:
                    return None

                def find_file_offset(obj_num: int) -> Optional[int]:
                    """type 1 (직접 저장) object의 파일 offset을 찾는다."""
                    for kind, lookup in lookups:
                        if kind == "traditional":
                            offset = lookup(obj_num)
                            if offset is not None:
                                return offset
                        else:
                            entry = lookup(obj_num)
                            if entry is not None and entry[0] == 1:
                                return entry[1]
                    return None

                def read_object_data(obj_num: int, read_size: int = 4096) -> Optional[bytes]:
                    """object 데이터를 읽는다 (type 1: 직접, type 2: object stream)."""
                    # type 1 시도
                    offset = find_file_offset(obj_num)
                    if offset is not None:
                        f.seek(offset)
                        return f.read(read_size)
                    # type 2 시도 (object stream에 압축 저장)
                    for kind, lookup in lookups:
                        if kind != "stream":
                            continue
                        entry = lookup(obj_num)
                        if entry is not None and entry[0] == 2:
                            stream_offset = find_file_offset(entry[1])
                            if stream_offset is not None:
                                return Loader._read_from_obj_stream(f, stream_offset, obj_num)
                    return None

                # 3. Root object → /Pages 참조 찾기
                root_data = read_object_data(root_obj_num, 1024)
                if root_data is None:
                    return None
                pages_match = re.search(rb"/Pages\s+(\d+)\s+\d+\s+R", root_data)
                if not pages_match:
                    return None
                pages_obj_num = int(pages_match.group(1))

                # 4. Pages object → /Count 추출
                pages_data = read_object_data(pages_obj_num)
                if pages_data is None:
                    return None
                count_match = re.search(rb"/Count\s+(\d+)", pages_data)
                if count_match:
                    return int(count_match.group(1))
        except Exception:
            pass
        return None

    @staticmethod
    def read_file(file_path: Path, stat_result: Optional[os.stat_result] = None, skip_text: bool = False) -> Dict[int, Dict[str, Any]]:
        if file_path.is_file():
            sys.stdout.flush()
            # read metadata of each file (stat 결과 재사용)
            st = stat_result if stat_result else file_path.stat()
            inode_num = st.st_ino
            file_size = st.st_size
            prefix = Loader.get_path_prefix(file_path)
            category = str(file_path.parent.relative_to(prefix))
            if category == ".":
                category = "_root"
            m = re.search(r"^\[(?P<author>[^\]]+)\]\s*(?P<title>.+)$", file_path.stem)
            if m:
                author = m.group("author")
                title = m.group("title")
            else:
                author = ""
                title = file_path.stem

            file_type = file_path.suffix[1:]

            # skip_text: 텍스트 추출 건너뛰기 (만화 등 이미지 기반 파일)
            if skip_text:
                summary = ""
                line_count = 0
                page_count = 0
                isbn_list = []
                # PDF만 예외: page_count 추출 유지 (경량 방식 우선, 실패 시 pypdf fallback)
                if file_type == "pdf":
                    page_count = Loader._fast_pdf_page_count(file_path)
                    if page_count is None:
                        page_count = 0
                        try:
                            with file_path.open("rb") as f:
                                page_count = len(pypdf.PdfReader(f).pages)
                        except Exception as e:
                            LOGGER.error(file_path)
                            LOGGER.error(e)
                    Stat.pdf_count += 1
                # 지원하지 않는 확장자는 기존 동작 유지 (빈 dict 반환)
                supported_types = {"txt", "epub", "pdf", "docx", "doc", "hwp", "rtf", "html", "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg", "cbz"}
                if file_type not in supported_types:
                    return {}

                return {
                    inode_num: {"category": category, "title": title, "author": author, "file_path": str(file_path.relative_to(prefix)), "file_type": file_type, "file_size": int(file_size), "line_count": line_count, "page_count": page_count, "isbn": "", "summary": summary, "updated_time": datetime.now().isoformat()}
                }

            # read content of each file
            summary = ""
            line_count = 0
            page_count = 0
            isbn_list = []
            raw_content = ""  # TXT 파일용 원본 content
            if file_type == "txt":
                summary, line_count, page_count, raw_content = Loader.read_from_text(file_path)
            elif file_type == "epub":
                summary, line_count, page_count = Loader.read_from_epub(file_path)
            elif file_type == "pdf":
                if file_path.is_relative_to(Loader.comics_path_prefix):
                    # comics: 텍스트 추출은 생략하되 페이지 수만 추출 (경량 방식 우선)
                    page_count = Loader._fast_pdf_page_count(file_path)
                    if page_count is None:
                        page_count = 0
                        try:
                            with file_path.open("rb") as f:
                                page_count = len(pypdf.PdfReader(f).pages)
                        except Exception as e:
                            LOGGER.error(file_path)
                            LOGGER.error(e)
                    summary, line_count = "", 0
                    Stat.pdf_count += 1
                else:
                    summary, line_count, page_count = Loader.read_from_pdf(file_path)
            elif file_type == "docx":
                summary, line_count, page_count = Loader.read_from_docx(file_path)
            elif file_type == "doc":
                summary, line_count, page_count = Loader.read_from_doc(file_path)
            elif file_type == "hwp":
                summary, line_count, page_count = Loader.read_from_hwp(file_path)
            elif file_type == "rtf":
                summary, line_count, page_count = Loader.read_from_rtf(file_path)
            elif file_type == "html":
                summary, line_count, page_count = Loader.read_from_html(file_path)
            elif file_type in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg"):
                summary, line_count, page_count = Loader.read_from_image(file_path)
            elif file_type == "cbz":
                summary, line_count, page_count = Loader.read_from_image(file_path)
            else:
                return {}

            # ISBN 추출 (TXT는 이미 읽은 content 재사용)
            if file_type in ("txt", "epub", "pdf", "djvu", "hwp"):
                isbn_list = extract_isbn(file_path, content=raw_content if file_type == "txt" else None)

            return {
                inode_num: {
                    "category": category,
                    "title": title,
                    "author": author,
                    "file_path": str(file_path.relative_to(prefix)),
                    "file_type": file_type,
                    "file_size": int(file_size),
                    "line_count": line_count,
                    "page_count": page_count,
                    "isbn": isbn_list[0] if isbn_list else "",
                    "summary": summary,
                    "updated_time": datetime.now().isoformat(),
                }
            }

        return {}

    @staticmethod
    def get_file_list(path: Path, num_files: int = sys.maxsize, recursive: bool = False) -> List[Path]:
        """파일 목록을 반환 (파싱 없이)

        recursive=False인 경우:
        1. 하위 디렉토리 각각에서 첫 번째 파일 1개씩
        2. 지정된 디렉토리에 바로 속한 파일들
        """
        if path.is_dir():
            if recursive:
                file_path_list = [p for p in path.rglob("*") if p.is_file() and not any(part.startswith(".") for part in p.relative_to(path).parts)]
            else:
                file_path_list = []
                # 1. 하위 디렉토리 각각에서 첫 번째 파일 1개씩
                for subdir in path.iterdir():
                    if subdir.is_dir() and not subdir.name.startswith("."):
                        first_file = next((p for p in subdir.iterdir() if p.is_file() and not p.name.startswith(".")), None)
                        if first_file:
                            file_path_list.append(first_file)
                # 2. 지정된 디렉토리에 바로 속한 파일들
                file_path_list.extend(p for p in path.iterdir() if p.is_file() and not p.name.startswith("."))
        else:
            file_path_list = [path]
        return file_path_list[:num_files]

    @staticmethod
    def get_stat(file_path: Path) -> os.stat_result:
        """파일의 stat 결과를 반환"""
        return file_path.stat()

    @staticmethod
    def read_files(path: Path, num_files: int = sys.maxsize, recursive: bool = False) -> Dict[int, Dict[str, Any]]:
        """파일들을 읽어서 데이터 딕셔너리로 반환 (테스트 및 하위 호환성용)"""
        data: Dict[int, Dict[str, Any]] = {}
        file_list = Loader.get_file_list(path, num_files, recursive)

        for child_path in file_list:
            print(f"* {child_path}")
            data_item = Loader.read_file(child_path)
            if data_item:
                data.update(data_item)

        return data


def print_usage(program_name: str):
    print(f"Usage:\t{program_name}\t[ --delete ] [ --reload ] [ --recursive ] <index_name> [file or directory path ...]")
    print("\t\tindex_name: book | comics")
    print("\t\t--delete: delete index and exit (no file path required)")
    print("\t\t--reload: force reload even if file already exists in ES")
    print("\t\t--recursive: scan subdirectories recursively")
    print()
    print("\t\tNote: When a file (not directory) is specified, it will be force-reloaded automatically.")
    sys.exit(0)


def main() -> int:
    BATCH_SIZE = 100

    do_delete = False
    do_reload = False
    do_recursive = False
    args: List[str] = []
    try:
        opts, args = getopt.getopt(sys.argv[1:], "", ["delete", "reload", "recursive"])
        for opt, _ in opts:
            if opt == "--delete":
                do_delete = True
            elif opt == "--reload":
                do_reload = True
            elif opt == "--recursive":
                do_recursive = True
    except getopt.GetoptError as e:
        LOGGER.error(e)
        print_usage(sys.argv[0])

    # 인덱스명 파싱
    INDEX_MAP = {"book": os.environ["TM_ES_BOOK_INDEX"], "comics": os.environ["TM_ES_COMICS_INDEX"]}

    if len(args) < 1:
        print_usage(sys.argv[0])
        return 1

    index_name_arg = args[0]
    if index_name_arg not in INDEX_MAP:
        LOGGER.error(f"유효하지 않은 인덱스명: '{index_name_arg}' (book 또는 comics만 가능)")
        print_usage(sys.argv[0])
        return 1

    file_args = args[1:]

    if not do_delete and len(file_args) < 1:
        print_usage(sys.argv[0])
        return 1

    start_time: datetime = datetime.now()

    # ESManager 초기화 (한 번의 실행에서 하나의 인덱스만 사용)
    idx = INDEX_MAP[index_name_arg]
    es_manager = ESManager(index_name=idx)
    try:
        if not es_manager.es.ping():
            LOGGER.error("Elasticsearch 서버에 연결할 수 없습니다.")
            sys.exit(-1)
    except Exception as e:
        LOGGER.error(f"Elasticsearch 접속 실패: {e}")
        sys.exit(-1)
    es_manager.create_index()
    print(f"ES 인덱스: {idx}")

    if do_delete:
        try:
            if es_manager.do_exist_index():
                es_manager.delete_index()
                print(f"인덱스 '{es_manager.index_name}' 삭제 완료")
            else:
                print(f"인덱스 '{es_manager.index_name}'가 존재하지 않습니다")
        except Exception as e:
            LOGGER.error(e)
            return -1
        return 0

    def process_file_iter(file_iter: Iterable[Path], skip_check: bool = False, skip_text: bool = False) -> Tuple[int, int, int]:
        """파일 iterator를 배치 처리하여 ES에 저장. 반환: (처리 수, 건너뜀 수, 경로동기화 수)"""
        skipped_count = 0
        processed_count = 0
        synced_count = 0
        file_iterator = iter(file_iter)

        while True:
            # generator에서 배치 단위로 가져오기
            batch_files = list(islice(file_iterator, BATCH_SIZE))
            if not batch_files:
                break

            # stat 수집 (한 번만 호출하여 inode와 함께 저장)
            file_stat_map: Dict[int, Tuple[Path, os.stat_result]] = {}
            for file_path in batch_files:
                try:
                    st = Loader.get_stat(file_path)
                    file_stat_map[st.st_ino] = (file_path, st)
                except OSError:
                    continue

            if skip_check:
                # --reload 모드: 존재 여부 무시, 전부 파싱
                new_inodes = set(file_stat_map.keys())
                path_changed_inodes: Set[int] = set()
                existing_paths: Dict[int, str] = {}
            else:
                # 기존 경로 조회 (inode → file_path)
                existing_paths = es_manager.get_existing_paths(list(file_stat_map.keys()))

                # 경로 변경 감지: ES에 있고 file_path가 다른 경우 → 재적재 대상
                path_changed_inodes: Set[int] = set()
                for inode, es_file_path in existing_paths.items():
                    if inode not in file_stat_map:
                        continue
                    file_path, _ = file_stat_map[inode]
                    prefix = Loader.get_path_prefix(file_path)
                    current_file_path = str(file_path.relative_to(prefix))
                    if current_file_path != es_file_path:
                        path_changed_inodes.add(inode)
                        print(f"  [경로 변경 감지] inode={inode}: {es_file_path} → {current_file_path}")

                # 신규 파일 + 경로 변경 파일: 전체 재적재 대상
                new_inodes = (set(file_stat_map.keys()) - set(existing_paths.keys())) | path_changed_inodes
                skipped_count += len(existing_paths) - len(path_changed_inodes)
                synced_count += len(path_changed_inodes)

            # 파일 파싱 (stat 결과 재사용)
            batch_data: Dict[int, Dict[str, Any]] = {}
            for inode in new_inodes:
                file_path, st = file_stat_map[inode]
                print(f"* {file_path}")
                data_item = Loader.read_file(file_path, stat_result=st, skip_text=skip_text)
                if data_item:
                    batch_data.update(data_item)

            # 데이터 저장 (중복 file_path 문서 선제거 후 insert)
            if batch_data:
                new_file_paths = [v["file_path"] for v in batch_data.values() if "file_path" in v]
                new_ids = list(batch_data.keys())
                cleaned = es_manager.delete_by_file_paths(new_file_paths, exclude_ids=new_ids)
                if cleaned > 0:
                    print(f"  [중복 제거: {cleaned}개 기존 문서 삭제]")
                es_manager.insert(batch_data)
                processed_count += len(batch_data)
                Stat.index_count += len(batch_data)
                if skip_check:
                    print(f"  [배치 저장: {len(batch_data)}개]")
                else:
                    synced_msg = f", 경로변경 재적재: {len(path_changed_inodes)}개" if path_changed_inodes else ""
                    skip_batch = len(existing_paths) - len(path_changed_inodes)
                    print(f"  [배치 저장: {len(batch_data)}개, 건너뜀: {skip_batch}개{synced_msg}]")

        return processed_count, skipped_count, synced_count

    for arg in file_args:
        target_path = Path(arg)
        if not target_path.exists():
            LOGGER.error("can't find such a file or directory '%s'", target_path)
            return 0
        if not target_path.is_relative_to(Loader.path_prefix) and not target_path.is_relative_to(Loader.comics_path_prefix):
            LOGGER.error(f"{target_path} is not in $TM_BOOK_DIR({Loader.path_prefix}) or $TM_COMICS_DIR({Loader.comics_path_prefix}).")
            continue
        print(f"====== {target_path} ======")

        # comics 인덱스는 텍스트 추출 건너뛰기
        skip_text = index_name_arg == "comics"

        # 파일이 지정된 경우: 강제 재적재 (skip_check=True)
        if target_path.is_file():
            print(f"  [파일 강제 재적재] {target_path.name}")
            processed, _, _ = process_file_iter([target_path], skip_check=True, skip_text=skip_text)
            if processed > 0:
                print("  파일 재적재 완료")
            else:
                print("  파일 적재 실패 (지원하지 않는 형식일 수 있음)")
        elif do_recursive:
            # 전체 파일 등록 (generator 사용으로 메모리 효율화, hidden directory 제외)
            file_iter = (p for p in target_path.rglob("*") if p.is_file() and not any(part.startswith(".") for part in p.relative_to(target_path).parts))
            skip_check = do_reload
            processed, skipped_count, synced_count = process_file_iter(file_iter, skip_check=skip_check, skip_text=skip_text)
            print(f"  총 {processed}개 파일 처리됨")
            if skipped_count > 0:
                print(f"  총 {skipped_count}개 중복 파일 건너뜀")
            if synced_count > 0:
                print(f"  총 {synced_count}개 경로 동기화")
        else:
            skip_check = do_reload

            # 1단계: 하위 디렉토리 각각에서 첫 번째 파일 1개씩 (모아서 한꺼번에 저장)
            print("  [1단계] 하위 디렉토리별 샘플 파일 등록")
            sample_files: List[Tuple[str, Path]] = []  # (subdir_name, file_path)
            for subdir in target_path.iterdir():
                if subdir.is_dir() and not subdir.name.startswith("."):
                    # 첫 번째 파일만 가져옴 (정렬 불필요, iterator 사용)
                    first_file = next((p for p in subdir.iterdir() if p.is_file() and not p.name.startswith(".")), None)
                    if first_file:
                        sample_files.append((subdir.name, first_file))
                    else:
                        print(f"    {subdir.name}/ -> (파일 없음)")

            if sample_files:
                print(f"    {len(sample_files)}개 디렉토리에서 샘플 파일 선정 완료")
                for subdir_name, sample_file in sample_files:
                    print(f"      {subdir_name}/ -> {sample_file.name}")

                # 모아서 한꺼번에 ES에 저장
                sample_file_paths = [f for _, f in sample_files]
                processed, skipped1, synced1 = process_file_iter(sample_file_paths, skip_check=skip_check, skip_text=skip_text)
                print(f"    {processed}개 카테고리 샘플 저장, {skipped1}개 중복 건너뜀")
                if synced1 > 0:
                    print(f"    {synced1}개 경로 동기화")

            # 2단계: 지정된 디렉토리에 바로 속한 파일들
            print("  [2단계] 현재 디렉토리 파일 등록")
            current_dir_files = [p for p in target_path.iterdir() if p.is_file() and not p.name.startswith(".")]
            print(f"    {len(current_dir_files)}개 파일 발견")
            _, skipped2, synced2 = process_file_iter(current_dir_files, skip_check=skip_check, skip_text=skip_text)
            if skipped2 > 0:
                print(f"    {skipped2}개 중복 파일 건너뜀")
            if synced2 > 0:
                print(f"    {synced2}개 경로 동기화")

        print("================================")

    # 모든 작업 완료 후 인덱스 refresh
    es_manager.refresh()

    end_time = datetime.now()
    Stat.index_total_time = (end_time - start_time).total_seconds()
    Stat.print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
