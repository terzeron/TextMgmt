//! HWP 5.x(OLE 복합 파일) 검사. 포맷 사실은 `docs/bookdoctor-hwp5-notes.md`에 있다.
//! 파일을 실행하거나 렌더링하지 않는다.

use crate::Category;
use crate::CheckOptions;
use crate::FileReport;
use crate::Severity;
use crate::byte_reader::u16_at;
use crate::byte_reader::u32_at;
use crate::doc::open_ole;
use crate::doc::report_anomalies;
use crate::magic_sniffer::Sniffed;
use crate::magic_sniffer::sniff;
use crate::ole_parser::Entry;
use crate::ole_parser::EntryKind;
use crate::ole_parser::OleFile;
use crate::text_sanitizer::sanitize_display;
use crate::zipcheck::fast_zip_checker::ZIP_EXEC_EXTS;
use flate2::read::DeflateDecoder;
use flate2::read::ZlibDecoder;
use std::io::Read;
use std::path::Path;

/// `.hwp` 하나를 검사한다. 내용이 OLE면 HWP 5.x, `HWP Document File V` 서명이면 HWP 3.x 이하다.
/// HWP 3.x는 OLE가 아니므로 서명만 확인한다(갭 분석에 적었다).
pub fn scan_hwp_file(path: &Path, opts: &CheckOptions, sniffed: Sniffed, report: &mut FileReport) {
    match sniffed {
        Sniffed::Ole => {
            if let Some(mut file) = open_ole(path, opts, report) {
                inspect(&mut file, report);
            }
        }
        Sniffed::Hwp3 => {}
        // 실행 파일은 `common`이 이미 Critical로 알렸다.
        Sniffed::Elf | Sniffed::MachO | Sniffed::Pe => {}
        // 이름만 HWP인 다른 내용은 여기서 분석하지 않는다. 이름 불일치는 `common`이 알린다.
        other => report.mark_incomplete(&format!("content is not an HWP document ({other:?}); not analyzed")),
    }
}

/// `.hwpx` 이름인데 내용이 OLE인 파일을 검사한다. `FileHeader`가 있으면 HWP 5.x를 이름만 바꾼 것이다(확장자 불일치는 `common`이 알린다).
pub fn scan_ole_named_hwpx(path: &Path, opts: &CheckOptions, report: &mut FileReport) {
    let Some(mut file) = open_ole(path, opts, report) else { return };
    inspect(&mut file, report);
}

/// 열린 HWP 5.x를 검사해 `report`에 더한다.
fn inspect(file: &mut OleFile, report: &mut FileReport) {
    report_anomalies(file, report);
    let Some(entry) = file.find("FileHeader").cloned() else {
        report.push(Category::Defect, Severity::Medium, "FORMAT_MISMATCH", "FileHeader", "OLE 파일이지만 HWP 문서가 아니다(FileHeader 스트림이 없다)");
        // 이름만 HWP인 Word 문서나 암호화 OOXML이면 그쪽 분석기로 본다. 매크로를 놓치지 않기 위해서다.
        if file.find("WordDocument").is_some() || file.find("EncryptedPackage").is_some() {
            crate::doc::inspect(file, report);
        } else {
            report.mark_incomplete("OLE container cannot be inspected: no HWP FileHeader and not a Word document");
        }
        return;
    };
    let bytes = match file.read_stream_prefix(&entry, HEADER_LEN) {
        Ok(b) => b,
        Err(e) => {
            report.mark_incomplete(&format!("FileHeader could not be read: {e:?}"));
            return;
        }
    };
    let header = match parse_file_header(&bytes) {
        Ok(h) => h,
        Err(e) => {
            report.push(Category::Defect, Severity::Medium, "HWP_BAD_HEADER", "FileHeader", &e.detail());
            return;
        }
    };
    // 암호·DRM 문서는 본문과 BinData를 읽을 수 없다. 깨끗하다고 하지 않고 미완료로 남긴다.
    if let Some(reason) = header.encryption_reason() {
        report.mark_incomplete(reason);
        return;
    }
    // 배포용 문서는 본문이 `ViewText`에 따로 저장된다. 이 명세에는 그 구조가 없어 읽지 않는다.
    if header.distributable() && file.find("BodyText").is_none() && file.find("ViewText").is_some() {
        report.mark_incomplete("distribution document: ViewText not analyzed");
    }
    inspect_script(file, &header, report);
    // DocInfo는 한 번만 읽고 푼다. 레코드 검증과 BinData 연결 파일 검사가 같은 바이트를 쓴다.
    let docinfos = load_docinfos(file, &header, report);
    for bytes in &docinfos {
        validate(bytes, "DocInfo", report);
    }
    inspect_bindata(file, &docinfos, report);
    inspect_sections(file, &header, report);
}

