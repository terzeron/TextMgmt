//! 경로·URL 유틸. Python `posixpath`와 `urllib.parse`의 동작에 맞춘다.
//! EPUB 구조 검사(`bookdoctor`)와 분류 도구가 함께 쓴다.

use std::fs;
use std::path::{Path, PathBuf};

pub fn posix_normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let initial = if path.starts_with("//") && !path.starts_with("///") { 2 } else { usize::from(path.starts_with('/')) };
    let mut comps: Vec<&str> = Vec::new();
    for c in path.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(c);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let joined = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if joined.is_empty() { ".".into() } else { joined }
}
pub fn posix_join(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        b.to_owned()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}
/// `posixpath.dirname`: 마지막 '/'까지에서 끝의 '/'를 걷어낸다. 루트 슬래시만 남으면 유지한다.
pub fn posix_dirname(p: &str) -> String {
    let Some(i) = p.rfind('/') else { return String::new() };
    let head = &p[..=i];
    let trimmed = head.trim_end_matches('/');
    if trimmed.is_empty() { head.to_owned() } else { trimmed.to_owned() }
}
/// `urllib.parse.unquote`: 퍼센트 인코딩을 풀고 잘못된 UTF-8은 U+FFFD로 바꾼다.
pub fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}
/// 분류 경로(`DEFAULT_LIBRARY_ROOT` 아래 첫 구간). 밖이면 빈 문자열.
pub fn category_of(path: &Path) -> String {
    let root = Path::new(crate::DEFAULT_LIBRARY_ROOT);
    if !path.to_string_lossy().starts_with(crate::DEFAULT_LIBRARY_ROOT) {
        return String::new();
    }
    path.strip_prefix(root).ok().and_then(|p| p.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default()
}

/// `Path.resolve(strict=False)`: 있는 만큼은 심볼릭 링크를 풀고, 없는 꼬리는 그대로 붙인다.
pub fn resolve_lenient(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let normalized = PathBuf::from(posix_normpath(&abs.to_string_lossy()));
    let mut head = normalized.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    // 정규화한 절대 경로는 `/`까지 올라가면 반드시 존재하므로 file_name이 없는 경우는 오지 않는다.
    while !head.exists() {
        let Some(name) = head.file_name().map(|n| n.to_owned()) else { break };
        tail.push(name);
        head.pop();
    }
    let mut resolved = head.canonicalize().unwrap_or(head);
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// 파일 이동·삭제 뒤 비어 있는 상위 서브디렉토리를 안전하게 재귀 청소한다.
///
/// `stop_dir` 자신과 그 바깥은 절대 지우지 않는다. 두 경로를 정규화하지 않고 비교하면 `..`가 낀
/// 표기로는 `stop_dir`를 알아보지 못해 조상 디렉토리까지 올라간다. 비었는지 판정도 rmdir과
/// 같은 기준(숨김 파일 포함)으로 본다.
pub fn clean_empty_parent_dirs(parent_dir: &Path, stop_dir: &Path) {
    let mut curr = resolve_lenient(parent_dir);
    let stop = resolve_lenient(stop_dir);
    while curr != stop && curr.starts_with(&stop) && curr.is_dir() {
        // rmdir과 같은 기준: 숨김 파일을 포함해 항목이 하나라도 있으면 멈춘다.
        let empty = fs::read_dir(&curr).is_ok_and(|mut it| it.next().is_none());
        if !empty || fs::remove_dir(&curr).is_err() {
            break;
        }
        curr.pop();
    }
}
