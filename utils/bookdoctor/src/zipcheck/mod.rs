//! ZIP으로 묶인 포맷(ZIP·DOCX·HWPX)의 검사. EPUB 검사(`epub::security`)와 같은 압축 해제 예산을 쓴다.
//! 모듈 이름을 `zip`이 아니라 `zipcheck`로 둔 것은 외부 crate `zip`을 가리지 않으려는 것이다.

pub mod fast;
pub mod ooxml;

use std::path::Path;

use scan_common::FileReport;
use scan_common::sniff::Sniffed;

use crate::cli::Options;
use crate::epub::security::Limits;

/// `.zip` 파일 하나를 검사해 `report`에 더한다.
pub fn scan_zip_file(path: &Path, opts: &Options, report: &mut FileReport) {
    let limits = Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..Limits::default() };
    fast::check(path, &limits, opts.zip_deep, report);
}

/// DOCX 계열(`.docx`·`.docm`·`.dotx`·`.dotm`) 하나를 검사한다. 내용이 OLE면 ZIP이 아니므로 열지 않고 미완료로 남긴다.
/// 구형 `.doc`인지 암호화된 OOXML인지는 CFB를 읽어야 알 수 있다. 확인하지 못한 것을 암호화라고 하지 않는다.
pub fn scan_docx_file(path: &Path, opts: &Options, sniffed: Sniffed, report: &mut FileReport) {
    if sniffed == Sniffed::Ole {
        report.mark_incomplete("OLE container cannot be inspected yet: .docx extension with OLE content (legacy .doc or encrypted OOXML)");
        return;
    }
    let limits = Limits { max_total: opts.max_total_mb.saturating_mul(1024 * 1024), ..Limits::default() };
    if fast::check(path, &limits, opts.zip_deep, report) {
        ooxml::inspect_structure(path, &limits, report);
    }
}
