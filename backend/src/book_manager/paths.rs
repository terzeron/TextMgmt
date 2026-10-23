//! 경로 검증과 파일 종류 판정. Python `BookManager`의 `_is_safe_category_name`, `_normalize_stored_file_path`,
//! `_is_indexable_file_path` 등에 대응한다.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use super::BookManager;
use super::util::{posix_normpath, relative_to};

/// 색인 대상 확장자.
pub const INDEXABLE_FILE_TYPES: [&str; 17] = ["txt", "epub", "pdf", "docx", "doc", "hwp", "rtf", "html", "jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "svg", "cbz"];

fn indexable_set() -> &'static BTreeSet<&'static str> {
    static SET: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| INDEXABLE_FILE_TYPES.iter().copied().collect())
}

/// 이름만 보고 판정한 색인 대상 확장자인가.
pub fn is_indexable_type(file_type: &str) -> bool {
    indexable_set().contains(file_type)
}

/// 확장자 소문자(점 없음). 확장자가 없으면 빈 문자열.
pub fn declared_type(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_lowercase(),
        None => String::new(),
    }
}

impl BookManager {
    /// `Path.resolve(strict=False)`: 있는 부분은 심볼릭 링크를 풀고, 없는 부분은 그대로 이어 붙인다.
    pub(crate) fn resolve_path(&self, path: &Path) -> io::Result<PathBuf> {
        let components: Vec<Component> = path.components().collect();
        for n in (0..=components.len()).rev() {
            let prefix: PathBuf = if n == 0 { PathBuf::from(".") } else { components[..n].iter().collect() };
            match self.fs.canonicalize(&prefix) {
                Ok(mut base) => {
                    for component in &components[n..] {
                        match component {
                            Component::ParentDir => {
                                base.pop();
                            }
                            Component::CurDir => {}
                            other => base.push(other.as_os_str()),
                        }
                    }
                    return Ok(base);
                }
                Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(path.to_path_buf())
    }

    /// 책 루트를 풀어 쓴 경로. 실패하면 그대로다.
    pub(crate) fn resolved_root(&self) -> io::Result<PathBuf> {
        self.resolve_path(&self.path_prefix)
    }

    /// `path.resolve().is_relative_to(path_prefix.resolve())`.
    pub(crate) fn is_inside_root(&self, path: &Path) -> io::Result<bool> {
        let resolved = self.resolve_path(path)?;
        let root = self.resolved_root()?;
        Ok(resolved.starts_with(root))
    }

    /// `_normalize_stored_file_path`: ES에 저장된 경로를 책 루트 기준 상대 경로로 맞춘다.
    pub(crate) fn normalize_stored_file_path(&self, file_path: &str) -> String {
        if file_path.is_empty() {
            return String::new();
        }
        let path = Path::new(file_path);
        if path.is_absolute() {
            let relative = self.resolve_path(path).and_then(|p| self.resolved_root().map(|root| (p, root))).ok().and_then(|(p, root)| relative_to(&p, &root));
            return relative.unwrap_or_else(|| file_path.trim_start_matches('/').to_string());
        }
        let normalized = posix_normpath(&file_path.replace('\\', "/"));
        if normalized == "." { String::new() } else { normalized }
    }

    /// `_is_safe_category_name`.
    pub(crate) fn is_safe_category_name(&self, category: &str) -> bool {
        if category == Self::ROOT_CATEGORY {
            return true;
        }
        if category.is_empty() || category.contains("..") || category.contains('\0') {
            return false;
        }
        match (self.resolve_path(&self.path_prefix.join(category)), self.resolved_root()) {
            (Ok(path), Ok(root)) => path.starts_with(root),
            _ => false,
        }
    }

    /// `_is_indexable_file_path`: 확장자로 판정하고, 안 되면 매직바이트로 판정한다(로더와 같은 규칙).
    pub(crate) fn is_indexable_file_path(&self, file_path: &Path) -> bool {
        let name = file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            return false;
        }
        let declared = declared_type(&name);
        if is_indexable_type(&declared) {
            return true;
        }
        super::file_reader::detect_file_type(&*self.fs, file_path, &declared).is_some_and(|t| is_indexable_type(&t))
    }

    /// `_category_dir`.
    pub(crate) fn category_dir(&self, category: &str) -> PathBuf {
        if category == Self::ROOT_CATEGORY { self.path_prefix.clone() } else { self.path_prefix.join(category) }
    }

    /// `_is_top_level_target_category`.
    pub(crate) fn is_top_level_target_category(category: &str) -> bool {
        !category.is_empty() && category != Self::ROOT_CATEGORY && !category.contains('/')
    }

    /// 심볼릭 링크를 따라가지 않고 보통 파일인가.
    pub(crate) fn is_regular_file_nofollow(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
    }

    /// 심볼릭 링크를 따라가지 않고 디렉터리인가.
    pub(crate) fn is_dir_nofollow(path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir())
    }

    /// 파일 이름 조각.
    pub(crate) fn file_name_of(path: &Path) -> OsString {
        path.file_name().map(|n| n.to_os_string()).unwrap_or_default()
    }

    /// `shutil.move`: 이름 바꾸기가 안 되면(다른 장치) 복사 뒤 지운다.
    pub(crate) fn move_file(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self.fs.rename(from, to) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                std::fs::copy(from, to)?;
                self.fs.remove_file(from)
            }
            Err(e) => Err(e),
        }
    }

    /// `clean_empty_parent_dirs`: 비어 있는 상위 디렉터리를 `stop_dir` 직전까지 지운다. `stop_dir` 자신과 바깥은 지우지 않는다.
    pub(crate) fn clean_empty_parent_dirs(&self, parent_dir: &Path, stop_dir: &Path) {
        let (Ok(mut current), Ok(stop)) = (self.resolve_path(parent_dir), self.resolve_path(stop_dir)) else { return };
        while current != stop && current.starts_with(&stop) && current.is_dir() {
            match std::fs::read_dir(&current) {
                Ok(mut it) => {
                    if it.next().is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
            if std::fs::remove_dir(&current).is_err() {
                break;
            }
            if !current.pop() {
                break;
            }
        }
    }
}
