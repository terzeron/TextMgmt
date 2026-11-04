//! 겹치지 않는 임시 파일·디렉터리 이름을 만든다. 다 쓴 파일은 `rename`으로 제자리에 놓는다.

use std::io;
use std::path::{Path, PathBuf};

/// 이 프로세스 안에서 겹치지 않는 임시 경로 이름 조각.
pub fn unique_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// `path`와 같은 디렉토리의 고유한 임시 경로. 다 쓴 뒤 `rename`해야 읽는 쪽이 쓰는 중인 파일을 보지 않는다.
pub fn temp_sibling(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.tmp", unique_token()))
}

/// 지울 때 알아서 정리되는 임시 디렉터리. `tempfile`은 개발 의존이라 직접 만든다.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> io::Result<TempDir> {
        let path = std::env::temp_dir().join(format!("{prefix}{}", unique_token()));
        std::fs::create_dir_all(&path)?;
        Ok(TempDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_siblings_stay_in_the_same_directory() {
        assert_ne!(unique_token(), unique_token());
        let sibling = temp_sibling(Path::new("/a/b/cover.jpg"));
        assert_eq!(sibling.parent(), Some(Path::new("/a/b")));
        let name = sibling.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".cover.jpg.") && name.ends_with(".tmp"), "{name}");
    }

    #[test]
    fn temp_dir_is_created_and_removed_on_drop() {
        let path = {
            let dir = TempDir::new("tm-temp-file-maker-").unwrap();
            assert!(dir.path().is_dir());
            dir.path().to_path_buf()
        };
        assert!(!path.exists());
    }
}
