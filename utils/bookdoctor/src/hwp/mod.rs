//! HWP 5.x(OLE 복합 파일) 검사. 포맷 사실은 `docs/bookdoctor-hwp5-notes.md`에 있다.
//! 파일을 실행하거나 렌더링하지 않는다.

pub mod bindata_checker;
pub mod record_reader;
pub mod script_checker;

use std::path::Path;

use crate::magic_sniffer::Sniffed;
use crate::ole_parser::OleFile;
use crate::{Category, FileReport, Severity};

use crate::cli_parser::Options;
use crate::doc::{open_ole, report_anomalies};
use flate2::read::{DeflateDecoder, ZlibDecoder};
use std::io::Read;

/// `.hwp` 하나를 검사한다. 내용이 OLE면 HWP 5.x, `HWP Document File V` 서명이면 HWP 3.x 이하다.
/// HWP 3.x는 OLE가 아니므로 서명만 확인한다(갭 분석에 적었다).
pub fn scan_hwp_file(path: &Path, opts: &Options, sniffed: Sniffed, report: &mut FileReport) {
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
pub fn scan_ole_named_hwpx(path: &Path, opts: &Options, report: &mut FileReport) {
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
    script_checker::inspect(file, &header, report);
    // DocInfo는 한 번만 읽고 푼다. 레코드 검증과 BinData 연결 파일 검사가 같은 바이트를 쓴다.
    let docinfos = record_reader::load_docinfos(file, &header, report);
    for bytes in &docinfos {
        record_reader::validate(bytes, "DocInfo", report);
    }
    bindata_checker::inspect(file, &docinfos, report);
    record_reader::inspect_sections(file, &header, report);
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

fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// `FileHeader` 바이트를 읽는다. 256바이트 미만이거나 서명이 다르면 오류다.
pub fn parse_file_header(bytes: &[u8]) -> Result<FileHeader, HeaderError> {
    if bytes.len() < HEADER_LEN {
        return Err(HeaderError::TooShort(bytes.len()));
    }
    if !bytes.starts_with(SIGNATURE) {
        return Err(HeaderError::BadSignature);
    }
    Ok(FileHeader { version: u32_at(bytes, 32), flags: u32_at(bytes, 36), flags2: u32_at(bytes, 40), encrypt_version: u32_at(bytes, 44) })
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