// ---- `FileHeader` 스트림(256바이트, 명세 표 3). ----
/// 서명 32바이트 + 버전 4 + 속성 4 + 속성2 4 + EncryptVersion 4 + KOGL 1 + 예약 207.
pub const HEADER_LEN: usize = 256;
/// 서명은 `"HWP Document File"`이고 32바이트 칸의 나머지는 채움이다.
const SIGNATURE: &[u8] = b"HWP Document File";

/// 읽은 `FileHeader`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub version: u32,
    pub flags: u32,
    pub flags2: u32,
    pub encrypt_version: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    TooShort(usize),
    BadSignature,
}

impl HeaderError {
    pub fn detail(&self) -> String {
        match self {
            HeaderError::TooShort(len) => format!("FileHeader is {len} bytes (expected {HEADER_LEN})"),
            HeaderError::BadSignature => "FileHeader does not start with \"HWP Document File\"".to_string(),
        }
    }
}

/// `FileHeader` 바이트를 읽는다. 256바이트 미만이거나 서명이 다르면 오류다.
pub fn parse_file_header(bytes: &[u8]) -> Result<FileHeader, HeaderError> {
    if bytes.len() < HEADER_LEN {
        return Err(HeaderError::TooShort(bytes.len()));
    }
    if !bytes.starts_with(SIGNATURE) {
        return Err(HeaderError::BadSignature);
    }
    Ok(FileHeader { version: u32_at(bytes, 32).unwrap_or(0), flags: u32_at(bytes, 36).unwrap_or(0), flags2: u32_at(bytes, 40).unwrap_or(0), encrypt_version: u32_at(bytes, 44).unwrap_or(0) })
}

impl FileHeader {
    fn bit(&self, n: u32) -> bool {
        self.flags & (1 << n) != 0
    }

    pub fn compressed(&self) -> bool {
        self.bit(0)
    }

    pub fn encrypted(&self) -> bool {
        self.bit(1)
    }

    pub fn distributable(&self) -> bool {
        self.bit(2)
    }

    pub fn has_scripts(&self) -> bool {
        self.bit(3)
    }

    pub fn drm(&self) -> bool {
        self.bit(4)
    }

    pub fn cert_encrypted(&self) -> bool {
        self.bit(8)
    }

    pub fn cert_drm(&self) -> bool {
        self.bit(10)
    }

    /// 본문을 읽을 수 없게 하는 설정이 있으면 미완료 사유를 돌려준다. `encrypted: `로 시작해 cyan으로 표시된다.
    pub fn encryption_reason(&self) -> Option<&'static str> {
        if self.encrypted() {
            Some("encrypted: HWP password (FileHeader bit1)")
        } else if self.drm() {
            Some("encrypted: HWP DRM document (FileHeader bit4)")
        } else if self.cert_encrypted() {
            Some("encrypted: HWP certificate encryption (FileHeader bit8)")
        } else if self.cert_drm() {
            Some("encrypted: HWP certificate DRM (FileHeader bit10)")
        } else {
            None
        }
    }
}

