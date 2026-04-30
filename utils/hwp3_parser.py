"""HWP V2.10/V3.00 바이너리 포맷 텍스트 추출기.

구형 한글(HWP) 문서 파일에서 본문 텍스트를 추출한다.
LibreOffice가 처리하지 못하는 V1.20~V3.00 파일의 fallback으로 사용.

References:
- 한컴 공식 "한글 문서 파일 형식 3.0" 사양서
- ddoleye/java-hwp HwpTextExtractorV3.java
- namhyung/libhwp ghwp-file-v3.c
"""

import logging
import struct
import zlib
from pathlib import Path

from utils.hwp3_tables import NONE, FILL, L_MAP, V_MAP, T_MAP, HNC_L1, HNC_V1, HNC_T1, HNC_L2, HNC_V2, HNC_T2, KSC5601_TO_UNI, HNC2UNI

LOGGER = logging.getLogger(__name__)

_SIGNATURE_PREFIX = b"HWP Document File"
_MAX_RECURSION_DEPTH = 10
_MAX_TEXT_LENGTH = 1_000_000


class _HwpStreamError(Exception):
    pass


class _HwpStream:
    """바이트 버퍼 + 위치 트래킹 래퍼."""

    __slots__ = ("_data", "_pos")

    def __init__(self, data: bytes):
        self._data = data
        self._pos = 0

    def remaining(self) -> int:
        return len(self._data) - self._pos

    def read_uint8(self) -> int:
        if self._pos + 1 > len(self._data):
            raise _HwpStreamError("unexpected end of stream")
        val = self._data[self._pos]
        self._pos += 1
        return val

    def read_uint16(self) -> int:
        if self._pos + 2 > len(self._data):
            raise _HwpStreamError("unexpected end of stream")
        val = struct.unpack_from("<H", self._data, self._pos)[0]
        self._pos += 2
        return val

    def read_uint32(self) -> int:
        if self._pos + 4 > len(self._data):
            raise _HwpStreamError("unexpected end of stream")
        val = struct.unpack_from("<I", self._data, self._pos)[0]
        self._pos += 4
        return val

    def skip(self, n: int) -> None:
        if n < 0 or self._pos + n > len(self._data):
            raise _HwpStreamError(f"cannot skip {n} bytes (remaining: {self.remaining()})")
        self._pos += n

    def read_bytes(self, n: int) -> bytes:
        if self._pos + n > len(self._data):
            raise _HwpStreamError("unexpected end of stream")
        result = self._data[self._pos : self._pos + n]
        self._pos += n
        return result


def _safe_chr(code: int) -> str:
    """surrogate 범위(U+D800-U+DFFF)를 필터하여 chr() 변환."""
    if 0xD800 <= code <= 0xDFFF:
        return ""
    return chr(code)


