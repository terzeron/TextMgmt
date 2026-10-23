//! 파일 시스템 보조 도구. `touch`, `set_mtime`, `sha256_hex`, 임시 파일 잔존 검사를 한곳에 모았다.
//! 설계: docs/rust-test-seams.md 5장. 실패 주입은 `faulty_fs.rs`의 `FaultyFs`를 쓴다.
//!
//! 사용: `use common::fsx::{leftover, set_mtime, sha256_hex};`

#![allow(dead_code)]

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 이름이 `prefix`로 시작하고 `suffix`로 끝나는 직속 항목 이름을 정렬해서 돌려준다.
/// "임시 파일이 남지 않았는가"를 이 함수 하나로 검사한다. 디렉터리가 없으면 빈 목록이다.
/// Python `sorted(p.name for p in dir.glob(f"{prefix}*{suffix}"))`.
pub fn leftover(dir: &Path, prefix: &str, suffix: &str) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with(prefix) && n.ends_with(suffix)).collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

/// 편집 중 만든 `.xxx.part` 임시 파일. `leftover(dir, "", ".part")`와 같다.
pub fn part_files(dir: &Path) -> Vec<String> {
    leftover(dir, "", ".part")
}

/// `leftover(..).len()`. Python `len(list(dir.glob("prefix*suffix")))`.
pub fn count_matching(dir: &Path, prefix: &str, suffix: &str) -> usize {
    leftover(dir, prefix, suffix).len()
}

/// 직속 항목 이름을 정렬해서 돌려준다. Python `sorted(p.name for p in dir.iterdir())`.
pub fn dir_names(dir: &Path) -> Vec<String> {
    leftover(dir, "", "")
}

/// 하위 디렉터리까지 모든 파일(정렬). Python `dir.rglob("*")` 중 파일만.
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

pub fn mkdir(path: impl AsRef<Path>) {
    std::fs::create_dir_all(path).unwrap();
}

/// 부모 디렉터리까지 만들고 파일을 쓴다.
pub fn write_file(path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// 빈 파일을 만든다(부모 포함). 이미 있으면 그대로 둔다. main_bulk_rename의 `touch(dir, names)`를 한 파일씩 부르는 형태다.
pub fn create_empty(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if !path.exists() {
        write_file(path, b"");
    }
    path.to_path_buf()
}

/// 접근·수정 시각을 `seconds_since_epoch`로 맞춘다. Python `os.utime(path, (t, t))`.
pub fn touch(path: &Path, seconds_since_epoch: u64) {
    set_times(path, UNIX_EPOCH + Duration::from_secs(seconds_since_epoch), UNIX_EPOCH + Duration::from_secs(seconds_since_epoch));
}

/// 수정 시각만 `t`로 맞춘다. Python `os.utime(path, (t, t))`의 `SystemTime` 형태.
pub fn set_mtime(path: &Path, t: SystemTime) {
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(t).unwrap();
}

/// 지금으로부터 `seconds_ago`초 전으로 mtime을 옮긴다.
pub fn set_mtime_ago(path: &Path, seconds_ago: u64) {
    set_mtime(path, SystemTime::now() - Duration::from_secs(seconds_ago));
}

/// 에포크 이후 초 단위로 mtime을 정한다.
pub fn set_mtime_secs(path: &Path, seconds_since_epoch: u64) {
    set_mtime(path, UNIX_EPOCH + Duration::from_secs(seconds_since_epoch));
}

/// 접근 시각과 수정 시각을 ns 단위로 정한다. Python `os.utime(path, ns=(atime, mtime))`.
pub fn set_times_ns(path: &Path, atime: u64, mtime: u64) {
    set_times(path, UNIX_EPOCH + Duration::from_nanos(atime), UNIX_EPOCH + Duration::from_nanos(mtime));
}

fn set_times(path: &Path, accessed: SystemTime, modified: SystemTime) {
    let times = std::fs::FileTimes::new().set_accessed(accessed).set_modified(modified);
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_times(times).unwrap();
}

/// (접근, 수정) 시각을 ns로 돌려준다.
pub fn times_ns(path: &Path) -> (u64, u64) {
    let m = std::fs::metadata(path).unwrap();
    ((m.atime() as u64) * 1_000_000_000 + m.atime_nsec() as u64, (m.mtime() as u64) * 1_000_000_000 + m.mtime_nsec() as u64)
}

/// 수정 시각의 ns 정수. Python `stat().st_mtime`을 `_page_count_cache` 키로 쓸 때.
pub fn mtime_key(path: &Path) -> i64 {
    let md = std::fs::metadata(path).unwrap();
    md.mtime() * 1_000_000_000 + md.mtime_nsec()
}

/// Python `(path.stat().st_ino, path.stat().st_mtime_ns)`.
pub fn ino_mtime(path: &Path) -> (u64, i128) {
    let meta = std::fs::metadata(path).unwrap();
    (meta.ino(), meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128)
}

/// Python `hashlib.sha256(data).hexdigest()`.
pub fn sha256_hex(data: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data.as_ref()).iter().map(|b| format!("{b:02x}")).collect()
}
