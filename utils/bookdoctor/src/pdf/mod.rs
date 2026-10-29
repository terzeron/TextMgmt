//! PDF 검사: 보안(`security`)과 구조·렌더링(`structure`).

pub mod security;
pub mod structure;

use std::fs::File;
use std::io::Read;
use std::path::Path;

use scan_common::FileReport;
use sysdeps::{CommandRunner, SystemRunner};

use crate::cli::Options;

/// PDF 파일 하나를 검사해 `report`에 더한다. 크기 제한은 `common::inspect`가 이미 확인했다.
pub fn scan_pdf_file(path: &Path, opts: &Options, report: &mut FileReport) {
    scan_pdf_file_with(path, opts, report, &SystemRunner);
}

/// `scan_pdf_file`에 Poppler 실행기를 주입한다. 테스트는 가짜 실행기를 넘긴다.
pub fn scan_pdf_file_with(path: &Path, opts: &Options, report: &mut FileReport, runner: &dyn CommandRunner) {
    // 크기 확인(`common::inspect`)과 읽기 사이에 파일이 자랄 수 있으므로 한도+1바이트까지만 읽는다.
    let limit = opts.max_total_mb.saturating_mul(1024 * 1024);
    let mut data = Vec::new();
    let read = File::open(path).and_then(|f| f.take(limit.saturating_add(1)).read_to_end(&mut data));
    if let Err(e) = read {
        report.mark_incomplete(&format!("unreadable: {e}"));
        return;
    }
    if data.len() as u64 > limit {
        report.mark_incomplete("skipped: file grew past --max-total-mb while reading");
        return;
    }
    if !security::has_header(&data) {
        // 구문 분석을 시도하지 않으므로 보안 검사도 하지 못했다. 그 사실은 incomplete로 남긴다.
        report.mark_incomplete("no %PDF- header");
        structure::inspect_with(&data, None, None, path, opts, report, runner);
        return;
    }
    let (doc, error) = match security::load(&data) {
        Ok(doc) => (Some(doc), None),
        Err(e) => (None, Some(e)),
    };
    security::scan_loaded(&data, doc.as_ref(), report);
    structure::inspect_with(&data, doc.as_ref(), error.as_deref(), path, opts, report, runner);
}
