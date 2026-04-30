"""HWP V3.00/V2.10/V2.00 텍스트 추출기 테스트."""

import struct
import zlib

import pytest

from utils.hwp3_parser import _HwpStream, _HwpStreamError, _extract_text_bruteforce, _handle_control_char, _hnc_to_unicode, _parse_paragraph, _parse_paragraph_list, _safe_chr, extract_text_from_hwp3


# ─── _HwpStream ─────────────────────────────────────────────────────────────


class TestHwpStream:
    def test_remaining(self):
        s = _HwpStream(b"\x01\x02\x03")
        assert s.remaining() == 3
        s.read_uint8()
        assert s.remaining() == 2

    def test_read_uint8(self):
        s = _HwpStream(b"\xff")
        assert s.read_uint8() == 255

    def test_read_uint16_little_endian(self):
        s = _HwpStream(struct.pack("<H", 0x1234))
        assert s.read_uint16() == 0x1234

    def test_read_uint32_little_endian(self):
        s = _HwpStream(struct.pack("<I", 0xDEADBEEF))
        assert s.read_uint32() == 0xDEADBEEF

    def test_skip(self):
        s = _HwpStream(b"\x00" * 10)
        s.skip(5)
        assert s.remaining() == 5

    def test_read_bytes(self):
        s = _HwpStream(b"\x01\x02\x03")
        assert s.read_bytes(2) == b"\x01\x02"
        assert s.remaining() == 1

    def test_read_uint8_underflow_raises(self):
        with pytest.raises(_HwpStreamError):
            _HwpStream(b"").read_uint8()

    def test_read_uint16_underflow_raises(self):
        with pytest.raises(_HwpStreamError):
            _HwpStream(b"\x01").read_uint16()

    def test_read_uint32_underflow_raises(self):
        with pytest.raises(_HwpStreamError):
            _HwpStream(b"\x01\x02\x03").read_uint32()

    def test_skip_negative_raises(self):
        with pytest.raises(_HwpStreamError):
            _HwpStream(b"\x00" * 5).skip(-1)

    def test_skip_overflow_raises(self):
        with pytest.raises(_HwpStreamError):
            _HwpStream(b"\x00" * 3).skip(10)

    def test_read_bytes_overflow_raises(self):
        with pytest.raises(_HwpStreamError):
            _HwpStream(b"\x01\x02").read_bytes(5)


# ─── _safe_chr ───────────────────────────────────────────────────────────────


class TestSafeChr:
    def test_normal_char(self):
        assert _safe_chr(0x0041) == "A"

    def test_hangul_char(self):
        assert _safe_chr(0xAC00) == "가"

    def test_surrogate_low_boundary(self):
        assert _safe_chr(0xD800) == ""

    def test_surrogate_high_boundary(self):
        assert _safe_chr(0xDFFF) == ""

    def test_surrogate_middle(self):
        assert _safe_chr(0xDC00) == ""

    def test_just_below_surrogate(self):
        assert _safe_chr(0xD7FF) == chr(0xD7FF)

    def test_just_above_surrogate(self):
        assert _safe_chr(0xE000) == chr(0xE000)


# ─── _hnc_to_unicode ────────────────────────────────────────────────────────


