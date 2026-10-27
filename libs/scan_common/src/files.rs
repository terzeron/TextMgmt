//! 입력 경로에서 검사할 파일을 모은다.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// 일반 파일이고 확장자가 `ext`(대소문자 무시)인가.
pub fn has_extension(path: &Path, ext: &str) -> bool {
    path.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// 디렉토리는 정렬된 순서로 훑는다. `recursive`가 아니면 1단계만 보고, 심볼릭 링크 디렉토리는 따라가지 않는다.
/// 읽을 수 없는 디렉토리는 조용히 건너뛰지 않고 경로째 남긴다. 검사기가 `INCOMPLETE`로 보고하게 하려는 것이다.
fn walk(dir: &Path, recursive: bool, wanted: &impl Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        out.push(dir.to_path_buf());
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() && !fs::symlink_metadata(&p).is_ok_and(|m| m.is_symlink()) {
            if recursive {
                walk(&p, recursive, wanted, out);
            }
        } else if wanted(&p) {
            out.push(p);
        }
    }
}

/// 입력 경로들에서 대상 파일을 모은다. 중복은 처음 것만 남긴다.
///
/// - 디렉토리: [`walk`]처럼 훑는다(맨 위 디렉토리가 링크여도 따라간다. 그 아래의 링크 디렉토리는 따라가지 않는다).
/// - 그 밖(파일, 없는 경로, 읽을 수 없는 경로): 그대로 남긴다. 건너뛰면 다른 파일이 깨끗할 때 "정상"으로 끝나서
///   검사하지 못한 사실이 사라진다. 검사기가 `INCOMPLETE`(종료 코드 2)로 보고한다.
pub fn collect_files(paths: &[PathBuf], recursive: bool, wanted: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for p in paths {
        if p.is_dir() {
            walk(p, recursive, &wanted, &mut found);
        } else {
            found.push(p.clone());
        }
    }
    let mut seen = HashSet::new();
    found.retain(|p| seen.insert(p.clone()));
    found
}
