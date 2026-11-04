//! HWP V2.00/V2.10/V3.00 바이너리 포맷 텍스트 추출기. 원본: `utils/hwp3_parser.py`.
//!
//! 구형 한글(HWP) 문서에서 본문 텍스트를 뽑는다. LibreOffice가 처리하지 못하는 V1.20~V3.00 파일의
//! 대비책이다. 파싱에 실패하거나 암호화된 파일, 알아볼 수 없는 글자가 많은 결과는 빈 문자열이다.
//!
//! 참고: 한컴 "한글 문서 파일 형식 3.0" 사양서, ddoleye/java-hwp `HwpTextExtractorV3.java`,
//! namhyung/libhwp `ghwp-file-v3.c`.
//! 제어 문자별 데이터 크기·글자 수(`WSIZE`)·끝 표식 확인은 teammilestone/hwarang `src/hwp30/para.rs`
//! (MIT, Copyright (c) 2025 Lee Wonsup)를 참고했다. hwarang은 LibreOffice hwpfilter `hpara.cxx`·`hwpread.cxx`의 읽기 순서를 따른다.
//!
//! 내부 단계(스트림, HNC 변환, 문단 파싱)도 `pub`으로 열어 둔다. Python 원본의 비공개 함수 테스트를 그대로
//! 옮기기 위해서다.

pub mod tables;

mod char_converter;
mod paragraph_parser;

pub use char_converter::{LONE_SURROGATE, hnc_to_unicode, safe_chr};
pub use paragraph_parser::{HwpStream, StreamError, WSIZE, handle_control_char, parse_paragraph, parse_paragraph_list};

use std::io::Read;
use std::path::Path;

use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};

use paragraph_parser::skip_fonts_and_styles;

const SIGNATURE_PREFIX: &[u8] = b"HWP Document File";

/// 읽을 수 있는 글자인지: 공백, 영숫자, 한글 음절, 한자, 옛한글 자모.
fn readable_char(c: char, with_compat_jamo: bool) -> bool {
    let n = c as u32;
    c.is_whitespace()
        || c.is_alphanumeric()
        || (0xAC00..=0xD7A3).contains(&n)
        || (with_compat_jamo && (0x3131..=0x318E).contains(&n))
        || (0x4E00..=0x9FFF).contains(&n)
        || (0x1100..=0x11FF).contains(&n)
}

/// 바이트 스트림에서 HNC 텍스트를 brute-force로 뽑는다(V2.00 대비책).
pub fn extract_text_bruteforce(body_data: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, parts: &mut Vec<String>| {
        if !run.is_empty() {
            parts.push(std::mem::take(run));
        }
    };
    for pair in body_data.chunks_exact(2) {
        let c = u16::from_le_bytes([pair[0], pair[1]]);
        match c {
            13 => run.push('\n'),
            9 => run.push('\t'),
            0x0020 => run.push(' '),
            0 => {}
            0x0021..=0x007E => run.push(char::from(c as u8)),
            0x0080.. => {
                let ch = hnc_to_unicode(c);
                if ch.is_empty() {
                    flush(&mut run, &mut parts);
                } else {
                    run.push_str(&ch);
                }
            }
            _ => flush(&mut run, &mut parts),
        }
    }
    flush(&mut run, &mut parts);

    // 품질 필터: 읽을 수 있는 글자가 70% 이상이고 8자 이상인 run만 남긴다.
    let mut clean: Vec<&str> = Vec::new();
    for part in &parts {
        let stripped = part.trim();
        let len = stripped.chars().count();
        if len < 8 {
            continue;
        }
        let readable = stripped.chars().filter(|c| readable_char(*c, false)).count();
        if readable as f64 / len as f64 >= 0.7 {
            // V2.00 레코드 구분자는 알려진 것이 모두 8자 미만이라 위의 길이 필터가 먼저 버린다.
            if stripped.contains("豼豼") && len < 20 {
                continue;
            }
            clean.push(stripped);
        }
    }
    clean.join("\n")
}

