//! HwpStream·문단·제어 문자 파싱 테스트 (`paragraph_parser` 모듈).
mod common;

use common::shared_fixtures::{codes, le16, pack};
use hwp3_parser::*;

// ---- HwpStream ----

#[test]
fn stream_reads_little_endian_values_and_tracks_position() {
    let mut s = HwpStream::new(vec![1, 2, 3]);
    assert_eq!(s.remaining(), 3);
    assert_eq!(s.read_uint8().unwrap(), 1);
    assert_eq!(s.remaining(), 2);
    assert_eq!(HwpStream::new(vec![0xff]).read_uint8().unwrap(), 255);
    assert_eq!(HwpStream::new(le16(0x1234)).read_uint16().unwrap(), 0x1234);
    assert_eq!(HwpStream::new(0xDEADBEEFu32.to_le_bytes().to_vec()).read_uint32().unwrap(), 0xDEADBEEF);
    let mut s = HwpStream::new(vec![0; 10]);
    s.skip(5).unwrap();
    assert_eq!(s.remaining(), 5);
    let mut s = HwpStream::new(vec![1, 2, 3]);
    assert_eq!(s.read_bytes(2).unwrap(), [1, 2]);
    assert_eq!(s.remaining(), 1);
    assert_eq!(s.position(), 2);
}

#[test]
fn stream_errors_on_underflow_negative_skip_and_overflow() {
    assert!(HwpStream::new(vec![]).read_uint8().is_err());
    assert!(HwpStream::new(vec![1]).read_uint16().is_err());
    assert!(HwpStream::new(vec![1, 2, 3]).read_uint32().is_err());
    assert!(HwpStream::new(vec![0; 5]).skip(-1).is_err());
    assert!(HwpStream::new(vec![0; 3]).skip(10).is_err());
    assert!(HwpStream::new(vec![1, 2]).read_bytes(5).is_err());
    assert_eq!(StreamError("x".into()).to_string(), "x");
}

#[test]
fn stream_skip_of_i64_max_returns_error_without_overflowing() {
    let mut stream = HwpStream::new(vec![0]);
    stream.read_uint8().unwrap();
    assert!(stream.skip(i64::MAX).is_err());
}

#[test]
fn stream_read_bytes_of_usize_max_returns_error_without_overflowing() {
    let mut stream = HwpStream::new(vec![0; 2]);
    stream.read_uint8().unwrap();
    assert!(stream.read_bytes(usize::MAX).is_err());
}

// ---- parse_paragraph ----

fn para_stream(prev: u8, n_chars: u16, n_lines: u16, csi: u8, chars: &[u16], prefix: &[u8]) -> HwpStream {
    let mut data = prefix.to_vec();
    data.push(prev);
    data.extend(le16(n_chars));
    data.extend(le16(n_lines));
    data.push(csi);
    data.extend(vec![0u8; 37]);
    if prev == 0 && n_chars > 0 {
        data.extend(vec![0u8; 187]);
    }
    data.extend(vec![0u8; n_lines as usize * 14]);
    if csi != 0 {
        data.extend(vec![1u8; n_chars as usize]);
    }
    data.extend(pack(chars));
    HwpStream::new(data)
}

fn raw_para(n_chars: u16, n_lines: u16) -> Vec<u8> {
    let mut d = vec![1u8];
    d.extend(le16(n_chars));
    d.extend(le16(n_lines));
    d.push(0);
    d.extend(vec![0u8; 37]);
    d
}

#[test]
fn parse_paragraph_basics_and_limits() {
    assert_eq!(parse_paragraph(&mut para_stream(1, 0, 0, 0, &[], b""), 0).unwrap(), None);
    assert!(parse_paragraph(&mut HwpStream::new(raw_para(30001, 0)), 0).is_err());
    assert!(parse_paragraph(&mut HwpStream::new(raw_para(1, 5001)), 0).is_err());
    assert_eq!(parse_paragraph(&mut para_stream(1, 5, 1, 0, &codes("Hello"), b""), 0).unwrap().as_deref(), Some("Hello"));
}

/// 제어 데이터: `before`바이트, 끝 표식(제어 코드), `after`바이트.
fn ctl(c: u16, before: usize, after: usize) -> Vec<u8> {
    let mut d = vec![0u8; before];
    d.extend(le16(c));
    d.extend(vec![0u8; after]);
    d
}