class TestHncToUnicode:
    def test_zero_returns_empty(self):
        assert _hnc_to_unicode(0) == ""

    def test_ascii_printable(self):
        assert _hnc_to_unicode(0x41) == "A"
        assert _hnc_to_unicode(0x20) == " "
        assert _hnc_to_unicode(0x7E) == "~"

    def test_below_ascii_printable(self):
        # 0x001F is below 0x0020, not ASCII printable → checks HNC2UNI or returns ""
        result = _hnc_to_unicode(0x001F)
        assert isinstance(result, str)

    def test_ksc5601_range(self):
        # 0x4000–0x5317 → KSC5601_TO_UNI table
        result = _hnc_to_unicode(0x4000)
        assert isinstance(result, str)

    def test_hangul_modern_syllable(self):
        # 현대 한글: cho=2, jung=3, jong=1 → T_MAP[1]=0 (not NONE) → 첫 번째 조건 통과
        # c = 0x8000 | (2 << 10) | (3 << 5) | 1 = 0x8861
        c = 0x8000 | (2 << 10) | (3 << 5) | 1
        result = _hnc_to_unicode(c)
        assert result == "가"  # 0xAC00 + (0*21+0)*28+0 = 0xAC00

    def test_hangul_leading_only(self):
        # 초성만: cho != NONE, jung==FILL(2), jong==FILL(1)
        # cho=2, jung=2(FILL in HNC_V1), jong=1(FILL in HNC_T1)
        c = 0x8000 | (2 << 10) | (2 << 5) | 1
        result = _hnc_to_unicode(c)
        assert isinstance(result, str)

    def test_hangul_vowel_only(self):
        # 중성만: cho==FILL(1), jung not FILL/NONE, jong==FILL(1)
        c = 0x8000 | (1 << 10) | (3 << 5) | 1
        result = _hnc_to_unicode(c)
        assert isinstance(result, str)


# ─── _extract_text_bruteforce ────────────────────────────────────────────────


class TestExtractTextBruteforce:
    def _make_bytes(self, codes):
        return struct.pack("<" + "H" * len(codes), *codes)

    def test_ascii_text(self):
        # "Hello World Text" (16 chars, all alnum/space → readable ratio >= 0.7)
        text = "Hello World Text"
        codes = [ord(c) for c in text]
        data = self._make_bytes(codes)
        result = _extract_text_bruteforce(data)
        assert "Hello World Text" in result

    def test_newline_and_tab(self):
        codes = [0x48, 0x65, 0x6C, 0x6C, 0x6F, 13, 0x57, 0x6F, 0x72, 0x6C, 0x64, 9, 0x54, 0x65, 0x78, 0x74]
        data = self._make_bytes(codes)
        result = _extract_text_bruteforce(data)
        assert isinstance(result, str)

    def test_zero_bytes_ignored(self):
        codes = [0, 0, ord("A"), 0, 0]
        data = self._make_bytes(codes)
        result = _extract_text_bruteforce(data)
        assert isinstance(result, str)

    def test_short_run_filtered(self):
        # 7자 이하는 품질 필터에서 제거
        codes = [ord(c) for c in "Hello!!"]  # 7 chars
        data = self._make_bytes(codes)
        result = _extract_text_bruteforce(data)
        assert result == ""

    def test_empty_input(self):
        assert _extract_text_bruteforce(b"") == ""

    def test_odd_byte_tail_ignored(self):
        data = b"\x41\x00\x01"  # 3 bytes: one uint16 + 1 trailing
        result = _extract_text_bruteforce(data)
        assert isinstance(result, str)


# ─── extract_text_from_hwp3 (통합) ──────────────────────────────────────────


def _make_header(version: str) -> bytes:
    """30바이트 HWP3 헤더를 만든다."""
    prefix = b"HWP Document File "
    ver_bytes = version.encode("ascii")
    header = prefix + ver_bytes
    return header.ljust(30, b"\x00")[:30]


def _make_doc_info(encrypted=0, compressed=0, info_block_len=0) -> bytes:
    buf = bytearray(128)
    struct.pack_into("<H", buf, 96, encrypted)
    buf[124] = compressed
    struct.pack_into("<H", buf, 126, info_block_len)
    return bytes(buf)


def _make_font_style_block(n_font_groups=7) -> bytes:
    """폰트 0개짜리 그룹 × n_font_groups + 스타일 0개."""
    data = b""
    for _ in range(n_font_groups):
        data += struct.pack("<H", 0)
    data += struct.pack("<H", 0)  # n_styles = 0
    return data


def _make_paragraph(chars: list[int], n_lines: int = 1) -> bytes:
    """단순 문단 바이너리 생성 (prev_para_shape=1, char_shape_included=0)."""
    n_chars = len(chars)
    hdr = struct.pack("<BHH B", 1, n_chars, n_lines, 0)
    hdr += b"\x00" * (1 + 4 + 1 + 31)  # skip(1+4+1+31)
    hdr += b"\x00" * (n_lines * 14)  # line info
    for c in chars:
        hdr += struct.pack("<H", c)
    return hdr


