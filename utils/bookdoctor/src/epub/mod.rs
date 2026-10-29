//! EPUB 검사. 이후 task가 하위 모듈을 더한다.

pub mod security;

use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;

/// EPUB 파일 하나를 검사해 `report`에 더한다.
pub fn scan_epub_file(path: &Path, opts: &Options, report: &mut FileReport) {
    let limits = security::Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..security::Limits::default() };
    security::scan(path, &limits, report);
}