#[test]
fn control_characters_advance_the_character_counter_by_wsize() {
    // 문단 글자 수가 제어 문자 하나의 칸 수(WSIZE)와 같으면 그 제어 문자만 읽고 끝난다. 뒤의 표지 바이트는 남는다.
    let cases: [(u16, Vec<u8>); 7] = [(9, ctl(9, 4, 0)), (23, ctl(23, 6, 0)), (28, ctl(28, 60, 0)), (30, ctl(30, 0, 0)), (24, ctl(24, 2, 0)), (7, ctl(7, 80, 0)), (26, ctl(26, 242, 0))];
    for (c, payload) in cases {
        let mut data = raw_para(WSIZE[usize::from(c)] as u16, 1);
        data.extend(vec![0u8; 14]);
        data.extend(le16(c));
        data.extend(payload);
        data.extend([0xAA, 0xBB]);
        let mut s = HwpStream::new(data);
        assert!(parse_paragraph(&mut s, 0).unwrap().is_some(), "{c}");
        assert_eq!(s.remaining(), 2, "{c}");
    }
    assert_eq!((WSIZE[7], WSIZE[8], WSIZE[26], WSIZE[13]), (42, 48, 123, 1));
}

#[test]
fn paragraph_end_stops_reading_even_if_the_count_says_more() {
    // 글자 수는 10이지만 문단 끝(13)에서 멈추고 뒤 바이트를 읽지 않는다.
    let mut data = raw_para(10, 0);
    let mut chars = codes("AB");
    chars.push(13);
    data.extend(pack(&chars));
    data.extend([0xAA, 0xBB]);
    let mut s = HwpStream::new(data);
    assert_eq!(parse_paragraph(&mut s, 0).unwrap().unwrap(), "AB\n");
    assert_eq!(s.remaining(), 2);
}

#[test]
fn char_shape_marker_other_than_zero_has_no_shape_body() {
    // 표식 2는 0이 아니므로 글자 모양 본체가 없다(옛 코드는 1이 아니면 31바이트를 건너뛰었다).
    let mut csi = vec![1u8];
    csi.extend(le16(2));
    csi.extend(le16(0));
    csi.push(1);
    csi.extend(vec![0u8; 37]);
    csi.extend([2u8, 0]);
    csi.extend(vec![0u8; 31]);
    csi.extend(pack(&codes("OK")));
    assert_eq!(parse_paragraph(&mut HwpStream::new(csi), 0).unwrap().unwrap(), "OK");
}

#[test]
fn parse_paragraph_list_limits() {
    assert_eq!(parse_paragraph_list(&mut HwpStream::new(vec![0; 100]), 11), "");
    assert_eq!(parse_paragraph_list(&mut HwpStream::new(vec![0; 10]), 0), "");
    assert_eq!(parse_paragraph_list(&mut para_stream(1, 3, 1, 0, &codes("ABC"), b""), 0), "ABC");
    let mut empty = raw_para(0, 0);
    assert_eq!(empty.len(), 43);
    assert_eq!(parse_paragraph_list(&mut HwpStream::new(std::mem::take(&mut empty)), 0), "");
}

#[test]
fn parse_paragraph_list_stops_at_the_text_length_cap() {
    // 문단 하나는 3만 자까지라서, 같은 문단을 34번 이어 붙여 100만 자를 넘긴다.
    let one = {
        let mut p = vec![1u8];
        p.extend(le16(30000));
        p.extend(le16(0));
        p.push(0);
        p.extend(vec![0u8; 37]);
        p.extend(pack(&vec!['a' as u16; 30000]));
        p
    };
    let data: Vec<u8> = one.iter().cycle().take(one.len() * 40).copied().collect();
    let text = parse_paragraph_list(&mut HwpStream::new(data), 0);
    assert!(text.chars().count() > 1_000_000 && text.chars().count() < 40 * 30000);
}

// ---- handle_control_char ----

fn handle(c: u16, data: Vec<u8>) -> (Result<(), StreamError>, Vec<String>) {
    let mut parts = Vec::new();
    let r = handle_control_char(c, &mut HwpStream::new(data), &mut parts, 0);
    (r, parts)
}

