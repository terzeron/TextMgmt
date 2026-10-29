//! EPUB 검사. 이후 task가 하위 모듈을 더한다.

use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;

/// EPUB 파일 하나를 검사해 `report`에 더한다.
pub fn scan_epub_file(_path: &Path, _opts: &Options, _report: &mut FileReport) {}
