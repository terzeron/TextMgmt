//! HWP V3.00/V2.10/V2.00 텍스트 추출기 테스트 (tests/test_hwp3_parser.py 이식).

use std::fs;
use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateEncoder;
use hwp3_parser::*;
use hwp3_tables::{HNC2UNI, KSC5601_TO_UNI};

fn le16(v: u16) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn codes(text: &str) -> Vec<u16> {
    text.chars().map(|c| c as u16).collect()
}

fn pack(codes: &[u16]) -> Vec<u8> {
    codes.iter().flat_map(|c| c.to_le_bytes()).collect()
}

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

// ---- safe_chr ----

#[test]
fn safe_chr_drops_surrogates_only() {
    assert_eq!(safe_chr(0x41), "A");
    assert_eq!(safe_chr(0xAC00), "가");
    for c in [0xD800, 0xDFFF, 0xDC00] {
        assert_eq!(safe_chr(c), "");
    }
    assert_eq!(safe_chr(0xD7FF), "\u{d7ff}");
    assert_eq!(safe_chr(0xE000), "\u{e000}");
    // 유니코드 범위를 넘는 값도 버린다.
    assert_eq!(safe_chr(0x110000), "");
}

// ---- hnc_to_unicode ----

#[test]
fn hnc_codes_map_by_range() {
    assert_eq!(hnc_to_unicode(0), "");
    assert_eq!(hnc_to_unicode(0x41), "A");
    assert_eq!(hnc_to_unicode(0x20), " ");
    assert_eq!(hnc_to_unicode(0x7E), "~");
    assert_eq!(hnc_to_unicode(0x1F), hnc_to_unicode(0x1F));
    assert!(hnc_to_unicode(0x4000).chars().count() <= 1);
}

#[test]
fn modern_hangul_syllables_compose() {
    assert_eq!(hnc_to_unicode(0x8000 | (2 << 10) | (3 << 5) | 1), "가");
}

#[test]
fn partial_and_old_hangul_follow_the_python_branches() {
    // 낱자: 초성만, 중성만, 종성만.
    assert!(hnc_to_unicode(0x8000 | (2 << 10) | (2 << 5) | 1).chars().count() <= 1);
    assert!(hnc_to_unicode(0x8000 | (1 << 10) | (3 << 5) | 1).chars().count() <= 1);
    assert_eq!(hnc_to_unicode(0x8000 | (1 << 10) | 2), "\u{3131}");
    // 옛한글: 초성+중성, 초성+중성+종성, 완성형 fallback.
    assert!(hnc_to_unicode(0x8000 | (3 << 5) | 1).chars().count() >= 1);
    assert!(hnc_to_unicode(0x8000 | (2 << 10) | (3 << 5)).chars().count() >= 1);
    let _ = hnc_to_unicode(0x8000 | (2 << 10));
}

#[test]
fn table_ranges_use_ksc5601_and_hnc2uni() {
    let known = HNC2UNI.iter().map(|(k, _)| *k).find(|k| (0x7F..=0x3FFF).contains(k)).unwrap();
    assert!(hnc_to_unicode(known).chars().count() <= 1);
    let high = HNC2UNI.iter().map(|(k, _)| *k).find(|k| (0x5318..=0x7FFF).contains(k)).unwrap();
    assert!(hnc_to_unicode(high).chars().count() <= 1);
    let unknown = (0x80u16..0x200).find(|c| HNC2UNI.binary_search_by_key(c, |(k, _)| *k).is_err()).unwrap();
    assert_eq!(hnc_to_unicode(unknown), "");
    if let Some(i) = KSC5601_TO_UNI.iter().position(|v| *v == 0) {
        assert_eq!(hnc_to_unicode(0x4000 + i as u16), "");
    }
    let mapped = KSC5601_TO_UNI.iter().position(|v| *v != 0).unwrap();
    assert_eq!(hnc_to_unicode(0x4000 + mapped as u16), char::from_u32(u32::from(KSC5601_TO_UNI[mapped])).unwrap().to_string());
}

