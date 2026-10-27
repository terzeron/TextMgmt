//! 원본: `tests/test_loader.py` 후반부 HWP 단위 테스트.
//!
//! 대상 클래스: `TestHwp3ParserDirect`, `TestHwp3ParserEdgeCases`, `TestHwp3StreamMethods`, `TestHncToUnicodeExtended`,
//! `TestHandleControlCharCoverage`, `TestParseEdgeCases`, `TestHandleControlCharRecursive`, `TestSafeChr`, `TestControlCharTab`,
//! `TestControlCharDateLine`, `TestV200BruteForce`. Python의 여러 개별 사례는 Rust 테스트에서 묶어 검증한다.
//!
//! 실제 `hwp3_parser` crate의 공개 API만 쓴다. 없는 함수를 가정하지 않는다(가정한 API: 없음). Python과 다른 점은 다음과 같다.
//! - 비공개 이름(`_HwpStream`, `_hnc_to_unicode` 등)은 밑줄 없이 공개돼 있다: `HwpStream`, `hnc_to_unicode`, `safe_chr`.
//! - `parse_paragraph_list(stream, depth)`는 `depth`가 필수다. Python 기본값 0을 명시한다.
//! - `_HwpStreamError` 예외는 `Result<_, StreamError>`로 바뀌었다. `pytest.raises` 자리는 `is_err()`로 검사한다.
//! - `_MAX_TEXT_LENGTH`(1_000_000)는 crate에서 비공개 상수라 같은 값을 둔다. 재귀 제한은 Python 테스트의 depth=11 입력으로 확인한다.
//!
//! Cargo target은 `utils/loader/Cargo.toml`의 `loader_hwp3`이며, parser/table crates는 dev-dependencies로 연결한다.
//!
//! 샘플 파일(tests/books/_hwp)을 사용하며, 파일이 없으면 헬퍼가 panic한다.

use std::fs;
use std::path::{Path, PathBuf};

use hwp3_parser::{HwpStream, extract_text_from_hwp3, hnc_to_unicode, parse_paragraph, parse_paragraph_list, safe_chr};
use hwp3_tables::hnc2uni;

/// `_MAX_TEXT_LENGTH`. crate에서 비공개라 같은 값을 둔다.
const MAX_TEXT_LENGTH: usize = 1_000_000;

// ---- 헬퍼 ----

fn hwp_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/books/_hwp")
}

/// 샘플 파일 경로. 파일이 없으면 panic한다.
fn hwp_fixture(name: &str) -> PathBuf {
    let f = hwp_dir().join(name);
    assert!(f.exists(), "샘플 파일 없음: {}", f.display());
    f
}

/// `_get_hwp3_parser()()` 대응.
fn extract(path: &Path) -> String {
    extract_text_from_hwp3(path)
}

/// 바이트를 임시 `.hwp` 파일로 쓰고 추출한다(`tmp_path / name`에 `write_bytes` 후 추출).
fn extract_bytes(name: &str, bytes: &[u8]) -> String {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join(name);
    fs::write(&f, bytes).unwrap();
    extract(&f)
}

/// `b"HWP Document File Vx.xx \x1a\x01\x02\x03\x04\x05"` (30바이트).
fn sig(version: &str) -> Vec<u8> {
    format!("HWP Document File {version} \x1a\x01\x02\x03\x04\x05").into_bytes()
}

