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
/// 검사하는 `DocInfo` 스트림 수의 상한(정상 문서는 하나다).
const MAX_DOCINFO_COPIES: usize = 4;
/// `BodyText/Section*` 전체에서 푸는 바이트의 합계 상한.
const MAX_SECTIONS_TOTAL: usize = 64 * 1024 * 1024;
/// 검사하는 본문 섹션 수의 상한.
const MAX_SECTIONS: usize = 256;
/// 스트림 하나에서 세는 레코드 수의 상한.
const MAX_RECORDS: usize = 5_000_000;
/// `HWPTAG_BEGIN`(0x010) + 68.
const TAG_SHAPE_COMPONENT_OLE: u16 = 0x054;

/// `load`가 실패한 이유. `TooLarge`는 풀다가 상한을 넘긴 것이라 작업량은 이미 쓰였다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadError {
    Failed,
    TooLarge,
}

/// 레코드 스트림 하나(`entry`, 표시 이름 `path`)를 읽는다. 문서가 압축이면 푼다(압축 입력과 풀린 출력 모두 `limit` 이하).
/// 이름으로 다시 찾지 않고 열거한 항목을 그대로 읽는다(같은 이름의 항목이 여럿이어도 각자 읽힌다).
/// 읽거나 풀지 못하면 사유를 미완료로 남기고 오류를 돌려준다.
pub fn load(file: &mut OleFile, header: &FileHeader, path: &str, entry: &Entry, limit: usize, report: &mut FileReport) -> Result<Vec<u8>, LoadError> {
    let data = match file.read_stream_prefix(entry, limit) {
        Ok(d) => d,
        Err(e) => {
            report.mark_incomplete(&format!("{path} could not be read: {e:?}"));
            return Err(LoadError::Failed);
        }
    };
    if entry.size > limit as u64 {
        report.mark_incomplete(&format!("{path} exceeds the scan limit; only the first part was scanned"));
    }
    if !header.compressed() {
        return Ok(data);
    }
    match decompress(&data, limit) {
        Ok(b) => Ok(b),
        Err(InflateError::TooLarge) => {
            report.mark_incomplete(&format!("{path} decompresses beyond the scan limit; its records were not read"));
            Err(LoadError::TooLarge)
        }
        Err(InflateError::Invalid) => {
            report.mark_incomplete(&format!("{path} could not be decompressed; its records were not read"));
            Err(LoadError::Failed)
        }
    }
}

/// 루트 아래 `DocInfo` 스트림을 모두 읽는다(같은 이름이 여럿이면 모두, 최대 `MAX_DOCINFO_COPIES`개).
pub fn load_docinfos(file: &mut OleFile, header: &FileHeader, report: &mut FileReport) -> Vec<Vec<u8>> {
    let entries: Vec<Entry> = file.entries().iter().filter(|e| e.kind == EntryKind::Stream && file.path_of(e).eq_ignore_ascii_case("DocInfo")).take(MAX_DOCINFO_COPIES + 1).cloned().collect();
    if entries.len() > MAX_DOCINFO_COPIES {
        report.mark_incomplete(&format!("more than {MAX_DOCINFO_COPIES} DocInfo streams; the rest were not scanned"));
    }
    entries.iter().take(MAX_DOCINFO_COPIES).filter_map(|e| load(file, header, "DocInfo", e, MAX_DOCINFO, report).ok()).collect()
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
    for (index, (path, entry)) in sections.iter().enumerate() {
        if remaining == 0 {
            report.mark_incomplete(&format!("BodyText section budget used up; {} section(s) from {path} on were not validated", sections.len() - index));
            break;
        }
        match load(file, header, path, entry, remaining, report) {
            Ok(bytes) => {
                remaining = remaining.saturating_sub(bytes.len());
                validate(&bytes, path, report);
            }
            // 풀다가 상한을 넘긴 작업도 예산을 썼다. 뒤 섹션이 같은 작업을 되풀이하지 못하게 예산을 비운다.
            Err(LoadError::TooLarge) => remaining = 0,
            Err(LoadError::Failed) => {}
        }
    }
}
