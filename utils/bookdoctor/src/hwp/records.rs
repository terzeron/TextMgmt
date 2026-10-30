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

// --- 스트림 읽기와 검증 ---

use ole_reader::{Entry, EntryKind, OleFile};
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};

use super::header::FileHeader;
use super::inflate::{InflateError, decompress};

/// `DocInfo`를 읽는 최대 바이트(압축 입력과 풀린 출력 각각).
pub const MAX_DOCINFO: usize = 16 * 1024 * 1024;
/// `BodyText/Section*` 전체에서 푸는 바이트의 합계 상한.
const MAX_SECTIONS_TOTAL: usize = 64 * 1024 * 1024;
/// 검사하는 본문 섹션 수의 상한.
const MAX_SECTIONS: usize = 256;
/// 스트림 하나에서 세는 레코드 수의 상한.
const MAX_RECORDS: usize = 5_000_000;
/// `HWPTAG_BEGIN`(0x010) + 68.
const TAG_SHAPE_COMPONENT_OLE: u16 = 0x054;

/// 레코드 스트림 하나를 읽는다. 문서가 압축이면 푼다(압축 입력과 풀린 출력 모두 `limit` 이하).
/// 읽거나 풀지 못하면 사유를 미완료로 남기고 `None`을 돌려준다.
pub fn load(file: &mut OleFile, header: &FileHeader, path: &str, limit: usize, report: &mut FileReport) -> Option<Vec<u8>> {
    let entry = file.find(path).cloned()?;
    let data = match file.read_stream_prefix(&entry, limit) {
        Ok(d) => d,
        Err(e) => {
            report.mark_incomplete(&format!("{path} could not be read: {e:?}"));
            return None;
        }
    };
    if entry.size > limit as u64 {
        report.mark_incomplete(&format!("{path} exceeds the scan limit; only the first part was scanned"));
    }
    if !header.compressed() {
        return Some(data);
    }
    match decompress(&data, limit) {
        Ok(b) => Some(b),
        Err(InflateError::TooLarge) => {
            report.mark_incomplete(&format!("{path} decompresses beyond the scan limit; its records were not read"));
            None
        }
        Err(InflateError::Invalid) => {
            report.mark_incomplete(&format!("{path} could not be decompressed; its records were not read"));
            None
        }
    }
}

/// 레코드 스트림의 구조를 검증한다. 길이가 스트림을 넘으면 High 결함, 레벨이 한 번에 2 이상 깊어지면 Medium 결함(한 번만),
/// OLE 개체 레코드가 있으면 High 보안 지표다. 레코드 수 상한을 넘으면 미완료다.
pub fn validate(stream: &[u8], name: &str, report: &mut FileReport) {
    let shown = sanitize_display(name, 120, Some('…'));
    let mut records = walk(stream);
    let (mut count, mut previous, mut jumped, mut ole) = (0usize, 0u16, false, 0usize);
    for record in records.by_ref() {
        count += 1;
        if count > MAX_RECORDS {
            report.mark_incomplete(&format!("{shown} has more than {MAX_RECORDS} records; the rest were not validated"));
            break;
        }
        if !jumped && record.level > previous.saturating_add(1) {
            jumped = true;
            report.push(Category::Defect, Severity::Medium, "HWP_RECORD_LEVEL_JUMP", &shown, "record level deepens by more than one step");
        }
        previous = record.level;
        if record.tag == TAG_SHAPE_COMPONENT_OLE {
            ole += 1;
        }
    }
    if records.broken {
        report.push(Category::Defect, Severity::High, "HWP_BAD_RECORD", &shown, "a record header or length runs past the end of the stream");
    }
    if ole > 0 && !report.has("HWP_OLE_OBJECT") {
        report.push(Category::Security, Severity::High, "HWP_OLE_OBJECT", &shown, &format!("{ole} OLE object record(s)"));
    }
}

/// `BodyText/Section*`을 읽어 검증한다. 합계 풀린 바이트와 섹션 수에 상한이 있다.
pub fn inspect_sections(file: &mut OleFile, header: &FileHeader, report: &mut FileReport) {
    let mut sections: Vec<(String, Entry)> = file
        .entries()
        .iter()
        .filter(|e| e.kind == EntryKind::Stream)
        .map(|e| (file.path_of(e), e.clone()))
        .filter(|(p, _)| {
            let lower = p.to_lowercase();
            lower.starts_with("bodytext/section")
        })
        .collect();
    sections.sort_by(|a, b| a.0.cmp(&b.0));
    if sections.len() > MAX_SECTIONS {
        report.mark_incomplete(&format!("{} BodyText sections; only the first {MAX_SECTIONS} were validated", sections.len()));
        sections.truncate(MAX_SECTIONS);
    }
    let mut remaining = MAX_SECTIONS_TOTAL;
    for (path, _) in sections {
        if remaining == 0 {
            report.mark_incomplete("BodyText sections exceed the total scan budget; the rest were not validated");
            break;
        }
        let Some(bytes) = load(file, header, &path, remaining, report) else { continue };
        remaining = remaining.saturating_sub(bytes.len());
        validate(&bytes, &path, report);
    }
}
