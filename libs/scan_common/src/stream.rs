//! 보고서를 창 단위로 검사하고 바로 내보낸 뒤 버린다.
//!
//! 모든 보고서를 모았다가 마지막에 내보내면 지표 수에 비례해 메모리가 늘어난다(지표 225만 개에서 455MB).
//! 창 단위로 쓰고 버리면 같은 입력에서 37MB로 일정하다.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::severity::Severity;

/// 검사 결과 한 건. 도구마다 다른 구조체를 쓰고, 여기서는 요약에 필요한 것만 묻는다.
pub trait Report: Send {
    /// 지표 각각의 심각도.
    fn severities(&self) -> impl Iterator<Item = Severity> + '_;
    /// 파일 전체를 검사하지 못했는가(읽기 실패, 한도 초과 등).
    fn is_incomplete(&self) -> bool;
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
