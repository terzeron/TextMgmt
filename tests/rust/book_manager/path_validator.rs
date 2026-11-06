//! `book_manager::path_validator` 단위 테스트.

#[path = "common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{deps, make_manager, tmp_dir};
use book_manager::BookManager;
use common::es_fake::FakeEsManager;
use common::faulty_fs::{FaultyFs, other_err};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};
use sysdeps::{FsOps, RealFs};
use tempfile::TempDir;

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
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());

    // 1217: empty
    assert_eq!(manager.normalize_stored_file_path(""), "");

    // 1223-1224: absolute path outside prefix
    let res = manager.normalize_stored_file_path("/some/other/non_relative_root/file.txt");
    assert_eq!(res, "some/other/non_relative_root/file.txt");

    // 1228: normalized == "."
    assert_eq!(manager.normalize_stored_file_path("."), "");

    // 1230: normalized starts with ./
    assert_eq!(manager.normalize_stored_file_path("./A/file.txt"), "A/file.txt");
}

#[test]
fn test_is_safe_category_name_oserror() {
    // OSError in is_safe_category_name
    let tmp = tmp_dir();
    // 경로 정규화(resolve = canonicalize) 실패 훅. 이름에 "bad_oserror"가 든 경로만 실패한다.
    let faulty = FaultyFs::new();
    faulty.set_canonicalize(|p| p.to_string_lossy().contains("bad_oserror").then(|| other_err("permission denied")));
    let manager = BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(faulty));
    assert!(!manager.is_safe_category_name("bad_oserror"));
}

#[test]
fn test_is_indexable_file_path_hidden() {
    // dot file
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    assert!(!manager.is_indexable_file_path(Path::new(".hidden.epub")));
}

#[test]
fn test_iter_category_indexable_files_branches_shadowed() {
    // 같은 이름의 함수가 뒤에 하나 더 있어 이 정의는 쓰이지 않는다.
    // Rust는 한 모듈에 같은 이름을 둘 수 없어 앞쪽 정의에 _shadowed를 붙였다.
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

    let files = manager.iter_category_indexable_files("A", true);
    assert_eq!(files.len(), 0); // .hidden_dir ignored (1353), valid.txt resolve OSError (1359-1360)
}

#[test]
fn test_normalize_stored_file_path_dot_slash() {
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    assert_eq!(manager.normalize_stored_file_path("./foo/bar.txt"), "foo/bar.txt");
}

#[test]
fn test_iter_category_indexable_files_branches() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let cat_dir = tmp_path.join("cat");
    fs::create_dir_all(&cat_dir).unwrap();

    // hidden file
    fs::write(cat_dir.join(".hidden.txt"), "x").unwrap();

    // 1. relative_to raises CoverSourceError
    // 2. resolve not relative to root
    let outside_file = PathBuf::from("/tmp/outside_dummy.txt");
    let hidden = cat_dir.join(".hidden.txt");
    // iterdir 결과를 고정한 `FsOps`(구 `fs_iterdir_override`). 단계마다 다른 목록이라 매니저를 새로 만든다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_fs(ListDirFs::new(move |_dir| vec![outside_file.clone(), hidden.clone()])));
    let files = manager.iter_category_indexable_files("cat", false);
    assert_eq!(files, Vec::<PathBuf>::new());

    // resolve raises OSError
    let normal_file = cat_dir.join("normal.txt");
    fs::write(&normal_file, "x").unwrap();
    let nf = normal_file.clone();
    let nf2 = normal_file.clone();
    let faulty = FaultyFs::wrapping(ListDirFs::new(move |_dir| vec![nf2.clone()]));
    faulty.set_canonicalize(move |p| (p == nf).then(|| other_err("resolve failed")));
    let manager2 = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
    let files2 = manager2.iter_category_indexable_files("cat", false);
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
        assert_eq!(manager.normalize_stored_file_path("./A/x.txt"), "A/x.txt");
    }

    // ---- 카테고리 파일 열거 ----

    #[test]
    fn test_iter_category_files_skips_subdirectories_and_outside_symlinked_category() {
        let tmp = TempDir::new().unwrap();
        let manager = make_manager(tmp.path(), FakeEsManager::new());
        category_with_files(tmp.path(), "A", &["keep.txt"]);
        fs::create_dir(tmp.path().join("A").join("sub")).unwrap();

        let names: Vec<String> = manager.iter_category_indexable_files("A", false).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["keep.txt"]);

        // 같은 부모 디렉토리 아래에 corpus 밖 디렉토리를 만든다(부모는 별도 TempDir로 둔다).
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("leak.txt"), "밖").unwrap();
        std::os::unix::fs::symlink(outside.path(), tmp.path().join("L")).unwrap();
        assert!(manager.iter_category_indexable_files("L", false).is_empty());
    }
}

// ---- clean_empty_parent_dirs (분류기 껍데기 정리 때 classifier 테스트에서 옮겼다) ----

#[test]
fn test_clean_empty_parent_dirs_removes_empty_dirs_until_stop_dir() {
    let tmp = TempDir::new().unwrap();
    let stop_dir = tmp.path().join("library");
    let empty_child = stop_dir.join("series").join("volume");
    fs::create_dir_all(&empty_child).unwrap();

    make_manager(tmp.path(), &FakeEsManager::new()).clean_empty_parent_dirs(&empty_child, &stop_dir);

    assert!(stop_dir.exists());
    assert!(!stop_dir.join("series").exists());
}