/// 본문 스트림을 풀 때의 상한. 압축 폭탄이 메모리를 다 쓰지 않게 한다.
const MAX_INFLATED_BYTES: u64 = 64 * 1024 * 1024;

fn inflate(body: &[u8]) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    if DeflateDecoder::new(body).take(MAX_INFLATED_BYTES).read_to_end(&mut raw).is_ok() {
        return Some(raw);
    }
    // gzip 또는 zlib 헤더가 있는 경우(Python의 wbits=15+32).
    let mut out = Vec::new();
    let ok = if body.starts_with(&[0x1f, 0x8b]) {
        GzDecoder::new(body).take(MAX_INFLATED_BYTES).read_to_end(&mut out).is_ok()
    } else {
        ZlibDecoder::new(body).take(MAX_INFLATED_BYTES).read_to_end(&mut out).is_ok()
    };
    if ok {
        return Some(out);
    }
    // 잘린 raw deflate: 풀린 데까지 쓴다. 문단 파싱은 스트림이 끝나는 곳에서 멈춘다.
    (!raw.is_empty()).then_some(raw)
}

/// V1.20 문단 머리 길이: 글자 수(BE u16), 2바이트, 0 다섯 바이트, 모양, 문단 번호(BE u16), 0 여덟 바이트.
const V120_HEAD_LEN: usize = 20;
/// V1.20 문단 머리와 본문 사이 기록(줄·글자 모양 정보로 보인다)의 최대 길이. 길이가 문단마다 달라 바이트 단위로 찾는다.
const V120_MAX_GAP: usize = 256;

/// `pos`에 번호가 `index`인 V1.20 문단 머리가 있는지.
fn v120_head_at(data: &[u8], pos: usize, index: u16) -> bool {
    data.get(pos..pos + V120_HEAD_LEN).is_some_and(|h| h[0] < 0x20 && h[4..9].iter().all(|&b| b == 0) && u16::from_be_bytes([h[10], h[11]]) == index && h[12..20].iter().all(|&b| b == 0))
}

/// HWP V1.20 본문을 문단 단위로 읽는다. 서명(30)과 문서 정보(128) 바로 뒤에서 문단이 번호 순서로 이어진다.
/// 각 문단 본문은 머리와 가변 길이 기록 뒤의 `글자 수 × 2`바이트이고 문단 끝(13)으로 끝난다.
/// 기록 길이는 본문 끝이 13이고 바로 뒤에 다음 번호의 머리가 오는(또는 파일이 끝나는) 값으로 정한다.
/// 구조가 맞지 않는 곳에서 멈추고 그때까지 읽은 텍스트를 돌려준다.
pub fn extract_text_v120(data: &[u8]) -> String {
    let mut text = String::new();
    let (mut pos, mut index) = (30 + 128, 0u16);
    while v120_head_at(data, pos, index) {
        let n_chars = usize::from(u16::from_be_bytes([data[pos], data[pos + 1]]));
        let next = index.wrapping_add(1);
        let span = |gap: usize| {
            let start = pos + V120_HEAD_LEN + gap;
            let end = start + 2 * n_chars;
            (n_chars > 0 && data.get(end - 2..end) == Some(&[13, 0][..])).then_some((start, end))
        };
        // 다음 머리가 바로 뒤에 오는 간격을 찾는다. 없으면 마지막 문단이다(뒤에 꼬리 데이터가 붙는다).
        let followed = (0..=V120_MAX_GAP).find_map(|gap| span(gap).filter(|&(_, end)| v120_head_at(data, end, next)));
        let (body, last) = match followed {
            Some(b) => (Some(b), false),
            None => ((0..=V120_MAX_GAP).find_map(span), true),
        };
        let Some((start, end)) = body else { break };
        for pair in data[start..end].chunks_exact(2) {
            match u16::from_le_bytes([pair[0], pair[1]]) {
                13 => text.push('\n'),
                9 => text.push('\t'),
                c if c < 32 => {}
                // V1.20 기호 코드. 사양서를 찾지 못해 코퍼스의 짝 사용에서 추정했다:
                // 0x0102/0x0103은 대사를, 0x0107/0x0108은 지명을 감싼다. 뜻을 모르는 나머지 기호는 뺀다.
                0x0102 => text.push('“'),
                0x0103 => text.push('”'),
                0x0107 => text.push('‘'),
                0x0108 => text.push('’'),
                c if (0x80..0x8000).contains(&c) => {}
                c => text.push_str(&hnc_to_unicode(c)),
            }
        }
        if last {
            break;
        }
        pos = end;
        index = next;
    }
    text
}