fn le16(v: u16) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn le32(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn zeros(n: usize) -> Vec<u8> {
    vec![0u8; n]
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// `struct.pack("<BHHB", a, b, c, d)` (6바이트 문단 머리).
fn pack_bhhb(a: u8, b: u16, c: u16, d: u8) -> Vec<u8> {
    cat(&[&[a], &le16(b), &le16(c), &[d]])
}

/// 빈 문단(리스트 종료): 머리 6바이트 + 37 + 187.
fn empty_paragraph() -> Vec<u8> {
    cat(&[&pack_bhhb(0, 0, 0, 0), &zeros(37), &zeros(187)])
}

/// 한글 10자 `가나다라마바사아자차`의 HNC 코드.
const TEN_HANGUL: [u16; 10] = [0x8861, 0x8CC2, 0x9161, 0x9562, 0xA1A1, 0xA562, 0xAD61, 0xB161, 0xB961, 0xBD62];

fn pack_codes(codes: &[u16]) -> Vec<u8> {
    codes.iter().flat_map(|c| c.to_le_bytes()).collect()
}

fn parse_with_data(data: Vec<u8>) -> String {
    parse_paragraph_list(&mut HwpStream::new(data), 0)
}

// ---- TestHwp3ParserDirect ----

mod test_hwp3_parser_direct {
    use super::*;

    #[test]
    fn test_v3_00_extracts_korean_text() {
        let f = hwp_fixture("v3.00_현대시사전.hwp");
        let text = extract(&f);
        assert!(text.chars().count() > 10);
        assert!(text.contains("고정희") || text.contains("너")); // 시 제목/내용
    }

    #[test]
    fn test_v2_10_extracts_korean_text() {
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let text = extract(&f);
        assert!(text.chars().count() > 10);
        assert!(text.contains("영동") || text.contains("교가"));
    }

    #[test]
    fn test_v2_10_pager() {
        let f = hwp_fixture("v2.10_PAGER.hwp");
        let text = extract(&f);
        assert!(text.chars().count() > 100);
    }

    /// V1.20은 미지원이라 빈 문자열을 돌려주지만, 원본 단언은 `str` 타입뿐이다.
    #[test]
    fn test_v1_20_returns_empty() {
        let f = hwp_fixture("v1.20_부하의약혼녀.hwp");
        let _text: String = extract(&f);
    }

    #[test]
    fn test_invalid_file_returns_empty() {
        assert_eq!(extract_bytes("not_hwp.hwp", b"this is definitely not an HWP file"), "");
    }

    #[test]
    fn test_nonexistent_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(extract(&dir.path().join("does_not_exist.hwp")), "");
    }

    #[test]
    fn test_truncated_file_returns_empty() {
        let data = cat(&[b"HWP Document File V3.00 \x1a\x01\x02\x03\x04\x05", &zeros(50)]);
        assert_eq!(extract_bytes("truncated.hwp", &data), "");
    }

    /// 암호화 플래그가 설정된 파일.
    #[test]
    fn test_encrypted_file_returns_empty() {
        let mut doc_info = zeros(128);
        doc_info[96] = 1; // encryption flag (low byte of uint16)
        let data = cat(&[&sig("V3.00"), &doc_info, &zeros(1008)]);
        assert_eq!(extract_bytes("encrypted.hwp", &data), "");
    }
}

// ---- TestHwp3ParserEdgeCases ----

mod test_hwp3_parser_edge_cases {
    use super::*;

    /// V3.00 압축 파일 정상 파싱.
    #[test]
    fn test_compressed_v3_file() {
        let f = hwp_fixture("v3.00_현대시사전.hwp");
        // 압축 플래그 확인
        let data = fs::read(&f).unwrap();
        assert_ne!(data[30 + 124], 0, "테스트 파일이 압축되어 있어야 함");
        let text = extract(&f);
        assert!(!text.is_empty());
    }

    /// V2.10 파일 정상 파싱.
    #[test]
    fn test_v2_10_file_parses() {
        let f = hwp_fixture("v2.10_KYOKA.hwp");
        let text = extract(&f);
        assert!(!text.is_empty());
    }

    /// 추출된 텍스트에 null 바이트가 없어야 함.
    #[test]
    fn test_text_contains_no_null_bytes() {
        for name in ["v2.10_KYOKA.hwp", "v3.00_현대시사전.hwp", "v2.10_PAGER.hwp"] {
            let f = hwp_fixture(name);
            let text = extract(&f);
            assert!(!text.contains('\x00'), "{name}: null 바이트 포함");
        }
    }

    /// 추출된 텍스트가 유효한 유니코드 문자열이어야 함.
    #[test]
    fn test_text_is_valid_unicode() {
        for name in ["v2.10_KYOKA.hwp", "v3.00_현대시사전.hwp"] {
            let f = hwp_fixture(name);
            let text = extract(&f);
            // encode/decode가 에러 없이 수행되어야 함
            let encoded = text.clone().into_bytes();
            let decoded = String::from_utf8(encoded).unwrap();
            assert_eq!(decoded, text);
        }
    }

    /// 헤더만 있고 본문이 없는 파일.
    #[test]
    fn test_empty_body_after_header() {
        // 압축 안 함, 정보블록 0
        let data = cat(&[&sig("V3.00"), &zeros(128), &zeros(1008)]);
        assert_eq!(extract_bytes("empty_body.hwp", &data), "");
    }

    /// 압축 플래그가 설정되었지만 데이터가 손상된 경우.
    #[test]
    fn test_corrupted_compression() {
        let mut doc_info = zeros(128);
        doc_info[124] = 1; // compressed
        let data = cat(&[&sig("V3.00"), &doc_info, &zeros(1008), &[0xff, 0xfe, 0xfd, 0xfc].repeat(100)]);
        assert_eq!(extract_bytes("bad_compress.hwp", &data), "");
    }

    /// `_HwpStream` 범위 초과 시 예외 발생.
    #[test]
    fn test_hwp3_stream_bounds() {
        let mut stream = HwpStream::new(vec![1, 2, 3]);
        assert_eq!(stream.read_uint8().unwrap(), 1);
        assert_eq!(stream.remaining(), 2);
        assert!(stream.read_uint32().is_err()); // 4바이트 필요하지만 2바이트만 남음
    }

    /// `_HwpStream` 음수 skip 시 예외.
    #[test]
    fn test_hwp3_stream_skip_negative() {
        let mut stream = HwpStream::new(vec![1, 2, 3]);
        assert!(stream.skip(-1).is_err());
    }
}

// ---- TestHwp3StreamMethods ----

mod test_hwp3_stream_methods {
    use super::*;

    #[test]
    fn test_read_uint32() {
        let mut stream = HwpStream::new(vec![1, 0, 0, 0, 2, 0, 0, 0]);
        assert_eq!(stream.read_uint32().unwrap(), 1);
        assert_eq!(stream.read_uint32().unwrap(), 2);
    }

    #[test]
    fn test_read_uint32_overflow() {
        let mut stream = HwpStream::new(vec![1, 2]);
        assert!(stream.read_uint32().is_err());
    }

    #[test]
    fn test_read_bytes() {
        let mut stream = HwpStream::new(b"hello world".to_vec());
        assert_eq!(stream.read_bytes(5).unwrap(), b"hello");
        assert_eq!(stream.remaining(), 6);
    }

    #[test]
    fn test_read_bytes_overflow() {
        let mut stream = HwpStream::new(b"hi".to_vec());
        assert!(stream.read_bytes(10).is_err());
    }

    #[test]
    fn test_read_uint16_overflow() {
        let mut stream = HwpStream::new(vec![1]);
        assert!(stream.read_uint16().is_err());
    }

    #[test]
    fn test_read_uint8_overflow() {
        let mut stream = HwpStream::new(vec![]);
        assert!(stream.read_uint8().is_err());
    }

    #[test]
    fn test_skip_overflow() {
        let mut stream = HwpStream::new(vec![1]);
        assert!(stream.skip(100).is_err());
    }
}

// ---- TestHncToUnicodeExtended ----

mod test_hnc_to_unicode_extended {
    use super::*;

    /// 0x007F-0x3FFF: 특수문자 매핑. 0x0080 = 유로 기호.
    #[test]
    fn test_special_char_range() {
        assert_eq!(hnc_to_unicode(0x0080), "€");
    }

    /// 매핑 없는 특수문자는 빈 문자열(범위 밖).
    #[test]
    fn test_special_char_unmapped() {
        assert_eq!(hnc_to_unicode(0x0001), "");
    }

    /// 0x5318-0x7FFF: 2수준 한자. 매핑이 있는 첫 코드를 찾아 한 글자인지 본다.
    #[test]
    fn test_hanja_level2() {
        for code in 0x5318u16..0x5400 {
            if hnc2uni(code).is_some() {
                let result = hnc_to_unicode(code);
                assert_eq!(result.chars().count(), 1);
                break;
            }
        }
    }

    /// 매핑 없는 2수준 한자. 원본 단언은 `str` 타입뿐이다.
    #[test]
    fn test_hanja_level2_unmapped() {
        let _result: String = hnc_to_unicode(0x7FFF); // 매핑 없을 확률 높음
    }

    /// 초성만 있는 한글 (ㄱ): cho=2, jung=2(FILL), jong=1(FILL).
    #[test]
    fn test_hangul_choseong_only() {
        let c: u16 = 0x8000 | (2 << 10) | (2 << 5) | 1;
        assert_eq!(hnc_to_unicode(c), "\u{3131}");
    }

    /// 중성만 있는 한글 (ㅏ): cho=1(FILL), jung=3(ㅏ), jong=1(FILL).
    #[test]
    fn test_hangul_jungseong_only() {
        let c: u16 = 0x8000 | (1 << 10) | (3 << 5) | 1;
        assert_eq!(hnc_to_unicode(c), "\u{314f}");
    }

    /// 종성만 있는 한글: cho=1(FILL), jung=2(FILL), jong=2(ㄱ).
    #[test]
    fn test_hangul_jongseong_only() {
        let c: u16 = 0x8000 | (1 << 10) | (2 << 5) | 2;
        assert_eq!(hnc_to_unicode(c), "\u{3131}");
    }

    /// 옛한글: 초성+중성만. cho=21, jung=3, jong=1(FILL).
    #[test]
    fn test_old_hangul_cho_jung() {
        let c: u16 = 0x8000 | (21 << 10) | (3 << 5) | 1;
        let result = hnc_to_unicode(c);
        assert!(result.chars().count() >= 1); // 옛한글 자모 조합
    }

    /// 옛한글: 초성+중성+종성 모두. cho=21, jung=3, jong=18(옛종성).
    #[test]
    fn test_old_hangul_cho_jung_jong() {
        let c: u16 = 0x8000 | (21 << 10) | (3 << 5) | 18;
        let result = hnc_to_unicode(c);
        assert!(result.chars().count() >= 1);
    }

    /// jung=0인 완성형 옛한글 fallback. 원본 단언은 `str` 타입뿐이다.
    #[test]
    fn test_hangul_jung_zero_fallback() {
        let c: u16 = 0x8000 | (2 << 10);
        let _result: String = hnc_to_unicode(c);
    }

    /// 범위 밖 코드는 빈 문자열.
    #[test]
    fn test_out_of_range_code() {
        assert_eq!(hnc_to_unicode(0x0010), "");
    }
}

// ---- TestHandleControlCharCoverage ----

mod test_handle_control_char_coverage {
    use super::*;

    /// 제어문자가 포함된 최소 문단 바이너리를 생성한다.
    fn make_ctrl_paragraph(ctrl_code: u16, extra_data: &[u8]) -> Vec<u8> {
        let mut n_chars: u16 = match ctrl_code {
            23 => 4,
            24 | 25 => 2,
            28 => 31,
            30 | 31 => 1,
            _ => 3,
        };
        n_chars += 1; // 제어문자 자체 + CR
        let n_lines = 1;
        let char_shape = 0;

        let mut header = pack_bhhb(0, n_chars, n_lines, char_shape); // 6 bytes
        header.extend(zeros(37)); // padding to 43 bytes
        header.extend(zeros(187)); // para shape (prev_para_shape=0)
        let line_info = zeros(14); // 1 line
        // chars: ctrl_code + extra + CR(13)
        let chars = cat(&[&le16(ctrl_code), extra_data, &le16(13)]);
        // 빈 문단 (리스트 종료)
        let terminator = zeros(43);
        cat(&[&header, &line_info, &chars, &terminator])
    }

    /// 제어문자 5: 필드 코드. skip(6) + uint32(len=0) + skip(2) + skip(0).
    #[test]
    fn test_ctrl_5_field_code() {
        let extra = cat(&[&zeros(6), &le32(0), &zeros(2)]);
        let _result: String = parse_with_data(make_ctrl_paragraph(5, &extra));
    }

    /// 제어문자 6: 책갈피 (40바이트).
    #[test]
    fn test_ctrl_6_bookmark() {
        let _result: String = parse_with_data(make_ctrl_paragraph(6, &zeros(40)));
    }

    /// 제어문자 23: 글자겹침 (8바이트).
    #[test]
    fn test_ctrl_23_compose() {
        let _result: String = parse_with_data(make_ctrl_paragraph(23, &zeros(8)));
    }

    /// 제어문자 24: 하이픈 (4바이트).
    #[test]
    fn test_ctrl_24_hyphen() {
        let _result: String = parse_with_data(make_ctrl_paragraph(24, &zeros(4)));
    }

    /// 제어문자 28: 개요번호 (62바이트).
    #[test]
    fn test_ctrl_28_outline() {
        let _result: String = parse_with_data(make_ctrl_paragraph(28, &zeros(62)));
    }

    /// 제어문자 30: 묶음빈칸 (2바이트).
    #[test]
    fn test_ctrl_30_keep_space() {
        let _result: String = parse_with_data(make_ctrl_paragraph(30, &zeros(2)));
    }

    /// 제어문자 18: 자동번호 (6바이트).
    #[test]
    fn test_ctrl_18_auto_num() {
        let _result: String = parse_with_data(make_ctrl_paragraph(18, &zeros(6)));
    }
}

// ---- TestParseEdgeCases ----

mod test_parse_edge_cases {
    use super::*;

    /// n_chars가 비정상적으로 크면 예외(오류).
    #[test]
    fn test_sanity_check_invalid_n_chars() {
        // prev_para_shape=1, n_chars=50000(invalid), n_lines=1, csi=0
        let header = cat(&[&pack_bhhb(1, 50000, 1, 0), &zeros(37)]);
        let mut stream = HwpStream::new(cat(&[&header, &zeros(200)]));
        assert!(parse_paragraph(&mut stream, 0).is_err());
    }

    /// body_offset이 파일 크기를 초과하는 경우.
    #[test]
    fn test_body_offset_exceeds_file() {
        let mut doc_info = zeros(128);
        // info_block_len = 60000 (파일 크기 초과)
        doc_info[126..128].copy_from_slice(&60000u16.to_le_bytes());
        let data = cat(&[&sig("V3.00"), &doc_info, &zeros(1008)]);
        assert_eq!(extract_bytes("big_info_block.hwp", &data), "");
    }

    /// raw deflate 실패 후 gzip/zlib 자동 판별 시도. 파싱은 빈 결과지만 크래시하지 않아야 한다.
    #[test]
    fn test_gzip_fallback_decompression() {
        let mut doc_info = zeros(128);
        doc_info[124] = 1; // compressed
        // zlib.compress(b"\x00" * 100): zlib 헤더 포함
        let body: [u8; 12] = [0x78, 0x9c, 0x63, 0x60, 0xa0, 0x3d, 0x00, 0x00, 0x00, 0x64, 0x00, 0x01];
        let data = cat(&[&sig("V3.00"), &doc_info, &zeros(1008), &body]);
        let _result: String = extract_bytes("gzip_test.hwp", &data);
    }

    /// V1.20 시그니처는 빈 문자열.
    #[test]
    fn test_unsupported_version_v1() {
        let data = cat(&[&sig("V1.20"), &zeros(200)]);
        assert_eq!(extract_bytes("v1.hwp", &data), "");
    }

    /// 시그니처 영역이 깨진 바이너리. 첫 17바이트는 맞지만 나머지가 깨진 경우.
    #[test]
    fn test_header_decode_exception() {
        let data = cat(&[b"HWP Document File", &[0xffu8; 13], &zeros(200)]);
        assert_eq!(extract_bytes("bad_header.hwp", &data), "");
    }
}

// ---- TestHandleControlCharRecursive ----

mod test_handle_control_char_recursive {
    use super::*;

    /// 비압축 V3.00 HWP 파일 바이너리를 생성 (fonts=0, styles=0).
    fn build_hwp_with_paragraphs(body_data: &[u8]) -> Vec<u8> {
        let doc_info = zeros(128); // 비압축, 정보블록=0
        let summary = zeros(1008);
        let fonts = zeros(14); // 7 카테고리, 각 0개
        let styles = zeros(2); // styles: 0개
        cat(&[&sig("V3.00"), &doc_info, &summary, &fonts, &styles, body_data])
    }

    /// 문단 헤더 + 줄 정보 + 글자 데이터.
    fn make_paragraph(chars_data: &[u8], n_chars: u16, prev_shape: u8) -> Vec<u8> {
        let n_lines = 1;
        let mut header = cat(&[&pack_bhhb(prev_shape, n_chars, n_lines, 0), &zeros(37)]);
        if prev_shape == 0 {
            header.extend(zeros(187)); // 문단 모양
        }
        let line_info = zeros(14);
        cat(&[&header, &line_info, chars_data])
    }

    /// 제어문자 11: 그림 (skip(6)+uint32+skip(344)+skip(len)+캡션).
    #[test]
    fn test_ctrl_11_picture() {
        let pic_len: u32 = 10;
        // 제어문자 11 (n_chars_read += 3 -> 총 4)
        let ctrl = le16(11);
        let ctrl_data = cat(&[&zeros(6), &le32(pic_len), &zeros(344), &zeros(pic_len as usize)]);
        // 캡션 = 빈 문단 리스트
        let caption = empty_paragraph();
        let cr = le16(13);
        let chars = cat(&[&ctrl, &ctrl_data, &caption, &cr]);
        let para = make_paragraph(&chars, 4 + 1, 0);
        let body = cat(&[&para, &empty_paragraph()]);
        let data = build_hwp_with_paragraphs(&body);
        let _result: String = extract_bytes("picture.hwp", &data);
    }

    /// 제어문자 16: 머리말/꼬리말.
    #[test]
    fn test_ctrl_16_header_footer() {
        let ctrl = le16(16);
        let ctrl_data = zeros(16); // skip(6+10)
        let nested_end = empty_paragraph();
        let cr = le16(13);
        let chars = cat(&[&ctrl, &ctrl_data, &nested_end, &cr]);
        let para = make_paragraph(&chars, 4 + 1, 0);
        let body = cat(&[&para, &empty_paragraph()]);
        let data = build_hwp_with_paragraphs(&body);
        let _result: String = extract_bytes("header_footer.hwp", &data);
    }

    /// 제어문자 17: 각주/미주.
    #[test]
    fn test_ctrl_17_footnote() {
        let ctrl = le16(17);
        let ctrl_data = zeros(20); // skip(6+14)
        // 각주 내 텍스트: '가' + CR + 빈 문단
        let ga = le16(0x8861); // '가'
        let cr = le16(13);
        let footnote_para = make_paragraph(&cat(&[&ga, &cr]), 2, 0);
        let footnote_end = empty_paragraph();
        let outer_cr = le16(13);
        let chars = cat(&[&ctrl, &ctrl_data, &footnote_para, &footnote_end, &outer_cr]);
        let para = make_paragraph(&chars, 4 + 1, 0);
        let body = cat(&[&para, &empty_paragraph()]);
        let data = build_hwp_with_paragraphs(&body);
        let _result: String = extract_bytes("footnote.hwp", &data);
    }

    /// 문단 파싱 중 스트림 끝 도달 시 부분 텍스트 반환.
    #[test]
    fn test_paragraph_list_stream_error_partial() {
        // 유효한 첫 문단 (CR만) + 잘린 두 번째 문단 헤더
        let cr = le16(13);
        let para1 = make_paragraph(&cr, 1, 0);
        let truncated = pack_bhhb(0, 100, 1, 0); // 불완전한 헤더
        let result = parse_with_data(cat(&[&para1, &truncated]));
        assert!(result.contains('\n')); // 첫 문단의 CR은 포함
    }

    /// Python `test_recursion_depth_limit`: depth 10 초과면 즉시 빈 문자열이다.
    #[test]
    fn test_recursion_depth_limit() {
        assert_eq!(parse_paragraph_list(&mut HwpStream::new(vec![0; 100]), 11), "");
    }

    /// `_MAX_TEXT_LENGTH` 초과 시 파싱 중단.
    #[test]
    fn test_max_text_length_break() {
        // '가' 200개 + CR
        let ga = le16(0x8861);
        let cr = le16(13);
        let chars = cat(&[&ga.repeat(200), &cr]);
        let mut paragraphs: Vec<u8> = Vec::new();
        // 충분히 많은 문단
        for _ in 0..(MAX_TEXT_LENGTH / 200) + 10 {
            paragraphs.extend(make_paragraph(&chars, 201, 1));
        }
        paragraphs.extend(empty_paragraph());
        let result = parse_with_data(paragraphs);
        assert!(result.chars().count() <= MAX_TEXT_LENGTH + 500); // 약간의 여유
    }
}

// ---- TestSafeChr ----

mod test_safe_chr {
    use super::*;

    #[test]
    fn test_surrogate_high() {
        assert_eq!(safe_chr(0xD800), "");
        assert_eq!(safe_chr(0xD830), "");
    }
}

// ---- TestControlCharTab ----

mod test_control_char_tab {
    use super::*;

    /// 탭(9) 제어문자가 포함된 문단.
    #[test]
    fn test_ctrl_9_tab_in_paragraph() {
        // 탭(9) + 6바이트 데이터 -> n_chars_read += 3+1=4
        let tab = cat(&[&le16(9), &zeros(6)]);
        let ga = le16(0x8861); // '가'
        let cr = le16(13);
        let chars = cat(&[&ga, &tab, &ga, &cr]); // 가\t가\n, n_chars=1+4+1+1=7
        // 문단 헤더
        let header = cat(&[&pack_bhhb(0, 7, 1, 0), &zeros(37), &zeros(187)]); // para shape 포함
        let line_info = zeros(14);
        let terminator = empty_paragraph();
        let data = cat(&[&header, &line_info, &chars, &terminator]);
        let result = parse_with_data(data);
        assert!(result.contains('\t'));
        assert!(result.contains("가"));
    }
}

// ---- TestControlCharDateLine ----

mod test_control_char_date_line {
    use super::*;

    fn build_ctrl_para(ctrl_code: u16, skip_bytes: usize, n_read_add: u16, extra_nested: bool) -> String {
        let n_chars = 1 + n_read_add + 1; // ctrl + n_read_add + CR
        let mut ctrl = cat(&[&le16(ctrl_code), &zeros(skip_bytes)]);
        if extra_nested {
            // 빈 문단 리스트 (종료)
            ctrl.extend(empty_paragraph());
        }
        let cr = le16(13);
        let chars = cat(&[&ctrl, &cr]);
        let header = cat(&[&pack_bhhb(0, n_chars, 1, 0), &zeros(37), &zeros(187)]);
        let line_info = zeros(14);
        let terminator = empty_paragraph();
        parse_with_data(cat(&[&header, &line_info, &chars, &terminator]))
    }

    #[test]
    fn test_ctrl_7_date_format() {
        let _result: String = build_ctrl_para(7, 84, 3, false);
    }

    #[test]
    fn test_ctrl_8_date_code() {
        let _result: String = build_ctrl_para(8, 96, 3, false);
    }

    #[test]
    fn test_ctrl_14_line() {
        let _result: String = build_ctrl_para(14, 92, 3, false);
    }

    #[test]
    fn test_ctrl_15_hidden_comment() {
        let _result: String = build_ctrl_para(15, 16, 3, true);
    }
}

// ---- TestV200BruteForce ----

mod test_v200_brute_force {
    use super::*;
    use hwp3_parser::extract_text_bruteforce;

    /// V2.00 파일에서 텍스트가 추출되는지 확인.
    #[test]
    fn test_v200_extracts_text() {
        let f = hwp_fixture("v2.00_박노해_참된시작.hwp");
        let text = extract(&f);
        assert!(text.chars().count() > 100);
    }

    /// V2.00 추출 결과가 UTF-8 안전한지 확인(`text.encode("utf-8")`가 예외를 던지지 않아야 한다).
    #[test]
    fn test_v200_utf8_safe() {
        let f = hwp_fixture("v2.00_박노해_참된시작.hwp");
        let text = extract(&f);
        std::str::from_utf8(text.as_bytes()).unwrap();
    }

    /// 쓰레기 비율이 높으면 빈 문자열 반환. 전부 특수문자 범위(0x3F00+i)의 HNC 코드다.
    #[test]
    fn test_bruteforce_garbage_ratio_filter() {
        let garbage: Vec<u8> = (0..100u16).flat_map(|i| (0x3F00 + i).to_le_bytes()).collect();
        let result = extract_text_bruteforce(&garbage);
        assert_eq!(result, "");
    }

    /// 유효한 텍스트가 포함된 데이터에서 추출. '가나다라마바사아자차' (한글 10자) + CR.
    #[test]
    fn test_bruteforce_with_valid_text() {
        let mut data = pack_codes(&TEN_HANGUL);
        data.extend(le16(13)); // CR
        let result = extract_text_bruteforce(&data);
        assert!(!result.is_empty());
    }

    /// V2.00에서 글꼴 영역 skip 실패 시 전체 brute-force. 원본 단언은 `str` 타입뿐이다.
    #[test]
    fn test_v200_font_skip_failure_fallback() {
        // 글꼴 영역이 깨진 데이터 (첫 font count가 비정상): n_fonts = 60000 -> skip 실패
        let broken_fonts = le16(60000);
        // 그 뒤에 유효한 텍스트
        let mut text_data = pack_codes(&TEN_HANGUL);
        text_data.extend(le16(13));
        let body = cat(&[&broken_fonts, &zeros(50), &text_data]);
        let data = cat(&[&sig("V2.00"), &zeros(128), &zeros(1008), &body]);
        let _text: String = extract_bytes("v200_broken.hwp", &data);
    }
}