def _hnc_to_unicode(c: int) -> str:
    """HNC 2바이트 코드를 유니코드 문자열로 변환."""
    if c == 0:
        return ""
    # ASCII
    if 0x0020 <= c <= 0x007E:
        return chr(c)
    # 특수문자/확장 (0x007F-0x3FFF)
    if 0x007F <= c <= 0x3FFF:
        uni = HNC2UNI.get(c)
        return _safe_chr(uni) if uni else ""
    # 1수준 한자 (4888자)
    if 0x4000 <= c <= 0x5317:
        uni = KSC5601_TO_UNI[c - 0x4000]
        return _safe_chr(uni) if uni else ""
    # 2수준 한자
    if 0x5318 <= c <= 0x7FFF:
        uni = HNC2UNI.get(c)
        return _safe_chr(uni) if uni else ""
    # 한글 (0x8000-0xFFFF)
    if c >= 0x8000:
        cho = (c & 0x7C00) >> 10
        jung = (c & 0x03E0) >> 5
        jong = c & 0x001F
        l_idx = L_MAP[cho]
        v_idx = V_MAP[jung]
        t_idx = T_MAP[jong]
        # 현대 한글 완성형 음절
        if l_idx != NONE and v_idx != NONE and t_idx != NONE:
            return chr(0xAC00 + (l_idx * 21 + v_idx) * 28 + t_idx)
        # 초성만
        if HNC_L1[cho] != FILL and (HNC_V1[jung] == FILL or HNC_V1[jung] == NONE) and HNC_T1[jong] == FILL:
            return _safe_chr(HNC_L1[cho])
        # 중성만
        if HNC_L1[cho] == FILL and HNC_V1[jung] not in (FILL, NONE) and HNC_T1[jong] == FILL:
            return _safe_chr(HNC_V1[jung])
        # 종성만
        if HNC_L1[cho] == FILL and (HNC_V1[jung] == FILL or HNC_V1[jung] == NONE) and HNC_T1[jong] != FILL:
            return _safe_chr(HNC_T1[jong])
        # 옛한글 (초성+중성)
        if HNC_L1[cho] != FILL and HNC_V1[jung] not in (FILL, NONE) and HNC_T1[jong] == FILL:
            return _safe_chr(HNC_L2[cho]) + _safe_chr(HNC_V2[jung])
        # 옛한글 (초성+중성+종성)
        if HNC_L1[cho] != FILL and HNC_V1[jung] not in (FILL, NONE) and HNC_T1[jong] != FILL:
            return _safe_chr(HNC_L2[cho]) + _safe_chr(HNC_V2[jung]) + _safe_chr(HNC_T2[jong])
        # 완성형 옛한글 fallback
        if jung == 0:
            uni = HNC2UNI.get(c)
            return _safe_chr(uni) if uni else ""
    return ""


def _parse_paragraph_list(stream: _HwpStream, depth: int = 0) -> str:
    """문단 리스트를 파싱하여 텍스트를 반환한다."""
    if depth > _MAX_RECURSION_DEPTH:
        return ""
    parts = []
    while stream.remaining() >= 43:
        try:
            text = _parse_paragraph(stream, depth)
        except _HwpStreamError:
            break  # 스트림 끝 도달 — 지금까지 수집한 텍스트 반환
        if text is None:
            break  # 빈 문단 (리스트 종료)
        parts.append(text)
        if sum(len(p) for p in parts) > _MAX_TEXT_LENGTH:
            break
    return "".join(parts)


def _parse_paragraph(stream: _HwpStream, depth: int) -> str | None:
    """단일 문단을 파싱한다. None이면 빈 문단(리스트 종료)."""
    prev_para_shape = stream.read_uint8()
    n_chars = stream.read_uint16()
    n_lines = stream.read_uint16()
    char_shape_included = stream.read_uint8()
    stream.skip(1 + 4 + 1 + 31)  # flags + special + istyle + para_shape_part

    if prev_para_shape == 0 and n_chars > 0:
        stream.skip(187)  # 문단 모양 정보

    if n_chars == 0:
        return None

    # sanity check
    if n_chars > 30000 or n_lines > 5000:
        raise _HwpStreamError(f"invalid paragraph: n_chars={n_chars}, n_lines={n_lines}")

    # 줄 정보
    stream.skip(n_lines * 14)

    # 글자 모양
    if char_shape_included != 0:
        for _ in range(n_chars):
            flag = stream.read_uint8()
            if flag != 1:
                stream.skip(31)

    # 글자 데이터 파싱
    parts = []
    n_chars_read = 0
    while n_chars_read < n_chars:
        c = stream.read_uint16()
        n_chars_read += 1

        if c < 32:
            _handle_control_char(c, stream, parts, depth)
            # 제어문자별 추가 n_chars_read 증가
            if c in (5, 6, 7, 8, 9, 10, 11, 14, 15, 16, 17, 18, 19, 20, 21):
                n_chars_read += 3
            elif c == 23:
                n_chars_read += 4
            elif c in (24, 25, 26, 29):
                n_chars_read += 2
            elif c == 28:
                n_chars_read += 31
            elif c in (30, 31):
                n_chars_read += 1
        else:
            ch = _hnc_to_unicode(c)
            if ch:
                parts.append(ch)

    return "".join(parts)


