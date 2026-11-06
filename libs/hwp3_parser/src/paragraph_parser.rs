//! HWP V2.00~V3.00 본문 스트림의 문단·제어 문자 파싱. 읽기 순서는 LibreOffice hwpfilter를 따른다.

use std::fmt;

use crate::char_converter::hnc_to_unicode;

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

/// 글꼴 이름(7 카테고리)과 스타일을 건너뛴다.
pub(crate) fn skip_fonts_and_styles(stream: &mut HwpStream) -> Result<(), StreamError> {
    for _ in 0..7 {
        let n_fonts = stream.read_uint16()?;
        stream.skip(i64::from(n_fonts) * 40)?;
    }
    let n_styles = stream.read_uint16()?;
    stream.skip(i64::from(n_styles) * 238)
}