/// HWP 3.x 이하(`HWP Document File V` 서명) 파일인지 앞 19바이트로 판별한다. OLE인 HWP 5.x는 아니다.
pub fn is_hwp3(path: &Path) -> bool {
    let mut head = [0u8; 19];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head.starts_with(SIGNATURE_PREFIX) && head[SIGNATURE_PREFIX.len()..] == *b" V"
}

/// HWP V1.20/V2.00/V2.10/V3.00 파일에서 텍스트를 추출한다. 실패하면 빈 문자열이다.
pub fn extract_text_from_hwp3(path: &Path) -> String {
    let Ok(data) = std::fs::read(path) else { return String::new() };

    if data.len() < 30 || !data[..17].starts_with(SIGNATURE_PREFIX) {
        return String::new();
    }
    let header = String::from_utf8_lossy(&data[..30]);
    if !["V3.00", "V2.10", "V2.00", "V1.20"].iter().any(|v| header.contains(v)) {
        return String::new();
    }
    if data.len() < 30 + 128 {
        return String::new();
    }
    // V1.20은 문서 정보 구조가 달라 문단을 따로 읽는다. 못 읽으면 아래 brute-force로 간다.
    if header.contains("V1.20") {
        let text = extract_text_v120(&data);
        if !text.trim().is_empty() {
            return finish_text(text);
        }
    }

    // 문서 정보 (offset 30, 128바이트)
    let doc_info = &data[30..30 + 128];
    if u16::from_le_bytes([doc_info[96], doc_info[97]]) != 0 {
        return String::new(); // 암호화된 문서
    }
    let compressed = doc_info[124];
    let info_block_len = usize::from(u16::from_le_bytes([doc_info[126], doc_info[127]]));

    // 문서 요약과 정보 블록을 건너뛴다.
    let body_offset = 30 + 128 + 1008 + info_block_len;
    if body_offset > data.len() {
        return String::new();
    }
    let mut body = data[body_offset..].to_vec();
    if compressed != 0 {
        match inflate(&body) {
            Some(b) => body = b,
            None => return String::new(),
        }
    }

    let mut stream = HwpStream::new(body.clone());
    let mut text = match skip_fonts_and_styles(&mut stream) {
        Ok(()) => parse_paragraph_list(&mut stream, 0),
        Err(_) => String::new(),
    };

    // V2.00·V1.20: 구조 파싱이 비면 brute-force로 뽑는다. 글꼴·스타일 뒤부터 읽어 메타데이터 쓰레기를 줄인다.
    if text.trim().is_empty() && (header.contains("V2.00") || header.contains("V1.20")) {
        let mut skip = HwpStream::new(body.clone());
        text = match skip_fonts_and_styles(&mut skip) {
            Ok(()) => extract_text_bruteforce(&body[skip.position()..]),
            Err(_) => extract_text_bruteforce(&body),
        };
    }

    finish_text(text)
}

/// 추출 결과를 마무리한다: surrogate 표시 제거, 품질 검사, 앞뒤 공백 정리.
fn finish_text(mut text: String) -> String {
    // surrogate 문자 제거(UTF-8 인코딩 오류 방지). 품질 검사 전에 지운다(원본과 같은 순서).
    text.retain(|c| c != LONE_SURROGATE);

    // 쓰레기 문자 비율이 높으면 파싱 실패로 본다. 앞 500자만 센다.
    if !text.is_empty() {
        let head: Vec<char> = text.chars().take(500).collect();
        let readable = head.iter().filter(|c| readable_char(**c, true)).count();
        if (readable as f64 / head.len() as f64) < 0.3 {
            return String::new();
        }
    }
    text.trim().to_owned()
}
