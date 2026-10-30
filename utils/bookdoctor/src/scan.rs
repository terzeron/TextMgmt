//! 파일 하나를 포맷별 검사기로 보낸다. 포맷은 확장자가 아니라 내용(매직바이트)을 우선한다.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use scan_common::sniff::Sniffed;
use scan_common::{FileReport, Format};

use crate::cli::Options;
use crate::{common, epub, exe, pdf, txt, viewer};

/// 검사할 포맷을 정한다. `.pdf`·`.epub`은 내용이 PDF·ZIP이면 내용을 따르고(`.pdf`인데 ZIP이면 EPUB),
/// 그 밖에는 확장자를 따른다. 모르는 확장자는 `None`이다.
pub fn dispatch_format(path: &Path, sniffed: Sniffed) -> Option<Format> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if matches!(ext.as_str(), "pdf" | "epub") {
        match sniffed {
            Sniffed::Pdf => return Some(Format::Pdf),
            Sniffed::Zip => return Some(Format::Epub),
            _ => {}
        }
    }
    match ext.as_str() {
        "pdf" => Some(Format::Pdf),
        "epub" => Some(Format::Epub),
        "txt" => Some(Format::Txt),
        e if exe::is_exec_extension(e) => Some(Format::Executable),
        _ => None,
    }
}

fn scan_inner(path: &Path, opts: &Options, report: &mut FileReport) {
    let Some((_head, sniffed)) = common::inspect_with_sniff(path, opts, report) else { return };
    report.format = dispatch_format(path, sniffed);
    match report.format {
        Some(Format::Epub) => epub::scan_epub_file(path, opts, report),
        Some(Format::Pdf) => pdf::scan_pdf_file(path, opts, report),
        Some(Format::Txt) => txt::scan_txt_file(path, opts, report),
        Some(Format::Executable) => exe::scan_exe_file(sniffed, report),
        None => report.mark_incomplete("unsupported format"),
    }
}

/// 파일 하나를 검사한다. 포맷 모듈의 panic은 `incomplete`로 바꾸고, `--only`가 있으면 그 분류만 남긴다. 뷰어 차단 여부는 걸러내기 전에 채운다.
pub fn scan_file(path: &Path, opts: &Options) -> FileReport {
    let mut report = FileReport::new(path, None);
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| scan_inner(path, opts, &mut report))) {
        let msg = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
        report.mark_incomplete(&format!("panic: {msg}"));
    }
    // 조합 승격과 점수는 `--only`가 지표를 걸러내기 전의 전체 지표로 정한다.
    scan_common::risk::escalate(&mut report);
    report.risk_score = scan_common::risk::risk_score(&report.findings);
    // `--only`가 지표를 걸러내기 전에 채워서 남은 지표가 모두 값을 가진다.
    viewer::annotate(&mut report);
    if !opts.only.is_empty() {
        report.findings.retain(|f| opts.only.contains(&f.category));
    }
    report
}