def _handle_control_char(c: int, stream: _HwpStream, parts: list, depth: int) -> None:
    """제어 문자를 처리한다."""
    if c == 13:  # 문단 끝
        parts.append("\n")
    elif c == 9:  # 탭
        stream.skip(6)
        parts.append("\t")
    elif c == 5:  # 필드 코드
        stream.skip(6)
        field_len = stream.read_uint32()
        stream.skip(2)
        stream.skip(field_len)
    elif c == 6:  # 책갈피
        stream.skip(6 + 34)
    elif c == 10:  # 표
        stream.skip(6)
        stream.skip(80)
        n_cells = stream.read_uint16()
        stream.skip(2)
        stream.skip(27 * n_cells)
        for _ in range(n_cells):
            text = _parse_paragraph_list(stream, depth + 1)
            if text:
                parts.append(text)
        # 캡션
        _parse_paragraph_list(stream, depth + 1)
    elif c == 11:  # 그림
        stream.skip(6)
        pic_len = stream.read_uint32()
        stream.skip(344)
        stream.skip(pic_len)
        # 캡션
        _parse_paragraph_list(stream, depth + 1)
    elif c == 16:  # 머리말/꼬리말
        stream.skip(6 + 10)
        _parse_paragraph_list(stream, depth + 1)
    elif c == 17:  # 각주/미주
        stream.skip(6 + 14)
        text = _parse_paragraph_list(stream, depth + 1)
        if text:
            parts.append(text)
    elif c == 7:  # 날짜 형식
        stream.skip(6 + 78)
    elif c == 8:  # 날짜 코드
        stream.skip(6 + 90)
    elif c == 14:  # 선
        stream.skip(6 + 86)
    elif c == 15:  # 숨은 설명
        stream.skip(6 + 10)
        _parse_paragraph_list(stream, depth + 1)
    elif c in (18, 19, 20, 21):
        stream.skip(6)
    elif c == 23:  # 글자 겹침
        stream.skip(8)
    elif c in (24, 25, 26, 29):  # 하이픈, 차례표시, 찾아보기표시, 상호참조
        stream.skip(4)
    elif c == 28:  # 개요 번호
        stream.skip(62)
    elif c in (30, 31):
        stream.skip(2)
    # 0-4, 12, 22, 27: 드물게 사용되거나 가변 길이
    # 데이터를 정확히 skip할 수 없으므로 무시 (대부분 0x0000 패딩)


def _extract_text_bruteforce(body_data: bytes) -> str:
    """바이트 스트림에서 HNC 텍스트를 brute-force로 추출한다 (V2.00 fallback)."""
    parts: list[str] = []
    current_run: list[str] = []
    i = 0

    while i + 2 <= len(body_data):
        c = struct.unpack_from("<H", body_data, i)[0]
        i += 2

        if c == 13:
            current_run.append("\n")
        elif c == 9:
            current_run.append("\t")
        elif c == 0x0020:
            current_run.append(" ")
        elif c == 0:
            continue
        elif 0x0021 <= c <= 0x007E:
            current_run.append(chr(c))
        elif c >= 0x0080:
            ch = _hnc_to_unicode(c)
            if ch:
                current_run.append(ch)
            else:
                if current_run:
                    parts.append("".join(current_run))
                current_run = []
        else:
            if current_run:
                parts.append("".join(current_run))
            current_run = []

    if current_run:
        parts.append("".join(current_run))

    # 품질 필터: 읽을 수 있는 문자 비율이 70% 이상이고 최소 8자인 run만 유지
    clean: list[str] = []
    for part in parts:
        stripped = part.strip()
        if len(stripped) < 8:
            continue
        readable = sum(1 for ch in stripped if ch.isspace() or ch.isalnum() or 0xAC00 <= ord(ch) <= 0xD7A3 or 0x4E00 <= ord(ch) <= 0x9FFF or 0x1100 <= ord(ch) <= 0x11FF)
        if readable / len(stripped) >= 0.7:
            # V2.00 레코드 구분자 패턴 제거
            if stripped in ("豼豼d", "Āú塴豼豼", "āĀĀú塴豼豼", "*ú塴豼豼", "āĀú塴豼豼", "Ǵ塴豼豼Āāā"):
                continue
            if "豼豼" in stripped and len(stripped) < 20:
                continue
            clean.append(stripped)

    return "\n".join(clean)