// ---- HWP 압축 스트림 해제. 명세는 zlib이라 쓰지만 오픈소스 구현은 헤더 없는 raw deflate로 읽는다(`docs/bookdoctor-hwp5-notes.md`). ----
// 둘 다 받고, 출력은 항상 상한 안에서 읽는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflateError {
    /// 풀린 크기가 상한을 넘었다. 압축 폭탄일 수 있다.
    TooLarge,
    /// 어느 형식으로도 읽히지 않거나 중간에 끊겼다.
    Invalid,
}

/// zlib 헤더(RFC 1950)로 유효한가: CMF의 방식이 8(deflate)이고 `CMF*256+FLG`가 31의 배수.
fn looks_like_zlib(data: &[u8]) -> bool {
    matches!(data, [cmf, flg, ..] if cmf & 0x0F == 8 && (u32::from(*cmf) * 256 + u32::from(*flg)) % 31 == 0)
}

/// `reader`를 `limit`바이트까지만 읽는다. 상한을 넘기면 `TooLarge`, 읽기 오류면 `Invalid`다.
fn read_limited(reader: impl Read, limit: usize) -> Result<Vec<u8>, InflateError> {
    let mut out = Vec::new();
    // 한 바이트 더 읽어 상한 초과를 알아낸다. 그보다 더 읽지 않는다.
    reader.take(limit as u64 + 1).read_to_end(&mut out).map_err(|_| InflateError::Invalid)?;
    if out.len() > limit { Err(InflateError::TooLarge) } else { Ok(out) }
}

/// 한 형식으로 풀어 보는 함수.
type Attempt = fn(&[u8], usize) -> Result<Vec<u8>, InflateError>;

fn zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    read_limited(ZlibDecoder::new(data), limit)
}

fn raw(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    read_limited(DeflateDecoder::new(data), limit)
}

/// 압축 스트림을 풀어 `limit`바이트 이하로 돌려준다. zlib 헤더가 유효하면 zlib을, 아니면 raw deflate를 먼저 시도하고,
/// 첫 시도가 `Invalid`면 다른 쪽을 한 번 더 시도한다. `TooLarge`는 바로 돌려준다. 빈 입력은 빈 출력이다.
pub fn decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, InflateError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let (first, second): (Attempt, Attempt) = if looks_like_zlib(data) { (zlib, raw) } else { (raw, zlib) };
    match first(data, limit) {
        Err(InflateError::Invalid) => second(data, limit),
        other => other,
    }
}

/// 앞 `n`바이트만 푼다. 출력이 더 커도 오류가 아니고, 스트림이 끊겨도 그때까지 풀린 앞부분을 돌려준다.
/// 어느 형식으로도 한 바이트도 못 풀면 `Invalid`다. 내용의 매직만 볼 때 쓴다.
pub fn decompress_prefix(data: &[u8], n: usize) -> Result<Vec<u8>, InflateError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let try_zlib = |data: &[u8]| {
        let mut out = Vec::new();
        // 읽기 오류가 나도 `read_to_end`는 그때까지 읽은 바이트를 `out`에 남긴다.
        let _ = ZlibDecoder::new(data).take(n as u64).read_to_end(&mut out);
        out
    };
    let try_raw = |data: &[u8]| {
        let mut out = Vec::new();
        let _ = DeflateDecoder::new(data).take(n as u64).read_to_end(&mut out);
        out
    };
    let out = if looks_like_zlib(data) {
        Some(try_zlib(data)).filter(|o| !o.is_empty()).or_else(|| Some(try_raw(data)))
    } else {
        Some(try_raw(data)).filter(|o| !o.is_empty()).or_else(|| Some(try_zlib(data)))
    };
    out.filter(|o| !o.is_empty()).ok_or(InflateError::Invalid)
}

// ---- bindata_checker ----
// `BinData` 검사(명세 3.2.5, 표 17·18). `DocInfo`의 `HWPTAG_BIN_DATA`로 연결 파일을 찾고, `BinData/*` 스트림은 내용의 매직으로 본다.
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