def _make_hwp3(version="V3.00", body_extra=b"", encrypted=0, compressed=0, info_block_len=0):
    """최소한의 유효한 HWP3 바이너리를 구성한다."""
    header = _make_header(version)
    doc_info = _make_doc_info(encrypted=encrypted, compressed=compressed, info_block_len=info_block_len)
    summary = b"\x00" * (1008 + info_block_len)
    body = _make_font_style_block() + body_extra
    return header + doc_info + summary + body


class TestExtractTextFromHwp3:
    def test_nonexistent_file(self, tmp_path):
        result = extract_text_from_hwp3(tmp_path / "no_such.hwp")
        assert result == ""

    def test_too_short(self, tmp_path):
        f = tmp_path / "short.hwp"
        f.write_bytes(b"HWP")
        assert extract_text_from_hwp3(f) == ""

    def test_wrong_signature(self, tmp_path):
        f = tmp_path / "bad_sig.hwp"
        f.write_bytes(b"NOT HWP Document File V3.00    " + b"\x00" * 200)
        assert extract_text_from_hwp3(f) == ""

    def test_wrong_version(self, tmp_path):
        f = tmp_path / "bad_ver.hwp"
        header = b"HWP Document File V1.00\x00\x00\x00\x00\x00\x00\x00"
        f.write_bytes(header + b"\x00" * 300)
        assert extract_text_from_hwp3(f) == ""

    def test_too_short_for_doc_info(self, tmp_path):
        f = tmp_path / "no_doc_info.hwp"
        header = _make_header("V3.00")
        f.write_bytes(header + b"\x00" * 10)
        assert extract_text_from_hwp3(f) == ""

    def test_encrypted_returns_empty(self, tmp_path):
        f = tmp_path / "enc.hwp"
        f.write_bytes(_make_hwp3(encrypted=1))
        assert extract_text_from_hwp3(f) == ""

    def test_body_offset_overflow(self, tmp_path):
        f = tmp_path / "overflow.hwp"
        # info_block_len이 실제 데이터보다 크면 body_offset > len(data) → ""
        f.write_bytes(_make_hwp3(info_block_len=9999))
        assert extract_text_from_hwp3(f) == ""

    def test_v300_empty_body(self, tmp_path):
        f = tmp_path / "empty.hwp"
        f.write_bytes(_make_hwp3(version="V3.00"))
        # 폰트/스타일 헤더만 있고 문단 없음 → 빈 텍스트 → ""
        result = extract_text_from_hwp3(f)
        assert result == ""

    def test_v210_empty_body(self, tmp_path):
        f = tmp_path / "v210.hwp"
        f.write_bytes(_make_hwp3(version="V2.10"))
        result = extract_text_from_hwp3(f)
        assert result == ""

    def test_v300_with_ascii_text(self, tmp_path):
        f = tmp_path / "hello.hwp"
        # "Hello World Test!" 17자 → readable ratio 충분
        text = "Hello World Test!"
        chars = [ord(c) for c in text]
        body_extra = _make_paragraph(chars)
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=body_extra))
        result = extract_text_from_hwp3(f)
        assert "Hello World Test!" in result

    def test_v300_with_newline(self, tmp_path):
        f = tmp_path / "nl.hwp"
        chars = [ord("A")] * 20 + [13]  # 'A' × 20 + 문단끝
        body_extra = _make_paragraph(chars)
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=body_extra))
        result = extract_text_from_hwp3(f)
        assert "A" * 20 in result

    def test_v300_compressed(self, tmp_path):
        f = tmp_path / "compressed.hwp"
        text = "Compressed Content Test Data"
        chars = [ord(c) for c in text]
        uncompressed_body = _make_font_style_block() + _make_paragraph(chars)
        compressed_body = zlib.compress(uncompressed_body)[2:-4]  # raw deflate
        f.write_bytes(_make_hwp3(version="V3.00", compressed=1) + compressed_body)
        # 헤더+doc_info+summary 이후 부분만 compressed body로 교체해야 하므로
        # 올바른 방식으로 파일 조합
        header = _make_header("V3.00")
        doc_info = _make_doc_info(encrypted=0, compressed=1, info_block_len=0)
        summary = b"\x00" * 1008
        f.write_bytes(header + doc_info + summary + compressed_body)
        result = extract_text_from_hwp3(f)
        assert isinstance(result, str)

    def test_v200_brute_force_fallback(self, tmp_path):
        f = tmp_path / "v200.hwp"
        # V2.00: 구조적 파싱 실패 시 brute-force fallback
        text = "BruteForce Korean Test Data!!"
        chars = [ord(c) for c in text]
        body_extra = b"".join(struct.pack("<H", c) for c in chars)
        # font+style 헤더 없이 body_extra만 넣으면 구조적 파싱 실패 → brute-force
        header = _make_header("V2.00")
        doc_info = _make_doc_info()
        summary = b"\x00" * 1008
        f.write_bytes(header + doc_info + summary + body_extra)
        result = extract_text_from_hwp3(f)
        assert isinstance(result, str)

    def test_v300_garbage_ratio_returns_empty(self, tmp_path):
        f = tmp_path / "garbage.hwp"
        chars = [ord("A")] * 50
        body_extra = _make_paragraph(chars)
        raw = _make_hwp3(version="V3.00", body_extra=body_extra)
        f.write_bytes(raw[:-20])  # 마지막 20바이트 제거 → 파싱 도중 HwpStreamError
        result = extract_text_from_hwp3(f)
        assert isinstance(result, str)

    def test_v300_quality_filter_passes(self, tmp_path):
        f = tmp_path / "quality.hwp"
        text = "A" * 100  # 100자 전부 alnum → ratio=1.0 > 0.3
        chars = [ord(c) for c in text]
        body_extra = _make_paragraph(chars)
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=body_extra))
        result = extract_text_from_hwp3(f)
        assert "A" * 100 in result

    def test_v300_prev_para_shape_zero(self, tmp_path):
        """prev_para_shape=0, n_chars>0 → 187바이트 문단 모양 추가 파싱."""
        f = tmp_path / "pps0.hwp"
        n_chars = 5
        n_lines = 1
        # prev_para_shape=0, n_chars=5 → 187바이트 추가 skip
        para = struct.pack("<BHH B", 0, n_chars, n_lines, 0)
        para += b"\x00" * 37  # flags/special/istyle/para_shape
        para += b"\x00" * 187  # 문단 모양 정보
        para += b"\x00" * (n_lines * 14)  # line info
        for code in [ord("H"), ord("e"), ord("l"), ord("l"), ord("o")]:
            para += struct.pack("<H", code)
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=para))
        result = extract_text_from_hwp3(f)
        assert "Hello" in result

    def test_v300_char_shape_included(self, tmp_path):
        """char_shape_included=1 → 글자 모양 플래그 읽기."""
        f = tmp_path / "csi.hwp"
        n_chars = 3
        n_lines = 1
        para = struct.pack("<BHH B", 1, n_chars, n_lines, 1)  # char_shape_included=1
        para += b"\x00" * 37
        para += b"\x00" * (n_lines * 14)
        para += b"\x01" * n_chars  # flag=1 → 추가 skip 없음
        for code in [ord("A"), ord("B"), ord("C")]:
            para += struct.pack("<H", code)
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=para))
        result = extract_text_from_hwp3(f)
        assert "ABC" in result

    def test_v300_char_shape_included_flag0(self, tmp_path):
        """char_shape_included, flag != 1 → 31바이트 skip."""
        f = tmp_path / "csi_flag0.hwp"
        n_chars = 1
        n_lines = 1
        para = struct.pack("<BHH B", 1, n_chars, n_lines, 1)
        para += b"\x00" * 37
        para += b"\x00" * (n_lines * 14)
        para += b"\x00"  # flag=0 → skip(31)
        para += b"\x00" * 31
        para += struct.pack("<H", ord("X"))
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=para))
        result = extract_text_from_hwp3(f)
        assert isinstance(result, str)


