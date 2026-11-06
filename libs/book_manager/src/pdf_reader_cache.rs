//! 매니저가 소유하는 PDF 캐시: 경로 기준 LRU 리더 캐시(`PdfReaderCache`)와 쪽 수 캐시(`PageCountCache`).
//! PDF 열기·쪽 처리 능력은 `pdf_engine::pdf_page_reader`에 있다.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use pdf_engine::pdf_page_reader::PdfReaderHandle;

/// PDF 리더 캐시 한도. 호환성을 위해 원본 파일 크기를 예산으로 쓴다. `None`은 제한 없음이다.
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
        let reader = Arc::new(PdfReaderHandle::new(std::path::PathBuf::from(key), pages.max(0) as usize));
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
