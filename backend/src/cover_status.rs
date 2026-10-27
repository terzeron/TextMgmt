//! 정식 표지 판정과 공유 MySQL cache.
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Mutex;

use crate::db::{ConnectionProvider, DbValue, MysqlConfig, mysql_provider};
use lopdf::{Document, Object};

pub const STANDARD_COVER_SIZES: &[(u32, u32)] = &[(480, 623), (750, 824), (202, 164)];

/// 첫 쪽에서 실제로 그리는 첫 이미지의 픽셀 크기. Form 안쪽은 대상이 아니다.
pub fn pdf_first_image_size(path: &Path, lock: &Mutex<()>) -> Result<Option<(u32, u32)>, String> {
    let _guard = lock.lock().map_err(|e| e.to_string())?;
    let pdf = Document::load(path).map_err(|e| e.to_string())?;
    let Some(&id) = pdf.get_pages().values().next() else { return Ok(None) };
    // Resources는 페이지 트리에서 상속된다. 가장 가까운 정의를 쓴다.
    let mut current = id;
    let mut visited = BTreeSet::new();
    let resources = loop {
        if !visited.insert(current) {
            return Err("PDF 페이지 트리가 순환합니다".into());
        }
        let dict = pdf.get_dictionary(current).map_err(|e| e.to_string())?;
        if let Ok(resources) = dict.get(b"Resources") {
            break Some(pdf.dereference(resources).map_err(|e| e.to_string())?.1.as_dict().map_err(|e| e.to_string())?);
        }
        let Ok(parent) = dict.get(b"Parent") else { break None };
        current = parent.as_reference().map_err(|e| e.to_string())?;
    };
    let xobjects = resources.and_then(|r| r.get(b"XObject").ok()).map(|v| pdf.dereference(v)?.1.as_dict()).transpose().map_err(|e| e.to_string())?;
    let content = pdf.get_page_content_with_limit(id, 16 * 1024 * 1024).map_err(|e| e.to_string())?;
    let operations = lopdf::content::Content::decode(&content).map_err(|e| e.to_string())?;
    for op in operations.operations {
        let stream = match op.operator.as_str() {
            "BI" => op.operands.first().ok_or("PDF inline 이미지가 없습니다")?.as_stream().map_err(|e| e.to_string())?,
            "Do" if op.operands.len() == 1 => {
                let name = op.operands[0].as_name().map_err(|e| e.to_string())?;
                let object = xobjects.ok_or("PDF XObject 리소스가 없습니다")?.get(name).map_err(|e| e.to_string())?;
                let stream = pdf.dereference(object).map_err(|e| e.to_string())?.1.as_stream().map_err(|e| e.to_string())?;
                if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image") {
                    continue;
                }
                stream
            }
            _ => continue,
        };
        let dimension = |key: &[u8], short: &[u8]| -> Result<u32, String> {
            let value = stream.dict.get(key).or_else(|e| if op.operator == "BI" { stream.dict.get(short) } else { Err(e) }).map_err(|e| e.to_string())?;
            let n = pdf.dereference(value).map_err(|e| e.to_string())?.1.as_i64().map_err(|e| e.to_string())?;
            let n = u32::try_from(n).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("PDF 이미지 크기가 0입니다".into());
            }
            Ok(n)
        };
        return Ok(Some((dimension(b"Width", b"W")?, dimension(b"Height", b"H")?)));
    }
    Ok(None)
}

pub fn has_official_cover(path: &Path, file_type: &str, lock: &Mutex<()>) -> Result<bool, String> {
    match file_type {
        "pdf" => Ok(pdf_first_image_size(path, lock)?.is_some_and(|size| STANDARD_COVER_SIZES.contains(&size))),
        "epub" => crate::epub_cover::inspect_epub(path).map(|v| v["has_cover"].as_bool().unwrap_or(false)).map_err(|e| e.to_string()),
        _ => Ok(true),
    }
}

pub struct CoverStatusCache {
    provider: ConnectionProvider,
    ready: Mutex<bool>,
}

impl CoverStatusCache {
    pub fn new() -> Self {
        Self::with_connection_provider(mysql_provider(MysqlConfig::from_env(&|key| std::env::var(key).ok(), "textmanager")))
    }

    pub fn with_connection_provider(provider: ConnectionProvider) -> Self {
        Self { provider, ready: Mutex::new(false) }
    }

    fn connect(&self) -> Result<Box<dyn crate::db::DbConnection>, String> {
        let mut ready = self.ready.lock().map_err(|e| e.to_string())?;
        let mut connection = (self.provider)().map_err(|e| e.to_string())?;
        if !*ready {
            connection
                .execute(
                    "CREATE TABLE IF NOT EXISTS cover_status (file_path_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin NOT NULL PRIMARY KEY, file_path TEXT NOT NULL, mtime_ns BIGINT NOT NULL, size BIGINT UNSIGNED NOT NULL, has_cover BOOLEAN NOT NULL) ENGINE=InnoDB",
                    &[],
                )
                .map_err(|e| e.to_string())?;
            connection.commit().map_err(|e| e.to_string())?;
            *ready = true;
        }
        Ok(connection)
    }