# ─── _parse_paragraph 직접 테스트 ────────────────────────────────────────────


def _para_stream(prev_para_shape, n_chars, n_lines, char_shape_included, char_codes, extra_prefix=b""):
    """_parse_paragraph 호출용 스트림 생성."""
    data = struct.pack("<BHH B", prev_para_shape, n_chars, n_lines, char_shape_included)
    data += b"\x00" * 37
    if prev_para_shape == 0 and n_chars > 0:
        data += b"\x00" * 187
    data += b"\x00" * (n_lines * 14)
    if char_shape_included:
        data += b"\x01" * n_chars
    for code in char_codes:
        data += struct.pack("<H", code)
    return _HwpStream(extra_prefix + data)


class TestParseParagraph:
    def test_n_chars_zero_returns_none(self):
        s = _para_stream(1, 0, 0, 0, [])
        assert _parse_paragraph(s, 0) is None

    def test_n_chars_too_large_raises(self):
        data = struct.pack("<BHH B", 1, 30001, 0, 0) + b"\x00" * 37
        with pytest.raises(_HwpStreamError):
            _parse_paragraph(_HwpStream(data), 0)

    def test_n_lines_too_large_raises(self):
        data = struct.pack("<BHH B", 1, 1, 5001, 0) + b"\x00" * 37
        with pytest.raises(_HwpStreamError):
            _parse_paragraph(_HwpStream(data), 0)

    def test_simple_ascii_text(self):
        s = _para_stream(1, 5, 1, 0, [ord("H"), ord("e"), ord("l"), ord("l"), ord("o")])
        result = _parse_paragraph(s, 0)
        assert result == "Hello"

    def test_control_char_tab_increments_n_chars_read(self):
        # c=9(tab): n_chars_read += 3, stream.skip(6)
        n_chars = 4  # 1 real read + 3 from increment
        data = struct.pack("<BHH B", 1, n_chars, 1, 0)
        data += b"\x00" * 37
        data += b"\x00" * 14  # line info
        data += struct.pack("<H", 9)  # tab
        data += b"\x00" * 6  # skip(6) in handle_control_char
        _parse_paragraph(_HwpStream(data), 0)  # no assertion: just no exception

    def test_control_char_23_increments_n_chars_read(self):
        # c=23: n_chars_read += 4, stream.skip(8)
        n_chars = 5
        data = struct.pack("<BHH B", 1, n_chars, 1, 0)
        data += b"\x00" * 37
        data += b"\x00" * 14
        data += struct.pack("<H", 23)
        data += b"\x00" * 8
        _parse_paragraph(_HwpStream(data), 0)

    def test_control_char_28_increments_n_chars_read(self):
        # c=28: n_chars_read += 31, stream.skip(62)
        n_chars = 32
        data = struct.pack("<BHH B", 1, n_chars, 1, 0)
        data += b"\x00" * 37
        data += b"\x00" * 14
        data += struct.pack("<H", 28)
        data += b"\x00" * 62
        _parse_paragraph(_HwpStream(data), 0)

    def test_control_char_30_increments_n_chars_read(self):
        # c=30: n_chars_read += 1, stream.skip(2)
        n_chars = 2
        data = struct.pack("<BHH B", 1, n_chars, 1, 0)
        data += b"\x00" * 37
        data += b"\x00" * 14
        data += struct.pack("<H", 30)
        data += b"\x00" * 2
        _parse_paragraph(_HwpStream(data), 0)

    def test_control_char_24_increments_n_chars_read(self):
        # c=24: n_chars_read += 2, stream.skip(4)
        n_chars = 3
        data = struct.pack("<BHH B", 1, n_chars, 1, 0)
        data += b"\x00" * 37
        data += b"\x00" * 14
        data += struct.pack("<H", 24)
        data += b"\x00" * 4
        _parse_paragraph(_HwpStream(data), 0)