def extract_text_from_hwp3(file_path: Path) -> str:
    """HWP V2.00/V2.10/V3.00 파일에서 텍스트를 추출한다.

    Args:
        file_path: HWP 파일 경로

    Returns:
        추출된 텍스트. 파싱 실패 시 빈 문자열.
    """
    try:
        data = file_path.read_bytes()
    except OSError as e:
        LOGGER.error("cannot read file '%s': %s", file_path, e)
        return ""

    # 시그니처 확인
    if len(data) < 30 or not data[:17].startswith(_SIGNATURE_PREFIX):
        return ""

    # 버전 확인
    try:
        header_str = data[:30].decode("ascii", errors="replace")
    except Exception:
        return ""
    if "V3.00" not in header_str and "V2.10" not in header_str and "V2.00" not in header_str:
        return ""

    if len(data) < 30 + 128:
        return ""

    # 문서 정보 파싱 (offset 30, 128 bytes)
    doc_info = data[30 : 30 + 128]
    encrypted = struct.unpack_from("<H", doc_info, 96)[0]
    if encrypted != 0:
        LOGGER.info("encrypted HWP file, skipping: %s", file_path)
        return ""

    compressed = doc_info[124]
    info_block_len = struct.unpack_from("<H", doc_info, 126)[0]

    # 문서 요약 + 정보 블록 건너뛰기
    body_offset = 30 + 128 + 1008 + info_block_len
    if body_offset > len(data):
        return ""

    body_data = data[body_offset:]

    # 압축 해제
    if compressed != 0:
        try:
            body_data = zlib.decompress(body_data, -15)
        except zlib.error:
            # gzip 헤더가 있는 경우 시도
            try:
                body_data = zlib.decompress(body_data, 15 + 32)
            except zlib.error as e:
                LOGGER.error("decompression failed for '%s': %s", file_path, e)
                return ""

    stream = _HwpStream(body_data)

    try:
        # 글꼴 이름 건너뛰기 (7 카테고리)
        for _ in range(7):
            n_fonts = stream.read_uint16()
            stream.skip(n_fonts * 40)

        # 스타일 건너뛰기
        n_styles = stream.read_uint16()
        stream.skip(n_styles * 238)

        # 문단 리스트 파싱
        text = _parse_paragraph_list(stream)
    except _HwpStreamError:
        text = ""

    # V2.00: 구조적 파싱이 빈 결과이면 brute-force 텍스트 추출
    # 글꼴/스타일 영역 이후부터 추출하여 메타데이터 쓰레기를 최소화
    if not text.strip() and "V2.00" in header_str:
        try:
            skip_stream = _HwpStream(body_data)
            for _ in range(7):
                nf = skip_stream.read_uint16()
                skip_stream.skip(nf * 40)
            ns = skip_stream.read_uint16()
            skip_stream.skip(ns * 238)
            text = _extract_text_bruteforce(body_data[skip_stream._pos :])
        except _HwpStreamError:
            text = _extract_text_bruteforce(body_data)

    # surrogate 문자 제거 (UTF-8 인코딩 에러 방지)
    text = "".join(ch for ch in text if not (0xD800 <= ord(ch) <= 0xDFFF))

    # 쓰레기 문자 비율이 높으면 파싱 실패로 간주
    if text:
        readable = sum(
            1
            for ch in text[:500]
            if ch.isspace()
            or ch.isalnum()
            or 0xAC00 <= ord(ch) <= 0xD7A3  # 한글 음절
            or 0x3131 <= ord(ch) <= 0x318E  # 한글 자모
            or 0x4E00 <= ord(ch) <= 0x9FFF  # CJK 한자
            or 0x1100 <= ord(ch) <= 0x11FF
        )  # 한글 자모 (옛한글)
        ratio = readable / min(len(text), 500)
        if ratio < 0.3:
            return ""

    return text.strip()
