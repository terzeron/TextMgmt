//! TXT 검사. 이후 task가 채운다.

use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;

/// TXT 파일 하나를 검사해 `report`에 더한다.
pub fn scan_txt_file(_path: &Path, _opts: &Options, _report: &mut FileReport) {}