/// `at`에서 WORD 글자 수 + UTF-16 문자열을 읽는다. 문자열과 다음 위치를 돌려준다.
fn wstring(b: &[u8], at: usize) -> Option<(String, usize)> {
    let chars = usize::from(u16_at(b, at)?);
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
        let Some(attr) = u16_at(record.data, 0).map(usize::from) else { continue };
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
        report.push(Category::Security, Severity::Critical, "EMBEDDED_EXECUTABLE", &shown, &format!("BinData file type .{ext}"));
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
        report.push(Category::Security, Severity::Critical, "EMBEDDED_EXECUTABLE", &shown, "BinData content is an executable (PE/ELF/Mach-O)");
    } else if kinds.contains(&Content::Ole) {
        report.push(Category::Security, Severity::High, "HWP_OLE_OBJECT", &shown, "Embedded OLE object");
    } else if kinds.contains(&Content::Eps) {
        // EPS는 정상 문서에도 들어가지만 한컴이 Ghostscript로 그리는 알려진 공격 경로다. 명세에는 없다(노트 참조).
        report.push(Category::Security, Severity::Medium, "HWP_EPS_OBJECT", &shown, "Embedded EPS (PostScript) object");
    }
}

/// `DocInfo`의 연결 파일과 `BinData/*` 스트림의 내용을 검사한다.
pub fn inspect_bindata(file: &mut OleFile, docinfos: &[Vec<u8>], report: &mut FileReport) {
    for bytes in docinfos {
        inspect_docinfo(bytes, report);
    }
    let streams: Vec<(String, Entry)> =
        file.entries().iter().filter(|e| e.kind == EntryKind::Stream).map(|e| (file.path_of(e), e.clone())).filter(|(p, _)| p.to_lowercase().starts_with("bindata/")).collect();
    if streams.len() > MAX_BINDATA {
        report.mark_incomplete(&format!("{} BinData streams; only the first {MAX_BINDATA} were scanned", streams.len()));
    }
    for (path, entry) in streams.into_iter().take(MAX_BINDATA) {
        inspect_stream(file, &path, &entry, report);
    }
}

// ---- record_reader ----
// 데이터 레코드 순회(명세 4.1). 헤더는 32비트: TagID 10, Level 10, Size 12. Size가 모두 1이면 뒤따르는 DWORD가 길이다.
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

// ---- script_checker ----
// `Scripts` 스토리지 검사(명세 3.2.9, 표 8·9). 스크립트는 텍스트로만 본다.
/// 스크립트 스트림에서 읽는 최대 바이트.
const MAX_SCRIPT: usize = 4 * 1024 * 1024;
/// 실행·내려받기·환경 접근을 뜻하는 키워드(소문자). 단어 경계가 필요한 `eval(`·`new function`은 따로 본다.
const KEYWORDS: &[&str] = &["activexobject", "wscript", "shell.", "shellexecute", "xmlhttp", "adodb.stream", "powershell", "cmd.exe", "urldownloadtofile"];
/// 검사하는 `Scripts` 아래 스트림 수의 상한.
const MAX_STREAMS: usize = 32;
/// `DefaultJScript`의 끝 표시(DWORD `-1`).
const END_FLAG: u32 = u32::MAX;

