//! ZIP으로 묶인 포맷(ZIP·DOCX·HWPX)의 검사. EPUB 검사(`epub::security`)와 같은 압축 해제 예산을 쓴다.
//! 모듈 이름을 `zip`이 아니라 `zipcheck`로 둔 것은 외부 crate `zip`을 가리지 않으려는 것이다.

pub mod fast_zip_checker;
pub mod hwpx_checker;
pub mod ooxml_checker;

use std::path::Path;

use crate::FileReport;

use crate::CheckOptions;
use crate::zip_member_reader::Limits;

/// `.zip` 파일 하나를 검사해 `report`에 더한다.
pub fn scan_zip_file(path: &Path, opts: &CheckOptions, report: &mut FileReport) {
    let limits = Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..Limits::default() };
    fast_zip_checker::check(path, &limits, opts.zip_deep, report);
}

/// ZIP 내용인 DOCX 계열(`.docx`·`.docm`·`.dotx`·`.dotm`) 하나를 검사한다. 내용이 OLE인 파일은 `format_dispatcher`가 DOC 검사기로 보낸다.
pub fn scan_docx_file(path: &Path, opts: &CheckOptions, report: &mut FileReport) {
    let limits = Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..Limits::default() };
    if fast_zip_checker::check(path, &limits, opts.zip_deep, report) {
        // 구조와 보안 검사가 같은 압축 해제 예산을 쓴다.
        let mut spent = 0u64;
        ooxml_checker::inspect_structure(path, &limits, &mut spent, report);
        // 매크로를 허용하는 형식(.docm·.dotm)인지에 따라 `vbaProject.bin`의 심각도가 갈린다.
        let macro_enabled = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("docm") || e.eq_ignore_ascii_case("dotm"));
        ooxml_checker::inspect_security(path, &limits, macro_enabled, &mut spent, report);
    }
}

/// ZIP 내용인 `.hwpx` 하나를 검사한다. 내용이 OLE인 파일(HWP 5.x를 이름만 바꾼 것)은 `format_dispatcher`가 HWP 검사기로 보낸다.
pub fn scan_hwpx_file(path: &Path, opts: &CheckOptions, report: &mut FileReport) {
    let limits = Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..Limits::default() };
    if fast_zip_checker::check(path, &limits, opts.zip_deep, report) {
        hwpx_checker::inspect(path, &limits, report);
    }
}
