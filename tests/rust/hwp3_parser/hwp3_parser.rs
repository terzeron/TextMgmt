//! HWP V3.00/V2.10/V2.00 텍스트 추출기 테스트 (tests/test_hwp3_parser.py 이식).
mod common;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use hwp3_parser::tables::HNC2UNI;
use hwp3_parser::*;
use std::fs;
use std::io::Write;

use common::shared_fixtures::{codes, le16, pack};

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
    // flag 0이면 글자 모양 31바이트가 뒤따른다.
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
fn is_hwp3_checks_the_legacy_signature() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.hwp");
    for (bytes, want) in [(header("V3.00"), true), (header("V1.20"), true), (vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1], false), (b"HWP Document".to_vec(), false)] {
        fs::write(&p, &bytes).unwrap();
        assert_eq!(is_hwp3(&p), want, "{bytes:?}");
    }
    assert!(!is_hwp3(&d.path().join("missing.hwp")));
}

#[test]
fn v120_uses_the_bruteforce_fallback_like_v200() {
    // 구조 파싱이 비는 V1.20 본문도 V2.00처럼 brute-force로 뽑는다.
    let mut body = font_style_block();
    body.extend(pack(&codes("Version One Twenty Text")));
    let mut file = header("V1.20");
    file.extend(doc_info(0, 0, 0));
    file.extend(vec![0u8; 1008]);
    file.extend(&body);
    assert!(extract_bytes(&file).contains("Version One Twenty Text"));
}

// ---- V1.20 ----

/// V1.20 문단: 머리 20바이트(글자 수 BE, 2바이트, 0×5, 모양, 번호 BE, 0×8) + 기록 `records` + 본문(문단 끝 포함).
fn v120_para(index: u16, chars: &[u16], records: &[u8]) -> Vec<u8> {
    let mut all = chars.to_vec();
    all.push(13);
    let mut b = (all.len() as u16).to_be_bytes().to_vec();
    b.extend([0u8, 0]);
    b.extend([0u8; 5]);
    b.push(0xfe);
    b.extend(index.to_be_bytes());
    b.extend([0u8; 8]);
    b.extend(records);
    b.extend(pack(&all));
    b
}

fn v120_file(paras: &[Vec<u8>], trailer: &[u8]) -> Vec<u8> {
    let mut b = header("V1.20");
    b.extend(vec![0u8; 128]);
    for p in paras {
        b.extend(p);
    }
    b.extend(trailer);
    b
}

#[test]
fn v120_paragraphs_are_read_in_order_with_variable_record_lengths() {
    let rec7 = [0x05, 0, 0, 1, 0, 0x80, 0];
    let rec4 = [1, 0, 0x80, 0];
    let long: Vec<u16> = vec![u16::from(b'x'); 300]; // 255자를 넘는 문단은 글자 수 상위 바이트가 1이다.
    let data = v120_file(&[v120_para(0, &codes("Hello"), &rec7), v120_para(1, &[], &rec4), v120_para(2, &long, &[rec7, rec7].concat()), v120_para(3, &codes("End"), &rec7)], &[0xEE; 40]);
    let text = extract_text_v120(&data);
    assert_eq!(text, format!("Hello\n\n{}\nEnd\n", "x".repeat(300)));
}

#[test]
fn v120_symbols_are_mapped_or_dropped() {
    let chars = [0x0102, u16::from(b'A'), 0x0103, 0x0107, u16::from(b'B'), 0x0108, 0x0437];
    let data = v120_file(&[v120_para(0, &chars, &[1, 0, 0x80, 0])], &[]);
    assert_eq!(extract_text_v120(&data), "“A”‘B’\n");
}

#[test]
fn v120_stops_where_the_structure_breaks() {
    // 두 번째 문단 번호가 틀리면 첫 문단은 마지막 문단으로 읽히고 거기서 멈춘다.
    let data = v120_file(&[v120_para(0, &codes("First"), &[1, 0, 0x80, 0]), v120_para(5, &codes("Lost"), &[1, 0, 0x80, 0])], &[]);
    assert_eq!(extract_text_v120(&data), "First\n");
    assert_eq!(extract_text_v120(&header("V1.20")), "");
}

#[test]
fn v120_sample_reads_clean_paragraphs() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|a| a.join("tests/books/_hwp/v1.20_부하의약혼녀.hwp")).find(|p| p.exists()).expect("V1.20 표본");
    let text = extract_text_from_hwp3(&p);
    assert!(text.starts_with("<  부하의 약혼녀 >"), "{}", &text[..80.min(text.len())]);
    assert!(text.contains("호텔 방은 상당히 취향을 살리어 꾸며져 있었다."));
    // 예전 brute-force 결과에 섞이던 머리 바이트(0x80·0x100)가 글자로 나오지 않는다.
    assert!(!text.contains('€') && !text.contains('Ā'));
}

#[test]
fn truncated_compressed_body_keeps_the_inflated_head() {
    // 압축 스트림이 중간에서 잘리면 풀린 데까지의 문단을 쓴다.
    let mut body = font_style_block();
    body.extend(paragraph(&codes("Truncated Head Paragraph Text"), 1));
    body.extend(paragraph(&vec![u16::from(b'z'); 20000], 1));
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::none());
    enc.write_all(&body).unwrap();
    let raw = enc.finish().unwrap();
    let mut file = header("V3.00");
    file.extend(doc_info(0, 1, 0));
    file.extend(vec![0u8; 1008]);
    file.extend(&raw[..raw.len() / 2]);
    assert!(extract_bytes(&file).contains("Truncated Head Paragraph Text"));
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
