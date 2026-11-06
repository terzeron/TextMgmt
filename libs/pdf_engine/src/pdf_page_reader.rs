//! PDF 열기·쪽수·쪽 추출 능력. 설계: docs/rust-test-seams.md 3.11.
//!
//! 열기·구문 검사와 쪽 읽기·쓰기를 맡는다. 서비스는 이 파일의 trait만 알고,
//! 실제 구현(`LopdfOpener`, `LopdfPageOps`)은 `lopdf`로 만든다. 테스트는 대역으로 바꿔 끼운다.
//!
//! 페이지 미리보기는 Rust worker에서 lopdf/MuPDF가 필요한 객체를 읽는다. reader는 경로와 쪽수만 캐시한다.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use lopdf::{Document, Object, xref::XrefRecovery};

/// PDF 라이브러리 오류. `kind`는 오류 종류 이름이다(`"OpenError"`, `"ReadError"` 등).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfError {
    pub kind: String,
    pub message: String,
}

impl PdfError {
    pub fn new(kind: &str, message: impl Into<String>) -> Self {
        Self { kind: kind.to_string(), message: message.into() }
    }
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PdfError {}

/// 열린 PDF 한 건.
pub trait PdfDoc: Send {
    fn page_count(&self) -> usize;
    fn pdf_version(&self) -> String;
    /// 문서 정보 사전. 키는 `/Title`처럼 슬래시를 붙인 이름이다.
    fn docinfo(&self) -> HashMap<String, String>;
    /// `pdf.check_pdf_syntax()`: 구문 문제 설명 목록.
    fn check_pdf_syntax(&self) -> Result<Vec<String>, PdfError>;
    fn close(&self);
}

/// PDF를 연다.
pub trait PdfOpener: Send + Sync {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>, PdfError>;
}

/// PDF 쪽 읽기·쓰기.
pub trait PdfPageOps: Send + Sync {
    fn page_count(&self, path: &Path) -> Result<usize, PdfError>;
    /// 0부터 세는 반열린 구간 `[start, end)`의 쪽만 담은 새 PDF 바이트를 돌려준다.
    fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, PdfError>;

    /// 재사용 가능한 reader를 연다. 대역은 기존 경로 기반 동작을 사용한다.
    fn open_reader(&self, path: &Path) -> Result<PdfReaderHandle, PdfError> {
        Ok(PdfReaderHandle::new(path.to_path_buf(), self.page_count(path)?))
    }

    /// 열린 reader에서 쪽을 추출한다. 기본 구현은 경로 기반 추출로 하위 호환한다.
    fn extract_pages_from_reader(&self, reader: &PdfReaderHandle, start: usize, end: usize) -> Result<Vec<u8>, PdfError> {
        self.extract_pages(&reader.path, start, end)
    }
}

fn open_error(e: lopdf::Error) -> PdfError {
    match e {
        // 파일을 읽지 못한 것(없음, 권한, 장치 오류)은 PDF가 깨진 것과 다르다. 호출자가 저장소 오류로 구분한다.
        lopdf::Error::IO(io) => PdfError::new("OSError", io.to_string()),
        other => PdfError::new("OpenError", other.to_string()),
    }
}

/// PDF 문자열(UTF-16BE BOM 또는 PDFDocEncoding)을 문자열로 바꾼다. PDFDocEncoding은 Latin-1로 근사한다.
fn decode_pdf_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    bytes.iter().map(|&b| b as char).collect()
}

struct LopdfDoc {
    doc: Document,
    syntax_issues: Vec<String>,
}

impl PdfDoc for LopdfDoc {
    fn page_count(&self) -> usize {
        self.doc.get_pages().len()
    }

    fn pdf_version(&self) -> String {
        self.doc.version.clone()
    }

    fn docinfo(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        let Ok(info) = self.doc.trailer.get(b"Info") else { return out };
        let Ok((_, info)) = self.doc.dereference(info) else { return out };
        let Ok(dict) = info.as_dict() else { return out };
        for (key, value) in dict.iter() {
            let value = self.doc.dereference(value).map(|(_, v)| v).unwrap_or(value);
            let text = match value {
                Object::String(bytes, _) => decode_pdf_text(bytes),
                Object::Name(name) => String::from_utf8_lossy(name).into_owned(),
                _ => continue,
            };
            out.insert(format!("/{}", String::from_utf8_lossy(key)), text);
        }
        out
    }

    fn check_pdf_syntax(&self) -> Result<Vec<String>, PdfError> {
        Ok(self.syntax_issues.clone())
    }

    fn close(&self) {}
}

/// `lopdf` 기반 `PdfOpener`.
pub struct LopdfOpener;

impl PdfOpener for LopdfOpener {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>, PdfError> {
        let doc = Document::load(path).map_err(open_error)?;
        let syntax_issues = match doc.xref_recovery {
            Some(XrefRecovery::OffsetCorrected) => vec!["Cross-reference section offset required repair".to_string()],
            Some(XrefRecovery::Reconstructed) => vec!["Cross-reference table was reconstructed".to_string()],
            Some(_) => vec!["Cross-reference data was repaired".to_string()],
            None => Vec::new(),
        };
        Ok(Box::new(LopdfDoc { doc, syntax_issues }))
    }
}

/// 원본 객체를 보존하는 lopdf 추출과 MuPDF 쪽수 조회. 파일 내용은 reader에 보관하지 않는다.
pub struct LopdfPageOps;
impl PdfPageOps for LopdfPageOps {
    fn page_count(&self, path: &Path) -> Result<usize, PdfError> {
        std::fs::metadata(path).map_err(|e| PdfError::new("OSError", e.to_string()))?;
        crate::pdf_worker_client::page_count(&sysdeps::SystemRunner, path).map_err(|e| PdfError::new("ReadError", e))
    }
    fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, PdfError> {
        if start >= end {
            return Err(PdfError::new("ReadError", "Invalid PDF page range"));
        }
        std::fs::metadata(path).map_err(|e| PdfError::new("OSError", e.to_string()))?;
        crate::pdf_worker_client::extract_pages(&sysdeps::SystemRunner, path, start + 1, end).map_err(|e| PdfError::new("ReadError", e))
    }
}

/// PDF reader. 파일 내용 없이 경로와 총 쪽수만 보관한다.
#[derive(Debug)]
pub struct PdfReaderHandle {
    pub path: std::path::PathBuf,
    pub page_count: usize,
}

impl PdfReaderHandle {
    /// 경로 기반 PDF 대역용 reader handle.
    pub fn new(path: std::path::PathBuf, page_count: usize) -> Self {
        Self { path, page_count }
    }
}