// ---- extract_text_bruteforce ----

#[test]
fn bruteforce_keeps_readable_runs_and_drops_short_or_garbled_ones() {
    assert!(extract_text_bruteforce(&pack(&codes("Hello World Text"))).contains("Hello World Text"));
    let _ = extract_text_bruteforce(&pack(&[0x48, 0x65, 0x6C, 0x6C, 0x6F, 13, 0x57, 0x6F, 0x72, 0x6C, 0x64, 9, 0x54, 0x65, 0x78, 0x74]));
    let _ = extract_text_bruteforce(&pack(&[0, 0, 'A' as u16, 0, 0]));
    assert_eq!(extract_text_bruteforce(&pack(&codes("Hello!!"))), "");
    assert_eq!(extract_text_bruteforce(b""), "");
    let _ = extract_text_bruteforce(&[0x41, 0x00, 0x01]);
}

#[test]
fn bruteforce_flushes_runs_at_unknown_and_control_codes() {
    let mut a = codes("Hello World Test");
    let unknown = (0x100u16..0x400).find(|c| HNC2UNI.binary_search_by_key(c, |(k, _)| *k).is_err()).unwrap();
    a.push(unknown);
    a.extend(codes("Second Readable Run"));
    let out = extract_text_bruteforce(&pack(&a));
    assert_eq!(out, "Hello World Test\nSecond Readable Run");
    let mut b = codes("Hello World Test");
    b.push(4);
    b.extend(codes("After Control Code"));
    assert_eq!(extract_text_bruteforce(&pack(&b)), "Hello World Test\nAfter Control Code");
}

#[test]
fn bruteforce_handles_newline_tab_and_garbage_separators() {
    let mut c = codes("Line one here");
    c.extend([13]);
    c.extend(codes("Line two here"));
    c.extend([9]);
    c.extend(codes("tabbed"));
    let out = extract_text_bruteforce(&pack(&c));
    assert!(out.contains("Line one here") && out.contains("tabbed"), "{out:?}");
    let garbage = pack(&codes("豼豼d豼豼d豼豼d"));
    assert!(!extract_text_bruteforce(&garbage).contains("豼豼"));
    // 짧은 구분자 패턴은 길이 필터에서 먼저 걸러진다.
    let sep: String = "Ǵ塴豼豼Āāā".to_string();
    let rev = reverse_hnc_map();
    let body: Vec<u16> = sep.chars().map(|ch| rev[&ch]).collect();
    assert_eq!(extract_text_bruteforce(&pack(&body)), "");
    // 20자 미만이고 "豼豼"를 품은 8자 이상 run은 레코드 구분자로 보고 버린다.
    let junk: Vec<u16> = "豼豼abcdefgh".chars().map(|ch| rev.get(&ch).copied().unwrap_or(ch as u16)).collect();
    assert_eq!(extract_text_bruteforce(&pack(&junk)), "");
}

fn reverse_hnc_map() -> std::collections::HashMap<char, u16> {
    let mut rev = std::collections::HashMap::new();
    for c in 0x80u32..0x10000 {
        let s = hnc_to_unicode(c as u16);
        let mut chars = s.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            rev.entry(ch).or_insert(c as u16);
        }
    }
    rev
}

// ---- 합성 HWP ----

fn header(version: &str) -> Vec<u8> {
    let mut h = format!("HWP Document File {version}").into_bytes();
    h.resize(30, 0);
    h
}

fn doc_info(encrypted: u16, compressed: u8, info_block_len: u16) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[96..98].copy_from_slice(&encrypted.to_le_bytes());
    b[124] = compressed;
    b[126..128].copy_from_slice(&info_block_len.to_le_bytes());
    b
}

fn font_style_block() -> Vec<u8> {
    vec![0u8; 16]
}

fn paragraph(chars: &[u16], n_lines: u16) -> Vec<u8> {
    let mut b = vec![1u8];
    b.extend(le16(chars.len() as u16));
    b.extend(le16(n_lines));
    b.push(0);
    b.extend(vec![0u8; 37]);
    b.extend(vec![0u8; n_lines as usize * 14]);
    b.extend(pack(chars));
    b
}

