//! PDF 열기·쪽수·쪽 추출 능력. 설계: docs/rust-test-seams.md 3.11.
//!
//! Python `pikepdf`(열기·구문 검사)와 `pypdf`(쪽 읽기·쓰기)에 대응한다. 서비스는 이 파일의 trait만 알고,
//! 실제 구현(`LopdfOpener`, `LopdfPageOps`)은 `lopdf`로 만든다. 테스트는 대역으로 바꿔 끼운다.
//!
//! 이 파일은 `BookManager`가 쓰는 최소 표면만 있다. 쪽 편집(자르기, 삽입), 리더 캐시(`PdfReaderCache`),
//! `PdfCacheLimits`는 PDF 쪽 기능을 맡는 에이전트가 이 파일에 덧붙인다.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use lopdf::{Document, Object};

/// PDF 라이브러리 오류. `kind`는 Python 예외 클래스 이름에 대응한다(`"OpenError"`, `"RuntimeError"` 등).
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

/// 열린 PDF 한 건. `pikepdf.Pdf`에 대응한다.
pub trait PdfDoc: Send {
    fn page_count(&self) -> usize;
    fn pdf_version(&self) -> String;
    /// 문서 정보 사전. 키는 `/Title`처럼 슬래시를 붙인 이름이다.
    fn docinfo(&self) -> HashMap<String, String>;
    /// `pdf.check_pdf_syntax()`: 구문 문제 설명 목록.
    fn check_pdf_syntax(&self) -> Result<Vec<String>, PdfError>;
    fn close(&self);
}

/// `pikepdf.open`.
pub trait PdfOpener: Send + Sync {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>, PdfError>;
}

/// `pypdf`의 `PdfReader`/`PdfWriter`.
pub trait PdfPageOps: Send + Sync {
    fn page_count(&self, path: &Path) -> Result<usize, PdfError>;
    /// 0부터 세는 반열린 구간 `[start, end)`의 쪽만 담은 새 PDF 바이트를 돌려준다. Python의 `range(start - 1, end)`와 같다.
    fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, PdfError>;
}

fn open_error(e: impl fmt::Display) -> PdfError {
    PdfError::new("OpenError", e.to_string())
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
        // lopdf에는 qpdf 같은 구문 검사기가 없다. 열기에 성공했으면 문제 없음으로 본다.
        Ok(Vec::new())
    }

    fn close(&self) {}
}

/// `lopdf` 기반 `PdfOpener`.
pub struct LopdfOpener;

impl PdfOpener for LopdfOpener {
    fn open(&self, path: &Path) -> Result<Box<dyn PdfDoc>, PdfError> {
        let doc = Document::load(path).map_err(open_error)?;
        Ok(Box::new(LopdfDoc { doc }))
    }
}

/// `lopdf` 기반 `PdfPageOps`.
pub struct LopdfPageOps;

impl PdfPageOps for LopdfPageOps {
    fn page_count(&self, path: &Path) -> Result<usize, PdfError> {
        let doc = Document::load(path).map_err(open_error)?;
        Ok(doc.get_pages().len())
    }

    fn extract_pages(&self, path: &Path, start: usize, end: usize) -> Result<Vec<u8>, PdfError> {
        let mut doc = Document::load(path).map_err(open_error)?;
        let total = doc.get_pages().len();
        // lopdf 쪽 번호는 1부터다.
        let drop: Vec<u32> = (1..=total as u32)
            .filter(|n| {
                let index = *n as usize - 1;
                index < start || index >= end
            })
            .collect();
        doc.delete_pages(&drop);
        doc.prune_objects();
        let mut out = Vec::new();
        doc.save_to(&mut out).map_err(|e| PdfError::new("RuntimeError", e.to_string()))?;
        Ok(out)
    }
}
