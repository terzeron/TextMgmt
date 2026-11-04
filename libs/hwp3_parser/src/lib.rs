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

use std::fmt;
use std::io::Read;
use std::path::Path;

use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use tables::{FILL, HNC_L1, HNC_L2, HNC_T1, HNC_T2, HNC_V1, HNC_V2, KSC5601_TO_UNI, L_MAP, NONE, T_MAP, V_MAP, hnc2uni};

/// Python은 현대 음절 조합 계산(`chr(0xAC00 + ...)`)에서 lone surrogate 문자를 만든다. 이것은 run을 끊지 않은 채
/// 마지막에 일괄 제거된다. Rust의 `String`은 surrogate를 담을 수 없어서 같은 자리를 이 표시 문자로 채우고
/// `extract_text_from_hwp3`가 마지막에 지운다. 매핑 표의 어떤 값과도 겹치지 않는 U+10FFFF를 쓴다.
pub const LONE_SURROGATE: char = '\u{10FFFF}';

const SIGNATURE_PREFIX: &[u8] = b"HWP Document File";
const MAX_RECURSION_DEPTH: usize = 10;
const MAX_TEXT_LENGTH: usize = 1_000_000;

/// 제어 문자(0~31)가 문단 글자 수에서 차지하는 칸 수. LibreOffice hwpfilter `HBox::WSize`와 같다.
pub const WSIZE: [usize; 32] = [
    1, 4, 4, 4, 4, 4, 4, 42, //
    48, 4, 4, 4, 4, 1, 4, 4, //
    4, 4, 4, 4, 4, 4, 12, 5, //
    3, 3, 123, 4, 32, 4, 2, 2,
];

/// 스트림 끝을 넘어 읽으려 했다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError(pub String);

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StreamError {}

/// 바이트 버퍼와 읽기 위치.
pub struct HwpStream {
    data: Vec<u8>,
    pos: usize,
}

