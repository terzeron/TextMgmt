//! 코퍼스를 다루는 도구들이 함께 쓰는 공용 코드.
//!
//! - [`reader`]: 텍스트·EPUB 본문 표본 읽기, 인코딩 판별, ZIP 열기(`open_zip`)
//! - [`util`]: Python `posixpath`/`urllib` 호환 경로·URL 도구, 카테고리 판정
//! - 경로 목록, 병렬 풀, 리포트 경로 같은 작은 도구들
//!
//! 원래 `detect_mojibake`에 있던 것을 라이브러리 계층으로 옮겼다. 도구 crate끼리 서로 의존하지 않게 하려는 것이다.

pub mod reader;
pub mod util;

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use pyfmt::py_round;
use regex::Regex;

pub const DEFAULT_LIBRARY_ROOT: &str = "/mnt/data/text";

const TEXT_EXTS: [&str; 2] = ["txt", "epub"];

/// /mnt/data는 회전 디스크라 워커를 늘려도 I/O로 막힌다. 2개가 실측상 가장 빨랐다.
pub const DEFAULT_WORKERS: usize = 2;

/// 로그 한 줄을 받는 쪽. 메시지 앞에 `INFO`/`ERROR` 수준이 붙어 있다.
pub type Log<'a> = &'a (dyn Fn(&str) + Sync);

pub fn round1(x: f64) -> f64 {
    py_round(x, 1)
}

static SLUG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w.-]+").unwrap());

/// 대상 디렉토리마다 다른 리포트 경로. 다른 디렉토리를 스캔해도 이전 리포트를 덮어쓰지 않는다.
///
/// 라이브러리 하위는 상대 경로(2_소설외국, 하위는 `__`로 잇는다), 라이브러리 루트는 all,
/// 라이브러리 밖은 디렉토리 이름을 쓴다.
pub fn default_report_path(kind: &str, root: &Path, repo_root: &Path) -> PathBuf {
    let abs = if root.is_absolute() { root.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(root) };
    let root = PathBuf::from(util::posix_normpath(&abs.to_string_lossy()));
    let parts: Vec<String> = match root.strip_prefix(DEFAULT_LIBRARY_ROOT) {
        Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect(),
        Err(_) => vec![root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()],
    };
    let slug = parts.iter().map(|p| SLUG_RE.replace_all(p, "_").into_owned()).collect::<Vec<_>>().join("__");
    let slug = if slug.is_empty() { "all".to_owned() } else { slug };
    repo_root.join("tmp").join(format!("{kind}_report_{slug}.json"))
}

/// 라이브러리 전체의 txt/epub 목록. 도구 캐시 디렉토리는 뺀다.
pub fn list_library_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
            // os.walk처럼 심볼릭 링크가 디렉토리를 가리키면 디렉토리로 보되 따라 들어가지 않는다.
            let is_dir = if is_link { std::fs::metadata(&path).is_ok_and(|m| m.is_dir()) } else { entry.file_type().is_ok_and(|t| t.is_dir()) };
            if is_dir {
                if !is_link && !name.starts_with('.') && name != "__pycache__" {
                    walk(&path, out);
                }
            } else if !name.starts_with('.') && path.extension().is_some_and(|e| TEXT_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str())) {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    files
}

pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn pool(workers: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(workers.max(1)).build().expect("thread pool")
}

pub fn limited(mut files: Vec<PathBuf>, limit: usize) -> Vec<PathBuf> {
    if limit > 0 {
        files.truncate(limit);
    }
    files
}
