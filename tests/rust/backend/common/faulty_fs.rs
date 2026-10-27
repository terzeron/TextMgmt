//! 호출을 골라서 실패시키는 `FsOps` 대역. 설계: docs/rust-test-seams.md 3.5.
//! book_manager_1의 `FaultyFs`(훅 9개)를 `sysdeps::FsOps`에 맞춰 모든 테스트 파일이 공유한다.
//! book_manager_2의 전역 가드 `fs_*_override`는 `manager.with_fs(Arc::new(faulty))`로 바뀐다.
//!
//! 가정한 API (아직 구현 전이다): `sysdeps::{FsOps, RealFs}` (`RealFs`는 단위 구조체)
//!
//! 사용: `let fs = FaultyFs::new(); fs.set_rename(fail_on_nth_call(2, "rollback fail")); deps.with_fs(fs.clone())`
//! 훅이 `Some(err)`를 돌려주면 그 오류를 내고, `None`이면 안쪽 `FsOps`(기본 `RealFs`)에 맡긴다.

#![allow(dead_code)]

use std::fs::{File, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sysdeps::{FsOps, RealFs};

pub type PathHook = Arc<dyn Fn(&Path) -> Option<io::Error> + Send + Sync>;
pub type PairHook = Arc<dyn Fn(&Path, &Path) -> Option<io::Error> + Send + Sync>;
pub type FileHook = Arc<dyn Fn(&File) -> Option<io::Error> + Send + Sync>;
pub type LockHook = Arc<dyn Fn(&File, bool) -> Option<io::Error> + Send + Sync>;

#[derive(Default)]
struct Hooks {
    read_dir: Option<PathHook>,
    metadata: Option<PathHook>,
    is_file: Option<PathHook>,
    rename: Option<PairHook>,
    remove_file: Option<PathHook>,
    remove_dir: Option<PathHook>,
    remove_dir_all: Option<PathHook>,
    canonicalize: Option<PathHook>,
    same_file: Option<PairHook>,
    open: Option<PathHook>,
    write: Option<PathHook>,
    disk_free: Option<PathHook>,
    sync_file: Option<FileHook>,
    lock_exclusive: Option<LockHook>,
    /// `disk_free`가 돌려줄 값. 없으면 안쪽 `FsOps`에 맡긴다.
    disk_free_value: Option<u64>,
}

pub struct FaultyFs {
    inner: Arc<dyn FsOps>,
    hooks: Mutex<Hooks>,
    /// (메서드 이름, 첫 경로). 호출 순서대로 쌓는다.
    calls: Mutex<Vec<(String, PathBuf)>>,
}

macro_rules! setters {
    ($($name:ident => $field:ident : $hook:ty),* $(,)?) => {
        $(
            pub fn $name(&self, f: impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static) {
                let hook: $hook = Arc::new(f);
                self.hooks.lock().unwrap().$field = Some(hook);
            }
        )*
    };
}

impl FaultyFs {
    /// 실제 파일 시스템 위에서 훅만 얹는다.
    pub fn new() -> Arc<Self> {
        Self::wrapping(Arc::new(RealFs))
    }

    /// 다른 `FsOps` 위에서 훅을 얹는다.
    pub fn wrapping(inner: Arc<dyn FsOps>) -> Arc<Self> {
        Arc::new(Self { inner, hooks: Mutex::new(Hooks::default()), calls: Mutex::new(Vec::new()) })
    }

    setters! {
        set_read_dir => read_dir: PathHook,
        set_metadata => metadata: PathHook,
        set_is_file => is_file: PathHook,
        set_remove_file => remove_file: PathHook,
        set_remove_dir => remove_dir: PathHook,
        set_remove_dir_all => remove_dir_all: PathHook,
        set_canonicalize => canonicalize: PathHook,
        set_open => open: PathHook,
        set_write => write: PathHook,
        set_disk_free => disk_free: PathHook,
    }

    pub fn set_rename(&self, f: impl Fn(&Path, &Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().rename = Some(Arc::new(f));
    }

    pub fn set_same_file(&self, f: impl Fn(&Path, &Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().same_file = Some(Arc::new(f));
    }

    pub fn set_sync_file(&self, f: impl Fn(&File) -> Option<io::Error> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().sync_file = Some(Arc::new(f));
    }

    /// 두 번째 인자는 `nonblocking`이다. `Some(io::ErrorKind::WouldBlock)` 오류가 `BlockingIOError`에 해당한다.
    pub fn set_lock_exclusive(&self, f: impl Fn(&File, bool) -> Option<io::Error> + Send + Sync + 'static) {
        self.hooks.lock().unwrap().lock_exclusive = Some(Arc::new(f));
    }

    /// `disk_free`가 항상 `bytes`를 돌려주게 한다. `StoreFs.disk_free` 대역이다.
    pub fn set_disk_free_value(&self, bytes: u64) {
        self.hooks.lock().unwrap().disk_free_value = Some(bytes);
    }

    // ── 이전 이름 (book_manager_1 호환). scandir과 read_dir은 read_dir 하나로 합쳤다. ──

    pub fn set_scandir(&self, f: impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.set_read_dir(f);
    }

    pub fn set_stat(&self, f: impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.set_metadata(f);
    }

    pub fn set_unlink(&self, f: impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.set_remove_file(f);
    }

    pub fn set_resolve(&self, f: impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.set_canonicalize(f);
    }

    pub fn set_samefile(&self, f: impl Fn(&Path, &Path) -> Option<io::Error> + Send + Sync + 'static) {
        self.set_same_file(f);
    }

    // ── 호출 기록 ──

    /// 호출 순서대로 (메서드 이름, 첫 경로).
    pub fn calls(&self) -> Vec<(String, PathBuf)> {
        self.calls.lock().unwrap().clone()
    }

    /// 한 메서드의 첫 경로 목록.
    pub fn calls_of(&self, method: &str) -> Vec<PathBuf> {
        self.calls.lock().unwrap().iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls_of(method).len()
    }

    fn note(&self, method: &str, path: &Path) {
        self.calls.lock().unwrap().push((method.to_string(), path.to_path_buf()));
    }

    fn fault1(&self, method: &str, path: &Path, pick: impl FnOnce(&Hooks) -> Option<PathHook>) -> io::Result<()> {
        self.note(method, path);
        // 훅 안에서 다시 FaultyFs를 불러도 막히지 않게 락 밖에서 부른다.
        let hook = pick(&self.hooks.lock().unwrap());
        match hook.and_then(|h| h(path)) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn fault2(&self, method: &str, a: &Path, b: &Path, pick: impl FnOnce(&Hooks) -> Option<PairHook>) -> io::Result<()> {
        self.note(method, a);
        let hook = pick(&self.hooks.lock().unwrap());
        match hook.and_then(|h| h(a, b)) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

impl FsOps for FaultyFs {
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        self.fault1("read_dir", p, |h| h.read_dir.clone())?;
        self.inner.read_dir(p)
    }

    fn metadata(&self, p: &Path) -> io::Result<Metadata> {
        self.fault1("metadata", p, |h| h.metadata.clone())?;
        self.inner.metadata(p)
    }

    fn is_file(&self, p: &Path) -> io::Result<bool> {
        self.fault1("is_file", p, |h| h.is_file.clone())?;
        self.inner.is_file(p)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.fault2("rename", from, to, |h| h.rename.clone())?;
        self.inner.rename(from, to)
    }

    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.fault1("remove_file", p, |h| h.remove_file.clone())?;
        self.inner.remove_file(p)
    }

    fn remove_dir(&self, p: &Path) -> io::Result<()> {
        self.fault1("remove_dir", p, |h| h.remove_dir.clone())?;
        self.inner.remove_dir(p)
    }

    fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        self.fault1("remove_dir_all", p, |h| h.remove_dir_all.clone())?;
        self.inner.remove_dir_all(p)
    }

    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        self.fault1("canonicalize", p, |h| h.canonicalize.clone())?;
        self.inner.canonicalize(p)
    }

    fn same_file(&self, a: &Path, b: &Path) -> io::Result<bool> {
        self.fault2("same_file", a, b, |h| h.same_file.clone())?;
        self.inner.same_file(a, b)
    }

    fn open(&self, p: &Path) -> io::Result<File> {
        self.fault1("open", p, |h| h.open.clone())?;
        self.inner.open(p)
    }

    fn write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        self.fault1("write", p, |h| h.write.clone())?;
        self.inner.write(p, data)
    }

    fn disk_free(&self, dir: &Path) -> io::Result<u64> {
        self.fault1("disk_free", dir, |h| h.disk_free.clone())?;
        let fixed = self.hooks.lock().unwrap().disk_free_value;
        match fixed {
            Some(bytes) => Ok(bytes),
            None => self.inner.disk_free(dir),
        }
    }

    fn sync_file(&self, f: &File) -> io::Result<()> {
        self.note("sync_file", Path::new(""));
        let hook = self.hooks.lock().unwrap().sync_file.clone();
        if let Some(e) = hook.and_then(|h| h(f)) {
            return Err(e);
        }
        self.inner.sync_file(f)
    }

    fn lock_exclusive(&self, f: &File, nonblocking: bool) -> io::Result<()> {
        self.note("lock_exclusive", Path::new(""));
        let hook = self.hooks.lock().unwrap().lock_exclusive.clone();
        if let Some(e) = hook.and_then(|h| h(f, nonblocking)) {
            return Err(e);
        }
        self.inner.lock_exclusive(f, nonblocking)
    }
}

// ── 훅 만들기 ──

/// `io::Error::other(msg)`.
pub fn other_err(msg: &str) -> io::Error {
    io::Error::other(msg.to_string())
}

/// 첫 n-1번째 호출은 통과시키고 n번째 호출만 실패시키는 두 경로 훅(`rename`, `same_file`용).
pub fn fail_on_nth_call(n: usize, msg: &'static str) -> impl Fn(&Path, &Path) -> Option<io::Error> + Send + Sync + 'static {
    let calls = Mutex::new(0usize);
    move |_from, _to| {
        let mut c = calls.lock().unwrap();
        *c += 1;
        if *c == n {
            Some(io::Error::other(msg))
        } else {
            None
        }
    }
}

/// `fail_on_nth_call`의 한 경로 판.
pub fn fail_on_nth_path(n: usize, msg: &'static str) -> impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static {
    let calls = Mutex::new(0usize);
    move |_path| {
        let mut c = calls.lock().unwrap();
        *c += 1;
        if *c == n {
            Some(io::Error::other(msg))
        } else {
            None
        }
    }
}

/// 경로 문자열에 `needle`이 들어 있을 때만 `kind` 오류를 낸다.
pub fn fail_if_path_contains(needle: &'static str, kind: io::ErrorKind) -> impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static {
    move |path| if path.to_string_lossy().contains(needle) { Some(io::Error::new(kind, format!("주입한 오류: {needle}"))) } else { None }
}

/// 모든 호출에 `kind` 오류를 낸다.
pub fn fail_always(kind: io::ErrorKind, msg: &'static str) -> impl Fn(&Path) -> Option<io::Error> + Send + Sync + 'static {
    move |_path| Some(io::Error::new(kind, msg))
}