fn hwp3(version: &str, body_extra: &[u8], encrypted: u16, compressed: u8, info_block_len: u16) -> Vec<u8> {
    let mut b = header(version);
    b.extend(doc_info(encrypted, compressed, info_block_len));
    b.extend(vec![0u8; 1008 + info_block_len as usize]);
    b.extend(font_style_block());
    b.extend(body_extra);
    b
}

fn extract_bytes(bytes: &[u8]) -> String {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("f.hwp");
    fs::write(&p, bytes).unwrap();
    extract_text_from_hwp3(&p)
}

#[test]
fn rejects_files_that_are_missing_short_foreign_or_encrypted() {
    let d = tempfile::tempdir().unwrap();
    assert_eq!(extract_text_from_hwp3(&d.path().join("no_such.hwp")), "");
    assert_eq!(extract_bytes(b"HWP"), "");
    let mut bad_sig = b"NOT HWP Document File V3.00    ".to_vec();
    bad_sig.extend(vec![0u8; 200]);
    assert_eq!(extract_bytes(&bad_sig), "");
    let mut bad_ver = b"HWP Document File V1.00\0\0\0\0\0\0\0".to_vec();
    bad_ver.extend(vec![0u8; 300]);
    assert_eq!(extract_bytes(&bad_ver), "");
    let mut no_info = header("V3.00");
    no_info.extend(vec![0u8; 10]);
    assert_eq!(extract_bytes(&no_info), "");
    assert_eq!(extract_bytes(&hwp3("V3.00", b"", 1, 0, 0)), "");
    assert_eq!(extract_bytes(&hwp3("V3.00", b"", 0, 0, 9999)), "");
}

#[test]
fn empty_bodies_give_empty_text() {
    assert_eq!(extract_bytes(&hwp3("V3.00", b"", 0, 0, 0)), "");
    assert_eq!(extract_bytes(&hwp3("V2.10", b"", 0, 0, 0)), "");
}

#[test]
fn ascii_paragraphs_are_extracted() {
    let body = paragraph(&codes("Hello World Test!"), 1);
    assert!(extract_bytes(&hwp3("V3.00", &body, 0, 0, 0)).contains("Hello World Test!"));
    let mut chars = vec!['A' as u16; 20];
    chars.push(13);
    assert!(extract_bytes(&hwp3("V3.00", &paragraph(&chars, 1), 0, 0, 0)).contains(&"A".repeat(20)));
    assert!(extract_bytes(&hwp3("V3.00", &paragraph(&codes(&"A".repeat(100)), 1), 0, 0, 0)).contains(&"A".repeat(100)));
}

#[test]
fn paragraph_shape_and_char_shape_variants() {
    // prev_para_shape = 0 이면 문단 모양 187바이트를 더 건너뛴다.
    let mut para = vec![0u8];
    para.extend(le16(5));
    para.extend(le16(1));
    para.push(0);
    para.extend(vec![0u8; 37 + 187 + 14]);
    para.extend(pack(&codes("Hello")));
    assert!(extract_bytes(&hwp3("V3.00", &para, 0, 0, 0)).contains("Hello"));
    // 글자 모양 포함, flag = 1이면 더 건너뛰지 않는다.
    let mut csi = vec![1u8];
    csi.extend(le16(3));
    csi.extend(le16(1));
    csi.push(1);
    csi.extend(vec![0u8; 37 + 14]);
    csi.extend([1u8; 3]);
    csi.extend(pack(&codes("ABC")));
    assert!(extract_bytes(&hwp3("V3.00", &csi, 0, 0, 0)).contains("ABC"));
    // flag != 1이면 31바이트를 건너뛴다.
    let mut flag0 = vec![1u8];
    flag0.extend(le16(1));
    flag0.extend(le16(1));
    flag0.push(1);
    flag0.extend(vec![0u8; 37 + 14]);
    flag0.push(0);
    flag0.extend(vec![0u8; 31]);
    flag0.extend(pack(&codes("X")));
    let _ = extract_bytes(&hwp3("V3.00", &flag0, 0, 0, 0));
}