    pub fn get_or_compute(&self, path: &Path, compute: &dyn Fn() -> Result<bool, String>) -> Result<bool, String> {
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let key = path.to_str().ok_or("파일 경로가 UTF-8이 아닙니다")?;
        let modified = metadata.mtime().checked_mul(1_000_000_000).and_then(|n| n.checked_add(metadata.mtime_nsec())).ok_or("파일 시각이 범위를 벗어났습니다")?;
        let size = i64::try_from(metadata.len()).map_err(|e| e.to_string())?;
        let mut connection = self.connect()?;
        let row = connection.fetch_one("SELECT mtime_ns,size,has_cover FROM cover_status WHERE file_path_hash=SHA2(%s,256)", &[DbValue::from(key)]).map_err(|e| e.to_string())?;
        if let Some(row) = row
            && let (Some(mtime), Some(bytes), Some(covered)) =
                (row.get("mtime_ns").and_then(DbValue::as_i64), row.get("size").and_then(DbValue::as_i64), row.get("has_cover").and_then(DbValue::as_i64))
            && mtime == modified
            && bytes == size
        {
            return Ok(covered != 0);
        }
        let covered = compute()?;
        connection
            .execute(
                "INSERT INTO cover_status (file_path_hash,file_path,mtime_ns,size,has_cover) VALUES (SHA2(%s,256),%s,%s,%s,%s) ON DUPLICATE KEY UPDATE file_path=VALUES(file_path),mtime_ns=VALUES(mtime_ns),size=VALUES(size),has_cover=VALUES(has_cover)",
                &[DbValue::from(key), DbValue::from(key), DbValue::from(modified), DbValue::from(size), DbValue::from(i64::from(covered))],
            )
            .map_err(|e| e.to_string())?;
        connection.commit().map_err(|e| e.to_string())?;
        Ok(covered)
    }
}

