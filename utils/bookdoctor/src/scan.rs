//! 파일 하나를 포맷별 검사기로 보낸다. 포맷은 확장자가 아니라 내용(매직바이트)을 우선한다.
//! 다른 모듈로 나눌 만큼 크지 않은 실행 파일·독립 HTML 검사기도 여기 둔다.

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use crate::cli::Options;
use crate::epub::security::scan_html_document;
use crate::sniff::Sniffed;
use crate::{Category, FileReport, Format, Severity};
use crate::{common, doc, epub, hwp, pdf, recurse, rules, txt, viewer, zipcheck};

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
        e if is_exec_extension(e) => Some(Format::Executable),
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
        Some(Format::Executable) => scan_exe_file(sniffed, report),
        Some(Format::Zip) => zipcheck::scan_zip_file(path, opts, report),
        Some(Format::Docx) => zipcheck::scan_docx_file(path, opts, sniffed, report),
        Some(Format::Hwpx) => zipcheck::scan_hwpx_file(path, opts, sniffed, report),
        Some(Format::Doc) => doc::scan_doc_file(path, opts, sniffed, report),
        Some(Format::Hwp) => hwp::scan_hwp_file(path, opts, sniffed, report),
        Some(Format::Html) => scan_html_file(path, sniffed, report),
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
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
        scan_inner(path, opts, &mut report);
        // 규칙은 분석기가 끝난 뒤 파일의 원시 바이트에 적용한다(꺼져 있으면 비용이 없다). 재귀가 부른 중첩 파일에도 같은 옵션으로 적용된다.
        if let Some(set) = &opts.ruleset {
            rules::scan_report(path, set, &mut report);
        }
    })) {
        let msg = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_default();
        report.mark_incomplete(&format!("panic: {msg}"));
    }
    // 조합 승격과 점수는 전체 지표로 정한다.
    crate::finding::escalate(&mut report);
    report.risk_score = crate::finding::risk_score(&report.findings);
    viewer::annotate(&mut report);
    report
}

// ---- 실행 파일 이중 확인. 확장자가 실행 파일이고 매직바이트도 실행 파일(ELF·Mach-O·PE)이어야 "확정"이다. ----
// 확장자만 맞고 내용이 다르면 확정하지 않고 낮은 심각도의 결함으로 남긴다.
/// 실행 파일로 보는 확장자. 디렉토리를 훑을 때 이 확장자도 모은다.
pub const EXEC_EXTENSIONS: &[&str] = &["exe", "dll", "sys", "scr", "ocx", "cpl", "drv", "efi", "so", "elf", "ko", "dylib", "bundle", "bin", "out"];

/// 실행 파일 확장자인가(대소문자 무시).
pub fn is_exec_extension(ext: &str) -> bool {
    EXEC_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

fn kind(sniffed: Sniffed) -> Option<&'static str> {
    match sniffed {
        Sniffed::Elf => Some("ELF"),
        Sniffed::MachO => Some("Mach-O"),
        Sniffed::Pe => Some("PE"),
        _ => None,
    }
}

/// 실행 파일 확장자인 파일을 검사한다. `sniffed`는 `common::inspect_with_sniff`가 정한 매직바이트 판별이다.
pub fn scan_exe_file(sniffed: Sniffed, report: &mut FileReport) {
    match kind(sniffed) {
        Some(k) => {
            report.confirmed_executable = true;
            report.push(Category::Security, Severity::High, "EXECUTABLE_FILE", "", &format!("실행 파일 확정: {k}"));
        }
        None => report.push(Category::Defect, Severity::Low, "EXECUTABLE_EXTENSION_NO_MAGIC", "", &format!("확장자는 실행 파일인데 매직바이트가 실행 파일이 아니다({sniffed:?})")),
    }
}

// ---- 독립 HTML 검사. 규칙은 EPUB의 HTML 규칙(`epub::security`)을 그대로 쓴다. ----
/// 읽는 최대 바이트. 넘으면 앞부분만 보고 미완료로 남긴다.
const MAX_HTML: u64 = 16 * 1024 * 1024;

/// `.html`·`.htm`·`.xhtml` 하나를 검사한다. 텍스트는 시작이 `<!DOCTYPE`이 아니어도 받는다(저장한 웹 페이지는 주석이나 `<meta>`로 시작하는 일이 흔하다).
pub fn scan_html_file(path: &Path, sniffed: Sniffed, report: &mut FileReport) {
    match sniffed {
        // 실행 파일은 `common`이 이미 Critical로 알렸다.
        Sniffed::Elf | Sniffed::MachO | Sniffed::Pe => return,
        // PDF·ZIP은 내용에 맞는 분석기로 넘어가므로 여기 오지 않는다. 그 밖의 바이너리는 분석하지 못한다.
        Sniffed::Ole | Sniffed::Rtf | Sniffed::Hwp3 | Sniffed::Pdf | Sniffed::Zip => {
            report.mark_incomplete(&format!("content is not HTML ({sniffed:?}); not analyzed"));
            return;
        }
        Sniffed::Html | Sniffed::Unknown => {}
    }
    let Ok(file) = std::fs::File::open(path) else {
        report.mark_incomplete("HTML file could not be opened");
        return;
    };
    let mut bytes = Vec::new();
    if file.take(MAX_HTML + 1).read_to_end(&mut bytes).is_err() {
        report.mark_incomplete("HTML file could not be read");
        return;
    }
    if bytes.len() as u64 > MAX_HTML {
        bytes.truncate(MAX_HTML as usize);
        report.mark_incomplete(&format!("skipped: HTML larger than the {} MiB read limit; only the first part was inspected", MAX_HTML / (1024 * 1024)));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    scan_html_document(&bytes, &name, report);
}