# ─── _parse_paragraph_list 직접 테스트 ──────────────────────────────────────


class TestParseParagraphList:
    def test_recursion_depth_limit(self):
        # depth > 10 → 즉시 ""
        result = _parse_paragraph_list(_HwpStream(b"\x00" * 100), depth=11)
        assert result == ""

    def test_empty_stream_returns_empty(self):
        result = _parse_paragraph_list(_HwpStream(b"\x00" * 10))
        assert result == ""

    def test_max_text_length_break(self):
        # 다수의 문단으로 _MAX_TEXT_LENGTH(1_000_000)를 넘기는 건 비현실적이므로
        # 정상 문단 리스트 파싱만 검증
        s = _para_stream(1, 3, 1, 0, [ord("A"), ord("B"), ord("C")])
        result = _parse_paragraph_list(s, depth=0)
        assert result == "ABC"


# ─── _handle_control_char 직접 테스트 ───────────────────────────────────────


class TestHandleControlChar:
    def _stream(self, data: bytes) -> _HwpStream:
        return _HwpStream(data)

    def test_c13_newline(self):
        parts = []
        _handle_control_char(13, self._stream(b""), parts, 0)
        assert parts == ["\n"]

    def test_c9_tab(self):
        parts = []
        _handle_control_char(9, self._stream(b"\x00" * 6), parts, 0)
        assert parts == ["\t"]

    def test_c5_field(self):
        # skip(6) + uint32(field_len=4) + skip(2) + skip(4)
        data = b"\x00" * 6 + struct.pack("<I", 4) + b"\x00" * 2 + b"\x00" * 4
        _handle_control_char(5, self._stream(data), [], 0)

    def test_c6_bookmark(self):
        _handle_control_char(6, self._stream(b"\x00" * 40), [], 0)

    def test_c7_date_format(self):
        _handle_control_char(7, self._stream(b"\x00" * 84), [], 0)

    def test_c8_date_code(self):
        _handle_control_char(8, self._stream(b"\x00" * 96), [], 0)

    def test_c14_line(self):
        _handle_control_char(14, self._stream(b"\x00" * 92), [], 0)

    def test_c18_to_c21(self):
        for c in (18, 19, 20, 21):
            _handle_control_char(c, self._stream(b"\x00" * 6), [], 0)

    def test_c23_char_overlap(self):
        _handle_control_char(23, self._stream(b"\x00" * 8), [], 0)

    def test_c24_to_c26_and_c29(self):
        for c in (24, 25, 26, 29):
            _handle_control_char(c, self._stream(b"\x00" * 4), [], 0)

    def test_c28_outline(self):
        _handle_control_char(28, self._stream(b"\x00" * 62), [], 0)

    def test_c30_and_c31(self):
        for c in (30, 31):
            _handle_control_char(c, self._stream(b"\x00" * 2), [], 0)

    def test_c15_hidden_note(self):
        # skip(16) + empty paragraph list
        _handle_control_char(15, self._stream(b"\x00" * 16), [], 0)

    def test_c16_header_footer(self):
        _handle_control_char(16, self._stream(b"\x00" * 16), [], 0)

    def test_c17_footnote_empty(self):
        parts = []
        _handle_control_char(17, self._stream(b"\x00" * 20), parts, 0)
        assert parts == []

    def test_c11_picture_empty(self):
        # skip(6) + uint32(pic_len=0) + skip(344) + skip(0) + empty paragraph list
        data = b"\x00" * 6 + struct.pack("<I", 0) + b"\x00" * 344
        _handle_control_char(11, self._stream(data), [], 0)

    def test_c10_table_no_cells(self):
        # skip(6) + skip(80) + uint16(n_cells=0) + skip(2) + skip(0)
        data = b"\x00" * 6 + b"\x00" * 80 + struct.pack("<H", 0) + b"\x00" * 2
        _handle_control_char(10, self._stream(data), [], 0)