impl Default for CoverStatusCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{CoverStatusCache, pdf_first_image_size};
    use crate::db::{ConnectionProvider, DbConnection, DbCursor, DbError, DbValue, Row};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Copy)]
    enum Failure {
        None,
        Create,
        InitializeCommit,
        Select,
        Insert,
    }

    struct EmptyConnection(Failure);

    impl DbConnection for EmptyConnection {
        fn cursor(&mut self) -> Box<dyn DbCursor> {
            Box::new(EmptyCursor(self.0))
        }

        fn commit(&mut self) -> Result<(), DbError> {
            if matches!(self.0, Failure::InitializeCommit) { Err(DbError::Other("commit failed".into())) } else { Ok(()) }
        }

        fn rollback(&mut self) -> Result<(), DbError> {
            Ok(())
        }

        fn close(&mut self) {}
    }

    struct EmptyCursor(Failure);

    impl DbCursor for EmptyCursor {
        fn execute(&mut self, sql: &str, _params: &[DbValue]) -> Result<(), DbError> {
            let message = match (self.0, sql.trim_start()) {
                (Failure::Create, sql) if sql.starts_with("CREATE TABLE") => Some("create failed"),
                (Failure::Select, sql) if sql.starts_with("SELECT") => Some("select failed"),
                (Failure::Insert, sql) if sql.starts_with("INSERT") => Some("insert failed"),
                _ => None,
            };
            message.map_or(Ok(()), |message| Err(DbError::Other(message.into())))
        }

        fn executemany(&mut self, _sql: &str, _seq: &[Vec<DbValue>]) -> Result<(), DbError> {
            Ok(())
        }

        fn fetchall(&mut self) -> Option<Vec<Row>> {
            Some(vec![])
        }

        fn fetchone(&mut self) -> Option<Row> {
            None
        }

        fn rowcount(&self) -> u64 {
            0
        }
    }

    fn file() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("book.pdf"), b"pdf").unwrap();
        dir
    }

    struct CachedConnection(Row);

    impl DbConnection for CachedConnection {
        fn cursor(&mut self) -> Box<dyn DbCursor> {
            Box::new(CachedCursor(self.0.clone()))
        }

        fn commit(&mut self) -> Result<(), DbError> {
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), DbError> {
            Ok(())
        }

        fn close(&mut self) {}
    }

    struct CachedCursor(Row);

    impl DbCursor for CachedCursor {
        fn execute(&mut self, _sql: &str, _params: &[DbValue]) -> Result<(), DbError> {
            Ok(())
        }

        fn executemany(&mut self, _sql: &str, _seq: &[Vec<DbValue>]) -> Result<(), DbError> {
            Ok(())
        }

        fn fetchall(&mut self) -> Option<Vec<Row>> {
            Some(vec![])
        }

        fn fetchone(&mut self) -> Option<Row> {
            Some(self.0.clone())
        }

        fn rowcount(&self) -> u64 {
            0
        }
    }

    #[test]
    fn pdf_first_image_size_reports_a_poisoned_lock() {
        let dir = file();
        let lock = Arc::new(Mutex::new(()));
        let poisoner = lock.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison test lock");
        })
        .join();

        assert!(pdf_first_image_size(&dir.path().join("book.pdf"), &lock).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cache_rejects_non_utf8_paths_before_opening_a_connection() {
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(std::ffi::OsString::from_vec(vec![0xff]));
        std::fs::write(&path, b"book").unwrap();
        let provider: ConnectionProvider = Arc::new(|| panic!("non-UTF8 path must fail first"));
        let cache = CoverStatusCache::with_connection_provider(provider);

        assert!(cache.get_or_compute(&path, &|| panic!("compute must not run")).is_err());
    }

    #[test]
    fn cache_returns_provider_errors_and_does_not_compute() {
        let dir = file();
        let provider: ConnectionProvider = Arc::new(|| Err(DbError::Other("offline".into())));
        let cache = CoverStatusCache::with_connection_provider(provider);

        assert!(cache.get_or_compute(&dir.path().join("book.pdf"), &|| panic!("compute must not run")).unwrap_err().contains("offline"));
    }

    #[test]
    fn cache_hit_returns_the_stored_value_without_recomputing() {
        use std::os::unix::fs::MetadataExt;

        let dir = file();
        let path = dir.path().join("book.pdf");
        let metadata = std::fs::metadata(&path).unwrap();
        let modified = metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec();
        for (stored, expected) in [(0_i64, false), (1, true)] {
            let row: Row = [("mtime_ns".into(), DbValue::from(modified)), ("size".into(), DbValue::from(metadata.len() as i64)), ("has_cover".into(), DbValue::from(stored))].into_iter().collect();
            let provider: ConnectionProvider = Arc::new(move || Ok(Box::new(CachedConnection(row.clone()))));
            let cache = CoverStatusCache::with_connection_provider(provider);

            assert_eq!(cache.get_or_compute(&path, &|| panic!("cache hit must skip computation")).unwrap(), expected);
        }
    }

    #[test]
    fn stale_cache_metadata_triggers_recomputation() {
        let dir = file();
        let path = dir.path().join("book.pdf");
        let row: Row = [("mtime_ns".into(), DbValue::from(0_i64)), ("size".into(), DbValue::from(0_i64)), ("has_cover".into(), DbValue::from(1_i64))].into_iter().collect();
        let provider: ConnectionProvider = Arc::new(move || Ok(Box::new(CachedConnection(row.clone()))));
        let cache = CoverStatusCache::with_connection_provider(provider);
        let computed = std::sync::atomic::AtomicBool::new(false);

        let result = cache
            .get_or_compute(&path, &|| {
                computed.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(false)
            })
            .unwrap();

        assert!(!result);
        assert!(computed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn cache_returns_compute_errors_without_writing_a_row() {
        let dir = file();
        let provider: ConnectionProvider = Arc::new(|| Ok(Box::new(EmptyConnection(Failure::None))));
        let cache = CoverStatusCache::with_connection_provider(provider);

        assert_eq!(cache.get_or_compute(&dir.path().join("book.pdf"), &|| Err("decode failed".into())).unwrap_err(), "decode failed");
    }

    #[test]
    fn cache_retries_initialization_on_the_next_request_after_a_connection_error() {
        let dir = file();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let provider_attempts = attempts.clone();
        let provider: ConnectionProvider =
            Arc::new(
                move || if provider_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 { Err(DbError::Other("offline".into())) } else { Ok(Box::new(EmptyConnection(Failure::None))) },
            );
        let cache = CoverStatusCache::with_connection_provider(provider);

        assert!(cache.get_or_compute(&dir.path().join("book.pdf"), &|| Ok(true)).is_err());
        assert!(cache.get_or_compute(&dir.path().join("book.pdf"), &|| Ok(true)).unwrap());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn cache_propagates_schema_query_and_insert_errors() {
        for (failure, expected) in [(Failure::Create, "create failed"), (Failure::InitializeCommit, "commit failed"), (Failure::Select, "select failed"), (Failure::Insert, "insert failed")] {
            let dir = file();
            let provider: ConnectionProvider = Arc::new(move || Ok(Box::new(EmptyConnection(failure))));
            let cache = CoverStatusCache::with_connection_provider(provider);
            let error = cache.get_or_compute(&dir.path().join("book.pdf"), &|| Ok(true)).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }
}
