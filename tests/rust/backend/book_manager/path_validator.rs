//! `backend::book_manager::path_validator` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{deps, make_manager, tmp_dir};
use backend::book_manager::BookManager;
use common::es_fake::FakeEsManager;
use common::faulty_fs::{FaultyFs, other_err};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};
use sysdeps::{FsOps, RealFs};

type ListDirFn = dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync;

// TODO(common): `FaultyFs`는 실패만 주입하고 `read_dir` 결과를 바꾸지 못한다. `FaultyFs::set_read_dir_result(Fn(&Path) -> Vec<PathBuf>)`가 생기면 지운다.
/// 구 `fs_iterdir_override`: `read_dir`만 고정된 목록을 돌려주고 나머지는 안쪽 `FsOps`에 맡긴다.
struct ListDirFs {
    inner: Arc<dyn FsOps>,
    listing: Box<ListDirFn>,
}

impl ListDirFs {
    fn new(listing: impl Fn(&Path) -> Vec<PathBuf> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { inner: Arc::new(RealFs), listing: Box::new(listing) })
    }
}

impl FsOps for ListDirFs {
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        Ok((self.listing)(p))
    }

    fn metadata(&self, p: &Path) -> io::Result<fs::Metadata> {
        self.inner.metadata(p)
    }

    fn is_file(&self, p: &Path) -> io::Result<bool> {
        self.inner.is_file(p)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.inner.rename(from, to)
    }

    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.inner.remove_file(p)
    }

    fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        self.inner.remove_dir_all(p)
    }

    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        self.inner.canonicalize(p)
    }

    fn same_file(&self, a: &Path, b: &Path) -> io::Result<bool> {
        self.inner.same_file(a, b)
    }

    fn open(&self, p: &Path) -> io::Result<fs::File> {
        self.inner.open(p)
    }

    fn write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        self.inner.write(p, data)
    }

    fn disk_free(&self, dir: &Path) -> io::Result<u64> {
        self.inner.disk_free(dir)
    }

    fn sync_file(&self, f: &fs::File) -> io::Result<()> {
        self.inner.sync_file(f)
    }

    fn lock_exclusive(&self, f: &fs::File, nonblocking: bool) -> io::Result<()> {
        self.inner.lock_exclusive(f, nonblocking)
    }
}

#[test]
fn test_normalize_stored_file_path_all_branches() {
    // Lines 1217, 1223-1224, 1228, 1230
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());

    // 1217: empty
    assert_eq!(manager._normalize_stored_file_path(""), "");

    // 1223-1224: absolute path outside prefix
    let res = manager._normalize_stored_file_path("/some/other/non_relative_root/file.txt");
    assert_eq!(res, "some/other/non_relative_root/file.txt");

    // 1228: normalized == "."
    assert_eq!(manager._normalize_stored_file_path("."), "");

    // 1230: normalized starts with ./
    assert_eq!(manager._normalize_stored_file_path("./A/file.txt"), "A/file.txt");
}

#[test]
fn test_is_safe_category_name_oserror() {
    // Lines 1244-1245: OSError in _is_safe_category_name
    let tmp = tmp_dir();
    // 경로 정규화(resolve = canonicalize) 실패 훅. 이름에 "bad_oserror"가 든 경로만 실패한다.
    let faulty = FaultyFs::new();
    faulty.set_canonicalize(|p| p.to_string_lossy().contains("bad_oserror").then(|| other_err("permission denied")));
    let manager = BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(faulty));
    assert!(!manager._is_safe_category_name("bad_oserror"));
}

#[test]
fn test_is_indexable_file_path_hidden() {
    // Line 1249: dot file
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    assert!(!manager._is_indexable_file_path(Path::new(".hidden.epub")));
}

