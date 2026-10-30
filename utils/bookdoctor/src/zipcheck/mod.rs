//! ZIP으로 묶인 포맷(ZIP·DOCX·HWPX)의 검사. EPUB 검사(`epub::security`)와 같은 압축 해제 예산을 쓴다.
//! 모듈 이름을 `zip`이 아니라 `zipcheck`로 둔 것은 외부 crate `zip`을 가리지 않으려는 것이다.

pub mod fast;

use std::path::Path;

use scan_common::FileReport;

use crate::cli::Options;
use crate::epub::security::Limits;

/// `.zip` 파일 하나를 검사해 `report`에 더한다.
pub fn scan_zip_file(path: &Path, opts: &Options, report: &mut FileReport) {
    let limits = Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..Limits::default() };
    fast::check(path, &limits, opts.zip_deep, report);
}