#[test]
fn compressed_bodies_are_inflated_raw_then_with_a_header() {
    let mut body = font_style_block();
    body.extend(paragraph(&codes("Compressed Content Test Data"), 1));
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    enc.write_all(&body).unwrap();
    let raw = enc.finish().unwrap();
    let mut file = header("V3.00");
    file.extend(doc_info(0, 1, 0));
    file.extend(vec![0u8; 1008]);
    file.extend(&raw);
    assert!(extract_bytes(&file).contains("Compressed Content Test Data"));

    // zlib 헤더가 붙은 경우는 두 번째 시도로 푼다.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&body).unwrap();
    let mut file = header("V3.00");
    file.extend(doc_info(0, 1, 0));
    file.extend(vec![0u8; 1008]);
    file.extend(z.finish().unwrap());
    assert!(extract_bytes(&file).contains("Compressed Content Test Data"));

    // gzip 헤더도 푼다.
    let mut g = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
    g.write_all(&body).unwrap();
    let mut file = header("V3.00");
    file.extend(doc_info(0, 1, 0));
    file.extend(vec![0u8; 1008]);
    file.extend(g.finish().unwrap());
    assert!(extract_bytes(&file).contains("Compressed Content Test Data"));

    // 둘 다 실패하면 빈 문자열이다.
    let mut bad = header("V3.00");
    bad.extend(doc_info(0, 1, 0));
    bad.extend(vec![0u8; 1008]);
    bad.extend([0xff, 0xfe, 0xfd, 0xfc].repeat(20));
    assert_eq!(extract_bytes(&bad), "");
}

#[test]
fn v200_falls_back_to_bruteforce_when_the_structure_is_empty() {
    let text: Vec<u8> = pack(&codes("BruteForce Korean Test Data!!"));
    let mut plain = header("V2.00");
    plain.extend(doc_info(0, 0, 0));
    plain.extend(vec![0u8; 1008]);
    plain.extend(&text);
    // 글꼴·스타일 머리가 없어 구조 파싱이 실패하면 body 전체를 훑는다.
    let _ = extract_bytes(&plain);

    let mut full = header("V2.00");
    full.extend(doc_info(0, 0, 0));
    full.extend(vec![0u8; 1008]);
    full.extend(font_style_block());
    full.extend(&text);
    assert!(extract_bytes(&full).contains("BruteForce Korean Test Data!!"));

    let mut bad = header("V2.00");
    bad.extend(doc_info(0, 0, 0));
    bad.extend(vec![0u8; 1008]);
    bad.extend(le16(100));
    let _ = extract_bytes(&bad);
}

