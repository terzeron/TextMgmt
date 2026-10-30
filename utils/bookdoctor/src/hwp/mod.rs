//! HWP 5.x(OLE 복합 파일) 검사. 포맷 사실은 `docs/bookdoctor-hwp5-notes.md`에 있다.
//! 파일을 실행하거나 렌더링하지 않는다.

use std::path::Path;

use ole_reader::OleFile;
use scan_common::FileReport;
use scan_common::sniff::Sniffed;

use crate::cli::Options;
use crate::doc::{open_ole, report_anomalies};

/// `.hwp` 하나를 검사한다. 내용이 OLE면 HWP 5.x, `HWP Document File V` 서명이면 HWP 3.x 이하다.
/// HWP 3.x는 OLE가 아니므로 서명만 확인한다(갭 분석에 적었다).
pub fn scan_hwp_file(path: &Path, opts: &Options, sniffed: Sniffed, report: &mut FileReport) {
    match sniffed {
        Sniffed::Ole => {
            if let Some(mut file) = open_ole(path, opts, report) {
                inspect(&mut file, report);
            }
        }
        Sniffed::Hwp3 => {}
        // 실행 파일은 `common`이 이미 Critical로 알렸다.
        Sniffed::Elf | Sniffed::MachO | Sniffed::Pe => {}
        // 이름만 HWP인 다른 내용은 여기서 분석하지 않는다. 이름 불일치는 `common`이 알린다.
        other => report.mark_incomplete(&format!("content is not an HWP document ({other:?}); not analyzed")),
    }
}

/// `.hwpx` 이름인데 내용이 OLE인 파일을 검사한다. `FileHeader`가 있으면 HWP 5.x를 이름만 바꾼 것이다.
/// 없으면 HWP인지 알 수 없으므로 일반 미완료로 남긴다(확장자 불일치는 `common`이 알린다).
pub fn scan_ole_named_hwpx(path: &Path, opts: &Options, report: &mut FileReport) {
    let Some(mut file) = open_ole(path, opts, report) else { return };
    if file.find("FileHeader").is_some() {
        inspect(&mut file, report);
    } else {
        report.mark_incomplete("OLE container cannot be inspected: .hwpx extension with OLE content that has no HWP FileHeader");
    }
}

/// 열린 HWP 5.x를 검사해 `report`에 더한다.
fn inspect(file: &mut OleFile, report: &mut FileReport) {
    report_anomalies(file, report);
}
