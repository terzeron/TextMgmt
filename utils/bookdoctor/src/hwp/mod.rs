//! HWP 5.x(OLE 복합 파일) 검사. 포맷 사실은 `docs/bookdoctor-hwp5-notes.md`에 있다.
//! 파일을 실행하거나 렌더링하지 않는다.

pub mod header;

use std::path::Path;

use ole_reader::OleFile;
use scan_common::{Category, FileReport, Severity};
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

/// `.hwpx` 이름인데 내용이 OLE인 파일을 검사한다. `FileHeader`가 있으면 HWP 5.x를 이름만 바꾼 것이다(확장자 불일치는 `common`이 알린다).
pub fn scan_ole_named_hwpx(path: &Path, opts: &Options, report: &mut FileReport) {
    let Some(mut file) = open_ole(path, opts, report) else { return };
    inspect(&mut file, report);
}

/// 열린 HWP 5.x를 검사해 `report`에 더한다.
fn inspect(file: &mut OleFile, report: &mut FileReport) {
    report_anomalies(file, report);
    let Some(entry) = file.find("FileHeader").cloned() else {
        report.push(Category::Defect, Severity::Medium, "FORMAT_MISMATCH", "FileHeader", "OLE 파일이지만 HWP 문서가 아니다(FileHeader 스트림이 없다)");
        // 이름만 HWP인 Word 문서나 암호화 OOXML이면 그쪽 분석기로 본다. 매크로를 놓치지 않기 위해서다.
        if file.find("WordDocument").is_some() || file.find("EncryptedPackage").is_some() {
            crate::doc::inspect(file, report);
        } else {
            report.mark_incomplete("OLE container cannot be inspected: no HWP FileHeader and not a Word document");
        }
        return;
    };
    let bytes = match file.read_stream_prefix(&entry, header::HEADER_LEN) {
        Ok(b) => b,
        Err(e) => {
            report.mark_incomplete(&format!("FileHeader could not be read: {e:?}"));
            return;
        }
    };
    let header = match header::parse(&bytes) {
        Ok(h) => h,
        Err(e) => {
            report.push(Category::Defect, Severity::Medium, "HWP_BAD_HEADER", "FileHeader", &e.detail());
            return;
        }
    };
    // 암호·DRM 문서는 본문과 BinData를 읽을 수 없다. 깨끗하다고 하지 않고 미완료로 남긴다.
    if let Some(reason) = header.encryption_reason() {
        report.mark_incomplete(reason);
        return;
    }
    // 배포용 문서는 본문이 `ViewText`에 따로 저장된다. 이 명세에는 그 구조가 없어 읽지 않는다.
    if header.distributable() && file.find("BodyText").is_none() && file.find("ViewText").is_some() {
        report.mark_incomplete("distribution document: ViewText not analyzed");
    }
}
