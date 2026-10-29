//! PDF 검사. 이후 task가 하위 모듈을 더한다.

pub mod security;

use std::fs;
use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;

/// PDF 파일 하나를 검사해 `report`에 더한다. 크기 제한은 `common::inspect`가 이미 확인했다.
pub fn scan_pdf_file(path: &Path, _opts: &Options, report: &mut FileReport) {
    match fs::read(path) {
        Ok(data) => security::scan(&data, report),
        Err(e) => report.mark_incomplete(&format!("unreadable: {e}")),
    }
}