fn utf16_lossy(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// 표 9 구조(헤더·소스·Pre·Post 각각 DWORD 글자 수 + WCHAR 배열, 끝에 `-1`)로 읽어 네 부분을 이은 텍스트를 돌려준다.
/// 길이가 스트림을 넘거나 끝 표시가 다르면 `None`이다.
fn parse(data: &[u8]) -> Option<String> {
    let mut pos = 0usize;
    let mut text = String::new();
    for _ in 0..4 {
        let chars = u32_at(data, pos)? as usize;
        pos += 4;
        let end = pos.checked_add(chars.checked_mul(2)?)?;
        text.push_str(&utf16_lossy(data.get(pos..end)?));
        text.push('\n');
        pos = end;
    }
    (u32_at(data, pos)? == END_FLAG).then_some(text)
}

/// `word`가 단어 경계에서 시작하고 그 뒤에 (공백을 건너뛰고) `(`가 오는가. `retrieval (`은 해당하지 않는다.
fn has_call(lower: &str, word: &str) -> bool {
    lower.match_indices(word).any(|(i, _)| {
        let before_ok = lower[..i].chars().next_back().is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '$'));
        before_ok && lower[i + word.len()..].trim_start().starts_with('(')
    })
}

fn suspicious(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    if let Some(k) = KEYWORDS.iter().find(|k| lower.contains(*k)) {
        return Some(k);
    }
    if has_call(&lower, "eval") {
        return Some("eval(");
    }
    // `new Function(...)`은 문자열을 코드로 실행한다. 사이 공백은 여럿일 수 있다.
    let squeezed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if has_call(&squeezed, "new function") {
        return Some("new Function(");
    }
    None
}

/// 원시 바이트를 UTF-16과 8비트로 모두 읽은 텍스트. 구조를 모를 때 키워드만 찾는 데 쓴다.
fn raw_text(data: &[u8]) -> String {
    let latin1: String = data.iter().map(|&b| b as char).collect();
    format!("{}\n{latin1}", utf16_lossy(data))
}

/// 스트림을 읽어 검사할 바이트 후보를 돌려준다: 원본, 그리고 압축 비트가 서 있으면 raw deflate나 zlib으로 푼 내용.
/// 표 2는 `Scripts`를 압축 대상으로 표시하지 않으므로 압축하지 않은 스트림도 받는다. 읽는 쪽이 어느 쪽으로 읽는지 알 수 없어 둘 다 본다.
/// 상한이나 풀기 문제로 덜 본 사유도 함께 돌려준다.
fn read_script(file: &mut OleFile, entry: &crate::ole_parser::Entry, compressed: bool) -> Result<(Vec<Vec<u8>>, Option<String>), String> {
    let data = file.read_stream_prefix(entry, MAX_SCRIPT).map_err(|e| format!("{} could not be read: {e:?}", entry.name))?;
    let mut note = (entry.size > MAX_SCRIPT as u64).then(|| format!("{} ({} bytes) exceeds the scan limit; only the first {} MiB were scanned", entry.name, entry.size, MAX_SCRIPT / (1024 * 1024)));
    let mut candidates = Vec::new();
    // 8바이트 이하(`JScriptVersion` 등)는 풀 필요가 없다.
    if compressed && data.len() > 8 {
        match decompress(&data, MAX_SCRIPT) {
            Ok(inflated) => candidates.push(inflated),
            Err(InflateError::TooLarge) => note = Some(format!("{} decompresses beyond the {} MiB scan limit", entry.name, MAX_SCRIPT / (1024 * 1024))),
            // 압축된 스트림이 아니었다. 원본만 본다.
            Err(InflateError::Invalid) => {}
        }
    }
    candidates.push(data);
    Ok((candidates, note))
}

