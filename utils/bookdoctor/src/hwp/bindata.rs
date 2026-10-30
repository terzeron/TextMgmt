//! `BinData` 검사(명세 3.2.5, 표 17·18). `DocInfo`의 `HWPTAG_BIN_DATA`로 연결 파일을 찾고, `BinData/*` 스트림은 내용의 매직으로 본다.

use ole_reader::{Entry, EntryKind, OleFile};
use scan_common::sniff::{Sniffed, sniff};
use scan_common::text::sanitize_display;
use scan_common::{Category, FileReport, Severity};

use super::inflate::decompress_prefix;
use super::records::walk;
use crate::zipcheck::fast::ZIP_EXEC_EXTS;

/// `HWPTAG_BEGIN`(0x010) + 2.
const TAG_BIN_DATA: u16 = 0x012;
/// `BinData` 스트림에서 입력으로 읽는 최대 바이트와, 매직을 보려고 푸는 앞부분.
const MAX_STREAM_READ: usize = 256 * 1024;
const MAGIC_PREFIX: usize = 4096;
/// 검사하는 `BinData` 스트림 수의 상한.
const MAX_BINDATA: usize = 4096;
/// 연결 경로 하나에서 읽는 최대 글자 수.
const MAX_PATH_CHARS: usize = 1024;
const DOS_EPS: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];

/// 앞부분 매직으로 가른 내용 종류.
#[derive(PartialEq)]
enum Content {
    Executable,
    Ole,
    Eps,
    Other,
}

fn content_kind(bytes: &[u8]) -> Content {
    match sniff(bytes) {
        Sniffed::Pe | Sniffed::Elf | Sniffed::MachO => Content::Executable,
        Sniffed::Ole => Content::Ole,
        _ if bytes.starts_with(b"%!PS-Adobe") || bytes.starts_with(&DOS_EPS) => Content::Eps,
        _ => Content::Other,
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    b.get(at..at.checked_add(2)?).map(|s| usize::from(u16::from_le_bytes([s[0], s[1]])))
}

/// `at`에서 WORD 글자 수 + UTF-16 문자열을 읽는다. 문자열과 다음 위치를 돌려준다.
fn wstring(b: &[u8], at: usize) -> Option<(String, usize)> {
    let chars = u16_at(b, at)?;
    let start = at + 2;
    let end = start.checked_add(chars.checked_mul(2)?)?;
    let units: Vec<u16> = b.get(start..end)?.chunks_exact(2).take(MAX_PATH_CHARS).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some((String::from_utf16_lossy(&units), end))
}

/// 연결 경로가 네트워크(UNC·URL)를 가리키는가.
fn is_remote(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.starts_with("\\\\") || lower.starts_with("//") || lower.contains("://") || lower.starts_with("file:")
}

/// 풀린 `DocInfo`의 `HWPTAG_BIN_DATA`를 읽어 연결 파일(Type 0)을 알린다.
fn inspect_docinfo(bytes: &[u8], report: &mut FileReport) {
    for record in walk(bytes).filter(|r| r.tag == TAG_BIN_DATA) {
        // 속성의 bit 0~3이 Type: 0 LINK, 1 EMBEDDING, 2 STORAGE(OLE). LINK만 외부 경로를 가진다.
        let Some(attr) = u16_at(record.data, 0) else { continue };
        if attr & 0xF != 0 {
            continue;
        }
        let Some((absolute, next)) = wstring(record.data, 2) else { continue };
        let relative = wstring(record.data, next).map(|(s, _)| s).unwrap_or_default();
        for path in [absolute, relative].into_iter().filter(|p| !p.is_empty()) {
            let severity = if is_remote(&path) { Severity::High } else { Severity::Medium };
            report.push(Category::Security, severity, "HWP_LINKED_FILE", &sanitize_display(&path, 120, Some('…')), "BinData links to an external file");
        }
    }
}

/// `BinData` 아래 스트림 하나의 앞부분(원본과 풀린 것)을 보고 지표를 더한다.
fn inspect_stream(file: &mut OleFile, path: &str, entry: &Entry, report: &mut FileReport) {
    let shown = sanitize_display(path, 120, Some('…'));
    let ext = entry.name.rsplit_once('.').map(|(_, e)| e.trim().to_lowercase()).unwrap_or_default();
    if ZIP_EXEC_EXTS.contains(&ext.as_str()) {
        report.push(Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, &format!("BinData file type .{ext}"));
    }
    let data = match file.read_stream_prefix(entry, MAX_STREAM_READ) {
        Ok(d) => d,
        Err(e) => {
            report.mark_incomplete(&format!("{path} could not be read: {e:?}"));
            return;
        }
    };
    // 압축 여부는 문서 비트와 항목별 속성(0x10 압축, 0x20 비압축)이 함께 정한다. 어느 쪽이든 놓치지 않도록 원본과 푼 앞부분을 모두 본다.
    let raw = &data[..data.len().min(MAGIC_PREFIX)];
    let inflated = decompress_prefix(&data, MAGIC_PREFIX).unwrap_or_default();
    let kinds = [content_kind(raw), content_kind(&inflated)];
    if kinds.contains(&Content::Executable) {
        report.push(Category::Security, Severity::High, "EMBEDDED_EXECUTABLE", &shown, "BinData content is an executable (PE/ELF/Mach-O)");
    } else if kinds.contains(&Content::Ole) {
        report.push(Category::Security, Severity::High, "HWP_OLE_OBJECT", &shown, "Embedded OLE object");
    } else if kinds.contains(&Content::Eps) {
        // EPS는 정상 문서에도 들어가지만 한컴이 Ghostscript로 그리는 알려진 공격 경로다. 명세에는 없다(노트 참조).
        report.push(Category::Security, Severity::Medium, "HWP_EPS_OBJECT", &shown, "Embedded EPS (PostScript) object");
    }
}

/// `DocInfo`의 연결 파일과 `BinData/*` 스트림의 내용을 검사한다.
pub fn inspect(file: &mut OleFile, docinfos: &[Vec<u8>], report: &mut FileReport) {
    for bytes in docinfos {
        inspect_docinfo(bytes, report);
    }
    let streams: Vec<(String, Entry)> = file.entries().iter().filter(|e| e.kind == EntryKind::Stream).map(|e| (file.path_of(e), e.clone())).filter(|(p, _)| p.to_lowercase().starts_with("bindata/")).collect();
    if streams.len() > MAX_BINDATA {
        report.mark_incomplete(&format!("{} BinData streams; only the first {MAX_BINDATA} were scanned", streams.len()));
    }
    for (path, entry) in streams.into_iter().take(MAX_BINDATA) {
        inspect_stream(file, &path, &entry, report);
    }
}
