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
