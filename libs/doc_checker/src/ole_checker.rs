//! OLE 복합 파일 공용 검사: 한도 안에서 열기와 구조 이상 보고. DOC와 HWP 검사기가 함께 쓴다.

use common::text_sanitizer::sanitize_display;
use container_parser::ole_parser::Anomaly;
use container_parser::ole_parser::Limits as OleLimits;
use container_parser::ole_parser::OleError;
use container_parser::ole_parser::OleFile;
use std::path::Path;

use crate::{Category, CheckOptions, FileReport, Severity};

/// 한 파일에서 보고하는 구조 이상 수의 상한.
const MAX_ANOMALY_FINDINGS: usize = 50;

/// OLE 파일을 한도 안에서 연다. 열지 못하면 사유를 미완료로 남기고 `None`을 돌려준다.
pub(crate) fn open_ole(path: &Path, opts: &CheckOptions, report: &mut FileReport) -> Option<OleFile> {
    let limits = OleLimits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..OleLimits::default() };
    match OleFile::open(path, &limits) {
        Ok(file) => Some(file),
        Err(OleError::Limit(message)) => {
            report.mark_incomplete(&format!("skipped: {message}"));
            None
        }
        Err(e) => {
            report.mark_incomplete(&format!("OLE container could not be read: {e:?}"));
            None
        }
    }
}

fn severity_of(a: &Anomaly) -> Severity {
    match a {
        Anomaly::FatChainCycle { .. }
        | Anomaly::SectorOutOfRange { .. }
        | Anomaly::FatSectorCountMismatch { .. }
        | Anomaly::DifatTruncated
        | Anomaly::DirectoryTruncated
        | Anomaly::DirectoryCycle
        | Anomaly::TooManyEntries(_)
        | Anomaly::StreamSizeExceedsFile { .. }
        | Anomaly::CrossLinkedSectors { .. } => Severity::High,
        _ => Severity::Medium,
    }
}

/// 파서를 속여 무한 루프·큰 할당·범위 밖 읽기를 유도하는 데 쓰는 이상인가.
fn abusable(a: &Anomaly) -> bool {
    matches!(
        a,
        Anomaly::FatChainCycle { .. }
            | Anomaly::SectorOutOfRange { .. }
            | Anomaly::DirectoryCycle
            | Anomaly::TooManyEntries(_)
            | Anomaly::StreamSizeExceedsFile { .. }
            | Anomaly::CrossLinkedSectors { .. }
    )
}

pub(crate) fn report_anomalies(file: &OleFile, report: &mut FileReport) {
    let anomalies = file.anomalies();
    for a in anomalies.iter().take(MAX_ANOMALY_FINDINGS) {
        report.push(Category::Defect, severity_of(a), a.code(), "", &sanitize_display(&a.detail(), 200, Some('…')));
    }
    if anomalies.len() > MAX_ANOMALY_FINDINGS {
        report.mark_incomplete(&format!("{} structure anomalies; only the first {MAX_ANOMALY_FINDINGS} are listed", anomalies.len()));
    }
    if anomalies.iter().any(abusable) {
        report.push_once(
            Category::Security,
            Severity::High,
            "OLE_MALFORMED_STRUCTURE",
            "",
            "OLE structure is malformed in ways that trigger parser bugs (loops, out-of-range sectors, shared sectors, size lies)",
        );
    }
}