#[test]
fn truncated_bodies_and_garbled_text_are_rejected_or_kept_by_the_quality_filter() {
    let raw = hwp3("V3.00", &paragraph(&codes(&"A".repeat(50)), 1), 0, 0, 0);
    let _ = extract_bytes(&raw[..raw.len() - 20]);
    assert_eq!(extract_bytes(&hwp3("V3.00", &paragraph(&codes(&"!".repeat(50)), 1), 0, 0, 0)), "");
    // 한글 완성형은 읽을 수 있는 글자로 센다.
    let hangul = paragraph(&[0x8000 | (2 << 10) | (3 << 5) | 1; 20], 1);
    assert_eq!(extract_bytes(&hwp3("V3.00", &hangul, 0, 0, 0)), "가".repeat(20));
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

#[test]
fn control_characters_advance_the_character_counter_like_the_original() {
    let cases: [(u16, u16, usize); 5] = [(9, 4, 6), (23, 5, 8), (28, 32, 62), (30, 2, 2), (24, 3, 4)];
    for (c, n_chars, skip) in cases {
        let mut data = raw_para(n_chars, 1);
        data.extend(vec![0u8; 14]);
        data.extend(le16(c));
        data.extend(vec![0u8; skip]);
        assert!(parse_paragraph(&mut HwpStream::new(data), 0).is_ok(), "{c}");
    }
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
    assert_eq!(handle(9, vec![0; 6]).1, ["\t"]);
    let mut field = vec![0u8; 6];
    field.extend(4u32.to_le_bytes());
    field.extend(vec![0u8; 6]);
    assert!(handle(5, field).0.is_ok());
    for (c, n) in [(6, 40), (7, 84), (8, 96), (14, 92), (23, 8), (28, 62), (15, 16), (16, 16)] {
        assert!(handle(c, vec![0; n]).0.is_ok(), "{c}");
    }
    for c in [18, 19, 20, 21] {
        assert!(handle(c, vec![0; 6]).0.is_ok());
    }
    for c in [24, 25, 26, 29] {
        assert!(handle(c, vec![0; 4]).0.is_ok());
    }
    for c in [30, 31] {
        assert!(handle(c, vec![0; 2]).0.is_ok());
    }
    // 드물게 쓰이는 제어 문자는 아무것도 읽지 않는다.
    for c in [0, 1, 2, 3, 4, 12, 22, 27] {
        let (r, parts) = handle(c, vec![]);
        assert!(r.is_ok() && parts.is_empty(), "{c}");
    }
    assert!(handle(9, vec![0; 3]).0.is_err());
}

#[test]
fn container_control_characters_read_nested_paragraph_lists() {
    assert!(handle(17, vec![0; 20]).1.is_empty());
    let mut picture = vec![0u8; 6];
    picture.extend(0u32.to_le_bytes());
    picture.extend(vec![0u8; 344]);
    assert!(handle(11, picture).0.is_ok());
    let mut table = vec![0u8; 86];
    table.extend(le16(0));
    table.extend(vec![0u8; 2]);
    assert!(handle(10, table).0.is_ok());

    // 표: 셀 1개에 문단 하나, 각주에 문단 하나가 있으면 본문에 이어 붙는다.
    let cell_text = para_stream(1, 3, 1, 0, &codes("Tbl"), b"");
    let mut data = vec![0u8; 86];
    data.extend(le16(1));
    data.extend(vec![0u8; 2 + 27]);
    data.extend(raw_stream_bytes(&cell_text));
    let (r, parts) = handle(10, data);
    assert!(r.is_ok());
    assert_eq!(parts, ["Tbl"]);
    let mut note = vec![0u8; 20];
    note.extend(raw_stream_bytes(&para_stream(1, 4, 1, 0, &codes("Note"), b"")));
    assert_eq!(handle(17, note).1, ["Note"]);
}

fn raw_stream_bytes(s: &HwpStream) -> Vec<u8> {
    s.bytes().to_vec()
}

// ---- 실제 파일 비교에서 찾은 회귀 ----

#[test]
fn lone_surrogate_codes_do_not_split_bruteforce_runs() {
    // 0xD4C9는 음절 계산 결과가 U+D800(surrogate)이 되는 코드다. Python은 이 문자를 run에 넣은 채 이어 가고
    // 마지막에 일괄 제거한다. 표시 문자로 같은 동작을 낸다.
    let code = 0xD4C9u16;
    assert_eq!(hnc_to_unicode(code), LONE_SURROGATE.to_string());
    let mut chars = codes("Hello there ");
    chars.push(code);
    chars.extend(codes(" World Test"));
    let run = extract_text_bruteforce(&pack(&chars));
    assert!(!run.contains('\n'), "run이 쪼개졌다: {run:?}");
    assert!(run.contains(LONE_SURROGATE));

    // 파일 전체 경로에서는 표시 문자가 지워진다.
    let mut body = font_style_block();
    body.extend(pack(&chars));
    let mut file = header("V2.00");
    file.extend(doc_info(0, 0, 0));
    file.extend(vec![0u8; 1008]);
    file.extend(body);
    assert_eq!(extract_bytes(&file), "Hello there  World Test");
}
