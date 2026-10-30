//! 파일 하나를 포맷별 검사기로 보낸다. 포맷은 확장자가 아니라 내용(매직바이트)을 우선한다.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use scan_common::sniff::Sniffed;
use scan_common::{FileReport, Format};

use crate::cli::Options;
use crate::{common, doc, html, hwp, recurse, epub, exe, pdf, txt, viewer, zipcheck};

/// 검사할 포맷을 정한다. `.pdf`·`.epub`은 내용이 PDF·ZIP이면 내용을 따르고(`.pdf`인데 ZIP이면 EPUB),
/// 그 밖에는 확장자를 따른다. 모르는 확장자는 `None`이다.
pub fn dispatch_format(path: &Path, sniffed: Sniffed) -> Option<Format> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if matches!(ext.as_str(), "pdf" | "epub") {
        match sniffed {
            Sniffed::Pdf => return Some(Format::Pdf),
            Sniffed::Zip => return Some(Format::Epub),
            _ => {}
        }
    }
    // ZIP 계열 확장자도 내용이 PDF면 PDF로 본다(위장). ZIP이면 확장자를 따른다.
    if is_zip_family(&ext) && sniffed == Sniffed::Pdf {
        return Some(Format::Pdf);
    }
    // 이름은 HTML인데 내용이 PDF·ZIP이면 내용을 따른다(확장자 불일치는 `common`이 알린다).
    if matches!(ext.as_str(), "html" | "htm" | "xhtml") {
        match sniffed {
            Sniffed::Pdf => return Some(Format::Pdf),
            Sniffed::Zip => return Some(Format::Zip),
            _ => {}
        }
    }
    // 이름은 `.hwp`인데 내용이 ZIP이면 HWPX를 이름만 바꾼 것이다.
    if ext == "hwp" && sniffed == Sniffed::Zip {
        return Some(Format::Hwpx);
    }
    // Word 계열 이름이나 `.rtf`인데 내용이 RTF면 RTF로 검사한다(Word는 이름과 무관하게 RTF로 연다).
    if sniffed == Sniffed::Rtf && matches!(ext.as_str(), "doc" | "dot" | "docx" | "docm" | "dotx" | "dotm" | "rtf") {
        return Some(Format::Doc);
    }
    // 이름은 `.doc`인데 내용이 ZIP이면 docx를 이름만 바꾼 것이다. 패키지로 검사한다(확장자 불일치는 `common`이 알린다).
    if matches!(ext.as_str(), "doc" | "dot") && sniffed == Sniffed::Zip {
        return Some(Format::Docx);
    }
    match ext.as_str() {
        "pdf" => Some(Format::Pdf),
        "epub" => Some(Format::Epub),
        "txt" => Some(Format::Txt),
        "zip" => Some(Format::Zip),
        "docx" | "docm" | "dotx" | "dotm" => Some(Format::Docx),
        "hwpx" => Some(Format::Hwpx),
        "doc" | "dot" | "rtf" => Some(Format::Doc),
        "hwp" => Some(Format::Hwp),
        "html" | "htm" | "xhtml" => Some(Format::Html),
        e if exe::is_exec_extension(e) => Some(Format::Executable),
        _ => None,
    }
}

/// 컨테이너 포맷(ZIP·OLE) 문서 확장자인가. 내용이 PDF면 PDF로 본다.
fn is_zip_family(ext: &str) -> bool {
    matches!(ext, "zip" | "docx" | "docm" | "dotx" | "dotm" | "hwpx" | "doc" | "dot" | "hwp")
}

fn scan_inner(path: &Path, opts: &Options, report: &mut FileReport) {
    let Some((_head, sniffed)) = common::inspect_with_sniff(path, opts, report) else { return };
    report.format = dispatch_format(path, sniffed);
    match report.format {
        Some(Format::Epub) => epub::scan_epub_file(path, opts, report),
        Some(Format::Pdf) => pdf::scan_pdf_file(path, opts, report),
        Some(Format::Txt) => txt::scan_txt_file(path, opts, report),
        Some(Format::Executable) => exe::scan_exe_file(sniffed, report),
        Some(Format::Zip) => zipcheck::scan_zip_file(path, opts, report),
        Some(Format::Docx) => zipcheck::scan_docx_file(path, opts, sniffed, report),
        Some(Format::Hwpx) => zipcheck::scan_hwpx_file(path, opts, sniffed, report),
        Some(Format::Doc) => doc::scan_doc_file(path, opts, sniffed, report),
        Some(Format::Hwp) => hwp::scan_hwp_file(path, opts, sniffed, report),
        Some(Format::Html) => html::scan_html_file(path, sniffed, report),
        None => report.mark_incomplete("unsupported format"),
    }
    // ZIP 안의 컨테이너(중첩 ZIP·PDF·RTF·Word·HWP)를 깊이·개수·바이트 상한 안에서 재귀 검사한다.
    if sniffed == Sniffed::Zip && matches!(report.format, Some(Format::Zip | Format::Docx | Format::Hwpx | Format::Epub)) {
        recurse::scan_zip_members(path, opts, report);
    }
}

/// 파일 하나를 검사한다. 포맷 모듈의 panic은 `incomplete`로 바꾸고, `--only`가 있으면 그 분류만 남긴다. 뷰어 차단 여부는 걸러내기 전에 채운다.
pub fn scan_file(path: &Path, opts: &Options) -> FileReport {
    // 최상위 검사면 재귀 예산을 만든다. 중첩 검사(재귀가 부른 `scan_file`)는 바깥 예산을 이어 쓴다.
    let _budget = recurse::begin(opts);
    let mut report = FileReport::new(path, None);
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| scan_inner(path, opts, &mut report))) {
        let msg = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
        report.mark_incomplete(&format!("panic: {msg}"));
    }
    // 조합 승격과 점수는 `--only`가 지표를 걸러내기 전의 전체 지표로 정한다.
    scan_common::risk::escalate(&mut report);
    report.risk_score = scan_common::risk::risk_score(&report.findings);
    // `--only`가 지표를 걸러내기 전에 채워서 남은 지표가 모두 값을 가진다.
    viewer::annotate(&mut report);
    if !opts.only.is_empty() {
        report.findings.retain(|f| opts.only.contains(&f.category));
    }
    report
}