#[test]
fn test_clean_empty_parent_dirs_stops_when_directory_has_visible_items() {
    let tmp = TempDir::new().unwrap();
    let stop_dir = tmp.path().join("library");
    let child = stop_dir.join("series");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("keep.txt"), "content").unwrap();

    make_manager(tmp.path(), &FakeEsManager::new()).clean_empty_parent_dirs(&child, &stop_dir);

    assert!(child.exists());
}

/// rmdir은 숨김 파일까지 봐야 성공한다. 보이는 항목만 세면 코드와 실제 동작이 어긋난다.
#[test]
fn test_clean_empty_parent_dirs_keeps_dir_holding_only_hidden_files() {
    let tmp = TempDir::new().unwrap();
    let stop_dir = tmp.path().join("library");
    let child = stop_dir.join("series");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join(".nomedia"), "").unwrap();

    make_manager(tmp.path(), &FakeEsManager::new()).clean_empty_parent_dirs(&child, &stop_dir);

    assert!(child.exists());
}

/// stop_dir를 정규화 없이 비교하면 못 알아보고 조상까지 거슬러 올라간다.
#[test]
fn test_clean_empty_parent_dirs_never_climbs_past_stop_dir() {
    let tmp = TempDir::new().unwrap();
    let stop_dir = tmp.path().join("library");
    let child = stop_dir.join("series");
    fs::create_dir_all(&child).unwrap();
    // 같은 디렉토리를 '..'를 낀 다른 표기로 넘긴다.
    let unnormalized_stop = stop_dir.join("series").join("..");

    make_manager(tmp.path(), &FakeEsManager::new()).clean_empty_parent_dirs(&child, &unnormalized_stop);

    assert!(stop_dir.exists());
    assert!(tmp.path().exists());
}

/// 호출자가 문자열을 넘기면 Path와 절대 같아지지 않아 stop_dir를 지나친다.
#[test]
fn test_clean_empty_parent_dirs_accepts_str_stop_dir() {
    let tmp = TempDir::new().unwrap();
    let stop_dir = tmp.path().join("library");
    let child = stop_dir.join("series");
    fs::create_dir_all(&child).unwrap();

    // `&str` 경로도 `AsRef<Path>`로 받는다.
    make_manager(tmp.path(), &FakeEsManager::new()).clean_empty_parent_dirs(&child, Path::new(stop_dir.to_str().unwrap()));

    assert!(stop_dir.exists());
    assert!(!child.exists());
}

#[test]
fn test_clean_empty_parent_dirs_ignores_unresolvable_path() {
    let tmp = TempDir::new().unwrap();
    let child = tmp.path().join("a");
    fs::create_dir(&child).unwrap();

    // 가정한 seam: 경로 해석(`FsOps::canonicalize`)을 `common::FaultyFs`로 실패시킨다. 해석이 실패하면 아무것도 지우지 않아야 한다.
    let faulty = FaultyFs::new();
    faulty.set_resolve(|_p| Some(other_err("경로 해석 실패")));

    BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(faulty)).clean_empty_parent_dirs(&child, tmp.path());

    assert!(child.exists());
}

#[test]
fn test_clean_empty_parent_dirs_stops_when_remove_fails() {
    let tmp = TempDir::new().unwrap();
    let stop = tmp.path().join("source");
    let child = stop.join("empty");
    fs::create_dir_all(&child).unwrap();
    let faulty = FaultyFs::new();
    faulty.set_remove_dir(|_| Some(other_err("permission denied")));

    BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(faulty)).clean_empty_parent_dirs(&child, &stop);

    assert!(child.exists(), "failed removal must leave the directory in place");
    assert!(stop.exists(), "cleanup must never remove the stop directory");
}

/// 디렉터리를 읽지 못하면 아무것도 지우지 않는다.
#[test]
fn test_clean_empty_parent_dirs_keeps_dirs_it_cannot_read() {
    let tmp = TempDir::new().unwrap();
    let sub = tmp.path().join("a").join("b");
    fs::create_dir_all(&sub).unwrap();
    let busy = FaultyFs::new();
    busy.set_scandir(|_| Some(other_err("Busy")));
    BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(busy)).clean_empty_parent_dirs(&sub, tmp.path());
    assert!(sub.exists());
}

#[test]
fn test_clean_empty_parent_dirs_tolerates_paths_that_do_not_exist() {
    let tmp = tmp_dir();
    let stop = tmp.path().join("stop");
    fs::create_dir(&stop).unwrap();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());

    // 없는 경로는 가까운 조상을 풀어 이어 붙이며, `..`는 한 단계 올라간다.
    manager.clean_empty_parent_dirs(&stop.join("x/../y"), &stop);
    manager.clean_empty_parent_dirs(&stop.join("x/y"), &stop);
    // 존재하는 조상이 하나도 없는 상대 경로도 그대로 해석한다.
    manager.clean_empty_parent_dirs(std::path::Path::new("no-such-dir-xyz/a"), std::path::Path::new("no-such-dir-xyz"));

    assert!(stop.is_dir(), "stop_dir 자신은 지우지 않는다");
}