#[test]
fn test_iter_category_indexable_files_branches_shadowed() {
    // 원본에는 같은 이름의 함수가 뒤에 하나 더 있어 Python에서는 이 정의가 가려진다(pytest는 뒤의 것만 실행한다).
    // Rust는 한 모듈에 같은 이름을 둘 수 없어 앞쪽 정의에 _shadowed를 붙였다.
    // Lines 1350-1353, 1359-1360
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let faulty = FaultyFs::new();
    faulty.set_canonicalize(|p| p.to_string_lossy().contains("valid.txt").then(|| other_err("resolve failure")));
    let manager = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_fs(faulty));
    let cat_dir = tmp_path.join("A");
    fs::create_dir_all(&cat_dir).unwrap();
    fs::create_dir_all(cat_dir.join(".hidden_dir")).unwrap();
    fs::write(cat_dir.join(".hidden_dir").join("ignored.txt"), "x").unwrap();
    let normal_file = cat_dir.join("valid.txt");
    fs::write(&normal_file, "y").unwrap();

    let files = manager._iter_category_indexable_files("A", true);
    assert_eq!(files.len(), 0); // .hidden_dir ignored (1353), valid.txt resolve OSError (1359-1360)
}

#[test]
fn test_normalize_stored_file_path_dot_slash() {
    // Line 1230
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    assert_eq!(manager._normalize_stored_file_path("./foo/bar.txt"), "foo/bar.txt");
}

#[test]
fn test_iter_category_indexable_files_branches() {
    // Lines 1350-1351, 1353, 1358, 1359-1360
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let cat_dir = tmp_path.join("cat");
    fs::create_dir_all(&cat_dir).unwrap();

    // Line 1353: hidden file
    fs::write(cat_dir.join(".hidden.txt"), "x").unwrap();

    // 1. Lines 1350-1351: relative_to raises ValueError
    // 2. Line 1358: resolve not relative to root
    let outside_file = PathBuf::from("/tmp/outside_dummy.txt");
    let hidden = cat_dir.join(".hidden.txt");
    // iterdir 결과를 고정한 `FsOps`(구 `fs_iterdir_override`). 단계마다 다른 목록이라 매니저를 새로 만든다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_fs(ListDirFs::new(move |_dir| vec![outside_file.clone(), hidden.clone()])));
    let files = manager._iter_category_indexable_files("cat", false);
    assert_eq!(files, Vec::<PathBuf>::new());

    // Lines 1359-1360: resolve raises OSError
    let normal_file = cat_dir.join("normal.txt");
    fs::write(&normal_file, "x").unwrap();
    let nf = normal_file.clone();
    let nf2 = normal_file.clone();
    let faulty = FaultyFs::wrapping(ListDirFs::new(move |_dir| vec![nf2.clone()]));
    faulty.set_canonicalize(move |p| (p == nf).then(|| other_err("resolve failed")));
    let manager2 = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
    let files2 = manager2._iter_category_indexable_files("cat", false);
    assert_eq!(files2, Vec::<PathBuf>::new());
}

mod stored_path_normalization {
    use super::common;
    use crate::common::dyn_es::make_manager;
    use crate::common::shared_fixtures::category_with_files;

    use std::fs;

    use common::es_fake::FakeEsManager;

    use tempfile::TempDir;

    #[test]
    fn test_normalize_stored_file_path_strips_leading_dot_slash() {
        let tmp = TempDir::new().unwrap();
        let manager = make_manager(tmp.path(), FakeEsManager::new());
        assert_eq!(manager._normalize_stored_file_path("./A/x.txt"), "A/x.txt");
    }

    // ---- 카테고리 파일 열거 ----

    #[test]
    fn test_iter_category_files_skips_subdirectories_and_outside_symlinked_category() {
        let tmp = TempDir::new().unwrap();
        let manager = make_manager(tmp.path(), FakeEsManager::new());
        category_with_files(tmp.path(), "A", &["keep.txt"]);
        fs::create_dir(tmp.path().join("A").join("sub")).unwrap();

        let names: Vec<String> = manager._iter_category_indexable_files("A", false).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["keep.txt"]);

        // 같은 부모 디렉토리 아래에 corpus 밖 디렉토리를 만든다(부모는 별도 TempDir로 둔다).
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("leak.txt"), "밖").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("L")).unwrap();
        assert!(manager._iter_category_indexable_files("L", false).is_empty());
    }
}
