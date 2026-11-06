//! 검사 실행: 입력 경로에서 대상 파일을 모으고, 보고서를 창 단위로 검사해 바로 내보낸다.

use crate::option_model::{BROKEN_DIR_NAME, Command, Options, TRASH_DIR_NAME};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::{FileReport, Severity};

// ---- 입력 경로에서 검사할 파일을 모은다. ----

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

// ---- 보고서를 창 단위로 검사하고 바로 내보낸 뒤 버린다. ----
// 모든 보고서를 모았다가 마지막에 내보내면 지표 수에 비례해 메모리가 늘어난다(지표 225만 개에서 455MB).
// 창 단위로 쓰고 버리면 같은 입력에서 37MB로 일정하다.

/// 검사 결과 한 건. 도구마다 다른 구조체를 쓰고, 여기서는 요약에 필요한 것만 묻는다.
pub trait Report: Send {
    /// 지표 각각의 심각도.
    fn severities(&self) -> impl Iterator<Item = Severity> + '_;
    /// 파일 전체를 검사하지 못했는가(읽기 실패, 한도 초과 등).
    fn is_incomplete(&self) -> bool;
}

/// 검사 보고서는 실행기가 세는 단위다.
impl Report for FileReport {
    fn severities(&self) -> impl Iterator<Item = Severity> + '_ {
        self.findings.iter().map(|f| f.severity)
    }

    fn is_incomplete(&self) -> bool {
        self.incomplete
    }
}

pub(crate) fn collect(opts: &Options, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut paths = collect_files(&opts.paths, true, keep);
    if opts.command != Command::Scan {
        paths.retain(|p| !in_reserved_dir(p, &opts.library_root));
    }
    paths
}

/// `path`가 `library_root` 바로 아래의 휴지통이나 격리 디렉터리 안에 있는가.
pub(crate) fn in_reserved_dir(path: &Path, library_root: &Path) -> bool {
    path.strip_prefix(library_root).ok().and_then(|rel| rel.components().next()).is_some_and(|first| first.as_os_str() == TRASH_DIR_NAME || first.as_os_str() == BROKEN_DIR_NAME)
}

/// 전체 실행의 요약.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    pub scanned: usize,
    pub has_findings: bool,
    pub has_incomplete: bool,
    pub by_severity: BTreeMap<&'static str, usize>,
}

impl Summary {
    /// 지표가 하나라도 있으면 1이다. 검사하지 못한 파일이 있어도 지표를 2 뒤에 숨기지 않는다.
    pub fn exit_code(&self) -> i32 {
        if self.has_findings {
            1
        } else if self.has_incomplete {
            2
        } else {
            0
        }
    }

    fn record(&mut self, report: &impl Report) {
        self.scanned += 1;
        self.has_incomplete |= report.is_incomplete();
        for severity in report.severities() {
            self.has_findings = true;
            *self.by_severity.entry(severity.label()).or_default() += 1;
        }
    }
}

/// `paths`를 `window`건씩 `pool`에서 병렬로 검사하고, 창이 끝나면 순서대로 `emit`한 뒤 보고서를 버린다.
/// `emit`이 실패하면 거기서 멈추고 오류를 돌려준다. 출력을 비우는 일(`flush`)은 `emit`이 맡는다.
/// 출력 객체를 `emit`과 따로 빌려 쓰는 두 번째 콜백을 두면 같은 객체를 두 번 가변으로 빌려야 해서 쓸 수 없다.
///
/// `window`는 지연과 처리량의 절충이다. 워커 수와 같으면 결과가 가장 빨리 나오지만 가장 느린 파일 하나를 기다리느라 놀고,
/// 크면 놀지 않지만 앞선 결과 출력이 같은 창의 느린 파일에 묶인다. 파일 하나가 큰 도구는 작게 잡는다.
pub fn scan_streaming<R, S, E>(paths: &[PathBuf], pool: &rayon::ThreadPool, window: usize, scan: S, mut emit: E) -> io::Result<Summary>
where
    R: Report,
    S: Fn(&Path) -> R + Sync,
    E: FnMut(&R) -> io::Result<()>,
{
    let mut summary = Summary::default();
    for chunk in paths.chunks(window.max(1)) {
        let reports: Vec<R> = pool.install(|| chunk.par_iter().map(|p| scan(p)).collect());
        for report in &reports {
            summary.record(report);
            emit(report)?;
        }
    }
    Ok(summary)
}

// ---- 병렬 풀과 워커 수. ----
/// /mnt/data는 회전 디스크라 워커를 늘려도 I/O로 막힌다. 2개가 실측상 가장 빨랐다.
pub const DEFAULT_WORKERS: usize = 2;

pub fn pool(workers: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(workers.max(1)).build().expect("thread pool")
}

/// 워커 수를 메모리로 제한한다. 워커마다 최악의 입력 하나(`per_worker_mb`)를 동시에 해석할 수 있으므로,
/// 전체가 `budget_mb`를 넘지 않게 하되 하나는 항상 돈다. 스레드별 malloc 아레나는 한 번 늘린 메모리를 쥐고 있어서
/// 큰 작업이 돌아가며 지나가도 최대 상주 메모리가 줄지 않으므로, 동시에 도는 수 자체를 줄여야 한다.
/// 엄밀한 한도가 아니라 보수적인 동시성 상한이다.
pub fn workers_within_memory(workers: usize, per_worker_mb: u64, budget_mb: u64) -> usize {
    let by_memory = (budget_mb / per_worker_mb.max(1)).max(1);
    workers.max(1).min(usize::try_from(by_memory).unwrap_or(usize::MAX))
}