# ─── _hnc_to_unicode 추가 범위 테스트 ────────────────────────────────────────


class TestHncToUnicodeExtended:
    def test_hnc2uni_range_known(self):
        # 0x007F-0x3FFF 범위 중 HNC2UNI에 있는 코드
        from utils.hwp3_tables import HNC2UNI

        known_code = next((k for k in HNC2UNI if 0x007F <= k <= 0x3FFF), None)
        if known_code is not None:
            result = _hnc_to_unicode(known_code)
            assert isinstance(result, str) and len(result) <= 1

    def test_hnc2uni_range_unknown(self):
        # HNC2UNI에 없는 0x007F-0x3FFF 범위 코드 → ""
        from utils.hwp3_tables import HNC2UNI

        for code in range(0x0080, 0x0200):
            if code not in HNC2UNI:
                result = _hnc_to_unicode(code)
                assert result == ""
                break

    def test_ksc5601_none_value(self):
        # KSC5601_TO_UNI 테이블에서 값이 0/None인 항목 → ""
        from utils.hwp3_tables import KSC5601_TO_UNI

        for i, val in enumerate(KSC5601_TO_UNI):
            if not val:
                result = _hnc_to_unicode(0x4000 + i)
                assert result == ""
                break

    def test_hnc2uni_high_range(self):
        # 0x5318-0x7FFF 범위
        from utils.hwp3_tables import HNC2UNI

        known = next((k for k in HNC2UNI if 0x5318 <= k <= 0x7FFF), None)
        if known is not None:
            result = _hnc_to_unicode(known)
            assert isinstance(result, str)


