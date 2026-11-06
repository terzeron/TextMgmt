//! 책 루트 기준 경로 검증: 안전한 카테고리 이름, 저장 경로 정규화, 색인 대상 파일 판정 등.
//! 파일시스템을 보지 않는 경로·파일 종류 규칙은 `book_record::path_validator`에 있다.

use std::io;
use std::path::{Component, Path, PathBuf};

use super::BookManager;
use book_record::path_validator::{declared_type, detect_file_type, is_indexable_type, relative_to};
use common::zip_reader::zip_entry_name;

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

    /// `normalize_stored_file_path`: ES에 저장된 경로를 책 루트 기준 상대 경로로 맞춘다.
    pub fn normalize_stored_file_path(&self, file_path: &str) -> String {
        if file_path.is_empty() {
            return String::new();
        }
        let path = Path::new(file_path);
        if path.is_absolute() {
            let relative = self.resolve_path(path).and_then(|p| self.resolved_root().map(|root| (p, root))).ok().and_then(|(p, root)| relative_to(&p, &root));
            return relative.unwrap_or_else(|| file_path.trim_start_matches('/').to_string());
        }
        // 책 루트 기준 상대 경로라 zip 항목 이름과 같은 규칙으로 정규화한다. 루트 밖으로 나가면 원문을 남겨 어떤 파일과도 맞지 않게 한다.
        let slashed = file_path.replace('\\', "/");
        zip_entry_name(&slashed).unwrap_or(slashed)
    }

    /// 카테고리 이름 검사. `with_category_name_check`로 바꾼 검사가 파일시스템 오류를 내면 `Err`다.
    pub(crate) fn try_is_safe_category_name(&self, category: &str) -> std::io::Result<bool> {
        match self.seams().category_name_check {
            Some(check) => check(category, &|c| self.is_safe_category_name(c)),
            None => Ok(self.is_safe_category_name(category)),
        }
    }

    /// 카테고리 이름이 안전한가. 루트 카테고리는 늘 안전하다. 그 밖에는 비어 있지 않고 `..`·NUL이 없으며, 실제 경로가 책 루트 안이어야 한다.
    pub fn is_safe_category_name(&self, category: &str) -> bool {
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

    /// `is_indexable_file_path`: 확장자로 판정하고, 안 되면 매직바이트로 판정한다(로더와 같은 규칙).
    pub fn is_indexable_file_path(&self, file_path: &Path) -> bool {
        let name = file_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            return false;
        }
        let declared = declared_type(&name);
        if is_indexable_type(&declared) {
            return true;
        }
        detect_file_type(&*self.fs, file_path, &declared).is_some_and(|t| is_indexable_type(&t))
    }

    /// `_category_dir`.
    pub(crate) fn category_dir(&self, category: &str) -> PathBuf {
        if category == Self::ROOT_CATEGORY { self.path_prefix.clone() } else { self.path_prefix.join(category) }
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
    pub fn clean_empty_parent_dirs(&self, parent_dir: &Path, stop_dir: &Path) {
        clean_empty_parent_dirs_with(parent_dir, stop_dir, self.fs.as_ref());
    }
}

/// 비어 있는 상위 디렉터리를 `stop_dir` 직전까지 지운다. 경로를 해석하지 못하거나 디렉터리를 읽거나 지우지 못하면 멈춘다.
/// 숨김 파일만 있는 디렉터리도 비어 있지 않은 것으로 본다(rmdir과 같다).
fn clean_empty_parent_dirs_with(parent_dir: &Path, stop_dir: &Path, fs: &dyn sysdeps::FsOps) {
    let resolve = |p: &Path| match fs.canonicalize(p) {
        Ok(real) => Some(real),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(resolve_lenient(p)),
        Err(_) => None,
    };
    let (Some(mut curr), Some(stop)) = (resolve(parent_dir), resolve(stop_dir)) else { return };
    while curr != stop && curr.starts_with(&stop) && fs.metadata(&curr).is_ok_and(|metadata| metadata.is_dir()) {
        match fs.read_dir(&curr) {
            Ok(entries) if entries.is_empty() => {}
            _ => break,
        }
        if fs.remove_dir(&curr).is_err() {
            break;
        }
        let Some(parent) = curr.parent() else { break };
        curr = parent.to_path_buf();
    }
}

/// 없는 경로도 해석한다: 존재하는 가장 가까운 조상을 실제 경로로 바꾸고 나머지 조각을 이어 붙인다(`..`는 한 단계 올라간다).
fn resolve_lenient(path: &Path) -> std::path::PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(real) = cur.canonicalize() {
            return tail.iter().rev().fold(real, |acc, c| if c == ".." { acc.parent().map(Path::to_path_buf).unwrap_or(acc) } else { acc.join(c) });
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}
