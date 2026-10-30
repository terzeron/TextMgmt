//! 데이터 레코드 순회(명세 4.1). 헤더는 32비트: TagID 10, Level 10, Size 12. Size가 모두 1이면 뒤따르는 DWORD가 길이다.

/// 레코드 하나. `data`는 스트림 안을 가리킨다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    pub tag: u16,
    pub level: u16,
    pub data: &'a [u8],
}

/// 레코드를 앞에서부터 하나씩 읽는다. 헤더나 길이가 스트림을 넘으면 `broken`을 켜고 멈춘다.
pub struct Records<'a> {
    stream: &'a [u8],
    pos: usize,
    pub broken: bool,
}

/// `stream`의 레코드를 순회한다.
pub fn walk(stream: &[u8]) -> Records<'_> {
    Records { stream, pos: 0, broken: false }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

impl<'a> Iterator for Records<'a> {
    type Item = Record<'a>;

    fn next(&mut self) -> Option<Record<'a>> {
        if self.broken || self.pos >= self.stream.len() {
            return None;
        }
        let Some(header) = u32_at(self.stream, self.pos) else {
            self.broken = true;
            return None;
        };
        let mut body = self.pos + 4;
        let mut size = (header >> 20) as usize;
        if size == 0xFFF {
            let Some(extended) = u32_at(self.stream, body) else {
                self.broken = true;
                return None;
            };
            size = extended as usize;
            body += 4;
        }
        let Some(data) = body.checked_add(size).and_then(|end| self.stream.get(body..end)) else {
            self.broken = true;
            return None;
        };
        self.pos = body + size;
        Some(Record { tag: (header & 0x3FF) as u16, level: ((header >> 10) & 0x3FF) as u16, data })
    }
}