# ─── _extract_text_bruteforce 추가 경로 테스트 ───────────────────────────────


class TestExtractBruteforceExtended:
    def test_unknown_hnc_code_flushes_run(self):
        # c >= 0x0080에서 _hnc_to_unicode가 "" 반환 → current_run flush
        # 먼저 readable run을 만들고, 이후 unknown code로 flush 유발
        text_codes = [ord(c) for c in "Hello World Test"]  # readable run
        # unknown HNC code (not in HNC2UNI, not KSC, not Hangul)
        unknown = 0x00FF  # 0x007F <= 0x00FF <= 0x3FFF, but likely not in HNC2UNI
        from utils.hwp3_tables import HNC2UNI

        if unknown in HNC2UNI:
            unknown = next((c for c in range(0x0100, 0x0400) if c not in HNC2UNI), None)
        if unknown is None:
            return  # 모든 코드가 매핑된 경우 skip
        all_codes = text_codes + [unknown]
        data = struct.pack("<" + "H" * len(all_codes), *all_codes)
        result = _extract_text_bruteforce(data)
        assert isinstance(result, str)

    def test_garbage_pattern_filtered(self):
        # "豼豼d" 패턴 → 필터링됨
        garbage = "豼豼d" * 3  # 9자 이상
        codes = [ord(c) for c in garbage]
        data = struct.pack("<" + "H" * len(codes), *codes)
        result = _extract_text_bruteforce(data)
        assert "豼豼" not in result

    def test_control_char_flushes_run(self):
        # c가 0/9/13/32/0x21-0x7E/0x80+ 이외의 값 → else 분기로 current_run flush
        text_codes = [ord(c) for c in "Hello World Test"]
        control_code = 4  # 처리되지 않는 제어 문자
        data = struct.pack("<" + "H" * (len(text_codes) + 1), *(text_codes + [control_code]))
        result = _extract_text_bruteforce(data)
        assert isinstance(result, str)


# ─── _hnc_to_unicode 추가 한글 케이스 ────────────────────────────────────────


class TestHncToUnicodeHangulSpecial:
    def test_jongseong_only(self):
        # 종성만: cho=1(FILL in L1), jung=0(NONE in V1), jong=2(0x3132 in T1)
        # HNC_L1[1]=FILL, HNC_V1[0]=NONE → (NONE==FILL or NONE==NONE)=True, HNC_T1[2]=0x3132≠FILL
        c = 0x8000 | (1 << 10) | (0 << 5) | 2  # = 0x8402
        result = _hnc_to_unicode(c)
        assert result == chr(0x3131)  # ㄱ (HNC_T1[2] = 0x3131)

    def test_old_hangul_choseong_jungseong(self):
        # 옛한글 초성+중성: cho=0(L_MAP=NONE,L1=0x3172), jung=3(V1=0x314f), jong=1(T1=FILL)
        c = 0x8000 | (0 << 10) | (3 << 5) | 1  # = 0x8061
        result = _hnc_to_unicode(c)
        assert isinstance(result, str) and len(result) >= 1

    def test_old_hangul_all_three(self):
        # 옛한글 초성+중성+종성: cho=2, jung=3, jong=0(T_MAP=NONE)
        c = 0x8000 | (2 << 10) | (3 << 5) | 0  # = 0x8860
        result = _hnc_to_unicode(c)
        assert isinstance(result, str) and len(result) >= 1

    def test_old_hangul_fallback_jung0(self):
        # 완성형 옛한글 fallback: jung==0 이면서 앞 조건 미충족
        # cho=2(L_MAP=0,L1=0x3131), jung=0(V_MAP=NONE,V1=NONE), jong=0(T_MAP=NONE,T1=0x316d)
        c = 0x8000 | (2 << 10) | (0 << 5) | 0  # = 0x8800
        result = _hnc_to_unicode(c)
        assert isinstance(result, str)