#[test]
fn simple_control_characters() {
    assert_eq!(handle(13, vec![]).1, ["\n"]);
    assert_eq!(handle(9, ctl(9, 4, 0)).1, ["\t"]);
    // 필드 코드: 크기·표식·종류 36바이트 + 길이 4개 + 데이터
    let mut field = vec![0u8; 36];
    for len in [2u32, 0, 0, 1] {
        field.extend(len.to_le_bytes());
    }
    field.extend([0u8; 3]);
    let mut s = HwpStream::new(field);
    assert!(handle_control_char(5, &mut s, &mut Vec::new(), 0).is_ok());
    assert_eq!(s.remaining(), 0);
    for (c, data) in [(6, ctl(6, 4, 34)), (7, ctl(7, 80, 0)), (14, ctl(14, 4, 84)), (23, ctl(23, 6, 0)), (28, ctl(28, 60, 0)), (22, ctl(22, 20, 0)), (26, ctl(26, 242, 0))] {
        let n = data.len();
        let mut s = HwpStream::new([data, vec![0xAA]].concat());
        assert!(handle_control_char(c, &mut s, &mut Vec::new(), 0).is_ok(), "{c}");
        assert_eq!(s.remaining(), 1, "{c}: {n}바이트만 읽는다");
    }
    for c in [18, 19, 20, 21] {
        assert!(handle(c, ctl(c, 4, 0)).0.is_ok());
    }
    for c in [24, 25] {
        assert!(handle(c, ctl(c, 2, 0)).0.is_ok());
    }
    // 묶음·고정폭 빈칸은 공백이다.
    for c in [30, 31] {
        assert_eq!(handle(c, ctl(c, 0, 0)).1, [" "]);
    }
    // 길이를 앞에 둔 제어 문자: 길이(4) + 표식 + 데이터
    for c in [0, 1, 2, 3, 4, 12, 27, 29] {
        let mut d = 3u32.to_le_bytes().to_vec();
        d.extend(le16(c));
        d.extend([0u8; 3]);
        let (r, parts) = handle(c, d);
        assert!(r.is_ok() && parts.is_empty(), "{c}");
    }
    assert!(handle(9, vec![0; 3]).0.is_err());
}

#[test]
fn control_marker_mismatch_is_an_error() {
    // 끝 표식이 제어 코드와 다르면 스트림이 어긋난 것이다.
    let (r, parts) = handle(9, ctl(8, 4, 0));
    assert!(r.unwrap_err().to_string().contains("marker mismatch"));
    assert!(parts.is_empty());
}

#[test]
fn composed_characters_and_date_codes_are_written_as_text() {
    let mut compose = Vec::new();
    for c in [u16::from(b'A'), u16::from(b'B'), 0] {
        compose.extend(le16(c));
    }
    compose.extend(le16(23));
    assert_eq!(handle(23, compose).1, ["A", "B"]);
    let mut date = vec![0u8; 80];
    for v in [1996u16, 3, 1, 15, 9, 30] {
        date.extend(le16(v));
    }
    date.extend(le16(8));
    assert_eq!(handle(8, date).1, ["1996년 3월 15일"]);
    // 연도가 0이면 아무것도 쓰지 않는다.
    assert!(handle(8, ctl(8, 92, 0)).1.is_empty());
}

#[test]
fn container_control_characters_read_nested_paragraph_lists() {
    assert!(handle(17, ctl(17, 4, 14)).1.is_empty());
    let mut picture = ctl(11, 4, 0);
    picture.extend(0u32.to_le_bytes());
    picture.extend(vec![0u8; 344]);
    assert!(handle(11, picture).0.is_ok());
    let mut table = ctl(10, 4, 80);
    table.extend(le16(0));
    table.extend(vec![0u8; 2]);
    assert!(handle(10, table).0.is_ok());
    for (c, after) in [(15, 8), (16, 10)] {
        assert!(handle(c, ctl(c, 4, after)).0.is_ok(), "{c}");
    }

    // 표: 셀 1개에 문단 하나, 각주에 문단 하나가 있으면 본문에 이어 붙는다.
    let cell_text = para_stream(1, 3, 1, 0, &codes("Tbl"), b"");
    let mut data = ctl(10, 4, 80);
    data.extend(le16(1));
    data.extend(vec![0u8; 2 + 27]);
    data.extend(raw_stream_bytes(&cell_text));
    let (r, parts) = handle(10, data);
    assert!(r.is_ok());
    assert_eq!(parts, ["Tbl"]);
    let mut note = ctl(17, 4, 14);
    note.extend(raw_stream_bytes(&para_stream(1, 4, 1, 0, &codes("Note"), b"")));
    assert_eq!(handle(17, note).1, ["Note"]);
}

fn raw_stream_bytes(s: &HwpStream) -> Vec<u8> {
    s.bytes().to_vec()
}
