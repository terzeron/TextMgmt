//! 독립 HTML 검사. 규칙은 EPUB의 HTML 규칙(`epub::security`)을 그대로 쓴다.

use std::io::Read;
use std::path::Path;

use scan_common::FileReport;
use scan_common::sniff::Sniffed;

use crate::epub::security::{decode_text, scan_html_document};

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
    scan_html_document(&decode_text(&bytes).0, &name, report);
}