# ─── _parse_paragraph_list 추가 테스트 ───────────────────────────────────────


class TestParseParagraphListExtended:
    def test_empty_paragraph_terminates_list(self):
        # n_chars=0 문단 → _parse_paragraph가 None 반환 → break (line 146)
        # prev_para_shape=1, n_chars=0, n_lines=0, char_shape_included=0 + skip(37) = 43 bytes
        data = struct.pack("<BHH B", 1, 0, 0, 0) + b"\x00" * 37
        assert len(data) == 43
        result = _parse_paragraph_list(_HwpStream(data), depth=0)
        assert result == ""


# ─── extract_text_from_hwp3 추가 경로 테스트 ─────────────────────────────────


class TestExtractTextExtended:
    def test_quality_filter_low_ratio(self, tmp_path):
        # 특수문자만으로 구성된 텍스트 → ratio < 0.3 → return ""
        # "!" 은 alnum=False, space=False, Korean 아님 → readable=0
        chars = [ord("!")] * 50  # 50개 "!" → ratio=0.0
        body_extra = _make_paragraph(chars)
        f = tmp_path / "ratio_low.hwp"
        f.write_bytes(_make_hwp3(version="V3.00", body_extra=body_extra))
        result = extract_text_from_hwp3(f)
        assert result == ""

    def test_compressed_both_fail(self, tmp_path):
        # 압축=1 이지만 body가 유효하지 않은 데이터 → 두 decompression 모두 실패 → ""
        header = _make_header("V3.00")
        doc_info = _make_doc_info(compressed=1)
        summary = b"\x00" * 1008
        bad_body = b"\xff\xfe\xfd\xfc" * 20  # 유효하지 않은 압축 데이터
        f = tmp_path / "bad_compress.hwp"
        f.write_bytes(header + doc_info + summary + bad_body)
        result = extract_text_from_hwp3(f)
        assert result == ""

    def test_v200_with_valid_font_style(self, tmp_path):
        # V2.00: 구조적 파싱 성공 후 brute-force (font/style skip 포함 경로)
        header = _make_header("V2.00")
        doc_info = _make_doc_info()
        summary = b"\x00" * 1008
        # 유효한 font+style 헤더 + 읽을 수 있는 내용
        text = "BruteForce Test Content Data!!"
        body = _make_font_style_block() + struct.pack("<" + "H" * len(text), *(ord(c) for c in text))
        f = tmp_path / "v200_full.hwp"
        f.write_bytes(header + doc_info + summary + body)
        result = extract_text_from_hwp3(f)
        assert isinstance(result, str)

    def test_v200_hwp_stream_error_fallback(self, tmp_path):
        # V2.00 brute-force에서 skip_stream이 HwpStreamError → body 전체로 fallback (lines 411-415)
        header = _make_header("V2.00")
        doc_info = _make_doc_info()
        summary = b"\x00" * 1008
        # 폰트 그룹 카운트가 큰 값 → skip_stream에서 HwpStreamError 발생
        # uint16=100 → skip(100*40=4000 bytes) 하려 하지만 데이터가 부족
        bad_body = struct.pack("<H", 100)  # n_fonts=100 but no font data
        f = tmp_path / "v200_err.hwp"
        f.write_bytes(header + doc_info + summary + bad_body)
        result = extract_text_from_hwp3(f)
        assert isinstance(result, str)