impl HwpStream {
    pub fn new(data: Vec<u8>) -> Self {
        HwpStream { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    fn take(&mut self, n: usize) -> Result<&[u8], StreamError> {
        if n > self.remaining() {
            return Err(StreamError("unexpected end of stream".into()));
        }
        let end = self.pos + n;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn read_uint8(&mut self) -> Result<u8, StreamError> {
        Ok(self.take(1)?[0])
    }

    pub fn read_uint16(&mut self) -> Result<u16, StreamError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn read_uint32(&mut self) -> Result<u32, StreamError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn skip(&mut self, n: i64) -> Result<(), StreamError> {
        let Some(skip) = usize::try_from(n).ok().filter(|skip| *skip <= self.remaining()) else {
            return Err(StreamError(format!("cannot skip {n} bytes (remaining: {})", self.remaining())));
        };
        self.pos += skip;
        Ok(())
    }

    pub fn read_bytes(&mut self, n: usize) -> Result<Vec<u8>, StreamError> {
        Ok(self.take(n)?.to_vec())
    }
}

/// surrogate 범위(U+D800~U+DFFF)와 유니코드 밖의 값은 빈 문자열로 바꾼다.
pub fn safe_chr(code: u32) -> String {
    char::from_u32(code).map(String::from).unwrap_or_default()
}

fn mapped(uni: Option<u32>) -> String {
    // 0은 "대응 없음"이다(Python의 `if uni`).
    uni.filter(|u| *u != 0).map(safe_chr).unwrap_or_default()
}

fn is_blank(v: u16) -> bool {
    v == FILL || v == NONE
}

/// HNC 2바이트 코드를 유니코드 문자열로 바꾼다.
pub fn hnc_to_unicode(c: u16) -> String {
    match c {
        0 => String::new(),
        0x0020..=0x007E => char::from(c as u8).to_string(),
        // 특수문자·확장
        0x007F..=0x3FFF => mapped(hnc2uni(c)),
        // 1수준 한자 (4888자)
        0x4000..=0x5317 => mapped(Some(u32::from(KSC5601_TO_UNI[usize::from(c - 0x4000)]))),
        // 2수준 한자
        0x5318..=0x7FFF => mapped(hnc2uni(c)),
        0x8000..=0xFFFF => hangul(c),
        _ => String::new(),
    }
}

fn hangul(c: u16) -> String {
    let (cho, jung, jong) = (usize::from((c & 0x7C00) >> 10), usize::from((c & 0x03E0) >> 5), usize::from(c & 0x001F));
    let (l_idx, v_idx, t_idx) = (L_MAP[cho], V_MAP[jung], T_MAP[jong]);
    // 현대 한글 완성형 음절
    if l_idx != NONE && v_idx != NONE && t_idx != NONE {
        let code = 0xAC00 + (u32::from(l_idx) * 21 + u32::from(v_idx)) * 28 + u32::from(t_idx);
        // 음절 범위를 넘는 값은 Python에서 surrogate 문자가 된다(`safe_chr`를 거치지 않는다).
        return char::from_u32(code).unwrap_or(LONE_SURROGATE).to_string();
    }
    let (l1, v1, t1) = (HNC_L1[cho], HNC_V1[jung], HNC_T1[jong]);
    let no_vowel = is_blank(v1);
    match (l1 != FILL, !no_vowel, t1 != FILL) {
        (true, false, false) => safe_chr(u32::from(l1)),                                                                                  // 초성만
        (false, true, false) => safe_chr(u32::from(v1)),                                                                                  // 중성만
        (false, false, true) => safe_chr(u32::from(t1)),                                                                                  // 종성만
        (true, true, false) => safe_chr(u32::from(HNC_L2[cho])) + &safe_chr(u32::from(HNC_V2[jung])),                                     // 옛한글 초성+중성
        (true, true, true) => safe_chr(u32::from(HNC_L2[cho])) + &safe_chr(u32::from(HNC_V2[jung])) + &safe_chr(u32::from(HNC_T2[jong])), // 옛한글 3벌
        // 완성형 옛한글 fallback
        _ if jung == 0 => mapped(hnc2uni(c)),
        _ => String::new(),
    }
}

/// 문단 리스트를 파싱해 텍스트를 돌려준다. 스트림이 끝나거나 문단이 잘못되면 지금까지 모은 텍스트를 돌려준다.
pub fn parse_paragraph_list(stream: &mut HwpStream, depth: usize) -> String {
    if depth > MAX_RECURSION_DEPTH {
        return String::new();
    }
    let (mut parts, mut total) = (String::new(), 0usize);
    while stream.remaining() >= 43 {
        match parse_paragraph(stream, depth) {
            Ok(Some(text)) => {
                total += text.chars().count();
                parts.push_str(&text);
                if total > MAX_TEXT_LENGTH {
                    break;
                }
            }
            // 빈 문단(리스트 끝)이거나 스트림 끝에 닿았다.
            Ok(None) | Err(_) => break,
        }
    }
    parts
}

/// 문단 하나를 파싱한다. `None`이면 빈 문단(리스트 종료)이다.
pub fn parse_paragraph(stream: &mut HwpStream, depth: usize) -> Result<Option<String>, StreamError> {
    let prev_para_shape = stream.read_uint8()?;
    let n_chars = stream.read_uint16()?;
    let n_lines = stream.read_uint16()?;
    let char_shape_included = stream.read_uint8()?;
    stream.skip(1 + 4 + 1 + 31)?; // flags + special + istyle + para_shape_part

    if prev_para_shape == 0 && n_chars > 0 {
        stream.skip(187)?; // 문단 모양 정보
    }
    if n_chars == 0 {
        return Ok(None);
    }
    if n_chars > 30000 || n_lines > 5000 {
        return Err(StreamError(format!("invalid paragraph: n_chars={n_chars}, n_lines={n_lines}")));
    }

    stream.skip(i64::from(n_lines) * 14)?; // 줄 정보
    if char_shape_included != 0 {
        // 글자마다 표식 1바이트. 0이면 글자 모양 31바이트가 뒤따른다(hpara.cxx).
        for _ in 0..n_chars {
            if stream.read_uint8()? == 0 {
                stream.skip(31)?;
            }
        }
    }

    let mut parts: Vec<String> = Vec::new();
    let mut n_read: usize = 0;
    while n_read < usize::from(n_chars) {
        let c = stream.read_uint16()?;
        if c < 32 {
            // 제어 문자는 데이터가 글자 몇 개 분량을 차지한다(`HBox::WSize`).
            n_read += WSIZE[usize::from(c)];
            handle_control_char(c, stream, &mut parts, depth)?;
            if c == 13 {
                break; // 문단 끝
            }
        } else {
            n_read += 1;
            let ch = hnc_to_unicode(c);
            if !ch.is_empty() {
                parts.push(ch);
            }
        }
    }
    Ok(Some(parts.concat()))
}

/// 제어 문자 `c` 뒤의 데이터를 읽는다. 대부분의 제어 데이터는 끝에 자기 코드를 한 번 더 적으므로(표식) 이를 확인해
/// 스트림이 어긋나면 일찍 멈춘다. 크기는 LibreOffice hwpfilter `hwpread.cxx`의 읽기 순서를 따른다.
pub fn handle_control_char(c: u16, stream: &mut HwpStream, parts: &mut Vec<String>, depth: usize) -> Result<(), StreamError> {
    match c {
        13 => parts.push("\n".into()), // 문단 끝
        // 예약·상호참조: 길이(4) + 표식 + 길이만큼의 데이터
        0..=4 | 12 | 27 | 29 => {
            let len = stream.read_uint32()?;
            check_marker(stream, c)?;
            stream.skip(i64::from(len))?;
        }
        5 => {
            // 필드 코드(누름틀): 크기(4) + 표식 + 종류·예약 30 + 문자열 길이 3개와 바이너리 길이 + 데이터
            stream.skip(4 + 2 + 30)?;
            let mut total = 0i64;
            for _ in 0..4 {
                total += i64::from(stream.read_uint32()?);
            }
            stream.skip(total)?;
        }
        6 => {
            // 책갈피: 길이(4) + 표식 + 34
            stream.skip(4)?;
            check_marker(stream, c)?;
            stream.skip(34)?;
        }
        7 => {
            // 날짜 형식: 서식 80 + 표식
            stream.skip(80)?;
            check_marker(stream, c)?;
        }
        8 => {
            // 날짜 코드: 서식 80 + [년, 월, 요일, 일, 시, 분] + 표식
            stream.skip(80)?;
            let mut date = [0u16; 6];
            for d in &mut date {
                *d = stream.read_uint16()?;
            }
            check_marker(stream, c)?;
            if date[0] > 0 {
                parts.push(format!("{}년 {}월 {}일", date[0], date[1], date[3]));
            }
        }
        9 => {
            // 탭: 너비·채움 4 + 표식
            stream.skip(4)?;
            check_marker(stream, c)?;
            parts.push("\t".into());
        }
        10 => {
            // 표·글상자: 예약 4 + 표식 + 배치 78 + 종류 + 칸 수 + 보호 + 칸 정보 + 칸마다 문단 리스트 + 캡션
            stream.skip(4)?;
            check_marker(stream, c)?;
            stream.skip(78 + 2)?;
            let n_cells = stream.read_uint16()?;
            stream.skip(2)?;
            stream.skip(27 * i64::from(n_cells))?;
            for _ in 0..n_cells {
                let text = parse_paragraph_list(stream, depth + 1);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            parse_paragraph_list(stream, depth + 1); // 캡션
        }
        11 => {
            // 그림: 예약 4 + 표식 + 첨부 길이 + 344 + 첨부 데이터 + 캡션
            stream.skip(4)?;
            check_marker(stream, c)?;
            let pic_len = stream.read_uint32()?;
            stream.skip(344)?;
            stream.skip(i64::from(pic_len))?;
            parse_paragraph_list(stream, depth + 1); // 캡션
        }
        14 => {
            // 선
            stream.skip(4)?;
            check_marker(stream, c)?;
            stream.skip(84)?;
        }
        15 => {
            // 숨은 설명: 본문에 넣지 않는다.
            stream.skip(4)?;
            check_marker(stream, c)?;
            stream.skip(8)?;
            parse_paragraph_list(stream, depth + 1);
        }
        16 => {
            // 머리말/꼬리말: 본문에 넣지 않는다.
            stream.skip(4)?;
            check_marker(stream, c)?;
            stream.skip(10)?;
            parse_paragraph_list(stream, depth + 1);
        }
        17 => {
            // 각주/미주
            stream.skip(4)?;
            check_marker(stream, c)?;
            stream.skip(14)?;
            let text = parse_paragraph_list(stream, depth + 1);
            if !text.is_empty() {
                parts.push(text);
            }
        }
        // 자동 번호, 새 번호, 쪽 번호 표시·제어
        18..=21 => {
            stream.skip(4)?;
            check_marker(stream, c)?;
        }
        22 => {
            // 메일 머지
            stream.skip(20)?;
            check_marker(stream, c)?;
        }
        23 => {
            // 글자 겹침: 겹친 글자를 그대로 쓴다.
            let mut compose = [0u16; 3];
            for ch in &mut compose {
                *ch = stream.read_uint16()?;
            }
            check_marker(stream, c)?;
            for ch in compose.into_iter().filter(|&ch| ch > 31) {
                parts.push(hnc_to_unicode(ch));
            }
        }
        // 하이픈, 차례 표시
        24 | 25 => {
            stream.skip(2)?;
            check_marker(stream, c)?;
        }
        26 => {
            // 찾아보기 표시: 낱말 2개 240 + 쪽 번호 + 표식
            stream.skip(240 + 2)?;
            check_marker(stream, c)?;
        }
        28 => {
            // 개요 번호
            stream.skip(60)?;
            check_marker(stream, c)?;
        }
        30 | 31 => {
            // 묶음 빈칸, 고정폭 빈칸
            check_marker(stream, c)?;
            parts.push(" ".into());
        }
        _ => {}
    }
    Ok(())
}

/// 제어 데이터 끝의 표식이 제어 코드와 같은지 확인한다. 다르면 스트림이 어긋난 것이다.
fn check_marker(stream: &mut HwpStream, c: u16) -> Result<(), StreamError> {
    let marker = stream.read_uint16()?;
    if marker != c {
        return Err(StreamError(format!("control {c} marker mismatch: {marker}")));
    }
    Ok(())
}

/// Python `str.isspace()`: 유니코드 공백에 `\x1c`~`\x1f`가 더해진다.
fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

/// 읽을 수 있는 글자인지: 공백, 영숫자, 한글 음절, 한자, 옛한글 자모.
fn readable_char(c: char, with_compat_jamo: bool) -> bool {
    let n = c as u32;
    py_isspace(c) || c.is_alphanumeric() || (0xAC00..=0xD7A3).contains(&n) || (with_compat_jamo && (0x3131..=0x318E).contains(&n)) || (0x4E00..=0x9FFF).contains(&n) || (0x1100..=0x11FF).contains(&n)
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
        let stripped = py_strip(part);
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

/// 글꼴 이름(7 카테고리)과 스타일을 건너뛴다.
fn skip_fonts_and_styles(stream: &mut HwpStream) -> Result<(), StreamError> {
    for _ in 0..7 {
        let n_fonts = stream.read_uint16()?;
        stream.skip(i64::from(n_fonts) * 40)?;
    }
    let n_styles = stream.read_uint16()?;
    stream.skip(i64::from(n_styles) * 238)
}

/// V1.20 문단 머리 길이: 글자 수(BE u16), 2바이트, 0 다섯 바이트, 모양, 문단 번호(BE u16), 0 여덟 바이트.
const V120_HEAD_LEN: usize = 20;
/// V1.20 문단 머리와 본문 사이 기록(줄·글자 모양 정보로 보인다)의 최대 길이. 길이가 문단마다 달라 바이트 단위로 찾는다.
const V120_MAX_GAP: usize = 256;

/// `pos`에 번호가 `index`인 V1.20 문단 머리가 있는지.
fn v120_head_at(data: &[u8], pos: usize, index: u16) -> bool {
    data.get(pos..pos + V120_HEAD_LEN).is_some_and(|h| {
        h[0] < 0x20 && h[4..9].iter().all(|&b| b == 0) && u16::from_be_bytes([h[10], h[11]]) == index && h[12..20].iter().all(|&b| b == 0)
    })
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
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head == *b"HWP Document File V"
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
        if !py_strip(&text).is_empty() {
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
    if py_strip(&text).is_empty() && (header.contains("V2.00") || header.contains("V1.20")) {
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
    py_strip(&text).to_owned()
}