/// `Scripts` 스토리지를 검사한다. 한글은 스크립트가 없어도 빈 `DefaultJScript`가 든 `Scripts` 저장소를 항상 쓰므로(실제 HWP5로 확인했다)
/// 저장소가 있다는 것만으로는 지표가 아니다. `FileHeader` 비트 3(스크립트 저장)이 서 있거나, 소스가 비어 있지 않거나, 비어 있음을 증명하지 못할 때만 `HWP_SCRIPT`다.
/// 비트만 서 있고 저장소가 없으면 결함이다. `DefaultJScript`는 표 9 구조로, 그 밖의 `Scripts` 아래 스트림은 원시 텍스트로 키워드를 찾는다.
pub fn inspect_script(file: &mut OleFile, header: &FileHeader, report: &mut FileReport) {
    let has_storage = file.find("Scripts").is_some();
    if header.has_scripts() && !has_storage {
        report.push(Category::Defect, Severity::Medium, "HWP_SCRIPT_FLAG_MISMATCH", "Scripts", "FileHeader bit3 (script saved) is set but there is no Scripts storage");
    }
    if !has_storage {
        return;
    }
    // 작성자가 스크립트 저장을 선언했으면 소스가 비었어도 스크립트가 있는 문서다.
    let mut present = header.has_scripts();
    let mut streams: Vec<(String, crate::ole_parser::Entry)> = file
        .entries()
        .iter()
        .filter(|e| e.kind == crate::ole_parser::EntryKind::Stream)
        .map(|e| (file.path_of(e), e.clone()))
        .filter(|(path, _)| path.to_lowercase().starts_with("scripts/"))
        .collect();
    // `DefaultJScript`를 먼저 보고, 상한 때문에 건너뛴 스트림이 있으면 깨끗하다고 하지 않는다.
    streams.sort_by_key(|(_, e)| !e.name.eq_ignore_ascii_case("DefaultJScript"));
    if streams.len() > MAX_STREAMS {
        report.mark_incomplete(&format!("{} streams under Scripts exceed the scan limit of {MAX_STREAMS}; the rest were not scanned", streams.len() - MAX_STREAMS));
        streams.truncate(MAX_STREAMS);
        // 못 본 스트림이 있으면 비어 있음을 증명하지 못한다.
        present = true;
    }
    for (path, entry) in streams {
        let (candidates, note) = match read_script(file, &entry, header.compressed()) {
            Ok(r) => r,
            Err(reason) => {
                // 읽지 못했으면 비어 있음을 증명하지 못한다.
                present = true;
                report.mark_incomplete(&reason);
                continue;
            }
        };
        if let Some(note) = note {
            present = true;
            report.mark_incomplete(&note);
        }
        let default = entry.name.eq_ignore_ascii_case("DefaultJScript");
        let within_limit = entry.size <= MAX_SCRIPT as u64;
        // 표 9 구조로 읽히는 후보는 그 텍스트를, 그렇지 않은 후보는 원시 바이트를 본다. 원본의 원시 바이트는 항상 함께 본다(표 뒤에 붙은 바이트 포함).
        let parsed: Vec<String> = if default && within_limit { candidates.iter().filter_map(|c| parse(c)).collect() } else { Vec::new() };
        if default && within_limit && parsed.is_empty() {
            report.mark_incomplete("Scripts/DefaultJScript does not follow the documented layout; scanned as raw bytes");
        }
        // `DefaultJScript`는 소스가 비어 있지 않거나 표로 읽히지 않을 때, 그 밖의 알 수 없는 스트림은 있다는 것만으로 스크립트가 있다고 본다(`JScriptVersion`은 버전 숫자뿐이다).
        if default {
            present |= parsed.is_empty() || parsed.iter().any(|t| !t.trim().is_empty());
        } else if !path.eq_ignore_ascii_case("Scripts/JScriptVersion") {
            // 버전 숫자뿐인 최상위 `Scripts/JScriptVersion`만 예외다. 다른 위치의 같은 이름은 알 수 없는 스트림이다.
            present = true;
        }
        let original = candidates.last().map(|d| raw_text(d)).unwrap_or_default();
        let inflated: Vec<String> = candidates.iter().rev().skip(1).map(|d| raw_text(d)).collect();
        let text = [parsed, inflated, vec![original]].concat().join("\n");
        if let Some(keyword) = suspicious(&text) {
            present = true;
            report.push(Category::Security, Severity::Critical, "HWP_SCRIPT_SUSPICIOUS", &sanitize_display(&path, 120, Some('…')), &format!("script uses {keyword}"));
        }
    }
    if present {
        report.push(Category::Security, Severity::High, "HWP_SCRIPT", "Scripts", "HWP document stores script code");
    }
}
