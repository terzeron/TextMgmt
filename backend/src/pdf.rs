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

// ---------------------------------------------------------------------------
// 캐시
// ---------------------------------------------------------------------------

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// PDF 리더 캐시 한도. 개수가 아니라 바이트로 막는다(리더 하나는 PDF 크기와 거의 1:1로 메모리를 쓴다). `None`은 제한 없음이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PdfCacheLimits {
    /// 이 크기를 넘는 파일은 캐시하지 않는다.
    pub max_file_bytes: Option<u64>,
    /// 캐시 전체 예산.
    pub max_bytes: Option<u64>,
    /// 작은 PDF가 무한히 쌓이지 않게 하는 개수 상한.
    pub max_entries: Option<usize>,
}

impl Default for PdfCacheLimits {
    fn default() -> Self {
        Self { max_file_bytes: Some(200 * 1024 * 1024), max_bytes: Some(512 * 1024 * 1024), max_entries: Some(32) }
    }
}

/// 열어 둔 PDF 한 건의 핸들. 쪽수를 이미 읽었다는 뜻이다. `PdfPageOps`가 파일 경로로 일하므로 파싱 결과는 들고 있지 않다.
#[derive(Debug)]
pub struct PdfReaderHandle {
    pub path: std::path::PathBuf,
    pub page_count: usize,
}

struct ReaderEntry {
    mtime: i64,
    reader: Arc<PdfReaderHandle>,
    pages: i64,
    size: u64,
}

/// 경로를 키로 하는 LRU 리더 캐시. `ComicsManager`와 책 매니저가 ID 공간이 겹쳐도 다른 파일의 리더를 주지 않도록 book_id가 아니라 경로로 키를 잡는다.
pub struct PdfReaderCache {
    limits: PdfCacheLimits,
    /// 오래된 것이 앞이다.
    inner: Mutex<(VecDeque<String>, std::collections::HashMap<String, ReaderEntry>)>,
}

impl PdfReaderCache {
    pub fn new(limits: PdfCacheLimits) -> Self {
        Self { limits, inner: Mutex::new((VecDeque::new(), std::collections::HashMap::new())) }
    }

    pub fn limits(&self) -> PdfCacheLimits {
        self.limits
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.0.clear();
        inner.1.clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).1.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, key: &str) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).1.contains_key(key)
    }

    /// 테스트와 예산 검증용으로 항목을 직접 넣는다.
    pub fn insert_raw(&self, key: &str, mtime: i64, pages: i64, size: u64) {
        let reader = Arc::new(PdfReaderHandle { path: std::path::PathBuf::from(key), page_count: pages.max(0) as usize });
        self.put(key, ReaderEntry { mtime, reader, pages, size });
    }

    fn put(&self, key: &str, entry: ReaderEntry) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.0.retain(|k| k != key);
        inner.0.push_back(key.to_string());
        inner.1.insert(key.to_string(), entry);
    }

    /// 파일이 바뀌지 않았으면(mtime 일치) 캐시된 리더와 쪽수를 돌려주고 최근 사용으로 올린다. mtime이 바뀐 낡은 항목은 지운다.
    pub(crate) fn get(&self, key: &str, mtime: i64) -> Option<(Arc<PdfReaderHandle>, i64)> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let hit = inner.1.get(key).map(|e| (e.mtime == mtime).then(|| (e.reader.clone(), e.pages)));
        match hit {
            Some(Some(found)) => {
                inner.0.retain(|k| k != key);
                inner.0.push_back(key.to_string());
                Some(found)
            }
            Some(None) => {
                inner.1.remove(key);
                inner.0.retain(|k| k != key);
                None
            }
            None => None,
        }
    }

    /// 한도 안이면 넣는다(파일이 한도보다 크면 넣지 않는다). 넣은 뒤 예산을 넘으면 오래된 것부터 뺀다.
    pub(crate) fn store(&self, key: &str, mtime: i64, reader: Arc<PdfReaderHandle>, pages: i64, size: u64) {
        if self.limits.max_file_bytes.is_some_and(|max| size > max) {
            log::debug!("PDF too large to cache reader ({:.0} MiB): {key}", size as f64 / 1_048_576.0);
            return;
        }
        self.put(key, ReaderEntry { mtime, reader, pages, size });
        self.evict();
    }

    /// 바이트 예산과 개수 상한 안으로 줄인다(LRU).
    pub fn evict(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut total: u64 = inner.1.values().map(|e| e.size).sum();
        while !inner.1.is_empty() && (self.limits.max_bytes.is_some_and(|max| total > max) || self.limits.max_entries.is_some_and(|max| inner.1.len() > max)) {
            let Some(oldest) = inner.0.pop_front() else { break };
            if let Some(entry) = inner.1.remove(&oldest) {
                total -= entry.size.min(total);
            }
        }
    }
}

/// `(경로, mtime ns) -> 쪽수` 캐시. 값이 정수 하나라 비용이 사실상 0이라 항상 둔다. 상한을 넘으면 오래된 것부터 뺀다.
pub struct PageCountCache {
    max: usize,
    inner: Mutex<VecDeque<((String, i64), i64)>>,
}

impl PageCountCache {
    pub const DEFAULT_MAX: usize = 4096;

    pub fn new(max: usize) -> Self {
        Self { max, inner: Mutex::new(VecDeque::new()) }
    }

    pub fn clear(&self) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, key: &(String, i64)) -> bool {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(k, _)| k == key)
    }

    /// 넣거나 값을 바꾸고 최근 사용으로 올린다. 상한 정리는 `evict`가 한다.
    pub fn insert(&self, key: (String, i64), pages: i64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.retain(|(k, _)| *k != key);
        inner.push_back((key, pages));
    }

    /// 최근 사용으로 올린다.
    pub fn touch(&self, key: &(String, i64)) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = inner.iter().position(|(k, _)| k == key)
            && let Some(entry) = inner.remove(pos)
        {
            inner.push_back(entry);
        }
    }

    pub fn get(&self, key: &(String, i64)) -> Option<i64> {
        let hit = self.inner.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| k == key).map(|(_, v)| *v);
        if hit.is_some() {
            self.touch(key);
        }
        hit
    }

    /// 상한을 넘은 만큼 오래된 것부터 뺀다.
    pub fn evict(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while inner.len() > self.max {
            inner.pop_front();
        }
    }
}
