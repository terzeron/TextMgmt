//! EPUB 검사. 이후 task가 하위 모듈을 더한다.

pub mod security;
pub mod structure;

use std::collections::HashSet;
use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;

/// EPUB 파일 하나를 검사해 `report`에 더한다.
pub fn scan_epub_file(path: &Path, opts: &Options, report: &mut FileReport) {
    let limits = security::Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..security::Limits::default() };
    structure::inspect(path, report);
    let structure_len = report.findings.len();
    let structure_codes: HashSet<String> = report.findings.iter().map(|f| f.code.clone()).collect();
    security::scan(path, &limits, report);
    // 같은 코드(옛 BAD_MIMETYPE처럼 두 검사에 모두 있는 것)는 구조 검사 쪽 하나만 남긴다.
    let mut index = 0;
    report.findings.retain(|f| {
        index += 1;
        index <= structure_len || !structure_codes.contains(&f.code)
    });
}
